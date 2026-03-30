use dicom_core::{dicom_value, DataDictionary, DataElement, PrimitiveValue, Tag, VR};
use dicom_core::header::Header;
use dicom_dictionary_std::{tags, uids};
use dicom_encoding::TransferSyntaxIndex;
use dicom_object::mem::InMemDicomObject;
use dicom_transfer_syntax_registry::entries;
use dicom_ul::{
    association::client::AsyncClientAssociation,
    pdu::{PDataValue, PDataValueType, Pdu},
};
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use snafu::{ResultExt, Whatever};
use std::collections::HashMap;
use std::io::Read;
use std::time::Instant;
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

use super::{
    Error, FindCallbacks, FindCompletedData, FindCompletedEvent, FindResult, FindResultEvent,
    FindScuArgs, QueryModel,
};

/// Build C-FIND request command
fn find_req_command(abstract_syntax: &str, message_id: u16) -> InMemDicomObject {
    InMemDicomObject::command_from_element_iter([
        // SOP Class UID
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from(abstract_syntax),
        ),
        // command field: C-FIND-RQ
        DataElement::new(tags::COMMAND_FIELD, VR::US, dicom_value!(U16, [0x0020])),
        // message ID
        DataElement::new(
            tags::MESSAGE_ID,
            VR::US,
            dicom_value!(U16, [message_id]),
        ),
        // priority: medium
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [0x0000])),
        // data set type: not empty
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [0x0001]),
        ),
    ])
}

/// Parse query parameters and build DICOM query object
fn build_query_object(
    query_params: &HashMap<String, String>,
    query_model: &QueryModel,
) -> Result<InMemDicomObject, Error> {
    let mut obj = InMemDicomObject::new_empty();

    // Add query parameters
    for (key, value) in query_params {
        let tag = parse_tag(key)?;
        let vr = infer_vr(tag);
        
        let primitive_value = if value.is_empty() {
            PrimitiveValue::Empty
        } else {
            PrimitiveValue::from(value.as_str())
        };

        obj.put(DataElement::new(tag, vr, primitive_value));
    }

    // Add QueryRetrieveLevel if not specified and not using modality worklist
    if *query_model != QueryModel::ModalityWorklist
        && obj.get(tags::QUERY_RETRIEVE_LEVEL).is_none()
    {
        let level = match query_model {
            QueryModel::PatientRoot => "PATIENT",
            QueryModel::StudyRoot => "STUDY",
            QueryModel::ModalityWorklist => unreachable!(),
        };

        obj.put(DataElement::new(
            tags::QUERY_RETRIEVE_LEVEL,
            VR::CS,
            PrimitiveValue::from(level),
        ));
    }

    Ok(obj)
}

/// Parse tag from string (supports tag names and hex format)
fn parse_tag(key: &str) -> Result<Tag, Error> {
    // Try parsing as hex first (e.g., "00100010" or "(0010,0010)")
    if let Some(tag) = try_parse_hex_tag(key) {
        return Ok(tag);
    }

    // Try parsing as tag name using dictionary
    dicom_dictionary_std::StandardDataDictionary
        .by_name(key)
        .map(|entry| entry.tag.inner())
        .ok_or_else(|| Error::Other {
            message: format!("Unknown DICOM tag: {}", key),
            source: None,
        })
}

/// Try to parse tag from hex format
fn try_parse_hex_tag(s: &str) -> Option<Tag> {
    let s = s.trim();
    
    // Remove parentheses and comma if present: "(0010,0010)" -> "00100010"
    let s = s.trim_start_matches('(').trim_end_matches(')').replace(",", "");
    
    // Must be exactly 8 hex digits
    if s.len() != 8 {
        return None;
    }

    let group = u16::from_str_radix(&s[0..4], 16).ok()?;
    let element = u16::from_str_radix(&s[4..8], 16).ok()?;
    
    Some(Tag(group, element))
}

/// Infer VR for a tag
fn infer_vr(tag: Tag) -> VR {
    dicom_dictionary_std::StandardDataDictionary
        .by_tag(tag)
        .and_then(|e| e.vr.exact())
        .unwrap_or(VR::LO)
}

/// Extract attributes from DICOM object into HashMap
fn extract_attributes(obj: &InMemDicomObject) -> HashMap<String, String> {
    let mut attributes = HashMap::new();

    for elem in obj.iter() {
        let tag = elem.tag();
        
        // Get tag name from dictionary
        let tag_name = dicom_dictionary_std::StandardDataDictionary
            .by_tag(tag)
            .map(|e| e.alias)
            .unwrap_or("Unknown");

        // Convert value to string
        if let Ok(value) = elem.to_str() {
            attributes.insert(tag_name.to_string(), value.to_string());
        }
    }

    attributes
}

/// Main async function to execute C-FIND
pub async fn run_find(
    args: FindScuArgs,
    callbacks: FindCallbacks,
) -> Result<Vec<FindResult>, Error> {
    let start_time = Instant::now();
    let FindScuArgs {
        addr,
        calling_ae_title,
        called_ae_title,
        max_pdu_length,
        verbose,
        query_model,
        query_params,
    } = args;

    // Parse address
    let (called_ae, socket_addr) = parse_address(&addr)?;
    let called_ae = called_ae_title.unwrap_or(called_ae);

    // Determine abstract syntax based on query model
    let abstract_syntax = match query_model {
        QueryModel::PatientRoot => uids::PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
        QueryModel::StudyRoot => uids::STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
        QueryModel::ModalityWorklist => uids::MODALITY_WORKLIST_INFORMATION_MODEL_FIND,
    };

    if verbose {
        info!("Establishing association with '{}'...", socket_addr);
    }

    // Build query object
    let query_obj = build_query_object(&query_params, &query_model)?;

    if verbose {
        info!("Query model: {:?}", query_model);
        debug!("Query parameters: {:?}", query_params);
    }

    // Establish association
    let mut scu_init = dicom_ul::ClientAssociationOptions::new()
        .with_abstract_syntax(abstract_syntax)
        .calling_ae_title(calling_ae_title)
        .called_ae_title(called_ae)
        .max_pdu_length(max_pdu_length);

    let mut scu = scu_init
        .establish_with_async(&socket_addr)
        .await
        .context(super::InitScuSnafu)?;

    if verbose {
        info!("Association established");
    }

    // Get presentation context
    let pc_selected = scu
        .presentation_contexts()
        .first()
        .ok_or_else(|| Error::Other {
            message: "No presentation context available".to_string(),
            source: None,
        })?;

    let pc_id = pc_selected.id;
    let ts = dicom_transfer_syntax_registry::TransferSyntaxRegistry
        .get(&pc_selected.transfer_syntax)
        .ok_or_else(|| Error::Other {
            message: "Unsupported transfer syntax".to_string(),
            source: None,
        })?;

    if verbose {
        debug!("Using presentation context ID: {}", pc_id);
        debug!("Transfer syntax: {}", ts.name());
    }

    // Build and send C-FIND request
    let cmd = find_req_command(abstract_syntax, 1);
    let mut cmd_data = Vec::with_capacity(128);
    cmd.write_dataset_with_ts(&mut cmd_data, &entries::IMPLICIT_VR_LITTLE_ENDIAN.erased())
        .context(super::CreateCommandSnafu)?;

    let mut query_data = Vec::with_capacity(512);
    query_obj
        .write_dataset_with_ts(&mut query_data, ts)
        .context(super::CreateCommandSnafu)?;

    if verbose {
        debug!(
            "Sending C-FIND request ({} + {} bytes)...",
            cmd_data.len(),
            query_data.len()
        );
    }

    // Send command
    scu.send(&Pdu::PData {
        data: vec![PDataValue {
            presentation_context_id: pc_id,
            value_type: PDataValueType::Command,
            is_last: true,
            data: cmd_data,
        }],
    })
    .await
    .map_err(|e| Error::Other {
        message: format!("Failed to send command: {}", e),
        source: None,
    })?;

    // Send query dataset
    scu.send(&Pdu::PData {
        data: vec![PDataValue {
            presentation_context_id: pc_id,
            value_type: PDataValueType::Data,
            is_last: true,
            data: query_data,
        }],
    })
    .await
    .map_err(|e| Error::Other {
        message: format!("Failed to send query data: {}", e),
        source: None,
    })?;

    // Receive responses
    let mut results = Vec::new();
    let mut result_count = 0u32;

    loop {
        let pdu = scu.receive().await.map_err(|e| Error::Other {
            message: format!("Failed to receive PDU: {}", e),
            source: None,
        })?;

        match pdu {
            Pdu::PData { data } => {
                if data.is_empty() {
                    if verbose {
                        debug!("Received empty PData");
                    }
                    break;
                }

                let data_value = &data[0];
                
                // Parse command
                let cmd_obj =
                    InMemDicomObject::read_dataset_with_ts(
                        &data_value.data[..],
                        &entries::IMPLICIT_VR_LITTLE_ENDIAN.erased(),
                    )
                    .context(super::ReadResponseSnafu)?;

                let status = cmd_obj
                    .get(tags::STATUS)
                    .ok_or_else(|| Error::Other {
                        message: "Status code missing from response".to_string(),
                        source: None,
                    })?
                    .to_int::<u16>()
                    .map_err(|e| Error::Other {
                        message: format!("Failed to read status code: {}", e),
                        source: None,
                    })?;

                if status == 0 {
                    // Success - no more results
                    if verbose {
                        info!("C-FIND completed successfully");
                    }
                    if result_count == 0 {
                        info!("No results matching query");
                    }
                    break;
                } else if status == 0xFF00 || status == 0xFF01 {
                    // Pending - result available
                    result_count += 1;

                    // Read dataset - it might be in the second PDataValue or in the next PDU
                    let dataset = if let Some(second_pdata) = data.get(1) {
                        // Dataset is in the same PDU
                        InMemDicomObject::read_dataset_with_ts(&second_pdata.data[..], ts)
                            .context(super::ReadResponseSnafu)?
                    } else {
                        // No second PDataValue means dataset comes in the next PDU
                        match scu.receive().await {
                            Ok(pdu) => match pdu {
                                Pdu::PData { ref data } => {
                                    if let Some(pdata_value) = data.first() {
                                        InMemDicomObject::read_dataset_with_ts(&pdata_value.data[..], ts)
                                            .context(super::ReadResponseSnafu)?
                                    } else {
                                        return Err(Error::Other {
                                            message: "Empty P-DATA received".to_string(),
                                            source: None,
                                        });
                                    }
                                }
                                _ => {
                                    return Err(Error::Other {
                                        message: format!("Unexpected PDU type received: {:?}", pdu),
                                        source: None,
                                    });
                                }
                            },
                            Err(e) => {
                                return Err(Error::Other {
                                    message: format!("Failed to receive dataset PDU: {}", e),
                                    source: None,
                                });
                            }
                        }
                    };

                    let attributes = extract_attributes(&dataset);

                    if verbose {
                        debug!("Result #{}: {} attributes", result_count, attributes.len());
                    }

                    // Emit result event
                    if let Some(callback) = &callbacks.on_result {
                        let event = FindResultEvent {
                            message: format!("Match #{}", result_count),
                            data: Some(attributes.clone()),
                        };
                        callback.call(Ok(event), ThreadsafeFunctionCallMode::Blocking);
                    }

                    results.push(FindResult { attributes });
                } else {
                    // Error status
                    warn!("C-FIND failed with status code: 0x{:04X}", status);
                    break;
                }
            }
            Pdu::ReleaseRQ => {
                if verbose {
                    debug!("Received release request from peer");
                }
                break;
            }
            _ => {
                if verbose {
                    warn!("Received unexpected PDU: {:?}", pdu);
                }
            }
        }
    }

    // Release association
    let _ = scu.release().await;

    let duration = start_time.elapsed();

    // Emit completion event
    if let Some(callback) = &callbacks.on_completed {
        let event = FindCompletedEvent {
            message: format!(
                "C-FIND completed: {} result(s) in {:.2}s",
                result_count,
                duration.as_secs_f64()
            ),
            data: Some(FindCompletedData {
                total_results: result_count,
                duration_seconds: duration.as_secs_f64(),
            }),
        };
        callback.call(Ok(event), ThreadsafeFunctionCallMode::Blocking);
    }

    Ok(results)
}

/// Parse address string (format: "AE@host:port" or "host:port")
fn parse_address(addr: &str) -> Result<(String, String), Error> {
    if let Some((ae, rest)) = addr.split_once('@') {
        Ok((ae.to_string(), rest.to_string()))
    } else {
        Ok(("ANY-SCP".to_string(), addr.to_string()))
    }
}

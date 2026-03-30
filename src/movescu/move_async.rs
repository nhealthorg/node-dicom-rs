use dicom_core::{dicom_value, DataElement, DataDictionary, PrimitiveValue, Tag, VR};
use dicom_core::header::Header;
use dicom_dictionary_std::{tags, StandardDataDictionary};
use dicom_object::{mem::InMemDicomObject, StandardDataDictionary as StdDict};
use dicom_ul::association::client::{ClientAssociationOptions};
use dicom_ul::pdu::{PDataValue, PDataValueType, Pdu};
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use snafu::{ResultExt, Whatever};
use std::collections::HashMap;
use std::time::Instant;
use tokio::time::Duration;

use super::{
    MoveArgs, MoveCallbacks, MoveCompletedData, MoveCompletedEvent, MoveResult,
    MoveScuError, MoveSubOperationData, MoveSubOperationEvent,
};

/// Parse address in format "AE@host:port" or "host:port"
fn parse_address(addr: &str) -> Result<(Option<String>, String, u16), String> {
    // Check if address contains AE title
    let (ae_title, host_port) = if let Some((ae, rest)) = addr.split_once('@') {
        (Some(ae.to_string()), rest)
    } else {
        (None, addr)
    };

    // Parse host and port
    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or_else(|| "Invalid address format: missing port".to_string())?;

    let port: u16 = port
        .parse()
        .map_err(|_| "Invalid port number".to_string())?;

    Ok((ae_title, host.to_string(), port))
}

/// Build a C-MOVE request command
fn move_req_command(
    abstract_syntax: &str,
    message_id: u16,
    move_destination: &str,
    priority: u16,
) -> InMemDicomObject<StdDict> {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            dicom_value!(Str, abstract_syntax),
        ),
        DataElement::new(tags::COMMAND_FIELD, VR::US, dicom_value!(U16, [0x0021])),
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [priority])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [0x0001]),
        ),
        DataElement::new(
            tags::MOVE_DESTINATION,
            VR::AE,
            dicom_value!(Str, move_destination),
        ),
    ])
}

/// Build query object from HashMap
fn build_query_object(
    query_params: &HashMap<String, String>,
    _query_model: super::MoveQueryModel,
) -> Result<InMemDicomObject<StdDict>, Whatever> {
    let mut obj = InMemDicomObject::new_empty();

    for (key, value) in query_params {
        let tag = parse_tag(key)?;
        let vr = infer_vr(tag);

        let element = if value.is_empty() {
            DataElement::new(tag, vr, PrimitiveValue::Empty)
        } else {
            DataElement::new(tag, vr, PrimitiveValue::from(value.as_str()))
        };

        obj.put(element);
    }

    Ok(obj)
}

/// Parse a tag from string (name or hex)
fn parse_tag(tag_str: &str) -> Result<Tag, Whatever> {
    // Try as tag name first
    if let Some(entry) = StandardDataDictionary.by_name(tag_str) {
        return Ok(entry.tag.inner());
    }

    // Try as hex code
    if let Some(tag) = try_parse_hex_tag(tag_str) {
        return Ok(tag);
    }

    snafu::whatever!("Unknown tag: {}", tag_str)
}

/// Try to parse hex tag in various formats
fn try_parse_hex_tag(s: &str) -> Option<Tag> {
    // Try (0010,0010) format
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        if let Some((g, e)) = inner.split_once(',') {
            if let (Ok(group), Ok(element)) = (
                u16::from_str_radix(g.trim(), 16),
                u16::from_str_radix(e.trim(), 16),
            ) {
                return Some(Tag(group, element));
            }
        }
    }

    // Try 00100010 format
    if s.len() == 8 {
        if let (Ok(group), Ok(element)) = (
            u16::from_str_radix(&s[0..4], 16),
            u16::from_str_radix(&s[4..8], 16),
        ) {
            return Some(Tag(group, element));
        }
    }

    None
}

/// Infer VR from tag
fn infer_vr(tag: Tag) -> VR {
    StandardDataDictionary
        .by_tag(tag)
        .and_then(|e| e.vr.exact())
        .unwrap_or(VR::LO)
}

/// Extract attributes from DICOM object into HashMap
fn extract_attributes(obj: &InMemDicomObject<StdDict>) -> HashMap<String, String> {
    let mut attrs = HashMap::new();

    for elem in obj.iter() {
        if let Some(entry) = StandardDataDictionary.by_tag(elem.tag()) {
            let value = elem.to_str().unwrap_or_default().to_string();
            attrs.insert(entry.alias.to_string(), value);
        }
    }

    attrs
}

/// Main function to run C-MOVE operation
pub async fn run_move(
    args: MoveArgs,
    callbacks: MoveCallbacks,
) -> Result<MoveResult, MoveScuError> {
    let start_time = Instant::now();

    // Parse address
    let (ae_from_addr, host, port) = parse_address(&args.addr)
        .map_err(|e| MoveScuError::Other {
            message: e.to_string(),
        })?;

    let called_ae_title = ae_from_addr.as_deref().unwrap_or(&args.called_ae_title);

    if args.verbose {
        println!(
            "Connecting to {}:{} (AE: {})",
            host, port, called_ae_title
        );
    }

    // Build association options
    let client_opts = ClientAssociationOptions::new()
        .calling_ae_title(&args.calling_ae_title)
        .called_ae_title(called_ae_title)
        .max_pdu_length(args.max_pdu_length)
        .with_abstract_syntax(args.query_model.sop_class_uid());

    // Establish association
    let addr_string = format!("{}:{}", host, port);
    let mut scu = client_opts
        .establish_async(&addr_string)
        .await
        .map_err(|e| MoveScuError::Association { source: e })?;

    if args.verbose {
        println!("Association established");
        println!("Sending C-MOVE request to {}", args.move_destination);
    }

    // Find presentation context for this abstract syntax
    let pc_id = scu
        .presentation_contexts()
        .iter()
        .find(|pc| pc.abstract_syntax == args.query_model.sop_class_uid())
        .map(|pc| pc.id)
        .ok_or_else(|| MoveScuError::Other {
            message: "No accepted presentation context for C-MOVE".to_string(),
        })?;

    let ts = &scu
        .presentation_contexts()
        .iter()
        .find(|pc| pc.id == pc_id)
        .unwrap()
        .transfer_syntax;

    // Build C-MOVE command
    let message_id = 1;
    let move_cmd = move_req_command(
        args.query_model.sop_class_uid(),
        message_id,
        &args.move_destination,
        0, // Priority: MEDIUM
    );

    // Build query object
    let query_obj = build_query_object(&args.query, args.query_model).map_err(|e| {
        MoveScuError::Other {
            message: e.to_string(),
        }
    })?;

    if args.verbose {
        println!("Query parameters:");
        for (key, value) in &args.query {
            println!("  {} = {}", key, value);
        }
    }

    // Send C-MOVE-RQ command
    let mut cmd_data = Vec::new();
    let ts_implicit = dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased();
    move_cmd
        .write_dataset_with_ts(
            &mut cmd_data,
            &ts_implicit,
        )
        .map_err(|e| MoveScuError::Other {
            message: format!("Failed to write move command: {}", e),
        })?;

    scu.send(&Pdu::PData {
        data: vec![PDataValue {
            presentation_context_id: pc_id,
            value_type: PDataValueType::Command,
            is_last: true,
            data: cmd_data,
        }],
    })
    .await
    .map_err(|e| MoveScuError::Other {
        message: format!("Failed to send C-MOVE command: {}", e),
    })?;

    // Send query dataset
    let mut query_data = Vec::new();
    let ts_explicit = dicom_transfer_syntax_registry::entries::EXPLICIT_VR_LITTLE_ENDIAN.erased();
    query_obj
        .write_dataset_with_ts(
            &mut query_data,
            &ts_explicit,
        )
        .map_err(|e| MoveScuError::Other {
            message: format!("Failed to write query dataset: {}", e),
        })?;

    scu.send(&Pdu::PData {
        data: vec![PDataValue {
            presentation_context_id: pc_id,
            value_type: PDataValueType::Data,
            is_last: true,
            data: query_data,
        }],
    })
    .await
    .map_err(|e| MoveScuError::Other {
        message: format!("Failed to send query dataset: {}", e),
    })?;

    if args.verbose {
        println!("C-MOVE request sent, waiting for responses...");
    }

    // Receive C-MOVE responses
    let mut total = 0u32;
    let mut completed = 0u32;
    let mut failed = 0u32;
    let mut warning = 0u32;
    let mut is_pending = true;

    while is_pending {
        // Add timeout for receiving responses
        let pdu = tokio::time::timeout(Duration::from_secs(300), scu.receive())
            .await
            .map_err(|_| MoveScuError::Other {
                message: "Timeout waiting for C-MOVE response".to_string(),
            })?
            .map_err(|e| MoveScuError::Other {
                message: format!("Failed to receive PDU: {}", e),
            })?;

        match pdu {
            Pdu::PData { data } => {
                for pdata_value in data {
                    if pdata_value.value_type == PDataValueType::Command && pdata_value.is_last {
                        // Parse response command
                        let ts_implicit_rsp = dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN
                                .erased();
                        let response = InMemDicomObject::read_dataset_with_ts(
                            &pdata_value.data[..],
                            &ts_implicit_rsp,
                        )
                        .map_err(|e| MoveScuError::Other {
                            message: format!("Failed to parse C-MOVE response: {}", e),
                        })?;

                        // Get status
                        let status = response
                            .element(tags::STATUS)
                            .map_err(|_| MoveScuError::Other {
                                message: "Missing status in C-MOVE response".to_string(),
                            })?
                            .uint16()
                            .map_err(|_| MoveScuError::Other {
                                message: "Invalid status value".to_string(),
                            })?;

                        // Extract counters if available
                        if let Ok(elem) = response.element(tags::NUMBER_OF_REMAINING_SUBOPERATIONS)
                        {
                            if let Ok(val) = elem.to_int::<u32>() {
                                total = completed + val;
                            }
                        }
                        if let Ok(elem) = response.element(tags::NUMBER_OF_COMPLETED_SUBOPERATIONS)
                        {
                            if let Ok(val) = elem.to_int::<u32>() {
                                completed = val;
                            }
                        }
                        if let Ok(elem) = response.element(tags::NUMBER_OF_FAILED_SUBOPERATIONS) {
                            if let Ok(val) = elem.to_int::<u32>() {
                                failed = val;
                            }
                        }
                        if let Ok(elem) = response.element(tags::NUMBER_OF_WARNING_SUBOPERATIONS) {
                            if let Ok(val) = elem.to_int::<u32>() {
                                warning = val;
                            }
                        }

                        match status {
                            0xFF00 => {
                                // Pending - sub-operation in progress
                                if args.verbose {
                                    println!(
                                        "Sub-operations: {} completed, {} remaining, {} failed, {} warning",
                                        completed,
                                        total - completed,
                                        failed,
                                        warning
                                    );
                                }

                                // Emit sub-operation event
                                if let Some(ref callback) = callbacks.on_sub_operation {
                                    let event = MoveSubOperationEvent {
                                        message: format!(
                                            "Sub-operation in progress: {} of {} completed",
                                            completed, total
                                        ),
                                        data: Some(MoveSubOperationData {
                                            remaining: total.saturating_sub(completed),
                                            completed,
                                            failed,
                                            warning,
                                        }),
                                    };
                                    callback.call(Ok(event), ThreadsafeFunctionCallMode::Blocking);
                                }
                            }
                            0x0000 => {
                                // Success
                                if args.verbose {
                                    println!("C-MOVE completed successfully");
                                    println!("Final: {} completed, {} failed, {} warning", completed, failed, warning);
                                }
                                is_pending = false;
                            }
                            0xB000 => {
                                // Warning - completed with warnings
                                if args.verbose {
                                    println!("C-MOVE completed with warnings");
                                }
                                is_pending = false;
                            }
                            _ => {
                                // Error or other status
                                let error_msg = response
                                    .element(tags::ERROR_COMMENT)
                                    .ok()
                                    .and_then(|e| e.to_str().ok())
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| "Unknown error".to_string());

                                if args.verbose {
                                    println!("C-MOVE failed with status 0x{:04X}: {}", status, error_msg);
                                }

                                is_pending = false;

                                return Err(MoveScuError::MoveFailed {
                                    message: format!(
                                        "C-MOVE failed with status 0x{:04X}: {}",
                                        status, error_msg
                                    ),
                                });
                            }
                        }
                    }
                }
            }
            Pdu::ReleaseRQ => {
                // Remote side is releasing
                scu.send(&Pdu::ReleaseRP)
                    .await
                    .map_err(|e| MoveScuError::Other {
                        message: format!("Failed to send release response: {}", e),
                    })?;
                break;
            }
            Pdu::AbortRQ { source } => {
                return Err(MoveScuError::Other {
                    message: format!("Association aborted by {:?}", source),
                });
            }
            _ => {
                // Ignore other PDUs
            }
        }
    }

    // Release association
    scu.release()
        .await
        .map_err(|e| MoveScuError::Other {
            message: format!("Failed to release association: {}", e),
        })?;

    let duration = start_time.elapsed();

    if args.verbose {
        println!("Association released");
        println!("Total time: {:.2}s", duration.as_secs_f64());
    }

    // Emit completed event
    if let Some(ref callback) = callbacks.on_completed {
        let event = MoveCompletedEvent {
            message: format!(
                "C-MOVE completed: {} of {} instances in {:.2}s",
                completed,
                total,
                duration.as_secs_f64()
            ),
            data: Some(MoveCompletedData {
                total,
                completed,
                failed,
                warning,
                duration_seconds: duration.as_secs_f64(),
            }),
        };
        callback.call(Ok(event), ThreadsafeFunctionCallMode::Blocking);
    }

    Ok(MoveResult {
        total,
        completed,
        failed,
        warning,
    })
}

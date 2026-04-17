use dicom_core::{dicom_value, DataElement, DataDictionary, PrimitiveValue, Tag, VR};
use dicom_core::header::Header;
use dicom_encoding::TransferSyntaxIndex;
use dicom_dictionary_std::{tags, StandardDataDictionary};
use dicom_object::{mem::InMemDicomObject, StandardDataDictionary as StdDict, FileDicomObject};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::client::ClientAssociationOptions;
use dicom_ul::pdu::{PDataValue, PDataValueType, Pdu};
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use snafu::{ResultExt, Whatever};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tokio::time::Duration;

use crate::utils::{build_s3_bucket, s3_put_object};

use super::{
    GetArgs, GetCallbacks, GetCompletedData, GetCompletedEvent, GetResult,
    GetScuError, GetStorageBackend, GetSubOperationData, GetSubOperationEvent,
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

/// Build a C-GET request command
fn get_req_command(
    abstract_syntax: &str,
    message_id: u16,
    priority: u16,
) -> InMemDicomObject<StdDict> {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            dicom_value!(Str, abstract_syntax),
        ),
        DataElement::new(tags::COMMAND_FIELD, VR::US, dicom_value!(U16, [0x0010])), // C-GET-RQ
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [priority])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [0x0001]), // Dataset present
        ),
    ])
}

/// Build C-STORE response command
fn store_rsp_command(
    affected_sop_class_uid: &str,
    affected_sop_instance_uid: &str,
    message_id: u16,
    status: u16,
) -> InMemDicomObject<StdDict> {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            dicom_value!(Str, affected_sop_class_uid),
        ),
        DataElement::new(tags::COMMAND_FIELD, VR::US, dicom_value!(U16, [0x8001])), // C-STORE-RSP
        DataElement::new(tags::MESSAGE_ID_BEING_RESPONDED_TO, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [0x0101]), // No dataset
        ),
        DataElement::new(tags::STATUS, VR::US, dicom_value!(U16, [status])),
        DataElement::new(
            tags::AFFECTED_SOP_INSTANCE_UID,
            VR::UI,
            dicom_value!(Str, affected_sop_instance_uid),
        ),
    ])
}

/// Build query object from HashMap
fn build_query_object(
    query_params: &HashMap<String, String>,
    _query_model: super::GetQueryModel,
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

/// Get all common storage SOP class UIDs for presentation contexts
fn get_storage_sop_classes() -> Vec<String> {
    use dicom_dictionary_std::uids;
    vec![
        // CT Image Storage
        uids::CT_IMAGE_STORAGE.to_string(),
        // MR Image Storage
        uids::MR_IMAGE_STORAGE.to_string(),
        // Enhanced MR Image Storage
        uids::ENHANCED_MR_IMAGE_STORAGE.to_string(),
        // Enhanced CT Image Storage
        uids::ENHANCED_CT_IMAGE_STORAGE.to_string(),
        // Digital X-Ray Image Storage
        uids::DIGITAL_X_RAY_IMAGE_STORAGE_FOR_PRESENTATION.to_string(),
        uids::DIGITAL_X_RAY_IMAGE_STORAGE_FOR_PROCESSING.to_string(),
        // Ultrasound Image Storage
        uids::ULTRASOUND_IMAGE_STORAGE.to_string(),
        uids::ENHANCED_US_VOLUME_STORAGE.to_string(),
        // Secondary Capture Image Storage
        uids::SECONDARY_CAPTURE_IMAGE_STORAGE.to_string(),
        // PET Image Storage
        uids::POSITRON_EMISSION_TOMOGRAPHY_IMAGE_STORAGE.to_string(),
        uids::ENHANCED_PET_IMAGE_STORAGE.to_string(),
        // Nuclear Medicine Image Storage
        uids::NUCLEAR_MEDICINE_IMAGE_STORAGE.to_string(),
        // RT Image Storage
        uids::RT_IMAGE_STORAGE.to_string(),
        // RT Dose Storage
        uids::RT_DOSE_STORAGE.to_string(),
        // RT Structure Set Storage
        uids::RT_STRUCTURE_SET_STORAGE.to_string(),
        // RT Plan Storage
        uids::RT_PLAN_STORAGE.to_string(),
    ]
}

/// Store received DICOM file to filesystem or S3
async fn store_dicom_file(
    dicom_obj: &FileDicomObject<InMemDicomObject>,
    storage_backend: &GetStorageBackend,
    out_dir: &Option<String>,
    s3_bucket: &Option<s3::Bucket>,
    store_with_file_meta: bool,
    verbose: bool,
) -> Result<String, GetScuError> {
    // Extract key identifiers
    let study_uid = dicom_obj
        .element(tags::STUDY_INSTANCE_UID)
        .map_err(|e| GetScuError::Other {
            message: format!("Missing StudyInstanceUID: {}", e),
        })?
        .to_str()
        .map_err(|e| GetScuError::Other {
            message: format!("Invalid StudyInstanceUID: {}", e),
        })?
        .trim()
        .to_string();

    let series_uid = dicom_obj
        .element(tags::SERIES_INSTANCE_UID)
        .map_err(|e| GetScuError::Other {
            message: format!("Missing SeriesInstanceUID: {}", e),
        })?
        .to_str()
        .map_err(|e| GetScuError::Other {
            message: format!("Invalid SeriesInstanceUID: {}", e),
        })?
        .trim()
        .to_string();

    let instance_uid = dicom_obj
        .element(tags::SOP_INSTANCE_UID)
        .map_err(|e| GetScuError::Other {
            message: format!("Missing SOPInstanceUID: {}", e),
        })?
        .to_str()
        .map_err(|e| GetScuError::Other {
            message: format!("Invalid SOPInstanceUID: {}", e),
        })?
        .trim()
        .to_string();

    match storage_backend {
        GetStorageBackend::Filesystem => {
            let base_dir = out_dir.as_deref().unwrap_or(".");
            let file_dir = PathBuf::from(base_dir)
                .join(&study_uid)
                .join(&series_uid);

            // Create directory structure
            tokio::fs::create_dir_all(&file_dir).await.map_err(|e| GetScuError::Io {
                source: e,
            })?;

            let file_path = file_dir.join(format!("{}.dcm", instance_uid));

            // Write DICOM file - with or without file meta header based on config
            let mut file_data = Vec::new();
            if store_with_file_meta {
                // Write complete DICOM file with preamble and meta header
                dicom_obj
                    .write_all(&mut file_data)
                    .map_err(|e| GetScuError::Other {
                        message: format!("Failed to write DICOM file: {}", e),
                    })?;
            } else {
                // Write dataset-only (no preamble, no meta header)
                let ts = TransferSyntaxRegistry
                    .get(dicom_obj.meta().transfer_syntax())
                    .ok_or_else(|| GetScuError::Other {
                        message: format!(
                            "Unknown transfer syntax: {}",
                            dicom_obj.meta().transfer_syntax()
                        ),
                    })?;
                dicom_obj
                    .write_dataset_with_ts(&mut file_data, ts)
                    .map_err(|e| GetScuError::Other {
                        message: format!("Failed to write dataset: {}", e),
                    })?;
            }

            tokio::fs::write(&file_path, file_data).await.map_err(|e| GetScuError::Io {
                source: e,
            })?;

            if verbose {
                println!("✓ Stored: {}", file_path.display());
            }

            Ok(file_path.display().to_string())
        }
        GetStorageBackend::S3 => {
            let bucket = s3_bucket.as_ref().ok_or_else(|| GetScuError::Other {
                message: "S3 bucket not configured".to_string(),
            })?;

            let key = format!("{}/{}/{}.dcm", study_uid, series_uid, instance_uid);

            // Write DICOM file to buffer - with or without file meta header based on config
            let mut file_data = Vec::new();
            if store_with_file_meta {
                // Write complete DICOM file with preamble and meta header
                dicom_obj
                    .write_all(&mut file_data)
                    .map_err(|e| GetScuError::Other {
                        message: format!("Failed to write DICOM file: {}", e),
                    })?;
            } else {
                // Write dataset-only (no preamble, no meta header)
                let ts = TransferSyntaxRegistry
                    .get(dicom_obj.meta().transfer_syntax())
                    .ok_or_else(|| GetScuError::Other {
                        message: format!(
                            "Unknown transfer syntax: {}",
                            dicom_obj.meta().transfer_syntax()
                        ),
                    })?;
                dicom_obj
                    .write_dataset_with_ts(&mut file_data, ts)
                    .map_err(|e| GetScuError::Other {
                        message: format!("Failed to write dataset: {}", e),
                    })?;
            }

            // Upload to S3
            s3_put_object(bucket, &key, &file_data)
                .await
                .map_err(|e| GetScuError::Other {
                    message: format!("S3 upload failed: {}", e),
                })?;

            if verbose {
                println!("✓ Stored to S3: s3://{}/{}", bucket.name(), key);
            }

            Ok(format!("s3://{}/{}", bucket.name(), key))
        }
        GetStorageBackend::Forward => {
            Err(GetScuError::Other {
                message: "Forward backend is handled in the C-STORE receive loop".to_string(),
            })
        }
    }
}

/// Main function to run C-GET operation
pub async fn run_get(
    args: GetArgs,
    callbacks: GetCallbacks,
) -> Result<GetResult, GetScuError> {
    let start_time = Instant::now();

    // Parse address
    let (ae_from_addr, host, port) = parse_address(&args.addr)
        .map_err(|e| GetScuError::Other {
            message: e.to_string(),
        })?;

    let called_ae_title = ae_from_addr.as_deref().unwrap_or(&args.called_ae_title);

    if args.verbose {
        println!(
            "Connecting to {}:{} (AE: {})",
            host, port, called_ae_title
        );
    }

    // Initialize S3 bucket if needed
    let s3_bucket = if args.storage_backend == GetStorageBackend::S3 {
        let s3_config = args.s3_config.as_ref().ok_or_else(|| GetScuError::Other {
            message: "S3 storage backend requires s3Config".to_string(),
        })?;
        Some(build_s3_bucket(s3_config))
    } else {
        None
    };

    // Open a persistent forward association if the Forward backend is selected.
    // We establish it before starting the C-GET so all received instances share
    // a single connection to the destination PACS.
    let storage_sop_classes = get_storage_sop_classes();

    let mut forward_assoc: Option<crate::utils::ForwardAssociation> = None;
    if args.storage_backend == GetStorageBackend::Forward {
        let target = args.forward_target.as_ref().ok_or_else(|| GetScuError::Other {
            message: "Forward backend requires forwardTarget configuration".to_string(),
        })?;
        if args.verbose {
            println!("Opening forward association to {}", target.addr);
        }
        let assoc = crate::utils::open_forward_association(target, &storage_sop_classes)
            .await
            .map_err(|e| GetScuError::Other {
                message: format!("Failed to open forward association: {}", e),
            })?;
        forward_assoc = Some(assoc);
        if args.verbose {
            println!("Forward association established to {}", target.addr);
        }
    }

    // Build association options with storage SOP classes for receiving C-STORE
    let mut client_opts = ClientAssociationOptions::new()
        .calling_ae_title(&args.calling_ae_title)
        .called_ae_title(called_ae_title)
        .max_pdu_length(args.max_pdu_length)
        .with_abstract_syntax(args.query_model.sop_class_uid()); // C-GET query model

    // Add presentation contexts for all storage SOP classes (to receive files)
    for sop_class in &storage_sop_classes {
        client_opts = client_opts.with_abstract_syntax(sop_class);
    }

    // Establish association
    let addr_string = format!("{}:{}", host, port);
    let mut scu = client_opts
        .establish_async(&addr_string)
        .await
        .map_err(|e| GetScuError::Association { source: e })?;

    if args.verbose {
        println!("Association established");
        println!("Sending C-GET request");
    }

    // Find presentation context for C-GET
    let pc_id = scu
        .presentation_contexts()
        .iter()
        .find(|pc| pc.abstract_syntax == args.query_model.sop_class_uid())
        .map(|pc| pc.id)
        .ok_or_else(|| GetScuError::Other {
            message: "No accepted presentation context for C-GET".to_string(),
        })?;

    // Build C-GET command
    let message_id = 1;
    let get_cmd = get_req_command(
        args.query_model.sop_class_uid(),
        message_id,
        0, // Priority: MEDIUM
    );

    // Build query object
    let query_obj = build_query_object(&args.query, args.query_model).map_err(|e| {
        GetScuError::Other {
            message: e.to_string(),
        }
    })?;

    if args.verbose {
        println!("Query parameters:");
        for (key, value) in &args.query {
            println!("  {} = {}", key, value);
        }
    }

    // Send C-GET-RQ command
    let mut cmd_data = Vec::new();
    let ts_implicit = dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased();
    get_cmd
        .write_dataset_with_ts(
            &mut cmd_data,
            &ts_implicit,
        )
        .map_err(|e| GetScuError::Other {
            message: format!("Failed to write GET command: {}", e),
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
    .map_err(|e| GetScuError::Other {
        message: format!("Failed to send C-GET command: {}", e),
    })?;

    // Send query dataset
    let mut query_data = Vec::new();
    let ts_explicit = dicom_transfer_syntax_registry::entries::EXPLICIT_VR_LITTLE_ENDIAN.erased();
    query_obj
        .write_dataset_with_ts(
            &mut query_data,
            &ts_explicit,
        )
        .map_err(|e| GetScuError::Other {
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
    .map_err(|e| GetScuError::Other {
        message: format!("Failed to send query dataset: {}", e),
    })?;

    if args.verbose {
        println!("C-GET request sent, waiting for responses...");
    }

    // Receive C-GET responses and C-STORE requests
    let mut total = 0u32;
    let mut completed = 0u32;
    let mut failed = 0u32;
    let mut warning = 0u32;
    let mut is_complete = false;

    // For handling multi-PDU C-STORE
    let mut pending_command: Option<InMemDicomObject<StdDict>> = None;
    let mut pending_data: Vec<u8> = Vec::new();
    let mut pending_pc_id: Option<u8> = None;

    while !is_complete {
        // Add timeout for receiving responses
        let pdu = tokio::time::timeout(Duration::from_secs(300), scu.receive())
            .await
            .map_err(|_| GetScuError::Other {
                message: "Timeout waiting for C-GET response".to_string(),
            })?
            .map_err(|e| GetScuError::Other {
                message: format!("Failed to receive PDU: {}", e),
            })?;

        match pdu {
            Pdu::PData { data } => {
                for pdata_value in data {
                    match pdata_value.value_type {
                        PDataValueType::Command => {
                            // Parse command
                            let cmd = InMemDicomObject::read_dataset_with_ts(
                                &pdata_value.data[..],
                                &ts_implicit,
                            )
                            .map_err(|e| GetScuError::Other {
                                message: format!("Failed to parse command: {}", e),
                            })?;

                            let command_field = cmd
                                .element(tags::COMMAND_FIELD)
                                .map_err(|_| GetScuError::Other {
                                    message: "Missing command field".to_string(),
                                })?
                                .uint16()
                                .map_err(|_| GetScuError::Other {
                                    message: "Invalid command field".to_string(),
                                })?;

                            match command_field {
                                0x0001 => {
                                    // C-STORE-RQ - incoming file
                                    pending_command = Some(cmd);
                                    pending_data.clear();
                                    pending_pc_id = Some(pdata_value.presentation_context_id);
                                }
                                0x8010 => {
                                    // C-GET-RSP - progress update
                                    let status = cmd
                                        .element(tags::STATUS)
                                        .map_err(|_| GetScuError::Other {
                                            message: "Missing status in C-GET response".to_string(),
                                        })?
                                        .uint16()
                                        .map_err(|_| GetScuError::Other {
                                            message: "Invalid status value".to_string(),
                                        })?;

                                    // Extract counters from the current response before deriving totals.
                                    let remaining = cmd
                                        .element(tags::NUMBER_OF_REMAINING_SUBOPERATIONS)
                                        .ok()
                                        .and_then(|elem| elem.to_int::<u32>().ok())
                                        .unwrap_or(0);
                                    let current_completed = cmd
                                        .element(tags::NUMBER_OF_COMPLETED_SUBOPERATIONS)
                                        .ok()
                                        .and_then(|elem| elem.to_int::<u32>().ok())
                                        .unwrap_or(completed);
                                    let current_failed = cmd
                                        .element(tags::NUMBER_OF_FAILED_SUBOPERATIONS)
                                        .ok()
                                        .and_then(|elem| elem.to_int::<u32>().ok())
                                        .unwrap_or(failed);
                                    let current_warning = cmd
                                        .element(tags::NUMBER_OF_WARNING_SUBOPERATIONS)
                                        .ok()
                                        .and_then(|elem| elem.to_int::<u32>().ok())
                                        .unwrap_or(warning);

                                    completed = completed.max(current_completed);
                                    failed = failed.max(current_failed);
                                    warning = warning.max(current_warning);
                                    total = total.max(completed + failed + warning + remaining);

                                    match status {
                                        0xFF00 => {
                                            // Pending
                                            if args.verbose {
                                                println!(
                                                    "C-GET pending: {} completed, {} remaining, {} failed",
                                                    completed,
                                                    remaining,
                                                    failed
                                                );
                                            }

                                            // Emit sub-operation event
                                            if let Some(ref callback) = callbacks.on_sub_operation {
                                                callback.call(
                                                    Ok(GetSubOperationEvent {
                                                        message: "C-GET in progress".to_string(),
                                                        data: Some(GetSubOperationData {
                                                            remaining,
                                                            completed,
                                                            failed,
                                                            warning,
                                                            file: None,
                                                            sop_instance_uid: None,
                                                            forwarded_to: None,
                                                            forward_status: None,
                                                            forward_error: None,
                                                        }),
                                                    }),
                                                    ThreadsafeFunctionCallMode::Blocking,
                                                );
                                            }
                                        }
                                        0x0000 => {
                                            // Success
                                            if args.verbose {
                                                println!("C-GET completed successfully");
                                            }
                                            is_complete = true;
                                        }
                                        0xB000 => {
                                            // Warning
                                            if args.verbose {
                                                println!("C-GET completed with warnings");
                                            }
                                            is_complete = true;
                                        }
                                        _ => {
                                            // Error
                                            let error_msg = format!("C-GET failed with status: 0x{:04X}", status);
                                            return Err(GetScuError::GetFailed {
                                                message: error_msg,
                                            });
                                        }
                                    }
                                }
                                _ => {
                                    // Unknown command
                                    if args.verbose {
                                        println!("Unknown command: 0x{:04X}", command_field);
                                    }
                                }
                            }
                        }
                        PDataValueType::Data => {
                            // Accumulate data
                            pending_data.extend_from_slice(&pdata_value.data);

                            if pdata_value.is_last {
                                // Complete C-STORE-RQ received
                                if let (Some(cmd), Some(pc_id)) = (pending_command.take(), pending_pc_id.take()) {
                                    let sop_class_uid = cmd
                                        .element(tags::AFFECTED_SOP_CLASS_UID)
                                        .map_err(|_| GetScuError::Other {
                                            message: "Missing SOP Class UID".to_string(),
                                        })?
                                        .to_str()
                                        .map_err(|_| GetScuError::Other {
                                            message: "Invalid SOP Class UID".to_string(),
                                        })?
                                        .trim()
                                        .to_string();

                                    let sop_instance_uid = cmd
                                        .element(tags::AFFECTED_SOP_INSTANCE_UID)
                                        .map_err(|_| GetScuError::Other {
                                            message: "Missing SOP Instance UID".to_string(),
                                        })?
                                        .to_str()
                                        .map_err(|_| GetScuError::Other {
                                            message: "Invalid SOP Instance UID".to_string(),
                                        })?
                                        .trim()
                                        .to_string();

                                    let store_message_id = cmd
                                        .element(tags::MESSAGE_ID)
                                        .map_err(|_| GetScuError::Other {
                                            message: "Missing message ID".to_string(),
                                        })?
                                        .uint16()
                                        .map_err(|_| GetScuError::Other {
                                            message: "Invalid message ID".to_string(),
                                        })?;

                                    // Find transfer syntax for this presentation context
                                    // Clone the TS string to avoid borrow conflicts with
                                    // the later scu.send() call.
                                    let ts_uid: String = scu
                                        .presentation_contexts()
                                        .iter()
                                        .find(|pc| pc.id == pc_id)
                                        .map(|pc| pc.transfer_syntax.clone())
                                        .ok_or_else(|| GetScuError::Other {
                                            message: "Presentation context not found".to_string(),
                                        })?;

                                    // Store or forward the received instance.
                                    let mut file_path: Option<String> = None;
                                    let mut forwarded_to: Option<String> = None;
                                    let mut forward_status: Option<String> = None;
                                    let mut forward_error: Option<String> = None;
                                    let mut store_rsp_status: u16 = 0x0000;

                                    match &args.storage_backend {
                                        GetStorageBackend::Forward => {
                                            let target =
                                                args.forward_target.as_ref().ok_or_else(|| {
                                                    GetScuError::Other {
                                                        message: "Forward backend requires \
                                                                  forwardTarget configuration"
                                                            .to_string(),
                                                    }
                                                })?;
                                            forwarded_to = Some(target.addr.clone());
                                            let fwd =
                                                forward_assoc.as_mut().ok_or_else(|| {
                                                    GetScuError::Other {
                                                        message: "Forward association not open"
                                                            .to_string(),
                                                    }
                                                })?;
                                            match crate::utils::forward_dicom_bytes(
                                                fwd,
                                                &sop_class_uid,
                                                &sop_instance_uid,
                                                &ts_uid,
                                                &pending_data,
                                                store_message_id,
                                                args.verbose,
                                            )
                                            .await
                                            {
                                                Ok(_) => {
                                                    forward_status = Some("ok".to_string());
                                                }
                                                Err(e) => {
                                                    let err = e.to_string();
                                                    if args.verbose {
                                                        println!(
                                                            "Forward to {} failed: {}",
                                                            target.addr, err
                                                        );
                                                    }
                                                    forward_status = Some("error".to_string());
                                                    forward_error = Some(err);
                                                    if args.strict_forward {
                                                        // Refused: Out of resources
                                                        store_rsp_status = 0xA700;
                                                    }
                                                }
                                            }
                                        }
                                        _ => {
                                            let ts_obj = TransferSyntaxRegistry
                                                .get(&ts_uid)
                                                .ok_or_else(|| GetScuError::Other {
                                                    message: format!(
                                                        "Unknown transfer syntax: {}",
                                                        ts_uid
                                                    ),
                                                })?;

                                            // Parse received DICOM dataset
                                            let dataset = InMemDicomObject::read_dataset_with_ts(
                                                &pending_data[..],
                                                ts_obj,
                                            )
                                            .map_err(|e| GetScuError::Other {
                                                message: format!("Failed to parse dataset: {}", e),
                                            })?;

                                            // Create file meta information
                                            use dicom_object::FileMetaTableBuilder;
                                            let meta = FileMetaTableBuilder::new()
                                                .media_storage_sop_class_uid(&sop_class_uid)
                                                .media_storage_sop_instance_uid(&sop_instance_uid)
                                                .transfer_syntax(&ts_uid)
                                                .build()
                                                .map_err(|e| GetScuError::Other {
                                                    message: format!(
                                                        "Failed to build meta: {}",
                                                        e
                                                    ),
                                                })?;

                                            let dicom_file = dataset.with_exact_meta(meta);

                                            file_path = Some(store_dicom_file(
                                                &dicom_file,
                                                &args.storage_backend,
                                                &args.out_dir,
                                                &s3_bucket,
                                                args.store_with_file_meta,
                                                args.verbose,
                                            )
                                            .await?);
                                        }
                                    }

                                    // Send C-STORE-RSP (success)
                                    let store_rsp = store_rsp_command(
                                        &sop_class_uid,
                                        &sop_instance_uid,
                                        store_message_id,
                                        store_rsp_status,
                                    );

                                    let mut rsp_data = Vec::new();
                                    store_rsp
                                        .write_dataset_with_ts(&mut rsp_data, &ts_implicit)
                                        .map_err(|e| GetScuError::Other {
                                            message: format!("Failed to write C-STORE response: {}", e),
                                        })?;

                                    scu.send(&Pdu::PData {
                                        data: vec![PDataValue {
                                            presentation_context_id: pc_id,
                                            value_type: PDataValueType::Command,
                                            is_last: true,
                                            data: rsp_data,
                                        }],
                                    })
                                    .await
                                    .map_err(|e| GetScuError::Other {
                                        message: format!("Failed to send C-STORE response: {}", e),
                                    })?;

                                    // Emit sub-operation event with file info
                                    if let Some(ref callback) = callbacks.on_sub_operation {
                                        let remaining = total.saturating_sub(completed + failed + warning);
                                        if total > 0 || completed > 0 || failed > 0 || warning > 0 {
                                            let message = match (&args.storage_backend, forward_status.as_deref()) {
                                                (GetStorageBackend::Forward, Some("ok")) => "File forwarded".to_string(),
                                                (GetStorageBackend::Forward, Some("error")) => "Forward failed".to_string(),
                                                _ => "File received and stored".to_string(),
                                            };
                                            callback.call(
                                                Ok(GetSubOperationEvent {
                                                    message,
                                                    data: Some(GetSubOperationData {
                                                        remaining,
                                                        completed,
                                                        failed,
                                                        warning,
                                                        file: file_path,
                                                        sop_instance_uid: Some(sop_instance_uid.clone()),
                                                        forwarded_to,
                                                        forward_status,
                                                        forward_error: forward_error.clone(),
                                                    }),
                                                }),
                                                ThreadsafeFunctionCallMode::Blocking,
                                            );
                                        }
                                    }

                                    if args.strict_forward {
                                        if let Some(err) = forward_error {
                                            return Err(GetScuError::GetFailed {
                                                message: format!(
                                                    "Strict forward failed for instance {}: {}",
                                                    sop_instance_uid, err
                                                ),
                                            });
                                        }
                                    }

                                    pending_data.clear();
                                }
                            }
                        }
                    }
                }
            }
            Pdu::ReleaseRQ => {
                // Remote side is releasing
                scu.send(&Pdu::ReleaseRP)
                    .await
                    .map_err(|e| GetScuError::Other {
                        message: format!("Failed to send release response: {}", e),
                    })?;
                break;
            }
            Pdu::AbortRQ { source } => {
                return Err(GetScuError::Other {
                    message: format!("Association aborted by {:?}", source),
                });
            }
            _ => {
                // Ignore other PDUs
            }
        }
    }

    // Release association
    // Release the forward association (best-effort) before the source association.
    if let Some(fwd) = forward_assoc {
        if args.verbose {
            println!("Releasing forward association");
        }
        let _ = fwd.release().await;
    }

    scu.release()
        .await
        .map_err(|e| GetScuError::Other {
            message: format!("Failed to release association: {}", e),
        })?;

    let duration = start_time.elapsed();

    if args.verbose {
        println!("Association released");
        println!("Total time: {:.2}s", duration.as_secs_f64());
    }

    // Emit completed event
    if let Some(ref callback) = callbacks.on_completed {
        let event = GetCompletedEvent {
            message: format!(
                "C-GET completed: {} of {} instances in {:.2}s",
                completed,
                total,
                duration.as_secs_f64()
            ),
            data: Some(GetCompletedData {
                total,
                completed,
                failed,
                warning,
                duration_seconds: duration.as_secs_f64(),
            }),
        };
        callback.call(Ok(event), ThreadsafeFunctionCallMode::Blocking);
    }

    Ok(GetResult {
        total,
        completed,
        failed,
        warning,
    })
}

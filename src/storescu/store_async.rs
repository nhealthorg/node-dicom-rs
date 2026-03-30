use std::sync::Arc;

use dicom_dictionary_std::tags;
use dicom_encoding::TransferSyntaxIndex;
use dicom_object::{open_file, FileDicomObject, InMemDicomObject};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::{
    association::Association,
    pdu::{PDataValue, PDataValueType},
    ClientAssociation, Pdu,
};
use indicatif::ProgressBar;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use snafu::{OptionExt, Report, ResultExt};
use tokio::{io::AsyncWriteExt, net::TcpStream, sync::Mutex};
use tracing::{debug, error, info, warn};

use crate::storescu::{
    check_presentation_contexts, into_ts, store_req_command, ConvertFieldSnafu, CreateCommandSnafu,
    DicomFile, Error, FileSendingEvent, FileSendingData, FileSentEvent, FileSentData, 
    FileErrorEvent, FileErrorData, FileSource, MissingAttributeSnafu, 
    ReadDatasetSnafu, ReadFilePathSnafu, ScuSnafu, StoreScu, UnsupportedFileTransferSyntaxSnafu, WriteDatasetSnafu,
};

#[derive(Clone)]
pub struct StoreCallbacks {
    pub on_file_sending: Option<Arc<ThreadsafeFunction<FileSendingEvent, ()>>>,
    pub on_file_sent: Option<Arc<ThreadsafeFunction<FileSentEvent, ()>>>,
    pub on_file_error: Option<Arc<ThreadsafeFunction<FileErrorEvent, ()>>>,
}

pub async fn send_file(
    mut scu: dicom_ul::association::client::AsyncClientAssociation<TcpStream>,
    file: DicomFile,
    s3_bucket: Option<&s3::Bucket>,
    message_id: u16,
    progress_bar: Option<&Arc<tokio::sync::Mutex<ProgressBar>>>,
    verbose: bool,
    fail_first: bool,
    callbacks: &StoreCallbacks,
    successful_count: Arc<Mutex<u32>>,
    failed_count: Arc<Mutex<u32>>,
) -> Result<dicom_ul::association::client::AsyncClientAssociation<TcpStream>, Error>
{
    let start_time = std::time::Instant::now();
    
    if let (Some(pc_selected), Some(ts_uid_selected)) = (file.pc_selected, file.ts_selected) {
        // Emit OnFileSending event
        let file_path = match &file.source {
            FileSource::Local(path) => path.display().to_string(),
            FileSource::S3(key) => format!("s3://{}", key),
        };
        
        if let Some(cb) = &callbacks.on_file_sending {
            cb.call(Ok(FileSendingEvent {
                message: "Sending file".to_string(),
                data: Some(FileSendingData {
                    file: file_path.clone(),
                    sop_instance_uid: file.sop_instance_uid.clone(),
                    sop_class_uid: file.sop_class_uid.clone(),
                }),
            }), ThreadsafeFunctionCallMode::NonBlocking);
        }
        let cmd = store_req_command(&file.sop_class_uid, &file.sop_instance_uid, message_id);

        let mut cmd_data = Vec::with_capacity(128);
        cmd.write_dataset_with_ts(
            &mut cmd_data,
            &dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased(),
        )
        .map_err(Box::from)
        .context(CreateCommandSnafu)?;

        let mut object_data = Vec::with_capacity(2048);
        
        // Load DICOM file from source (local filesystem or S3)
        let dicom_file: FileDicomObject<InMemDicomObject> = match &file.source {
            FileSource::Local(path) => {
                open_file(path)
                    .map_err(Box::from)
                    .context(ReadFilePathSnafu {
                        path: path.display().to_string(),
                    })?
            }
            FileSource::S3(key) => {
                // Download S3 file on-demand to minimize memory usage
                use crate::utils::s3_get_object;
                let bucket = s3_bucket.expect("S3 bucket should be available for S3 files");
                let s3_result = s3_get_object(bucket, key).await;
                let data = match s3_result {
                    Ok(d) => d,
                    Err(_e) => {
                        return Err(Error::ReadFilePath {
                            path: format!("s3://{}", key),
                            source: Box::new(dicom_object::ReadError::ReadFile {
                                filename: format!("s3://{}", key).into(),
                                source: std::io::Error::new(
                                    std::io::ErrorKind::Other,
                                    format!("Failed to download S3 file for sending: {}", key),
                                ),
                                backtrace: std::backtrace::Backtrace::capture(),
                            }),
                        });
                    }
                };
                
                // Auto-detect file format by checking for DICM magic bytes
                let has_dicm_magic = data.len() > 132 && &data[128..132] == b"DICM";
                
                if !has_dicm_magic {
                    // Dataset-only file (no DICOM meta header) - read as InMemDicomObject and create meta
                    let obj = InMemDicomObject::read_dataset_with_ts(
                        &data[..],
                        &dicom_transfer_syntax_registry::entries::EXPLICIT_VR_LITTLE_ENDIAN.erased(),
                    )
                    .or_else(|_| {
                        InMemDicomObject::read_dataset_with_ts(
                            &data[..],
                            &dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased(),
                        )
                    })
                    .context(ReadDatasetSnafu)?;
                    
                    // Create file meta information from dataset attributes
                    use dicom_dictionary_std::tags;
                    use dicom_object::FileMetaTableBuilder;
                    
                    let sop_class_uid = obj.element(tags::SOP_CLASS_UID)
                        .context(MissingAttributeSnafu { tag: tags::SOP_CLASS_UID })?
                        .to_str()
                        .context(ConvertFieldSnafu { tag: tags::SOP_CLASS_UID })?
                        .trim()
                        .to_string();
                    let sop_instance_uid = obj.element(tags::SOP_INSTANCE_UID)
                        .context(MissingAttributeSnafu { tag: tags::SOP_INSTANCE_UID })?
                        .to_str()
                        .context(ConvertFieldSnafu { tag: tags::SOP_INSTANCE_UID })?
                        .trim()
                        .to_string();
                    
                    let meta = FileMetaTableBuilder::new()
                        .media_storage_sop_class_uid(&sop_class_uid)
                        .media_storage_sop_instance_uid(&sop_instance_uid)
                        .transfer_syntax(dicom_transfer_syntax_registry::entries::EXPLICIT_VR_LITTLE_ENDIAN.uid())
                        .build()
                        .map_err(|e| Error::ReadDataset { 
                            source: dicom_object::ReadError::ParseMetaDataSet { source: e } 
                        })?;
                    
                    obj.with_exact_meta(meta)
                } else {
                    // Full DICOM file with meta header
                    dicom_object::from_reader(&data[..])
                        .context(ReadDatasetSnafu)?
                }
            }
        };
        
        let ts_selected = TransferSyntaxRegistry
            .get(&ts_uid_selected)
            .with_context(|| UnsupportedFileTransferSyntaxSnafu {
                uid: ts_uid_selected.to_string(),
            })?;

        // transcode file if necessary
        let dicom_file = into_ts(dicom_file, ts_selected, verbose)?;

        dicom_file
            .write_dataset_with_ts(&mut object_data, ts_selected)
            .map_err(Box::from)
            .context(WriteDatasetSnafu)?;

        let nbytes = cmd_data.len() + object_data.len();

        if verbose {
            let source_display = match &file.source {
                FileSource::Local(path) => path.display().to_string(),
                FileSource::S3(key) => format!("s3://{}", key),
            };
            info!(
                "Sending file {} (~ {} kB), uid={}, sop={}, ts={}",
                source_display,
                nbytes / 1_000,
                &file.sop_instance_uid,
                &file.sop_class_uid,
                ts_uid_selected,
            );
        }

        if nbytes < scu.acceptor_max_pdu_length().saturating_sub(100) as usize {
            let pdu = Pdu::PData {
                data: vec![
                    PDataValue {
                        presentation_context_id: pc_selected.id,
                        value_type: PDataValueType::Command,
                        is_last: true,
                        data: cmd_data,
                    },
                    PDataValue {
                        presentation_context_id: pc_selected.id,
                        value_type: PDataValueType::Data,
                        is_last: true,
                        data: object_data,
                    },
                ],
            };

            scu.send(&pdu).await.map_err(Box::from).context(ScuSnafu)?;
        } else {
            let pdu = Pdu::PData {
                data: vec![PDataValue {
                    presentation_context_id: pc_selected.id,
                    value_type: PDataValueType::Command,
                    is_last: true,
                    data: cmd_data,
                }],
            };

            scu.send(&pdu).await.map_err(Box::from).context(ScuSnafu)?;

            {
                let mut pdata = scu.send_pdata(pc_selected.id);
                pdata.write_all(&object_data).await.unwrap();
                //.whatever_context("Failed to send C-STORE-RQ P-Data")?;
            }
        }

        if verbose {
            debug!("Awaiting response...");
        }

        let rsp_pdu = scu.receive().await.map_err(Box::from).context(ScuSnafu)?;

        match rsp_pdu {
            Pdu::PData { data } => {
                let data_value = &data[0];

                let cmd_obj = InMemDicomObject::read_dataset_with_ts(
                    &data_value.data[..],
                    &dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased(),
                )
                .context(ReadDatasetSnafu)?;
                if verbose {
                    debug!("Full response: {:?}", cmd_obj);
                }
                let status = cmd_obj
                    .element(tags::STATUS)
                    .context(MissingAttributeSnafu { tag: tags::STATUS })?
                    .to_int::<u16>()
                    .context(ConvertFieldSnafu { tag: tags::STATUS })?;
                let storage_sop_instance_uid = file
                    .sop_instance_uid
                    .trim_end_matches(|c: char| c.is_whitespace() || c == '\0');

                match status {
                    // Success
                    0 => {
                        let elapsed = start_time.elapsed();
                        if verbose {
                            info!(
                                "Successfully stored instance {} in {:.2}s",
                                storage_sop_instance_uid,
                                elapsed.as_secs_f64()
                            );
                        }
                        
                        // Increment successful count
                        *successful_count.lock().await += 1;
                        
                        // Emit OnFileSent event
                        let file_path = match &file.source {
                            FileSource::Local(path) => path.display().to_string(),
                            FileSource::S3(key) => format!("s3://{}", key),
                        };
                        
                        if let Some(cb) = &callbacks.on_file_sent {
                            cb.call(Ok(FileSentEvent {
                                message: "File sent successfully".to_string(),
                                data: Some(FileSentData {
                                    file: file_path.clone(),
                                    sop_instance_uid: file.sop_instance_uid.clone(),
                                    sop_class_uid: file.sop_class_uid.clone(),
                                    transfer_syntax: ts_uid_selected.to_string(),
                                    duration_seconds: elapsed.as_secs_f64(),
                                }),
                            }), ThreadsafeFunctionCallMode::NonBlocking);
                        }
                    }
                    // Warning
                    1 | 0x0107 | 0x0116 | 0xB000..=0xBFFF => {
                        warn!(
                            "Possible issue storing instance `{}` (status code {:04X}H)",
                            storage_sop_instance_uid, status
                        );
                    }
                    0xFF00 | 0xFF01 => {
                        warn!(
                            "Possible issue storing instance `{}`: status is pending (status code {:04X}H)",
                            storage_sop_instance_uid, status
                        );
                    }
                    0xFE00 => {
                        error!(
                            "Could not store instance `{}`: operation cancelled",
                            storage_sop_instance_uid
                        );
                        if fail_first {
                            let _ = scu.abort().await;
                            std::process::exit(-2);
                        }
                    }
                    _ => {
                        let elapsed = start_time.elapsed();
                        error!(
                            "Failed to store instance `{}` (status code {:04X}H)",
                            storage_sop_instance_uid, status
                        );
                        
                        // Increment failed count
                        *failed_count.lock().await += 1;
                        
                        // Emit OnFileError event
                        let file_path = match &file.source {
                            FileSource::Local(path) => path.display().to_string(),
                            FileSource::S3(key) => format!("s3://{}", key),
                        };
                        
                        if let Some(cb) = &callbacks.on_file_error {
                            cb.call(Ok(FileErrorEvent {
                                message: format!("Failed to store file (status code {:04X}H)", status),
                                data: Some(FileErrorData {
                                    file: file_path,
                                    error: format!("Status code {:04X}H", status),
                                    sop_instance_uid: Some(storage_sop_instance_uid.to_string()),
                                    sop_class_uid: Some(file.sop_class_uid.clone()),
                                    file_transfer_syntax: Some(file.file_transfer_syntax.clone()),
                                }),
                            }), ThreadsafeFunctionCallMode::NonBlocking);
                        }
                        
                        if fail_first {
                            let _ = scu.abort().await;
                            std::process::exit(-2);
                        }
                    }
                }
            }

            pdu @ Pdu::Unknown { .. }
            | pdu @ Pdu::AssociationRQ { .. }
            | pdu @ Pdu::AssociationAC { .. }
            | pdu @ Pdu::AssociationRJ { .. }
            | pdu @ Pdu::ReleaseRQ
            | pdu @ Pdu::ReleaseRP
            | pdu @ Pdu::AbortRQ { .. } => {
                error!("Unexpected SCP response: {:?}", pdu);
                let _ = scu.abort().await;
                std::process::exit(-2);
            }
        }
    }
    if let Some(pb) = progress_bar.as_ref() {
        pb.lock().await.inc(1)
    };
    Ok(scu)
}

pub async fn inner(
    mut scu: dicom_ul::association::client::AsyncClientAssociation<TcpStream>,
    d_files: Arc<Mutex<Vec<DicomFile>>>,
    s3_bucket: Option<Arc<s3::Bucket>>,
    progress_bar: Option<&Arc<tokio::sync::Mutex<ProgressBar>>>,
    fail_first: bool,
    verbose: bool,
    never_transcode: bool,
    ignore_sop_class: bool,
    callbacks: &StoreCallbacks,
    successful_count: Arc<Mutex<u32>>,
    failed_count: Arc<Mutex<u32>>,
    throttle_delay_ms: u32,
) -> Result<(), Error>
{
    let mut message_id = 1;
    loop {
        let file = {
            let mut files = d_files.lock().await;
            files.pop()
        };
        let mut file = match file {
            Some(file) => file,
            None => break,
        };
        let r: Result<_, Error> = check_presentation_contexts(
            &file,
            scu.presentation_contexts(),
            ignore_sop_class,
            never_transcode,
        );
        match r {
            Ok((pc, ts)) => {
                if verbose {
                    let source_display = match &file.source {
                        FileSource::Local(path) => path.display().to_string(),
                        FileSource::S3(key) => format!("s3://{}", key),
                    };
                    debug!(
                        "{}: Selected presentation context: {:?}",
                        source_display,
                        pc
                    );
                }
                file.pc_selected = Some(pc);
                file.ts_selected = Some(ts);
            }
            Err(e) => {
                error!("{}", Report::from_error(e));
                if fail_first {
                    let _ = scu.abort().await;
                    std::process::exit(-2);
                }
            }
        }
        scu = send_file(scu, file, s3_bucket.as_deref(), message_id, progress_bar, verbose, fail_first, callbacks, successful_count.clone(), failed_count.clone()).await?;
        message_id += 1;
        
        // Apply throttle delay if configured (rate limiting)
        if throttle_delay_ms > 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(throttle_delay_ms as u64)).await;
        }
    }
    let _ = scu.release().await;
    Ok(())
}
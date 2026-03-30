use dicom_core::{dicom_value, DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_encoding::TransferSyntaxIndex;
use dicom_object::{mem::InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::{
    association::client::ClientAssociationOptions,
    pdu::{PDataValue, PDataValueType, Pdu},
};
use tokio::net::TcpStream;

/// Type alias for an open async DICOM client association used for forwarding.
pub type ForwardAssociation = dicom_ul::association::client::AsyncClientAssociation<TcpStream>;

/// Connection config for forwarding received DICOM instances to another PACS via C-STORE.
#[derive(Debug, Clone)]
#[napi(object)]
pub struct ForwardTargetConfig {
    /// Address of the destination PACS in format "host:port" or "AE@host:port"
    pub addr: String,
    /// Calling AE title for the forward association (default: "FORWARD-SCU")
    pub calling_ae_title: Option<String>,
    /// Called AE title on the destination PACS (default: extracted from addr or "ANY-SCP")
    pub called_ae_title: Option<String>,
    /// Maximum PDU length (default: 16384)
    pub max_pdu_length: Option<u32>,
}

/// Error type for forward operations.
#[derive(Debug)]
pub enum ForwardError {
    Association(String),
    SendFailed(String),
    ResponseError(String),
    Other(String),
}

impl std::fmt::Display for ForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Association(s) => write!(f, "Forward association error: {}", s),
            Self::SendFailed(s) => write!(f, "Forward send failed: {}", s),
            Self::ResponseError(s) => write!(f, "Forward response error: {}", s),
            Self::Other(s) => write!(f, "Forward error: {}", s),
        }
    }
}

/// Build a C-STORE-RQ command object.
///
/// Shared between `StoreScu` and the `GetScu` forward path so the wire format
/// is consistent in both directions.
pub fn store_req_command(
    storage_sop_class_uid: &str,
    storage_sop_instance_uid: &str,
    message_id: u16,
) -> InMemDicomObject<StandardDataDictionary> {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            dicom_value!(Str, storage_sop_class_uid),
        ),
        DataElement::new(tags::COMMAND_FIELD, VR::US, dicom_value!(U16, [0x0001])),
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [0x0000])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [0x0000]),
        ),
        DataElement::new(
            tags::AFFECTED_SOP_INSTANCE_UID,
            VR::UI,
            dicom_value!(Str, storage_sop_instance_uid),
        ),
    ])
}

/// Parse `"AE@host:port"` or `"host:port"` into (optional AE, host, port).
fn parse_forward_addr(addr: &str) -> Result<(Option<String>, String, u16), ForwardError> {
    let (ae, host_port) = if let Some((ae, rest)) = addr.split_once('@') {
        (Some(ae.to_string()), rest)
    } else {
        (None, addr)
    };
    let (host, port_str) = host_port
        .rsplit_once(':')
        .ok_or_else(|| ForwardError::Other("Missing port in forward target address".to_string()))?;
    let port: u16 = port_str
        .parse()
        .map_err(|_| ForwardError::Other("Invalid port in forward target address".to_string()))?;
    Ok((ae, host.to_string(), port))
}

/// Open a persistent C-STORE SCU association to the forward target, negotiating
/// each of the given SOP classes. Keep this association alive for the entire
/// lifetime of the C-GET operation so all retrieved instances share one
/// connection.
pub async fn open_forward_association(
    target: &ForwardTargetConfig,
    sop_classes: &[String],
) -> Result<ForwardAssociation, ForwardError> {
    let (ae_from_addr, host, port) = parse_forward_addr(&target.addr)?;

    let calling_ae = target
        .calling_ae_title
        .as_deref()
        .unwrap_or("FORWARD-SCU");

    let called_ae = target
        .called_ae_title
        .as_deref()
        .or(ae_from_addr.as_deref())
        .unwrap_or("ANY-SCP");

    let max_pdu = target.max_pdu_length.unwrap_or(16_384);

    let mut opts = ClientAssociationOptions::new()
        .calling_ae_title(calling_ae)
        .called_ae_title(called_ae)
        .max_pdu_length(max_pdu);

    for sop in sop_classes {
        opts = opts.with_abstract_syntax(sop);
    }

    let addr = format!("{}:{}", host, port);
    opts.establish_async(&addr)
        .await
        .map_err(|e| ForwardError::Association(e.to_string()))
}

/// Forward raw DICOM dataset bytes to an already-open forward association via
/// C-STORE. If the source and destination transfer syntaxes differ the dataset
/// is re-parsed and re-encoded; otherwise the raw bytes are sent as-is to avoid
/// an unnecessary re-encode round-trip.
pub async fn forward_dicom_bytes(
    assoc: &mut ForwardAssociation,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    source_ts_uid: &str,
    dataset_bytes: &[u8],
    message_id: u16,
    verbose: bool,
) -> Result<(), ForwardError> {
    // Find presentation context for this SOP class on the forward association.
    let pc = assoc
        .presentation_contexts()
        .iter()
        .find(|pc| pc.abstract_syntax == sop_class_uid)
        .cloned()
        .ok_or_else(|| {
            ForwardError::Association(format!(
                "No accepted presentation context for SOP class {} on forward association",
                sop_class_uid
            ))
        })?;

    // Build C-STORE-RQ command.
    let cmd = store_req_command(sop_class_uid, sop_instance_uid, message_id);
    let ts_implicit =
        dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased();
    let mut cmd_data = Vec::with_capacity(128);
    cmd.write_dataset_with_ts(&mut cmd_data, &ts_implicit)
        .map_err(|e| ForwardError::SendFailed(format!("Failed to write C-STORE command: {}", e)))?;

    // Re-encode dataset only if the negotiated transfer syntax differs.
    let dest_ts_uid = pc.transfer_syntax.clone();
    let object_data: Vec<u8> = if source_ts_uid == dest_ts_uid {
        dataset_bytes.to_vec()
    } else {
        let src_ts = TransferSyntaxRegistry.get(source_ts_uid).ok_or_else(|| {
            ForwardError::Other(format!("Unknown source transfer syntax: {}", source_ts_uid))
        })?;
        let dst_ts = TransferSyntaxRegistry.get(&dest_ts_uid).ok_or_else(|| {
            ForwardError::Other(format!(
                "Unknown destination transfer syntax: {}",
                dest_ts_uid
            ))
        })?;
        let dataset =
            InMemDicomObject::read_dataset_with_ts(dataset_bytes, src_ts).map_err(|e| {
                ForwardError::Other(format!("Failed to parse dataset for re-encoding: {}", e))
            })?;
        let mut buf = Vec::with_capacity(dataset_bytes.len());
        dataset
            .write_dataset_with_ts(&mut buf, dst_ts)
            .map_err(|e| {
                ForwardError::SendFailed(format!("Failed to re-encode dataset: {}", e))
            })?;
        buf
    };

    let nbytes = cmd_data.len() + object_data.len();

    if verbose {
        println!(
            "→ Forwarding {} ({} kB, ts: {} → {})",
            sop_instance_uid,
            nbytes / 1_000,
            source_ts_uid,
            dest_ts_uid,
        );
    }

    // Send command + data, splitting into fragments if needed.
    if nbytes < assoc.acceptor_max_pdu_length().saturating_sub(100) as usize {
        assoc
            .send(&Pdu::PData {
                data: vec![
                    PDataValue {
                        presentation_context_id: pc.id,
                        value_type: PDataValueType::Command,
                        is_last: true,
                        data: cmd_data,
                    },
                    PDataValue {
                        presentation_context_id: pc.id,
                        value_type: PDataValueType::Data,
                        is_last: true,
                        data: object_data,
                    },
                ],
            })
            .await
            .map_err(|e| ForwardError::SendFailed(e.to_string()))?;
    } else {
        assoc
            .send(&Pdu::PData {
                data: vec![PDataValue {
                    presentation_context_id: pc.id,
                    value_type: PDataValueType::Command,
                    is_last: true,
                    data: cmd_data,
                }],
            })
            .await
            .map_err(|e| ForwardError::SendFailed(e.to_string()))?;

        use tokio::io::AsyncWriteExt;
        let mut pdata = assoc.send_pdata(pc.id);
        pdata.write_all(&object_data).await.map_err(|e| {
            ForwardError::SendFailed(format!("Failed to send data fragment: {}", e))
        })?;
    }

    // Receive and validate C-STORE response.
    let rsp = assoc
        .receive()
        .await
        .map_err(|e| ForwardError::ResponseError(e.to_string()))?;

    match rsp {
        Pdu::PData { data } => {
            if let Some(pv) = data.first() {
                let rsp_obj =
                    InMemDicomObject::read_dataset_with_ts(&pv.data[..], &ts_implicit)
                        .map_err(|e| {
                            ForwardError::ResponseError(format!(
                                "Failed to parse C-STORE response: {}",
                                e
                            ))
                        })?;
                let status = rsp_obj
                    .element(tags::STATUS)
                    .ok()
                    .and_then(|e| e.uint16().ok())
                    .unwrap_or(0xFFFF);
                if status != 0x0000 {
                    return Err(ForwardError::ResponseError(format!(
                        "C-STORE at destination returned status 0x{:04X}",
                        status
                    )));
                }
            }
        }
        _ => {
            return Err(ForwardError::ResponseError(
                "Unexpected PDU in C-STORE response on forward association".to_string(),
            ));
        }
    }

    Ok(())
}

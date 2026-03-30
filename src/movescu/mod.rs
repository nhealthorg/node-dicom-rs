mod move_async;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunction;
use napi_derive::napi;
use std::collections::HashMap;
use std::sync::Arc;
use snafu::Snafu;

pub use move_async::run_move;

#[derive(Debug, Snafu)]
pub enum MoveScuError {
    #[snafu(display("Association error: {}", source))]
    Association {
        source: dicom_ul::association::Error,
    },
    
    #[snafu(display("Invalid query model: {}", model))]
    InvalidQueryModel { model: String },
    
    #[snafu(display("Move operation failed: {}", message))]
    MoveFailed { message: String },
    
    #[snafu(display("IO error: {}", source))]
    Io { source: std::io::Error },
    
    #[snafu(display("{}", message))]
    Other { message: String },
}

impl From<MoveScuError> for napi::Error {
    fn from(err: MoveScuError) -> Self {
        napi::Error::from_reason(err.to_string())
    }
}

/// Query model for C-MOVE operations
#[napi(string_enum)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveQueryModel {
    /// Study Root Query/Retrieve Information Model - MOVE
    StudyRoot,
    /// Patient Root Query/Retrieve Information Model - MOVE
    PatientRoot,
}

impl MoveQueryModel {
    pub fn sop_class_uid(&self) -> &'static str {
        use dicom_dictionary_std::uids::*;
        match self {
            MoveQueryModel::StudyRoot => STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE,
            MoveQueryModel::PatientRoot => PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE,
        }
    }
}

/// Options for creating a MoveScu instance
#[napi(object)]
#[derive(Debug, Clone)]
pub struct MoveScuOptions {
    /// Address of the PACS server in format "AE@host:port" or "host:port"
    pub addr: String,
    /// Application Entity title of this SCU (default: "MOVE-SCU")
    pub calling_ae_title: Option<String>,
    /// Application Entity title of the remote SCP (default: extracted from addr or "ANY-SCP")
    pub called_ae_title: Option<String>,
    /// Maximum PDU length (default: 16384)
    pub max_pdu_length: Option<u32>,
    /// Enable verbose logging
    pub verbose: Option<bool>,
}

/// Event emitted for each sub-operation (file being moved)
#[napi(object)]
#[derive(Debug, Clone)]
pub struct MoveSubOperationEvent {
    pub message: String,
    pub data: Option<MoveSubOperationData>,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct MoveSubOperationData {
    /// Number of remaining sub-operations
    pub remaining: u32,
    /// Number of completed sub-operations
    pub completed: u32,
    /// Number of failed sub-operations
    pub failed: u32,
    /// Number of warning sub-operations
    pub warning: u32,
}

/// Event emitted when the C-MOVE operation completes
#[napi(object)]
#[derive(Debug, Clone)]
pub struct MoveCompletedEvent {
    pub message: String,
    pub data: Option<MoveCompletedData>,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct MoveCompletedData {
    /// Total number of sub-operations
    pub total: u32,
    /// Number of completed sub-operations
    pub completed: u32,
    /// Number of failed sub-operations
    pub failed: u32,
    /// Number of warning sub-operations
    pub warning: u32,
    /// Duration in seconds
    pub duration_seconds: f64,
}

/// Result of a C-MOVE operation
#[napi(object)]
#[derive(Debug, Clone)]
pub struct MoveResult {
    /// Total number of sub-operations (instances to move)
    pub total: u32,
    /// Number of completed sub-operations
    pub completed: u32,
    /// Number of failed sub-operations
    pub failed: u32,
    /// Number of warning sub-operations
    pub warning: u32,
}

/// DICOM C-MOVE SCU (Service Class User) for retrieving studies/series/instances from a PACS
/// 
/// # Example
/// 
/// ```javascript
/// const { MoveScu } = require('@nuxthealth/node-dicom');
/// 
/// const moveScu = new MoveScu({
///     addr: '127.0.0.1:4242',
///     callingAeTitle: 'MY-SCU',
///     calledAeTitle: 'ORTHANC',
///     verbose: true
/// });
/// 
/// // Move a study to destination AE
/// const result = await moveScu.moveStudy({
///     query: {
///         QueryRetrieveLevel: 'STUDY',
///         StudyInstanceUID: '1.2.3.4.5'
///     },
///     moveDestination: 'DESTINATION-AE',
///     queryModel: 'StudyRoot',
///     onSubOperation: (err, event) => {
///         console.log(`Progress: ${event.data?.completed} of ${event.data?.remaining + event.data?.completed}`);
///     },
///     onCompleted: (err, event) => {
///         console.log(`Moved ${event.data?.completed} instances`);
///     }
/// });
/// 
/// console.log(`Moved ${result.completed} of ${result.total} instances`);
/// ```
#[napi]
pub struct MoveScu {
    addr: String,
    calling_ae_title: String,
    called_ae_title: String,
    max_pdu_length: u32,
    verbose: bool,
}

#[napi]
impl MoveScu {
    /// Create a new MoveScu instance
    #[napi(constructor)]
    pub fn new(options: MoveScuOptions) -> Result<Self> {
        let calling_ae_title = options
            .calling_ae_title
            .unwrap_or_else(|| "MOVE-SCU".to_string());
        
        // Try to extract AE title from address if not provided
        let called_ae_title = options.called_ae_title.unwrap_or_else(|| {
            if let Some((ae, _)) = options.addr.split_once('@') {
                ae.to_string()
            } else {
                "ANY-SCP".to_string()
            }
        });

        Ok(MoveScu {
            addr: options.addr,
            calling_ae_title,
            called_ae_title,
            max_pdu_length: options.max_pdu_length.unwrap_or(16384),
            verbose: options.verbose.unwrap_or(false),
        })
    }

    /// Perform a C-MOVE operation
    /// 
    /// @param options - Configuration object containing query, moveDestination, queryModel, and callbacks
    /// @returns Promise<MoveResult>
    #[napi(
        ts_args_type = "options: { query: Record<string, string>, moveDestination: string, queryModel?: 'StudyRoot' | 'PatientRoot', onSubOperation?: (err: Error | null, event: MoveSubOperationEvent) => void, onCompleted?: (err: Error | null, event: MoveCompletedEvent) => void }"
    )]
    pub fn move_study(
        &self,
        options: Object,
    ) -> Result<AsyncTask<MoveHandler>> {
        // Extract query object
        let query: Object = options
            .get("query")?
            .ok_or_else(|| napi::Error::from_reason("Missing required 'query' property"))?;
        let mut query_map = HashMap::new();
        if let Ok(keys) = query.get_property_names() {
            let len = keys.get_array_length().unwrap_or(0);
            for i in 0..len {
                if let Ok(key) = keys.get_element::<String>(i) {
                    if let Ok(value) = query.get_named_property::<String>(&key) {
                        query_map.insert(key, value);
                    }
                }
            }
        }

        // Extract move destination
        let move_destination: String = options
            .get("moveDestination")?
            .ok_or_else(|| napi::Error::from_reason("Missing required 'moveDestination' property"))?;

        // Extract other options
        let query_model_str: Option<String> = options.get("queryModel")?;
        let query_model_enum = match query_model_str.as_deref() {
            Some("PatientRoot") => MoveQueryModel::PatientRoot,
            _ => MoveQueryModel::StudyRoot, // Default
        };
        
        let on_sub_operation: Option<ThreadsafeFunction<MoveSubOperationEvent>> = options.get("onSubOperation")?;
        let on_completed: Option<ThreadsafeFunction<MoveCompletedEvent>> = options.get("onCompleted")?;

        Ok(AsyncTask::new(MoveHandler {
            addr: self.addr.clone(),
            calling_ae_title: self.calling_ae_title.clone(),
            called_ae_title: self.called_ae_title.clone(),
            max_pdu_length: self.max_pdu_length,
            verbose: self.verbose,
            query: query_map,
            move_destination,
            query_model: query_model_enum,
            on_sub_operation: on_sub_operation.map(Arc::new),
            on_completed: on_completed.map(Arc::new),
        }))
    }
}

/// Arguments for the async C-MOVE operation
pub struct MoveArgs {
    pub addr: String,
    pub calling_ae_title: String,
    pub called_ae_title: String,
    pub max_pdu_length: u32,
    pub verbose: bool,
    pub query: HashMap<String, String>,
    pub move_destination: String,
    pub query_model: MoveQueryModel,
}

/// Callbacks for C-MOVE events
pub struct MoveCallbacks {
    pub on_sub_operation: Option<Arc<ThreadsafeFunction<MoveSubOperationEvent>>>,
    pub on_completed: Option<Arc<ThreadsafeFunction<MoveCompletedEvent>>>,
}

pub struct MoveHandler {
    addr: String,
    calling_ae_title: String,
    called_ae_title: String,
    max_pdu_length: u32,
    verbose: bool,
    query: HashMap<String, String>,
    move_destination: String,
    query_model: MoveQueryModel,
    on_sub_operation: Option<Arc<ThreadsafeFunction<MoveSubOperationEvent>>>,
    on_completed: Option<Arc<ThreadsafeFunction<MoveCompletedEvent>>>,
}

#[napi]
impl Task for MoveHandler {
    type Output = MoveResult;
    type JsValue = MoveResult;

    fn compute(&mut self) -> Result<Self::Output> {
        let args = MoveArgs {
            addr: self.addr.clone(),
            calling_ae_title: self.calling_ae_title.clone(),
            called_ae_title: self.called_ae_title.clone(),
            max_pdu_length: self.max_pdu_length,
            verbose: self.verbose,
            query: self.query.clone(),
            move_destination: self.move_destination.clone(),
            query_model: self.query_model,
        };

        let callbacks = MoveCallbacks {
            on_sub_operation: self.on_sub_operation.clone(),
            on_completed: self.on_completed.clone(),
        };

        // Create a tokio runtime for the async operation
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| napi::Error::from_reason(format!("Failed to create runtime: {}", e)))?;

        runtime
            .block_on(run_move(args, callbacks))
            .map_err(|e| napi::Error::from(e))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

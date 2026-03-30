mod get_async;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunction;
use napi_derive::napi;
use std::collections::HashMap;
use std::sync::Arc;
use snafu::Snafu;
use crate::utils::S3Config;

pub use get_async::run_get;

#[derive(Debug, Snafu)]
pub enum GetScuError {
    #[snafu(display("Association error: {}", source))]
    Association {
        source: dicom_ul::association::Error,
    },
    
    #[snafu(display("Invalid query model: {}", model))]
    InvalidQueryModel { model: String },
    
    #[snafu(display("Get operation failed: {}", message))]
    GetFailed { message: String },
    
    #[snafu(display("IO error: {}", source))]
    Io { source: std::io::Error },
    
    #[snafu(display("{}", message))]
    Other { message: String },
}

impl From<GetScuError> for napi::Error {
    fn from(err: GetScuError) -> Self {
        napi::Error::from_reason(err.to_string())
    }
}

/// Query model for C-GET operations
#[napi(string_enum)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GetQueryModel {
    /// Study Root Query/Retrieve Information Model - GET
    StudyRoot,
    /// Patient Root Query/Retrieve Information Model - GET
    PatientRoot,
}

impl GetQueryModel {
    pub fn sop_class_uid(&self) -> &'static str {
        use dicom_dictionary_std::uids::*;
        match self {
            GetQueryModel::StudyRoot => STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_GET,
            GetQueryModel::PatientRoot => PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_GET,
        }
    }
}

/// Storage backend type for received DICOM files
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetStorageBackend {
    /// Store files on local filesystem
    Filesystem,
    /// Store files in S3-compatible object storage
    S3,
}

/// Options for creating a GetScu instance
#[napi(object)]
#[derive(Debug, Clone)]
pub struct GetScuOptions {
    /// Address of the PACS server in format "AE@host:port" or "host:port"
    pub addr: String,
    /// Application Entity title of this SCU (default: "GET-SCU")
    pub calling_ae_title: Option<String>,
    /// Application Entity title of the remote SCP (default: extracted from addr or "ANY-SCP")
    pub called_ae_title: Option<String>,
    /// Maximum PDU length (default: 16384)
    pub max_pdu_length: Option<u32>,
    /// Enable verbose logging
    pub verbose: Option<bool>,
    /// Base directory for filesystem storage
    pub out_dir: Option<String>,
    /// Storage backend type (default: Filesystem)
    pub storage_backend: Option<GetStorageBackend>,
    /// S3 configuration (required when storageBackend is S3)
    pub s3_config: Option<S3Config>,
}

/// Event emitted for each sub-operation (file being retrieved)
#[napi(object)]
#[derive(Debug, Clone)]
pub struct GetSubOperationEvent {
    pub message: String,
    pub data: Option<GetSubOperationData>,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct GetSubOperationData {
    /// Number of remaining sub-operations
    pub remaining: u32,
    /// Number of completed sub-operations
    pub completed: u32,
    /// Number of failed sub-operations
    pub failed: u32,
    /// Number of warning sub-operations
    pub warning: u32,
    /// Current file being stored (if available)
    pub file: Option<String>,
    /// SOP Instance UID of current file
    pub sop_instance_uid: Option<String>,
}

/// Event emitted when the C-GET operation completes
#[napi(object)]
#[derive(Debug, Clone)]
pub struct GetCompletedEvent {
    pub message: String,
    pub data: Option<GetCompletedData>,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct GetCompletedData {
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

/// Result of a C-GET operation
#[napi(object)]
#[derive(Debug, Clone)]
pub struct GetResult {
    /// Total number of sub-operations (instances to retrieve)
    pub total: u32,
    /// Number of completed sub-operations
    pub completed: u32,
    /// Number of failed sub-operations
    pub failed: u32,
    /// Number of warning sub-operations
    pub warning: u32,
}

/// DICOM C-GET SCU (Service Class User) for retrieving studies/series/instances from a PACS
/// 
/// # Example
/// 
/// ```javascript
/// const { GetScu } = require('@nuxthealth/node-dicom');
/// 
/// const getScu = new GetScu({
///     addr: '127.0.0.1:4242',
///     callingAeTitle: 'MY-SCU',
///     calledAeTitle: 'ORTHANC',
///     outDir: './retrieved-studies',
///     storageBackend: 'Filesystem',
///     verbose: true
/// });
/// 
/// // Retrieve a study to local filesystem
/// const result = await getScu.getStudy({
///     query: {
///         QueryRetrieveLevel: 'STUDY',
///         StudyInstanceUID: '1.2.3.4.5'
///     },
///     queryModel: 'StudyRoot',
///     onSubOperation: (err, event) => {
///         console.log(`Progress: ${event.data?.completed} of ${event.data?.total}`);
///     },
///     onCompleted: (err, event) => {
///         console.log(`Retrieved ${event.data?.completed} instances`);
///     }
/// });
/// 
/// console.log(`Retrieved ${result.completed} of ${result.total} instances`);
/// ```
#[napi]
pub struct GetScu {
    addr: String,
    calling_ae_title: String,
    called_ae_title: String,
    max_pdu_length: u32,
    verbose: bool,
    out_dir: Option<String>,
    storage_backend: GetStorageBackend,
    s3_config: Option<S3Config>,
}

#[napi]
impl GetScu {
    /// Create a new GetScu instance
    #[napi(constructor)]
    pub fn new(options: GetScuOptions) -> Result<Self> {
        let calling_ae_title = options
            .calling_ae_title
            .unwrap_or_else(|| "GET-SCU".to_string());
        
        // Try to extract AE title from address if not provided
        let called_ae_title = options.called_ae_title.unwrap_or_else(|| {
            if let Some((ae, _)) = options.addr.split_once('@') {
                ae.to_string()
            } else {
                "ANY-SCP".to_string()
            }
        });

        let storage_backend = options.storage_backend.unwrap_or(GetStorageBackend::Filesystem);

        Ok(GetScu {
            addr: options.addr,
            calling_ae_title,
            called_ae_title,
            max_pdu_length: options.max_pdu_length.unwrap_or(16384),
            verbose: options.verbose.unwrap_or(false),
            out_dir: options.out_dir,
            storage_backend,
            s3_config: options.s3_config,
        })
    }

    /// Perform a C-GET operation
    /// 
    /// @param options - Configuration object containing query, storage settings, queryModel, and callbacks
    /// @returns Promise<GetResult>
    #[napi(
        ts_args_type = "options: { query: Record<string, string>, queryModel?: 'StudyRoot' | 'PatientRoot', onSubOperation?: (err: Error | null, event: GetSubOperationEvent) => void, onCompleted?: (err: Error | null, event: GetCompletedEvent) => void }"
    )]
    pub fn get_study(
        &self,
        options: Object,
    ) -> Result<AsyncTask<GetHandler>> {
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

        // Extract query model
        let query_model_str: Option<String> = options.get("queryModel")?;
        let query_model_enum = match query_model_str.as_deref() {
            Some("PatientRoot") => GetQueryModel::PatientRoot,
            _ => GetQueryModel::StudyRoot, // Default
        };
        let on_sub_operation: Option<ThreadsafeFunction<GetSubOperationEvent>> = options.get("onSubOperation")?;
        let on_completed: Option<ThreadsafeFunction<GetCompletedEvent>> = options.get("onCompleted")?;

        Ok(AsyncTask::new(GetHandler {
            addr: self.addr.clone(),
            calling_ae_title: self.calling_ae_title.clone(),
            called_ae_title: self.called_ae_title.clone(),
            max_pdu_length: self.max_pdu_length,
            verbose: self.verbose,
            query: query_map,
            out_dir: self.out_dir.clone(),
            storage_backend: self.storage_backend.clone(),
            s3_config: self.s3_config.clone(),
            query_model: query_model_enum,
            on_sub_operation: on_sub_operation.map(Arc::new),
            on_completed: on_completed.map(Arc::new),
        }))
    }
}

/// Arguments for the async C-GET operation
pub struct GetArgs {
    pub addr: String,
    pub calling_ae_title: String,
    pub called_ae_title: String,
    pub max_pdu_length: u32,
    pub verbose: bool,
    pub query: HashMap<String, String>,
    pub out_dir: Option<String>,
    pub storage_backend: GetStorageBackend,
    pub s3_config: Option<S3Config>,
    pub query_model: GetQueryModel,
}

/// Callbacks for C-GET events
pub struct GetCallbacks {
    pub on_sub_operation: Option<Arc<ThreadsafeFunction<GetSubOperationEvent>>>,
    pub on_completed: Option<Arc<ThreadsafeFunction<GetCompletedEvent>>>,
}

pub struct GetHandler {
    addr: String,
    calling_ae_title: String,
    called_ae_title: String,
    max_pdu_length: u32,
    verbose: bool,
    query: HashMap<String, String>,
    out_dir: Option<String>,
    storage_backend: GetStorageBackend,
    s3_config: Option<S3Config>,
    query_model: GetQueryModel,
    on_sub_operation: Option<Arc<ThreadsafeFunction<GetSubOperationEvent>>>,
    on_completed: Option<Arc<ThreadsafeFunction<GetCompletedEvent>>>,
}

#[napi]
impl Task for GetHandler {
    type Output = GetResult;
    type JsValue = GetResult;

    fn compute(&mut self) -> Result<Self::Output> {
        let args = GetArgs {
            addr: self.addr.clone(),
            calling_ae_title: self.calling_ae_title.clone(),
            called_ae_title: self.called_ae_title.clone(),
            max_pdu_length: self.max_pdu_length,
            verbose: self.verbose,
            query: self.query.clone(),
            out_dir: self.out_dir.clone(),
            storage_backend: self.storage_backend.clone(),
            s3_config: self.s3_config.clone(),
            query_model: self.query_model,
        };

        let callbacks = GetCallbacks {
            on_sub_operation: self.on_sub_operation.clone(),
            on_completed: self.on_completed.clone(),
        };

        // Create a tokio runtime for the async operation
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| napi::Error::from_reason(format!("Failed to create runtime: {}", e)))?;

        runtime
            .block_on(run_get(args, callbacks))
            .map_err(|e| napi::Error::from(e))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

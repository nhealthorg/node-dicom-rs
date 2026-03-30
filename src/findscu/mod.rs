use napi::bindgen_prelude::{AsyncTask, JsObjectValue, Object};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, Result as NapiResult};
use serde::{Deserialize, Serialize};
use dicom_core::{dicom_value, DataDictionary, DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::mem::InMemDicomObject;
use snafu::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

mod find_async;
mod query_builder;

pub use query_builder::QueryBuilder;

/**
 * DICOM Find SCU (C-FIND) Client
 * 
 * Performs DICOM C-FIND queries to search for studies, patients, or modality worklist entries
 * in a DICOM archive. Supports the following query/retrieve information models:
 * 
 * - **Study Root** (default): Query/retrieve studies
 * - **Patient Root**: Query/retrieve patients
 * - **Modality Worklist**: Query scheduled procedures
 * 
 * ## Features
 * - Multiple query/retrieve information models
 * - Flexible query syntax using tag names or hex codes
 * - Event-driven result streaming
 * - Support for wildcards in queries
 * - Automatic query level inference
 * 
 * ## Usage
 * 
 * @example
 * ```typescript
 * import { FindScu } from '@nuxthealth/node-dicom';
 * 
 * // Basic study search
 * const finder = new FindScu({
 *   addr: 'PACS@192.168.1.100:104',
 *   calling_ae_title: 'MY-WORKSTATION'
 * });
 * 
 * const results = await finder.find({
 *   query: {
 *     PatientName: 'DOE^JOHN',
 *     StudyDate: '20240101-20240131',
 *     Modality: 'CT'
 *   },
 *   onResult: (err, result) => {
 *     console.log('Found study:', result.data?.StudyInstanceUID);
 *   }
 * });
 * ```
 * 
 * @example
 * ```typescript
 * // Patient root query
 * const results = await finder.find({
 *   queryModel: 'PatientRoot',
 *   query: {
 *     PatientID: 'PAT123',
 *     PatientBirthDate: '19900101'
 *   }
 * });
 * ```
 * 
 * @example
 * ```typescript
 * // Modality worklist query
 * const results = await finder.find({
 *   queryModel: 'ModalityWorklist',
 *   query: {
 *     'ScheduledProcedureStepSequence.Modality': 'MR',
 *     'ScheduledProcedureStepSequence.ScheduledProcedureStepStartDate': '20240315'
 *   }
 * });
 * ```
 */
#[napi]
pub struct FindScu {
    /// Socket address to Find SCP (e.g., "FIND-SCP@127.0.0.1:104")
    addr: String,
    /// Calling Application Entity title (default: "FIND-SCU")
    calling_ae_title: String,
    /// Called Application Entity title (optional, overrides AE title in address)
    called_ae_title: Option<String>,
    /// Maximum PDU length (4096..=131072, default: 16384)
    max_pdu_length: u32,
    /// Enable verbose logging
    verbose: bool,
}

/**
 * Query/Retrieve Information Model types for C-FIND operations.
 * 
 * Determines which DICOM information model to use for the query.
 */
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryModel {
    /// Study Root Query/Retrieve Information Model (default)
    StudyRoot,
    /// Patient Root Query/Retrieve Information Model
    PatientRoot,
    /// Modality Worklist Information Model
    ModalityWorklist,
}

/**
 * Options for configuring a DICOM Find SCU client.
 */
#[napi(object)]
pub struct FindScuOptions {
    /// Address of the Find SCP, optionally with AE title (e.g., "FIND-SCP@127.0.0.1:104" or "192.168.1.100:104")
    pub addr: String,
    /// Calling Application Entity title for this SCU (default: "FIND-SCU")
    pub calling_ae_title: Option<String>,
    /// Called Application Entity title, overrides AE title in address if present (default: "ANY-SCP")
    pub called_ae_title: Option<String>,
    /// Maximum PDU length in bytes, range 4096-131072 (default: 16384)
    pub max_pdu_length: Option<u32>,
    /// Enable verbose logging (default: false)
    pub verbose: Option<bool>,
}

/**
 * Callbacks for C-FIND query events.
 */
pub struct FindCallbackOptions {
    pub on_result: Option<ThreadsafeFunction<FindResultEvent, ()>>,
    pub on_completed: Option<ThreadsafeFunction<FindCompletedEvent, ()>>,
}

/**
 * Event emitted for each C-FIND result.
 */
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindResultEvent {
    pub message: String,
    pub data: Option<HashMap<String, String>>,
}

/**
 * Event emitted when C-FIND query completes.
 */
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindCompletedEvent {
    pub message: String,
    pub data: Option<FindCompletedData>,
}

#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindCompletedData {
    pub total_results: u32,
    pub duration_seconds: f64,
}

/**
 * Individual C-FIND query result.
 */
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindResult {
    /// DICOM attributes as key-value pairs
    pub attributes: HashMap<String, String>,
}

#[napi]
impl FindScu {
    /**
     * Create a new DICOM Find SCU client instance.
     * 
     * @param options - Client configuration options
     * @returns New FindScu instance
     * 
     * @example
     * ```typescript
     * const finder = new FindScu({
     *   addr: 'PACS@192.168.1.100:104',
     *   calling_ae_title: 'MY-SCU',
     *   verbose: true
     * });
     * ```
     */
    #[napi(constructor)]
    pub fn new(options: FindScuOptions) -> Self {
        let calling_ae_title = options.calling_ae_title.unwrap_or_else(|| "FIND-SCU".to_string());
        let max_pdu_length = options.max_pdu_length.unwrap_or(16384);
        let verbose = options.verbose.unwrap_or(false);

        Self {
            addr: options.addr,
            calling_ae_title,
            called_ae_title: options.called_ae_title,
            max_pdu_length,
            verbose,
        }
    }

    /**
     * Execute a C-FIND query to search for DICOM entities.
     * 
     * Performs a DICOM C-FIND operation with the specified query parameters.
     * Results are streamed via callbacks and also returned as an array.
     * 
     * @param options - Configuration object containing query, queryModel, and callbacks
     * @returns Array of all matching results
     * @throws Error if query fails or connection cannot be established
     * 
     * @example
     * ```typescript
     * // Search for studies by patient name
     * const results = await finder.find({
     *   query: {
     *     PatientName: 'DOE^JOHN',
     *     StudyDate: '20240101-',
     *     Modality: 'CT'
     *   },
     *   queryModel: 'StudyRoot',
     *   onResult: (err, result) => {
     *     if (!err) {
     *       console.log('Study UID:', result.data?.StudyInstanceUID);
     *     }
     *   },
     *   onCompleted: (err, event) => {
     *     console.log(`Found ${event.data?.totalResults} studies`);
     *   }
     * });
     * ```
     * 
     * @example
     * ```typescript
     * // Simple query without callbacks
     * const results = await finder.find({
     *   query: {
     *     StudyInstanceUID: '*',
     *     PatientName: '',
     *     StudyDate: '',
     *     StudyDescription: ''
     *   }
     * });
     * ```
     */
    #[napi(
        ts_args_type = "options: { query: Record<string, string>, queryModel?: 'StudyRoot' | 'PatientRoot' | 'ModalityWorklist', onResult?: (err: Error | null, event: FindResultEvent) => void, onCompleted?: (err: Error | null, event: FindCompletedEvent) => void }"
    )]
    pub fn find(
        &self,
        options: Object,
    ) -> NapiResult<AsyncTask<FindScuHandler>> {
        // Extract query object
        let query: Object = options
            .get("query")?
            .ok_or_else(|| napi::Error::from_reason("Missing required 'query' property"))?;
        let mut query_map: HashMap<String, String> = HashMap::new();
        let keys = query.get_property_names()?;
        let len = keys.get_array_length()?;
        
        for i in 0..len {
            let key: String = keys.get_element(i)?;
            if let Ok(value) = query.get_named_property::<String>(&key) {
                query_map.insert(key, value);
            }
        }

        // Extract other options
        let query_model_str: Option<String> = options.get("queryModel")?;
        let query_model = match query_model_str.as_deref() {
            Some("PatientRoot") => QueryModel::PatientRoot,
            Some("ModalityWorklist") => QueryModel::ModalityWorklist,
            _ => QueryModel::StudyRoot, // Default
        };
        
        let on_result: Option<ThreadsafeFunction<FindResultEvent, ()>> = options.get("onResult")?;
        let on_completed: Option<ThreadsafeFunction<FindCompletedEvent, ()>> = options.get("onCompleted")?;

        Ok(AsyncTask::new(FindScuHandler {
            addr: self.addr.clone(),
            calling_ae_title: self.calling_ae_title.clone(),
            called_ae_title: self.called_ae_title.clone(),
            max_pdu_length: self.max_pdu_length,
            verbose: self.verbose,
            query_model,
            query_params: query_map,
            on_result: on_result.map(Arc::new),
            on_completed: on_completed.map(Arc::new),
        }))
    }

    /**
     * Execute a C-FIND query using a QueryBuilder.
     * 
     * This is a more intuitive alternative to the `find()` method that accepts
     * a pre-built query with type-safe methods.
     * 
     * @param query - QueryBuilder instance with configured search criteria
     * @param callbacks - Optional callbacks for result and completion events
     * @returns Array of all matching results
     * @throws Error if query fails or connection cannot be established
     * 
     * @example
     * ```typescript
     * const query = QueryBuilder.study()
     *   .patientName("DOE^JOHN")
     *   .studyDateRange("20240101", "20240131")
     *   .modality("CT")
     *   .includeAllReturnAttributes();
     * 
     * const results = await finder.findWithQuery(query, {
     *   onResult: (err, result) => {
     *     if (!err) console.log('Found:', result.data);
     *   },
     *   onCompleted: (err, event) => {
     *     console.log(`Query complete: ${event.data?.totalResults} results`);
     *   }
     * });
     * ```
     */
    #[napi(
        ts_args_type = "query: QueryBuilder, callbacks?: { onResult?: (err: Error | null, event: FindResultEvent) => void, onCompleted?: (err: Error | null, event: FindCompletedEvent) => void }"
    )]
    pub fn find_with_query(
        &self,
        query: &QueryBuilder,
        callbacks: Option<Object>,
    ) -> NapiResult<AsyncTask<FindScuHandler>> {
        let (on_result, on_completed) = if let Some(cbs) = callbacks {
            (
                cbs.get::<ThreadsafeFunction<FindResultEvent, ()>>("onResult")?,
                cbs.get::<ThreadsafeFunction<FindCompletedEvent, ()>>("onCompleted")?,
            )
        } else {
            (None, None)
        };

        Ok(AsyncTask::new(FindScuHandler {
            addr: self.addr.clone(),
            calling_ae_title: self.calling_ae_title.clone(),
            called_ae_title: self.called_ae_title.clone(),
            max_pdu_length: self.max_pdu_length,
            verbose: self.verbose,
            query_model: query.query_model(),
            query_params: query.params(),
            on_result: on_result.map(Arc::new),
            on_completed: on_completed.map(Arc::new),
        }))
    }
}

struct FindScuHandler {
    addr: String,
    calling_ae_title: String,
    called_ae_title: Option<String>,
    max_pdu_length: u32,
    verbose: bool,
    query_model: QueryModel,
    query_params: HashMap<String, String>,
    on_result: Option<Arc<ThreadsafeFunction<FindResultEvent, ()>>>,
    on_completed: Option<Arc<ThreadsafeFunction<FindCompletedEvent, ()>>>,
}

#[napi]
impl napi::Task for FindScuHandler {
    type JsValue = Vec<FindResult>;
    type Output = Vec<FindResult>;

    fn compute(&mut self) -> napi::bindgen_prelude::Result<Self::Output> {
        let args = FindScuArgs {
            addr: self.addr.clone(),
            calling_ae_title: self.calling_ae_title.clone(),
            called_ae_title: self.called_ae_title.clone(),
            max_pdu_length: self.max_pdu_length,
            verbose: self.verbose,
            query_model: self.query_model.clone(),
            query_params: self.query_params.clone(),
        };

        let callbacks = FindCallbacks {
            on_result: self.on_result.clone(),
            on_completed: self.on_completed.clone(),
        };

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| napi::Error::from_reason(format!("Failed to create runtime: {}", e)))?;

        runtime.block_on(async {
            find_async::run_find(args, callbacks).await
                .map_err(|e| napi::Error::from_reason(format!("Find operation failed: {}", e)))
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> napi::bindgen_prelude::Result<Self::JsValue> {
        Ok(output)
    }
}

pub(crate) struct FindScuArgs {
    pub addr: String,
    pub calling_ae_title: String,
    pub called_ae_title: Option<String>,
    pub max_pdu_length: u32,
    pub verbose: bool,
    pub query_model: QueryModel,
    pub query_params: HashMap<String, String>,
}

pub(crate) struct FindCallbacks {
    pub on_result: Option<Arc<ThreadsafeFunction<FindResultEvent, ()>>>,
    pub on_completed: Option<Arc<ThreadsafeFunction<FindCompletedEvent, ()>>>,
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Could not initialize SCU: {}", source))]
    InitScu {
        source: dicom_ul::association::Error,
    },
    #[snafu(display("Could not construct DICOM command: {}", source))]
    CreateCommand {
        source: dicom_object::WriteError,
    },
    #[snafu(display("Could not read DICOM response: {}", source))]
    ReadResponse {
        source: dicom_object::ReadError,
    },
    #[snafu(whatever, display("{}", message))]
    Other {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + Send + Sync>, Some)))]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

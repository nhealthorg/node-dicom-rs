use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::RwLock;
use warp::Filter;

lazy_static::lazy_static! {
    // Global tokio runtime
    static ref RUNTIME: Runtime = Runtime::new().unwrap();
}

// ============================================================================
// DICOM JSON Model (PS3.18 Section F.2)
// ============================================================================

/// DICOM JSON Value representation (PS3.18 Section F.2.2)
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DicomJsonValue {
    /// Value Representation (e.g., "PN", "DA", "TM", "UI", "LO", "SH")
    pub vr: String,
    /// Array of values - always an array even for single values
    #[serde(rename = "Value")]
    pub value: Option<Vec<String>>,
}

/// DICOM JSON Attribute - a single tag with its value
/// Tag is the key in the parent object (e.g., "00100010")
pub type DicomJsonAttributes = HashMap<String, DicomJsonValue>;

// ============================================================================
// QIDO-RS Query Parameters (PS3.18 Table 10.6.1-2)
// ============================================================================

/// Search for Studies - All Studies
/// Endpoint: /studies
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchForStudiesQuery {
    // Standard query parameters
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub fuzzymatching: Option<bool>,
    pub includefield: Option<String>,
    
    // Study-level matching attributes (Table 10.6.1-2)
    #[serde(rename = "StudyDate")]
    pub study_date: Option<String>,
    #[serde(rename = "StudyTime")]
    pub study_time: Option<String>,
    #[serde(rename = "AccessionNumber")]
    pub accession_number: Option<String>,
    #[serde(rename = "ModalitiesInStudy")]
    pub modalities_in_study: Option<String>,
    #[serde(rename = "ReferringPhysicianName")]
    pub referring_physician_name: Option<String>,
    #[serde(rename = "PatientName")]
    pub patient_name: Option<String>,
    #[serde(rename = "PatientID")]
    pub patient_id: Option<String>,
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: Option<String>,
    #[serde(rename = "StudyID")]
    pub study_id: Option<String>,
}

/// Search for Series - All Series in a Study
/// Endpoint: /studies/{StudyInstanceUID}/series
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchForSeriesQuery {
    // Path parameter
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: String,
    
    // Standard query parameters
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub fuzzymatching: Option<bool>,
    pub includefield: Option<String>,
    
    // Series-level matching attributes
    #[serde(rename = "Modality")]
    pub modality: Option<String>,
    #[serde(rename = "SeriesInstanceUID")]
    pub series_instance_uid: Option<String>,
    #[serde(rename = "SeriesNumber")]
    pub series_number: Option<String>,
    #[serde(rename = "PerformedProcedureStepStartDate")]
    pub performed_procedure_step_start_date: Option<String>,
    #[serde(rename = "PerformedProcedureStepStartTime")]
    pub performed_procedure_step_start_time: Option<String>,
    #[serde(rename = "RequestAttributeSequence.ScheduledProcedureStepID")]
    pub scheduled_procedure_step_id: Option<String>,
    #[serde(rename = "RequestAttributeSequence.RequestedProcedureID")]
    pub requested_procedure_id: Option<String>,
}

/// Search for Instances - All Instances in a Study
/// Endpoint: /studies/{StudyInstanceUID}/instances
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchForStudyInstancesQuery {
    // Path parameter
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: String,
    
    // Standard query parameters
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub fuzzymatching: Option<bool>,
    pub includefield: Option<String>,
    
    // Instance-level matching attributes
    #[serde(rename = "SOPClassUID")]
    pub sop_class_uid: Option<String>,
    #[serde(rename = "SOPInstanceUID")]
    pub sop_instance_uid: Option<String>,
    #[serde(rename = "InstanceNumber")]
    pub instance_number: Option<String>,
}

/// Search for Instances - All Instances in a Series
/// Endpoint: /studies/{StudyInstanceUID}/series/{SeriesInstanceUID}/instances
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchForSeriesInstancesQuery {
    // Path parameters
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: String,
    #[serde(rename = "SeriesInstanceUID")]
    pub series_instance_uid: String,
    
    // Standard query parameters
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub fuzzymatching: Option<bool>,
    pub includefield: Option<String>,
    
    // Instance-level matching attributes
    #[serde(rename = "SOPClassUID")]
    pub sop_class_uid: Option<String>,
    #[serde(rename = "SOPInstanceUID")]
    pub sop_instance_uid: Option<String>,
    #[serde(rename = "InstanceNumber")]
    pub instance_number: Option<String>,
}

// ============================================================================
// Response Types - Properly typed for each query level
// ============================================================================

/// Study-level attributes returned by Search for Studies
/// Contains all Study-level tags as per PS3.18 Table 10.6.1-2
pub type StudyAttributes = DicomJsonAttributes;

/// Series-level attributes returned by Search for Series
/// Contains all Series-level tags
pub type SeriesAttributes = DicomJsonAttributes;

/// Instance-level attributes returned by Search for Instances
/// Contains all Instance-level tags
pub type InstanceAttributes = DicomJsonAttributes;

// ============================================================================
// Callback Types for Each Query Level
// ============================================================================

// Using Promise<String> allows callbacks to be either sync or async
type SearchForStudiesHandler = ThreadsafeFunction<SearchForStudiesQuery, Promise<String>>;
type SearchForSeriesHandler = ThreadsafeFunction<SearchForSeriesQuery, Promise<String>>;
type SearchForStudyInstancesHandler = ThreadsafeFunction<SearchForStudyInstancesQuery, Promise<String>>;
type SearchForSeriesInstancesHandler = ThreadsafeFunction<SearchForSeriesInstancesQuery, Promise<String>>;

// ============================================================================
// High-Level Builder APIs - Hide DICOM JSON complexity
// ============================================================================

/// Builder for creating Study-level DICOM JSON responses
/// Handles all the DICOM tags and VR types automatically
#[napi]
pub struct QidoStudyResult {
    attributes: HashMap<String, DicomJsonValue>,
}

#[napi]
impl QidoStudyResult {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            attributes: HashMap::new(),
        }
    }

    // Patient Module
    #[napi]
    pub fn patient_name(&mut self, value: String) -> &Self {
        self.attributes.insert("00100010".to_string(), DicomJsonValue { vr: "PN".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn patient_id(&mut self, value: String) -> &Self {
        self.attributes.insert("00100020".to_string(), DicomJsonValue { vr: "LO".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn patient_birth_date(&mut self, value: String) -> &Self {
        self.attributes.insert("00100030".to_string(), DicomJsonValue { vr: "DA".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn patient_sex(&mut self, value: String) -> &Self {
        self.attributes.insert("00100040".to_string(), DicomJsonValue { vr: "CS".to_string(), value: Some(vec![value]) });
        self
    }

    // Study Module
    #[napi]
    pub fn study_instance_uid(&mut self, value: String) -> &Self {
        self.attributes.insert("0020000D".to_string(), DicomJsonValue { vr: "UI".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn study_date(&mut self, value: String) -> &Self {
        self.attributes.insert("00080020".to_string(), DicomJsonValue { vr: "DA".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn study_time(&mut self, value: String) -> &Self {
        self.attributes.insert("00080030".to_string(), DicomJsonValue { vr: "TM".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn accession_number(&mut self, value: String) -> &Self {
        self.attributes.insert("00080050".to_string(), DicomJsonValue { vr: "SH".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn study_description(&mut self, value: String) -> &Self {
        self.attributes.insert("00081030".to_string(), DicomJsonValue { vr: "LO".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn study_id(&mut self, value: String) -> &Self {
        self.attributes.insert("00200010".to_string(), DicomJsonValue { vr: "SH".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn referring_physician_name(&mut self, value: String) -> &Self {
        self.attributes.insert("00080090".to_string(), DicomJsonValue { vr: "PN".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn modalities_in_study(&mut self, value: String) -> &Self {
        self.attributes.insert("00080061".to_string(), DicomJsonValue { vr: "CS".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn number_of_study_related_series(&mut self, value: String) -> &Self {
        self.attributes.insert("00201206".to_string(), DicomJsonValue { vr: "IS".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn number_of_study_related_instances(&mut self, value: String) -> &Self {
        self.attributes.insert("00201208".to_string(), DicomJsonValue { vr: "IS".to_string(), value: Some(vec![value]) });
        self
    }

    /// Internal method to get attributes
    pub fn get_attributes(&self) -> HashMap<String, DicomJsonValue> {
        self.attributes.clone()
    }
}

/// Builder for creating Series-level DICOM JSON responses
#[napi]
pub struct QidoSeriesResult {
    attributes: HashMap<String, DicomJsonValue>,
}

#[napi]
impl QidoSeriesResult {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            attributes: HashMap::new(),
        }
    }

    #[napi]
    pub fn series_instance_uid(&mut self, value: String) -> &Self {
        self.attributes.insert("0020000E".to_string(), DicomJsonValue { vr: "UI".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn modality(&mut self, value: String) -> &Self {
        self.attributes.insert("00080060".to_string(), DicomJsonValue { vr: "CS".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn series_number(&mut self, value: String) -> &Self {
        self.attributes.insert("00200011".to_string(), DicomJsonValue { vr: "IS".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn series_description(&mut self, value: String) -> &Self {
        self.attributes.insert("0008103E".to_string(), DicomJsonValue { vr: "LO".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn series_date(&mut self, value: String) -> &Self {
        self.attributes.insert("00080021".to_string(), DicomJsonValue { vr: "DA".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn series_time(&mut self, value: String) -> &Self {
        self.attributes.insert("00080031".to_string(), DicomJsonValue { vr: "TM".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn performing_physician_name(&mut self, value: String) -> &Self {
        self.attributes.insert("00081050".to_string(), DicomJsonValue { vr: "PN".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn number_of_series_related_instances(&mut self, value: String) -> &Self {
        self.attributes.insert("00201209".to_string(), DicomJsonValue { vr: "IS".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn body_part_examined(&mut self, value: String) -> &Self {
        self.attributes.insert("00180015".to_string(), DicomJsonValue { vr: "CS".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn protocol_name(&mut self, value: String) -> &Self {
        self.attributes.insert("00181030".to_string(), DicomJsonValue { vr: "LO".to_string(), value: Some(vec![value]) });
        self
    }

    pub fn get_attributes(&self) -> HashMap<String, DicomJsonValue> {
        self.attributes.clone()
    }
}

/// Builder for creating Instance-level DICOM JSON responses
#[napi]
pub struct QidoInstanceResult {
    attributes: HashMap<String, DicomJsonValue>,
}

#[napi]
impl QidoInstanceResult {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            attributes: HashMap::new(),
        }
    }

    #[napi]
    pub fn sop_instance_uid(&mut self, value: String) -> &Self {
        self.attributes.insert("00080018".to_string(), DicomJsonValue { vr: "UI".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn sop_class_uid(&mut self, value: String) -> &Self {
        self.attributes.insert("00080016".to_string(), DicomJsonValue { vr: "UI".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn instance_number(&mut self, value: String) -> &Self {
        self.attributes.insert("00200013".to_string(), DicomJsonValue { vr: "IS".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn rows(&mut self, value: String) -> &Self {
        self.attributes.insert("00280010".to_string(), DicomJsonValue { vr: "US".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn columns(&mut self, value: String) -> &Self {
        self.attributes.insert("00280011".to_string(), DicomJsonValue { vr: "US".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn bits_allocated(&mut self, value: String) -> &Self {
        self.attributes.insert("00280100".to_string(), DicomJsonValue { vr: "US".to_string(), value: Some(vec![value]) });
        self
    }

    #[napi]
    pub fn number_of_frames(&mut self, value: String) -> &Self {
        self.attributes.insert("00280008".to_string(), DicomJsonValue { vr: "IS".to_string(), value: Some(vec![value]) });
        self
    }

    pub fn get_attributes(&self) -> HashMap<String, DicomJsonValue> {
        self.attributes.clone()
    }
}

/// Create final JSON response from Study results
#[napi]
pub fn create_qido_studies_response(studies: Vec<&QidoStudyResult>) -> String {
    let json_array: Vec<HashMap<String, DicomJsonValue>> = studies
        .iter()
        .map(|s| s.get_attributes())
        .collect();
    serde_json::to_string(&json_array).unwrap_or_else(|_| "[]".to_string())
}

/// Create final JSON response from Series results
#[napi]
pub fn create_qido_series_response(series: Vec<&QidoSeriesResult>) -> String {
    let json_array: Vec<HashMap<String, DicomJsonValue>> = series
        .iter()
        .map(|s| s.get_attributes())
        .collect();
    serde_json::to_string(&json_array).unwrap_or_else(|_| "[]".to_string())
}

/// Create final JSON response from Instance results
#[napi]
pub fn create_qido_instances_response(instances: Vec<&QidoInstanceResult>) -> String {
    let json_array: Vec<HashMap<String, DicomJsonValue>> = instances
        .iter()
        .map(|i| i.get_attributes())
        .collect();
    serde_json::to_string(&json_array).unwrap_or_else(|_| "[]".to_string())
}

/// Helper to create empty response array
#[napi]
pub fn create_qido_empty_response() -> String {
    "[]".to_string()
}

// ============================================================================
// QIDO-RS Server with Typed Handlers
// ============================================================================

/// QIDO-RS Server Configuration
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QidoServerConfig {
    /// Enable CORS (Cross-Origin Resource Sharing) headers
    /// Default: false
    pub enable_cors: Option<bool>,
    
    /// CORS allowed origins (comma-separated list of origins)
    /// Examples: "http://localhost:3000", "https://example.com,https://app.example.com"
    /// If not specified, allows all origins (*) when CORS is enabled
    pub cors_allowed_origins: Option<String>,
    
    /// Enable verbose logging for debugging
    pub verbose: Option<bool>,
}

/// QIDO-RS Server (using warp + RUNTIME pattern like StoreSCP)
#[napi]
pub struct QidoServer {
    port: u16,
    config: QidoServerConfig,
    search_for_studies_handler: Arc<RwLock<Option<Arc<SearchForStudiesHandler>>>>,
    search_for_series_handler: Arc<RwLock<Option<Arc<SearchForSeriesHandler>>>>,
    search_for_study_instances_handler: Arc<RwLock<Option<Arc<SearchForStudyInstancesHandler>>>>,
    search_for_series_instances_handler: Arc<RwLock<Option<Arc<SearchForSeriesInstancesHandler>>>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

#[napi]
impl QidoServer {
    /**
     * Create a new QIDO-RS server.
     * 
     * QIDO-RS (Query based on ID for DICOM Objects) is the query service of DICOMweb.
     * It provides RESTful endpoints for searching DICOM studies, series, and instances.
     * 
     * **DICOM Standard Reference:** PS3.18 Section 10 - QIDO-RS
     * 
     * ## CORS Configuration
     * 
     * CORS (Cross-Origin Resource Sharing) is essential for web applications that need to
     * query DICOM data from a different domain than the QIDO-RS server.
     * 
     * ### When to Enable CORS:
     * - Web-based DICOM viewers (e.g., OHIF Viewer, Cornerstone-based apps)
     * - Single-page applications (SPAs) accessing PACS from different origin
     * - Development environments with separate frontend/backend servers
     * - Mobile apps using WebView accessing DICOM services
     * 
     * ### Security Considerations:
     * - **Production:** Specify exact allowed origins in `cors_allowed_origins`
     * - **Development:** Can use wildcard (*) but NOT recommended for production
     * - Always use HTTPS in production to prevent MITM attacks
     * - Consider implementing authentication/authorization with CORS
     * 
     * @param port - Port number to listen on (e.g., 8042 for PACS, 8080 for testing)
     * @param config - Server configuration including CORS settings
     * @returns QidoServer instance
     * 
     * @example
     * ```typescript
     * // Basic server without CORS (internal network only)
     * const qido = new QidoServer(8042, {
     *   verbose: true
     * });
     * ```
     * 
     * @example
     * ```typescript
     * // Development server with CORS enabled (allows all origins)
     * const qido = new QidoServer(8042, {
     *   enableCors: true,
     *   verbose: true
     * });
     * ```
     * 
     * @example
     * ```typescript
     * // Production server with specific allowed origins
     * const qido = new QidoServer(8042, {
     *   enableCors: true,
     *   corsAllowedOrigins: 'https://viewer.hospital.com,https://app.hospital.com',
     *   verbose: false
     * });
     * ```
     * 
     * @example
     * ```typescript
     * // Complete QIDO-RS server with handlers
     * import { QidoServer } from '@nuxthealth/node-dicom';
     * 
     * const qido = new QidoServer(8042, {
     *   enableCors: true,
     *   corsAllowedOrigins: 'http://localhost:3000',
     *   verbose: true
     * });
     * 
     * // Register search handlers
     * qido.onSearchForStudies((err, query) => {
     *   if (err) throw err;
     *   // Search database for studies matching query
     *   const results = searchStudies(query);
     *   return JSON.stringify(results);
     * });
     * 
     * qido.onSearchForSeries((err, query) => {
     *   if (err) throw err;
     *   const results = searchSeries(query);
     *   return JSON.stringify(results);
     * });
     * 
     * qido.start();
     * ```
     */
    #[napi(constructor)]
    pub fn new(port: u16, config: Option<QidoServerConfig>) -> Result<Self> {
        let config = config.unwrap_or(QidoServerConfig {
            enable_cors: Some(false),
            cors_allowed_origins: None,
            verbose: Some(false),
        });
        
        Ok(Self {
            port,
            config,
            search_for_studies_handler: Arc::new(RwLock::new(None)),
            search_for_series_handler: Arc::new(RwLock::new(None)),
            search_for_study_instances_handler: Arc::new(RwLock::new(None)),
            search_for_series_instances_handler: Arc::new(RwLock::new(None)),
            shutdown_tx: None,
        })
    }

    /// Register handler for "Search for Studies" query (GET /studies)
    /// Callback receives SearchForStudiesQuery and returns JSON string array
    #[napi(ts_args_type = "callback: (err: Error | null, query: SearchForStudiesQuery) => string | Promise<string>")]
    pub fn on_search_for_studies(&mut self, callback: SearchForStudiesHandler) -> Result<()> {
        RUNTIME.block_on(async {
            let mut handler = self.search_for_studies_handler.write().await;
            *handler = Some(Arc::new(callback));
        });
        Ok(())
    }

    /// Register handler for "Search for Series" query (GET /studies/{uid}/series)
    /// Callback receives SearchForSeriesQuery and returns JSON string array
    #[napi(ts_args_type = "callback: (err: Error | null, query: SearchForSeriesQuery) => string | Promise<string>")]
    pub fn on_search_for_series(&mut self, callback: SearchForSeriesHandler) -> Result<()> {
        RUNTIME.block_on(async {
            let mut handler = self.search_for_series_handler.write().await;
            *handler = Some(Arc::new(callback));
        });
        Ok(())
    }

    /// Register handler for "Search for Instances" in a Study (GET /studies/{uid}/instances)
    /// Callback receives SearchForStudyInstancesQuery and returns JSON string array
    #[napi(ts_args_type = "callback: (err: Error | null, query: SearchForStudyInstancesQuery) => string | Promise<string>")]
    pub fn on_search_for_study_instances(&mut self, callback: SearchForStudyInstancesHandler) -> Result<()> {
        RUNTIME.block_on(async {
            let mut handler = self.search_for_study_instances_handler.write().await;
            *handler = Some(Arc::new(callback));
        });
        Ok(())
    }

    /// Register handler for "Search for Instances" in a Series (GET /studies/{uid}/series/{uid}/instances)
    /// Callback receives SearchForSeriesInstancesQuery and returns JSON string array
    #[napi(ts_args_type = "callback: (err: Error | null, query: SearchForSeriesInstancesQuery) => string | Promise<string>")]
    pub fn on_search_for_series_instances(&mut self, callback: SearchForSeriesInstancesHandler) -> Result<()> {
        RUNTIME.block_on(async {
            let mut handler = self.search_for_series_instances_handler.write().await;
            *handler = Some(Arc::new(callback));
        });
        Ok(())
    }

    /// Start the QIDO server using RUNTIME pattern like StoreSCP
    #[napi]
    pub fn start(&mut self) -> Result<()> {
        let port = self.port;
        let config = self.config.clone();
        let studies_handler = self.search_for_studies_handler.clone();
        let series_handler = self.search_for_series_handler.clone();
        let study_instances_handler = self.search_for_study_instances_handler.clone();
        let series_instances_handler = self.search_for_series_instances_handler.clone();
        
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);
        
        if config.verbose.unwrap_or(false) {
            eprintln!("Starting QIDO server on port {}...", port);
            if config.enable_cors.unwrap_or(false) {
                eprintln!("  CORS enabled: {}", 
                    config.cors_allowed_origins.as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("* (all origins)"));
            }
        }
        
        // Spawn server task in RUNTIME (same pattern as StoreSCP)
        RUNTIME.spawn(async move {
            // CORS configuration
            let cors = if config.enable_cors.unwrap_or(false) {
                let mut cors_builder = warp::cors()
                    .allow_methods(vec!["GET", "OPTIONS"])
                    .allow_headers(vec!["Content-Type", "Accept", "Authorization"]);
                
                // Configure allowed origins
                if let Some(origins) = &config.cors_allowed_origins {
                    // Parse comma-separated origins
                    let origin_list: Vec<&str> = origins.split(',').map(|s| s.trim()).collect();
                    for origin in origin_list {
                        cors_builder = cors_builder.allow_origin(origin);
                    }
                } else {
                    // Allow all origins if none specified
                    cors_builder = cors_builder.allow_any_origin();
                }
                
                cors_builder
            } else {
                // CORS disabled - restrictive default
                warp::cors().allow_any_origin()
            };
            
            // GET /studies - Search for Studies
            let studies_route = warp::path!("studies")
                .and(warp::get())
                .and(warp::query::<HashMap<String, String>>())
                .and(warp::any().map(move || studies_handler.clone()))
                .and_then(handle_search_for_studies);
            
            // GET /studies/{StudyInstanceUID}/series - Search for Series
            let series_route = warp::path!("studies" / String / "series")
                .and(warp::get())
                .and(warp::query::<HashMap<String, String>>())
                .and(warp::any().map(move || series_handler.clone()))
                .and_then(handle_search_for_series);
            
            // GET /studies/{StudyInstanceUID}/instances - Search for Instances in Study
            let study_instances_route = warp::path!("studies" / String / "instances")
                .and(warp::get())
                .and(warp::query::<HashMap<String, String>>())
                .and(warp::any().map(move || study_instances_handler.clone()))
                .and_then(handle_search_for_study_instances);
            
            // GET /studies/{StudyInstanceUID}/series/{SeriesInstanceUID}/instances - Search for Instances in Series
            let series_instances_route = warp::path!("studies" / String / "series" / String / "instances")
                .and(warp::get())
                .and(warp::query::<HashMap<String, String>>())
                .and(warp::any().map(move || series_instances_handler.clone()))
                .and_then(handle_search_for_series_instances);
            
            let routes = studies_route
                .or(series_route)
                .or(study_instances_route)
                .or(series_instances_route)
                .with(cors);
            
            let bound = warp::serve(routes)
                .bind(([0, 0, 0, 0], port)).await;

            eprintln!("✓ QIDO server listening on http://0.0.0.0:{}", port);
            bound.graceful(async {
                shutdown_rx.await.ok();
            })
            .run().await;
        });
        
        Ok(())
    }

    /// Stop the QIDO server
    #[napi]
    pub fn stop(&mut self) -> Result<()> {
        eprintln!("Stopping QIDO server...");
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

// ============================================================================
// Route Handlers - Each properly typed for their query level
// ============================================================================

/// Handler for GET /studies - Search for Studies
async fn handle_search_for_studies(
    params: HashMap<String, String>,
    handler: Arc<RwLock<Option<Arc<SearchForStudiesHandler>>>>,
) -> std::result::Result<warp::reply::WithStatus<warp::reply::Json>, warp::Rejection> {
    let handler_lock = handler.read().await;
    let handler_arc = match &*handler_lock {
        Some(h) => h.clone(),
        None => {
            drop(handler_lock);
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({"error": "No handler registered for Search for Studies"})),
                warp::http::StatusCode::NOT_IMPLEMENTED,
            ));
        }
    };
    drop(handler_lock);
    
    // Parse query parameters into SearchForStudiesQuery
    let query: SearchForStudiesQuery = serde_json::from_value(serde_json::to_value(&params).unwrap())
        .unwrap_or_default();
    
    // Call JS callback with async support
    let promise = handler_arc.call_async(Ok(query));
    
    // Await the Promise - works for both sync and async callbacks
    match promise.await {
        Ok(json_future) => {
            match json_future.await {
                Ok(json_string) => {
                    match serde_json::from_str::<serde_json::Value>(&json_string) {
                        Ok(json) => Ok(warp::reply::with_status(
                            warp::reply::json(&json),
                            warp::http::StatusCode::OK,
                        )),
                        Err(e) => Ok(warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({"error": format!("Invalid JSON: {}", e)})),
                            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                        ))
                    }
                }
                Err(e) => Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"error": format!("Promise rejected: {:?}", e)})),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
        Err(e) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": format!("Callback error: {:?}", e)})),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

/// Handler for GET /studies/{StudyInstanceUID}/series - Search for Series
async fn handle_search_for_series(
    study_uid: String,
    params: HashMap<String, String>,
    handler: Arc<RwLock<Option<Arc<SearchForSeriesHandler>>>>,
) -> std::result::Result<warp::reply::WithStatus<warp::reply::Json>, warp::Rejection> {
    let handler_lock = handler.read().await;
    let handler_arc = match &*handler_lock {
        Some(h) => h.clone(),
        None => {
            drop(handler_lock);
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({"error": "No handler registered for Search for Series"})),
                warp::http::StatusCode::NOT_IMPLEMENTED,
            ));
        }
    };
    drop(handler_lock);
    
    // Parse query with StudyInstanceUID from path
    let mut all_params = params.clone();
    all_params.insert("StudyInstanceUID".to_string(), study_uid);
    let query: SearchForSeriesQuery = serde_json::from_value(serde_json::to_value(&all_params).unwrap())
        .unwrap_or_else(|_| {
            let mut q = SearchForSeriesQuery::default();
            q.study_instance_uid = all_params.get("StudyInstanceUID").unwrap().clone();
            q
        });
    
    // Call JS callback with async support
    let promise = handler_arc.call_async(Ok(query));
    
    // Await the Promise - works for both sync and async callbacks
    match promise.await {
        Ok(json_future) => {
            match json_future.await {
                Ok(json_string) => {
                    match serde_json::from_str::<serde_json::Value>(&json_string) {
                        Ok(json) => Ok(warp::reply::with_status(
                            warp::reply::json(&json),
                            warp::http::StatusCode::OK,
                        )),
                        Err(e) => Ok(warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({"error": format!("Invalid JSON: {}", e)})),
                            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                        ))
                    }
                }
                Err(e) => Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"error": format!("Promise rejected: {:?}", e)})),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
        Err(e) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": format!("Callback error: {:?}", e)})),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

/// Handler for GET /studies/{StudyInstanceUID}/instances - Search for Instances in Study
async fn handle_search_for_study_instances(
    study_uid: String,
    params: HashMap<String, String>,
    handler: Arc<RwLock<Option<Arc<SearchForStudyInstancesHandler>>>>,
) -> std::result::Result<warp::reply::WithStatus<warp::reply::Json>, warp::Rejection> {
    let handler_lock = handler.read().await;
    let handler_arc = match &*handler_lock {
        Some(h) => h.clone(),
        None => {
            drop(handler_lock);
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({"error": "No handler registered for Search for Study Instances"})),
                warp::http::StatusCode::NOT_IMPLEMENTED,
            ));
        }
    };
    drop(handler_lock);
    
    let mut all_params = params.clone();
    all_params.insert("StudyInstanceUID".to_string(), study_uid);
    let query: SearchForStudyInstancesQuery = serde_json::from_value(serde_json::to_value(&all_params).unwrap())
        .unwrap_or_else(|_| {
            let mut q = SearchForStudyInstancesQuery::default();
            q.study_instance_uid = all_params.get("StudyInstanceUID").unwrap().clone();
            q
        });
    
    // Call JS callback with async support
    let promise = handler_arc.call_async(Ok(query));
    
    // Await the Promise - works for both sync and async callbacks
    match promise.await {
        Ok(json_future) => {
            match json_future.await {
                Ok(json_string) => {
                    match serde_json::from_str::<serde_json::Value>(&json_string) {
                        Ok(json) => Ok(warp::reply::with_status(
                            warp::reply::json(&json),
                            warp::http::StatusCode::OK,
                        )),
                        Err(e) => Ok(warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({"error": format!("Invalid JSON: {}", e)})),
                            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                        ))
                    }
                }
                Err(e) => Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"error": format!("Promise rejected: {:?}", e)})),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
        Err(e) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": format!("Callback error: {:?}", e)})),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

/// Handler for GET /studies/{StudyInstanceUID}/series/{SeriesInstanceUID}/instances
async fn handle_search_for_series_instances(
    study_uid: String,
    series_uid: String,
    params: HashMap<String, String>,
    handler: Arc<RwLock<Option<Arc<SearchForSeriesInstancesHandler>>>>,
) -> std::result::Result<warp::reply::WithStatus<warp::reply::Json>, warp::Rejection> {
    let handler_lock = handler.read().await;
    let handler_arc = match &*handler_lock {
        Some(h) => h.clone(),
        None => {
            drop(handler_lock);
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({"error": "No handler registered for Search for Series Instances"})),
                warp::http::StatusCode::NOT_IMPLEMENTED,
            ));
        }
    };
    drop(handler_lock);
    
    let mut all_params = params.clone();
    all_params.insert("StudyInstanceUID".to_string(), study_uid);
    all_params.insert("SeriesInstanceUID".to_string(), series_uid);
    let query: SearchForSeriesInstancesQuery = serde_json::from_value(serde_json::to_value(&all_params).unwrap())
        .unwrap_or_else(|_| {
            let mut q = SearchForSeriesInstancesQuery::default();
            q.study_instance_uid = all_params.get("StudyInstanceUID").unwrap().clone();
            q.series_instance_uid = all_params.get("SeriesInstanceUID").unwrap().clone();
            q
        });
    
    // Call JS callback with async support
    let promise = handler_arc.call_async(Ok(query));
    
    // Await the Promise - works for both sync and async callbacks
    match promise.await {
        Ok(json_future) => {
            match json_future.await {
                Ok(json_string) => {
                    match serde_json::from_str::<serde_json::Value>(&json_string) {
                        Ok(json) => Ok(warp::reply::with_status(
                            warp::reply::json(&json),
                            warp::http::StatusCode::OK,
                        )),
                        Err(e) => Ok(warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({"error": format!("Invalid JSON: {}", e)})),
                            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                        ))
                    }
                }
                Err(e) => Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"error": format!("Promise rejected: {:?}", e)})),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                ))
            }
        }
        Err(e) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": format!("Callback error: {:?}", e)})),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}

// ============================================================================
// QIDO-RS CORS Configuration Guide
// ============================================================================

/*
 * QIDO-RS CORS (Cross-Origin Resource Sharing) Configuration
 * 
 * This module provides CORS support for QIDO-RS servers to enable web-based
 * DICOM applications to query medical imaging data from different origins.
 * 
 * ## DICOM Standard Reference
 * - **DICOM PS3.18 Section 10:** QIDO-RS (Query based on ID for DICOM Objects)
 * - **W3C CORS Specification:** https://www.w3.org/TR/cors/
 * 
 * ## What is CORS?
 * 
 * Cross-Origin Resource Sharing (CORS) is a security mechanism that allows
 * web applications running at one origin (domain) to access resources from
 * a different origin. Browsers enforce the Same-Origin Policy (SOP) by default,
 * which blocks cross-origin requests unless the server explicitly allows them
 * via CORS headers.
 * 
 * ## When to Enable CORS
 * 
 * Enable CORS when your QIDO-RS server needs to be accessed by:
 * 
 * 1. **Web-based DICOM Viewers:**
 *    - OHIF Viewer (https://ohif.org)
 *    - Cornerstone-based applications
 *    - Radiant DICOM Viewer web interface
 *    - Custom React/Vue/Angular medical imaging apps
 * 
 * 2. **Single-Page Applications (SPAs):**
 *    - Frontend served from different domain than PACS/QIDO server
 *    - Development environments (frontend: localhost:3000, backend: localhost:8042)
 *    - Microservices architecture with separate services
 * 
 * 3. **Mobile Applications:**
 *    - Hybrid apps using WebView (React Native, Ionic, Cordova)
 *    - Progressive Web Apps (PWAs) accessing DICOM services
 * 
 * 4. **Integration Scenarios:**
 *    - Hospital portals embedding DICOM viewers
 *    - Telemedicine platforms accessing imaging studies
 *    - Research platforms with web-based analysis tools
 * 
 * ## CORS Configuration Options
 * 
 * The `QidoServerConfig` object supports the following CORS options:
 * 
 * ### 1. `enableCors` (boolean, default: false)
 * 
 * Master switch to enable/disable CORS support.
 * 
 * ```typescript
 * // CORS disabled (default) - restrictive, internal network only
 * const qido = new QidoServer(8042, {
 *   enableCors: false
 * });
 * 
 * // CORS enabled - allows cross-origin requests
 * const qido = new QidoServer(8042, {
 *   enableCors: true
 * });
 * ```
 * 
 * ### 2. `corsAllowedOrigins` (string, optional)
 * 
 * Comma-separated list of allowed origins. If not specified, allows all origins (*).
 * 
 * **Recommended for Production:** Always specify exact origins in production.
 * 
 * ```typescript
 * // Allow single origin
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com'
 * });
 * 
 * // Allow multiple origins (comma-separated)
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com,https://app.hospital.com,https://research.hospital.com'
 * });
 * 
 * // Allow all origins (development only!)
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: undefined  // or omit this property
 * });
 * ```
 * 
 * ### 3. `verbose` (boolean, default: false)
 * 
 * Enable verbose logging to see CORS configuration at startup.
 * 
 * ```typescript
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com',
 *   verbose: true  // Log CORS configuration
 * });
 * ```
 * 
 * ## CORS Headers Sent by QIDO-RS
 * 
 * When CORS is enabled, the server automatically adds these headers:
 * 
 * - **Access-Control-Allow-Origin:** Allowed origin(s)
 * - **Access-Control-Allow-Methods:** GET, OPTIONS
 * - **Access-Control-Allow-Headers:** Content-Type, Accept, Authorization
 * 
 * ## Security Best Practices
 * 
 * ### 1. Production Deployments
 * 
 * **DO:**
 * - Always specify exact allowed origins
 * - Use HTTPS for all production endpoints
 * - Implement authentication/authorization with CORS
 * - Regularly review and update allowed origins
 * - Monitor CORS errors in logs
 * 
 * **DON'T:**
 * - Use wildcard (*) in production
 * - Allow HTTP origins in production (HTTPS only)
 * - Disable CORS without firewall protection
 * - Share credentials across untrusted origins
 * 
 * ```typescript
 * // ✅ GOOD: Production configuration
 * const qido = new QidoServer(443, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com',
 *   verbose: false
 * });
 * 
 * // ❌ BAD: Insecure production configuration
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   // No corsAllowedOrigins specified = allows all origins!
 *   verbose: false
 * });
 * ```
 * 
 * ### 2. Development Environments
 * 
 * For local development, you can use more permissive CORS settings:
 * 
 * ```typescript
 * // Development: Allow localhost origins
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'http://localhost:3000,http://localhost:5173',
 *   verbose: true
 * });
 * ```
 * 
 * ### 3. Network Segmentation
 * 
 * If your QIDO server is only accessible within a hospital network,
 * you may choose to disable CORS and rely on network-level security:
 * 
 * ```typescript
 * // Internal network only - no CORS needed
 * const qido = new QidoServer(8042, {
 *   enableCors: false  // Network firewall provides security
 * });
 * ```
 * 
 * ## Complete Usage Examples
 * 
 * ### Example 1: OHIF Viewer Integration
 * 
 * ```typescript
 * import { QidoServer } from '@nuxthealth/node-dicom';
 * 
 * // QIDO server for OHIF Viewer
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'http://localhost:3000',  // OHIF development server
 *   verbose: true
 * });
 * 
 * qido.onSearchForStudies((err, query) => {
 *   if (err) throw err;
 *   
 *   // Query your PACS/database
 *   const studies = database.searchStudies({
 *     patientName: query.PatientName,
 *     studyDate: query.StudyDate,
 *     limit: query.limit || 25
 *   });
 *   
 *   // Return DICOM JSON (PS3.18 Section F.2)
 *   return JSON.stringify(studies);
 * });
 * 
 * qido.start();
 * ```
 * 
 * ### Example 2: Multi-Origin Production Setup
 * 
 * ```typescript
 * // Production: Multiple hospital viewer origins
 * const qido = new QidoServer(443, {
 *   enableCors: true,
 *   corsAllowedOrigins: [
 *     'https://radiology.hospital.com',
 *     'https://cardiology.hospital.com',
 *     'https://oncology.hospital.com'
 *   ].join(','),
 *   verbose: false
 * });
 * 
 * // Register handlers...
 * qido.start();
 * ```
 * 
 * ### Example 3: Development with Hot Reload
 * 
 * ```typescript
 * // Development with Vite/React hot reload
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'http://localhost:5173,http://127.0.0.1:5173',
 *   verbose: true
 * });
 * 
 * qido.start();
 * console.log('🔥 QIDO-RS server ready for hot reload development');
 * ```
 * 
 * ## Testing CORS Configuration
 * 
 * ### Using curl
 * 
 * ```bash
 * # Test preflight OPTIONS request
 * curl -X OPTIONS \
 *      -H "Origin: http://localhost:3000" \
 *      -H "Access-Control-Request-Method: GET" \
 *      -H "Access-Control-Request-Headers: Content-Type" \
 *      -v http://localhost:8042/studies
 * 
 * # Test actual GET request with CORS
 * curl -H "Origin: http://localhost:3000" \
 *      -v http://localhost:8042/studies?limit=10
 * 
 * # Check for Access-Control-Allow-Origin header in response
 * ```
 * 
 * ### Using Browser DevTools
 * 
 * Open browser console and test:
 * 
 * ```javascript
 * // This will fail if CORS is not properly configured
 * fetch('http://localhost:8042/studies?limit=10')
 *   .then(res => res.json())
 *   .then(data => console.log('QIDO Studies:', data))
 *   .catch(err => console.error('CORS Error:', err));
 * ```
 * 
 * ## Troubleshooting CORS Issues
 * 
 * ### Error: "No 'Access-Control-Allow-Origin' header"
 * 
 * **Cause:** CORS not enabled on server
 * 
 * **Solution:**
 * ```typescript
 * const qido = new QidoServer(8042, {
 *   enableCors: true  // Enable CORS
 * });
 * ```
 * 
 * ### Error: "CORS policy: The 'Access-Control-Allow-Origin' header has a value that is not equal to the supplied origin"
 * 
 * **Cause:** Your origin is not in the allowed origins list
 * 
 * **Solution:**
 * ```typescript
 * const qido = new QidoServer(8042, {
 *   enableCors: true,
 *   corsAllowedOrigins: 'http://your-actual-origin.com'  // Add your origin
 * });
 * ```
 * 
 * ### Error: Requests work in Postman/curl but fail in browser
 * 
 * **Cause:** Browsers enforce CORS, command-line tools don't
 * 
 * **Solution:** Configure CORS properly for browser access
 * 
 * ## Related Documentation
 * 
 * - DICOM PS3.18 QIDO-RS: https://dicom.nema.org/medical/dicom/current/output/html/part18.html#sect_10
 * - MDN CORS Guide: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS
 * - OHIF Configuration: https://docs.ohif.org/
 * 
 * ## TypeScript Type Definitions
 * 
 * ```typescript
 * interface QidoServerConfig {
 *   // Enable CORS headers
 *   enableCors?: boolean;
 *   
 *   // Comma-separated list of allowed origins
 *   // Example: "https://viewer.hospital.com,https://app.hospital.com"
 *   // If omitted when CORS is enabled, allows all origins (*)
 *   corsAllowedOrigins?: string;
 *   
 *   // Enable verbose logging
 *   verbose?: boolean;
 * }
 * 
 * class QidoServer {
 *   constructor(port: number, config?: QidoServerConfig);
 *   onSearchForStudies(callback: (err: Error | null, query: SearchForStudiesQuery) => string): void;
 *   onSearchForSeries(callback: (err: Error | null, query: SearchForSeriesQuery) => string): void;
 *   onSearchForStudyInstances(callback: (err: Error | null, query: SearchForStudyInstancesQuery) => string): void;
 *   onSearchForSeriesInstances(callback: (err: Error | null, query: SearchForSeriesInstancesQuery) => string): void;
 *   start(): void;
 *   stop(): void;
 * }
 * ```
 */
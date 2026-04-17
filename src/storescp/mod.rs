use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use snafu::Report;
use tracing::{error, info, Level};

use tokio::sync::{broadcast, Notify, Mutex};
use tokio::runtime::Runtime;

use dicom_core::{dicom_value, DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_object::{InMemDicomObject, StandardDataDictionary};

mod sop_classes;

use crate::utils::{CustomTag, S3Config, build_s3_bucket, check_s3_connectivity};

mod transfer;
mod store_async;
use store_async::run_store_async;

type EventSender = broadcast::Sender<(StoreScpEvent, ScpEventData)>;
type EventReceiver = broadcast::Receiver<(StoreScpEvent, ScpEventData)>;

lazy_static::lazy_static! {
    static ref EVENT_CHANNEL: (EventSender, EventReceiver) = broadcast::channel(100);
    static ref RUNTIME: Runtime = Runtime::new().unwrap();
}

/**
 * Storage backend type for DICOM C-STORE SCP.
 * 
 * Determines where incoming DICOM files will be stored.
 * 
 * @example
 * ```typescript
 * // Use filesystem storage
 * const scp = new StoreScp({
 *   port: 11111,
 *   storageBackend: 'Filesystem',
 *   outDir: './dicom-data'
 * });
 * 
 * // Use S3 storage
 * const scpS3 = new StoreScp({
 *   port: 11111,
 *   storageBackend: 'S3',
 *   s3Config: {
 *     bucket: 'my-dicom-bucket',
 *     accessKey: 'ACCESS_KEY',
 *     secretKey: 'SECRET_KEY',
 *     endpoint: 'http://localhost:9000'
 *   }
 * });
 * ```
 */
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageBackendType {
    /// Store files on local filesystem
    Filesystem,
    /// Store files in S3-compatible object storage
    S3,
}

/// Abstract syntax (SOP Class) acceptance mode
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbstractSyntaxMode {
    /// Accept all known storage SOP classes (default preset)
    AllStorage,
    /// Accept any SOP class (promiscuous mode)
    All,
    /// Accept only specified SOP classes
    Custom,
}

/// Transfer syntax acceptance mode
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferSyntaxMode {
    /// Accept all supported transfer syntaxes (default)
    All,
    /// Accept only uncompressed transfer syntaxes
    UncompressedOnly,
    /// Accept only specified transfer syntaxes
    Custom,
}

/// DICOM C-STORE SCP
#[napi]
pub struct StoreScp {
    /// Verbose mode
    // short = 'v', long = "verbose"
    pub(crate) verbose: bool,
    /// Calling Application Entity title
    // long = "calling-ae-title", default_value = "STORE-SCP"
    pub(crate) calling_ae_title: String,
    /// Enforce max pdu length
    // short = 's', long = "strict"
    pub(crate) strict: bool,
    /// Maximum PDU length
    // short = 'm', long = "max-pdu-length", default_value = "16384"
    pub(crate) max_pdu_length: u32,
    /// Which port to listen on
    // short, default_value = "11111"
    pub(crate) port: u16,
    /// Study completion callback timeout
    /// Default is 30 seconds
    pub(crate) study_timeout: u32,
    /// Storage backend type
    // long = "storage-backend", default_value = "Filesystem"
    pub(crate) storage_backend: StorageBackendType,
    /// S3 configuration if using S3 as storage backend
    pub(crate) s3_config: Option<S3Config>,
    /// Output directory for incoming objects using Filesystem storage backend
    // short = 'o', default_value = "."
    pub(crate) out_dir: Option<String>,
    /// Store files with complete DICOM file meta header (true) or dataset-only (false)
    /// Default is false (dataset-only), which is more efficient and standard for PACS systems
    pub(crate) store_with_file_meta: bool,
    /// DICOM tags to extract (by name or hex)
    pub(crate) extract_tags: Vec<String>,
    /// Custom DICOM tags to extract (with user-defined names)
    pub(crate) extract_custom_tags: Vec<CustomTag>,
    /// Abstract syntax acceptance mode
    pub(crate) abstract_syntax_mode: AbstractSyntaxMode,
    /// Custom abstract syntaxes (SOP Class UIDs)
    pub(crate) abstract_syntaxes: Vec<String>,
    /// Transfer syntax acceptance mode
    pub(crate) transfer_syntax_mode: TransferSyntaxMode,
    /// Custom transfer syntaxes
    pub(crate) transfer_syntaxes: Vec<String>,
    /// Callback for modifying tags before storage (async, returns Promise)
    /// Tags are passed as JSON string due to NAPI-RS ThreadsafeFunction limitations with HashMap
    pub(crate) on_before_store: Option<Arc<ThreadsafeFunction<String, napi::bindgen_prelude::Promise<String>>>>,
    /// Shutdown channel for graceful shutdown
    pub(crate) shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}


/**
 * Events emitted by the DICOM C-STORE SCP server.
 * 
 * Use these events to monitor server activity and handle incoming DICOM files.
 * 
 * @example
 * ```typescript
 * const scp = new StoreScp({ port: 11111 });
 * 
 * scp.onServerStarted((data) => {
 *   console.log('Server started:', data.message);
 * });
 * 
 * scp.onFileStored((data) => {
 *   if (data.data) {
 *     console.log('File stored:', data.data.sopInstanceUid);
 *   }
 * });
 * 
 * scp.onStudyCompleted((data) => {
 *   if (data.data?.study) {
 *     console.log('Study completed:', data.data.study.studyInstanceUid);
 *   }
 * });
 * ```
 */
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreScpEvent {
    /// Server has started and is listening for connections
    OnServerStarted,
    /// An error occurred during file storage or processing
    OnError,
    /// A new DICOM connection has been established
    OnConnection,
    /// A DICOM file has been successfully stored
    OnFileStored,
    /// A complete study (all files) has been received and stored
    OnStudyCompleted
}

/**
 * Event data passed to event listeners.
 * 
 * Contains information about the event that occurred.
 */
#[napi(object)]
#[derive(Clone, Debug)]
pub struct ScpEventData {
    /// Human-readable message describing the event
    pub message: String,
    /// Optional event-specific details
    pub data: Option<ScpEventDetails>
}

/// Details about SCP events with typed tag extraction
#[napi(object)]
#[derive(Clone, Debug)]
pub struct ScpEventDetails {
    /// File path where DICOM file was stored
    pub file: Option<String>,
    /// SOP Instance UID
    pub sop_instance_uid: Option<String>,
    /// SOP Class UID
    pub sop_class_uid: Option<String>,
    /// Transfer Syntax UID
    pub transfer_syntax_uid: Option<String>,
    /// Study Instance UID
    pub study_instance_uid: Option<String>,
    /// Series Instance UID
    pub series_instance_uid: Option<String>,
    /// Extracted DICOM tags (flat key-value pairs)
    pub tags: Option<HashMap<String, String>>,
    /// Error message (for OnError events)
    pub error: Option<String>,
    /// Study completion data with full hierarchy
    pub study: Option<StudyHierarchyData>,
}

/// Study hierarchy data for OnStudyCompleted event
#[napi(object)]
#[derive(Clone, Debug)]
pub struct StudyHierarchyData {
    pub study_instance_uid: String,
    /// Patient + Study level tags only
    pub tags: Option<HashMap<String, String>>,
    pub series: Vec<SeriesHierarchyData>,
}

/// Tags wrapper for onBeforeStore callback
/// This wrapper is needed because ThreadsafeFunction doesn't directly support HashMap
/// Using Vec instead of HashMap for better serialization support
#[napi(object)]
#[derive(Clone, Debug)]
pub struct BeforeStoreArgs {
    pub tags: Vec<(String, String)>,
}

/// Series data within a study
#[napi(object)]
#[derive(Clone, Debug)]
pub struct SeriesHierarchyData {
    pub series_instance_uid: String,
    /// Series level tags only
    pub tags: Option<HashMap<String, String>>,
    pub instances: Vec<InstanceHierarchyData>,
}

/// Instance (file) data within a series
#[napi(object)]
#[derive(Clone, Debug)]
pub struct InstanceHierarchyData {
    pub sop_instance_uid: String,
    pub sop_class_uid: String,
    pub transfer_syntax_uid: String,
    pub file: String,
    /// Instance + Equipment level tags only
    pub tags: Option<HashMap<String, String>>,
}



async fn run(args: StoreScp, mut shutdown_rx: tokio::sync::oneshot::Receiver<()>) -> Result<(), Box<dyn std::error::Error>> {

  std::fs::create_dir_all(args.out_dir.as_deref().unwrap_or(".")).unwrap_or_else(|e| {
      error!("Could not create output directory: {}", e);
      std::process::exit(-2);
  });

  let listen_addr = SocketAddrV4::new(Ipv4Addr::from(0), args.port);
  let listener = tokio::net::TcpListener::bind(listen_addr).await?;
  info!(
      "{} listening on: tcp://{}",
      &args.calling_ae_title, listen_addr
  );

  StoreScp::emit_event(StoreScpEvent::OnServerStarted, ScpEventData {
      message: "Server started".to_string(),
      data: None,
  });

  loop {
      tokio::select! {
          _ = &mut shutdown_rx => {
              info!("Shutdown signal received");
              break;
          }
          result = listener.accept() => {
              let (socket, _addr) = result?;
              StoreScp::emit_event(StoreScpEvent::OnConnection, ScpEventData {
                  message: "New connection".to_string(),
                  data: None,
              });

              let args = StoreScp {
                  verbose: args.verbose,
                  calling_ae_title: args.calling_ae_title.clone(),
                  strict: args.strict,
                  max_pdu_length: args.max_pdu_length,
                  port: args.port,
                  out_dir: args.out_dir.clone(),
                  study_timeout: args.study_timeout,
                  storage_backend: args.storage_backend.clone(),
                  s3_config: args.s3_config.clone(),
                  store_with_file_meta: args.store_with_file_meta,
                  extract_tags: args.extract_tags.clone(),
                  extract_custom_tags: args.extract_custom_tags.clone(),
                  abstract_syntax_mode: args.abstract_syntax_mode.clone(),
                  abstract_syntaxes: args.abstract_syntaxes.clone(),
                  transfer_syntax_mode: args.transfer_syntax_mode.clone(),
                  transfer_syntaxes: args.transfer_syntaxes.clone(),
                  on_before_store: args.on_before_store.clone(),
                  shutdown_tx: None,
              };

              RUNTIME.spawn(async move {
                  tokio::select! {
                      _ = std::future::pending::<()>() => {
                          // This branch will never execute - connections handle their own lifecycle
                      }
                      result = run_store_async(socket, &args, move |event_details| {
                          StoreScp::emit_event(StoreScpEvent::OnFileStored, ScpEventData {
                              message: "File stored successfully".to_string(),
                              data: Some(event_details),
                          });
                      }, Arc::new(Mutex::new(move |study_hierarchy| {
                          StoreScp::emit_event(StoreScpEvent::OnStudyCompleted, ScpEventData {
                              message: "Study completed successfully".to_string(),
                              data: Some(ScpEventDetails {
                                  file: None,
                                  sop_instance_uid: None,
                                  sop_class_uid: None,
                                  transfer_syntax_uid: None,
                                  study_instance_uid: None,
                                  series_instance_uid: None,
                                  tags: None,
                                  error: None,
                                  study: Some(study_hierarchy),
                              }),
                          });
                      }))) => {
                          if let Err(e) = result {
                              StoreScp::emit_event(StoreScpEvent::OnError, ScpEventData {
                                  message: "Error storing file".to_string(),
                                  data: Some(ScpEventDetails {
                                      file: None,
                                      sop_instance_uid: None,
                                      sop_class_uid: None,
                                      transfer_syntax_uid: None,
                                      study_instance_uid: None,
                                      series_instance_uid: None,
                                      tags: None,
                                      error: Some(e.to_string()),
                                      study: None,
                                  }),
                              });
                              error!("{}", Report::from_error(e));
                          }
                      }
                  }
              });
          }
      }
  }

  Ok(())
}

/**
 * Configuration options for the DICOM C-STORE SCP server.
 * 
 * @example
 * ```typescript
 * // Basic filesystem storage
 * const options1: StoreScpOptions = {
 *   port: 11111,
 *   outDir: './dicom-storage',
 *   verbose: true
 * };
 * 
 * // S3 storage with tag extraction
 * const options2: StoreScpOptions = {
 *   port: 11111,
 *   storageBackend: 'S3',
 *   s3Config: {
 *     bucket: 'dicom-bucket',
 *     accessKey: 'ACCESS_KEY',
 *     secretKey: 'SECRET_KEY',
 *     endpoint: 'http://localhost:9000'
 *   },
 *   extractTags: ['PatientName', 'StudyDate', 'Modality'],
 *   groupingStrategy: 'ByScope',
 *   studyTimeout: 60
 * };
 * 
 * // Strict mode with uncompressed only
 * const options3: StoreScpOptions = {
 *   port: 11111,
 *   callingAeTitle: 'MY-SCP',
 *   strict: true,
 *   uncompressedOnly: true,
 *   maxPduLength: 32768
 * };
 * ```
 */
#[napi(object)]
pub struct StoreScpOptions {
    /// Enable verbose logging (default: false)
    pub verbose: Option<bool>,
    /// Application Entity title for this SCP (default: "STORE-SCP")
    pub calling_ae_title: Option<String>,
    /// Enforce strict PDU length limits (default: false)
    pub strict: Option<bool>,
    /// Maximum PDU length in bytes (default: 16384)
    pub max_pdu_length: Option<u32>,
    /// Abstract syntax (SOP Class) acceptance mode (default: 'AllStorage')
    pub abstract_syntax_mode: Option<AbstractSyntaxMode>,
    /// Custom abstract syntaxes (SOP Class UIDs) to accept when mode is 'Custom'
    #[napi(ts_type = "Array<'CTImageStorage' | 'EnhancedCTImageStorage' | 'MRImageStorage' | 'EnhancedMRImageStorage' | 'UltrasoundImageStorage' | 'UltrasoundMultiFrameImageStorage' | 'SecondaryCaptureImageStorage' | 'MultiFrameGrayscaleByteSecondaryCaptureImageStorage' | 'MultiFrameGrayscaleWordSecondaryCaptureImageStorage' | 'MultiFrameTrueColorSecondaryCaptureImageStorage' | 'ComputedRadiographyImageStorage' | 'DigitalXRayImageStorageForPresentation' | 'DigitalXRayImageStorageForProcessing' | 'DigitalMammographyXRayImageStorageForPresentation' | 'DigitalMammographyXRayImageStorageForProcessing' | 'BreastTomosynthesisImageStorage' | 'BreastProjectionXRayImageStorageForPresentation' | 'BreastProjectionXRayImageStorageForProcessing' | 'PositronEmissionTomographyImageStorage' | 'EnhancedPETImageStorage' | 'NuclearMedicineImageStorage' | 'RTImageStorage' | 'RTDoseStorage' | 'RTStructureSetStorage' | 'RTPlanStorage' | 'EncapsulatedPDFStorage' | 'EncapsulatedCDAStorage' | 'EncapsulatedSTLStorage' | 'GrayscaleSoftcopyPresentationStateStorage' | 'BasicTextSRStorage' | 'EnhancedSRStorage' | 'ComprehensiveSRStorage' | 'Verification' | (string & {})>")]
    pub abstract_syntaxes: Option<Vec<String>>,
    /// Transfer syntax acceptance mode (default: 'All')
    pub transfer_syntax_mode: Option<TransferSyntaxMode>,
    /// Custom transfer syntaxes to accept when mode is 'Custom'
    #[napi(ts_type = "Array<'ImplicitVRLittleEndian' | 'ExplicitVRLittleEndian' | 'ExplicitVRBigEndian' | 'DeflatedExplicitVRLittleEndian' | 'JPEGBaseline' | 'JPEGExtended' | 'JPEGLossless' | 'JPEGLosslessNonHierarchical' | 'JPEGLSLossless' | 'JPEGLSLossy' | 'JPEG2000Lossless' | 'JPEG2000' | 'RLELossless' | 'MPEG2MainProfile' | 'MPEG2MainProfileHighLevel' | 'MPEG4AVCH264HighProfile' | 'MPEG4AVCH264BDCompatibleHighProfile' | (string & {})>")]
    pub transfer_syntaxes: Option<Vec<String>>,
    /// TCP port to listen on (required)
    pub port: u16,
    /// Timeout in seconds before triggering OnStudyCompleted event (default: 30)
    pub study_timeout: Option<u32>,
    /// Storage backend: 'Filesystem' or 'S3' (default: 'Filesystem')
    pub storage_backend: Option<StorageBackendType>,
    /// S3 configuration (required if storageBackend is 'S3')
    pub s3_config: Option<S3Config>,
    /// Output directory for filesystem storage (default: current directory)
    pub out_dir: Option<String>,
    /// Store complete DICOM files with meta header vs dataset-only (default: true)
    pub store_with_file_meta: Option<bool>,
    /// DICOM tags to extract from received files (e.g., ['PatientName', 'StudyDate'])
    #[napi(ts_type = "Array<'AccessionNumber' | 'AcquisitionDate' | 'AcquisitionDateTime' | 'AcquisitionNumber' | 'AcquisitionTime' | 'ActualCardiacTriggerTimePriorToRPeak' | 'ActualFrameDuration' | 'AdditionalPatientHistory' | 'AdmissionID' | 'AdmittingDiagnosesDescription' | 'AnatomicalOrientationType' | 'AnatomicRegionSequence' | 'AnodeTargetMaterial' | 'BeamLimitingDeviceAngle' | 'BitsAllocated' | 'BitsStored' | 'BluePaletteColorLookupTableDescriptor' | 'BodyPartExamined' | 'BodyPartThickness' | 'BranchOfService' | 'BurnedInAnnotation' | 'ChannelSensitivity' | 'CineRate' | 'CollimatorType' | 'Columns' | 'CompressionForce' | 'ContentDate' | 'ContentTime' | 'ContrastBolusAgent' | 'ContrastBolusIngredient' | 'ContrastBolusIngredientConcentration' | 'ContrastBolusRoute' | 'ContrastBolusStartTime' | 'ContrastBolusStopTime' | 'ContrastBolusTotalDose' | 'ContrastBolusVolume' | 'ContrastFlowDuration' | 'ContrastFlowRate' | 'ConvolutionKernel' | 'CorrectedImage' | 'CountsSource' | 'DataCollectionDiameter' | 'DecayCorrection' | 'DeidentificationMethod' | 'DerivationDescription' | 'DetectorTemperature' | 'DeviceSerialNumber' | 'DistanceSourceToDetector' | 'DistanceSourceToPatient' | 'EchoTime' | 'EthnicGroup' | 'Exposure' | 'ExposureInMicroAmpereSeconds' | 'ExposureTime' | 'FilterType' | 'FlipAngle' | 'FocalSpots' | 'FrameDelay' | 'FrameIncrementPointer' | 'FrameOfReferenceUID' | 'FrameTime' | 'GantryAngle' | 'GeneratorPower' | 'GraphicAnnotationSequence' | 'GreenPaletteColorLookupTableDescriptor' | 'HeartRate' | 'HighBit' | 'ImageComments' | 'ImageLaterality' | 'ImageOrientationPatient' | 'ImagePositionPatient' | 'ImagerPixelSpacing' | 'ImageTriggerDelay' | 'ImageType' | 'ImagingFrequency' | 'ImplementationClassUID' | 'ImplementationVersionName' | 'InstanceCreationDate' | 'InstanceCreationTime' | 'InstanceNumber' | 'InstitutionName' | 'IntensifierSize' | 'IssuerOfAdmissionID' | 'KVP' | 'LargestImagePixelValue' | 'LargestPixelValueInSeries' | 'Laterality' | 'LossyImageCompression' | 'LossyImageCompressionMethod' | 'LossyImageCompressionRatio' | 'MagneticFieldStrength' | 'Manufacturer' | 'ManufacturerModelName' | 'MedicalRecordLocator' | 'MilitaryRank' | 'Modality' | 'MultiplexGroupTimeOffset' | 'NameOfPhysiciansReadingStudy' | 'NominalCardiacTriggerDelayTime' | 'NominalInterval' | 'NumberOfFrames' | 'NumberOfSlices' | 'NumberOfTemporalPositions' | 'NumberOfWaveformChannels' | 'NumberOfWaveformSamples' | 'Occupation' | 'OperatorsName' | 'OtherPatientIDs' | 'OtherPatientNames' | 'OverlayBitPosition' | 'OverlayBitsAllocated' | 'OverlayColumns' | 'OverlayData' | 'OverlayOrigin' | 'OverlayRows' | 'OverlayType' | 'PaddleDescription' | 'PatientAge' | 'PatientBirthDate' | 'PatientBreedDescription' | 'PatientComments' | 'PatientID' | 'PatientIdentityRemoved' | 'PatientName' | 'PatientPosition' | 'PatientSex' | 'PatientSize' | 'PatientSpeciesDescription' | 'PatientSupportAngle' | 'PatientTelephoneNumbers' | 'PatientWeight' | 'PerformedProcedureStepDescription' | 'PerformedProcedureStepID' | 'PerformedProcedureStepStartDate' | 'PerformedProcedureStepStartTime' | 'PerformedProtocolCodeSequence' | 'PerformingPhysicianName' | 'PhotometricInterpretation' | 'PhysiciansOfRecord' | 'PixelAspectRatio' | 'PixelPaddingRangeLimit' | 'PixelPaddingValue' | 'PixelRepresentation' | 'PixelSpacing' | 'PlanarConfiguration' | 'PositionerPrimaryAngle' | 'PositionerSecondaryAngle' | 'PositionReferenceIndicator' | 'PreferredPlaybackSequencing' | 'PresentationIntentType' | 'PresentationLUTShape' | 'PrimaryAnatomicStructureSequence' | 'PrivateInformationCreatorUID' | 'ProtocolName' | 'QualityControlImage' | 'RadiationMachineName' | 'RadiationSetting' | 'RadionuclideTotalDose' | 'RadiopharmaceuticalInformationSequence' | 'RadiopharmaceuticalStartDateTime' | 'RadiopharmaceuticalStartTime' | 'RadiopharmaceuticalVolume' | 'ReasonForTheRequestedProcedure' | 'ReceivingApplicationEntityTitle' | 'RecognizableVisualFeatures' | 'RecommendedDisplayFrameRate' | 'ReconstructionDiameter' | 'ReconstructionTargetCenterPatient' | 'RedPaletteColorLookupTableDescriptor' | 'ReferencedBeamNumber' | 'ReferencedImageSequence' | 'ReferencedPatientPhotoSequence' | 'ReferencedPerformedProcedureStepSequence' | 'ReferencedRTPlanSequence' | 'ReferencedSOPClassUID' | 'ReferencedSOPInstanceUID' | 'ReferencedStudySequence' | 'ReferringPhysicianName' | 'RepetitionTime' | 'RequestAttributesSequence' | 'RequestedContrastAgent' | 'RequestedProcedureDescription' | 'RequestedProcedureID' | 'RequestingPhysician' | 'RescaleIntercept' | 'RescaleSlope' | 'RescaleType' | 'ResponsibleOrganization' | 'ResponsiblePerson' | 'ResponsiblePersonRole' | 'Rows' | 'RTImageDescription' | 'RTImageLabel' | 'SamplesPerPixel' | 'SamplingFrequency' | 'ScanningSequence' | 'SendingApplicationEntityTitle' | 'SeriesDate' | 'SeriesDescription' | 'SeriesInstanceUID' | 'SeriesNumber' | 'SeriesTime' | 'SeriesType' | 'SliceLocation' | 'SliceThickness' | 'SmallestImagePixelValue' | 'SmallestPixelValueInSeries' | 'SoftwareVersions' | 'SOPClassUID' | 'SOPInstanceUID' | 'SoundPathLength' | 'SourceApplicationEntityTitle' | 'SourceImageSequence' | 'SpacingBetweenSlices' | 'SpecificCharacterSet' | 'StationName' | 'StudyComments' | 'StudyDate' | 'StudyDescription' | 'StudyID' | 'StudyInstanceUID' | 'StudyTime' | 'TableHeight' | 'TableTopLateralPosition' | 'TableTopLongitudinalPosition' | 'TableTopVerticalPosition' | 'TableType' | 'TemporalPositionIdentifier' | 'TemporalResolution' | 'TextObjectSequence' | 'TimezoneOffsetFromUTC' | 'TransducerFrequency' | 'TransducerType' | 'TransferSyntaxUID' | 'TriggerTime' | 'TriggerTimeOffset' | 'UltrasoundColorDataPresent' | 'Units' | 'VOILUTFunction' | 'WaveformOriginality' | 'WaveformSequence' | 'WindowCenter' | 'WindowCenterWidthExplanation' | 'WindowWidth' | 'XRayTubeCurrent' | (string & {})>")]
    pub extract_tags: Option<Vec<String>>,
    /// Custom private tags to extract with user-defined names
    pub extract_custom_tags: Option<Vec<CustomTag>>,
}

/**
 * DICOM C-STORE SCP (Service Class Provider) Server.
 * 
 * A complete DICOM storage server that receives DICOM files over the network
 * and stores them to filesystem or S3. Supports tag extraction, study completion
 * detection, and real-time event notifications.
 * 
 * ## Features
 * - Multiple storage backends (Filesystem, S3)
 * - Automatic tag extraction from incoming files
 * - Study completion detection with configurable timeout
 * - Real-time event notifications
 * - Support for compressed and uncompressed transfer syntaxes
 * - Configurable AE title, port, and PDU settings
 * 
 * @example
 * ```typescript
 * import { StoreScp } from '@nuxthealth/node-dicom';
 * 
 * // Create SCP server
 * const scp = new StoreScp({
 *   port: 11111,
 *   outDir: './dicom-storage',
 *   extractTags: ['PatientName', 'StudyDate', 'Modality'],
 *   groupingStrategy: 'ByScope',
 *   studyTimeout: 60
 * });
 * 
 * // Listen for events
 * scp.onServerStarted((data) => {
 *   console.log('Server started');
 * });
 * 
 * scp.onFileStored((data) => {
 *   if (data.data) {
 *     console.log('Stored:', data.data.sopInstanceUid);
 *   }
 * });
 * 
 * scp.onStudyCompleted((data) => {
 *   if (data.data?.study) {
 *     console.log('Study complete:', data.data.study.studyInstanceUid);
 *   }
 * });
 * 
 * // Start listening
 * await scp.listen();
 * 
 * // Stop server when done
 * await scp.close();
 * ```
 */
#[napi]
impl StoreScp {

    /**
     * Create a new DICOM C-STORE SCP server instance.
     * 
     * Initializes the server with the provided configuration. The server is not
     * started until `listen()` is called.
     * 
     * @param options - Server configuration options
     * @returns New StoreScp instance
     * 
     * @example
     * ```typescript
     * // Filesystem storage
     * const scp = new StoreScp({
     *   port: 11111,
     *   outDir: './dicom-data',
     *   verbose: true
     * });
     * 
     * // S3 storage with tag extraction
     * const scpS3 = new StoreScp({
     *   port: 11112,
     *   storageBackend: 'S3',
     *   s3Config: {
     *     bucket: 'my-dicom-bucket',
     *     accessKey: process.env.AWS_ACCESS_KEY!,
     *     secretKey: process.env.AWS_SECRET_KEY!,
     *     region: 'us-east-1'
     *   },
     *   extractTags: ['PatientID', 'StudyInstanceUID', 'SeriesInstanceUID'],
     *   studyTimeout: 120
     * });
     * ```
     */
    #[napi(constructor)]
    pub fn new(options: StoreScpOptions) -> Self {
        let mut verbose: bool = false;
        if options.verbose.is_some() {
            verbose = options.verbose.unwrap();
        }
        // set up global logger
        // Only set global logger if not already set (it can only be set once per process)
        // Use RUST_LOG env var if set, otherwise use verbose flag
        use tracing_subscriber::EnvFilter;
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| {
                if verbose {
                    EnvFilter::new("debug")
                } else {
                    EnvFilter::new("error")
                }
            });
        
        let _ = tracing::subscriber::set_global_default(
          tracing_subscriber::FmtSubscriber::builder()
              .with_env_filter(filter)
              .finish(),
        );

        let mut calling_ae_title: String = String::from("STORE-SCP");
        if options.calling_ae_title.is_some() {
            calling_ae_title = options.calling_ae_title.unwrap();
        }
        let mut strict: bool = false;
        if options.strict.is_some() {
            strict = options.strict.unwrap();
        }
        let mut max_pdu_length: u32 = 16384;
        if options.max_pdu_length.is_some() {
            max_pdu_length = options.max_pdu_length.unwrap();
        }
        let mut study_timeout: u32 = 30;
        if options.study_timeout.is_some() {
            study_timeout = options.study_timeout.unwrap();
        }
        let storage_backend = options.storage_backend.unwrap_or(StorageBackendType::Filesystem);
        let s3_config = options.s3_config;
        let store_with_file_meta = options.store_with_file_meta.unwrap_or(true);
        
        // Use provided tags or empty (user must specify)
        let extract_tags = options.extract_tags.unwrap_or_default();
        let extract_custom_tags = options.extract_custom_tags.unwrap_or_default();
        
        // Debug: Log what tags we're extracting
        info!("StoreSCP configured with {} extract_tags: {:?}", extract_tags.len(), extract_tags);
        info!("StoreSCP configured with {} extract_custom_tags", extract_custom_tags.len());
        
        // Handle syntax configuration
        let abstract_syntax_mode = options.abstract_syntax_mode.unwrap_or(AbstractSyntaxMode::AllStorage);
        let transfer_syntax_mode = options.transfer_syntax_mode.unwrap_or(TransferSyntaxMode::All);
        
        let abstract_syntaxes = options.abstract_syntaxes.unwrap_or_default();
        let transfer_syntaxes = options.transfer_syntaxes.unwrap_or_default();
        
        StoreScp {
            verbose,
            calling_ae_title,
            strict,
            max_pdu_length,
            port: options.port,
            out_dir: options.out_dir,
            study_timeout,
            storage_backend,
            s3_config,
            store_with_file_meta,
            extract_tags,
            extract_custom_tags,
            abstract_syntax_mode,
            abstract_syntaxes,
            transfer_syntax_mode,
            transfer_syntaxes,
            on_before_store: None,
            shutdown_tx: None,
        }
    }

    /**
     * Start the DICOM C-STORE SCP server and begin listening for connections.
     * 
     * This method starts the server asynchronously in a non-blocking manner.
     * The server will listen on the configured port and handle incoming DICOM associations.
     * Events will be emitted as files are received and stored.
     * 
     * For S3 storage, this method will verify S3 connectivity before starting.
     * 
     * @throws Error if S3 connectivity check fails (when using S3 backend)
     * 
     * @example
     * ```typescript
     * const scp = new StoreScp({
     *   port: 11111,
     *   outDir: './dicom-storage'
     * });
     * 
     * // Add event listeners before starting
     * scp.onFileStored((event) => {
     *   console.log('File stored:', event.data?.sopInstanceUid);
     * });
     * 
     * // Start server (non-blocking)
     * scp.start();
     * 
     * // Server is now running in the background
     * console.log('Server started on port 11111');
     * 
     * // Later, stop the server
     * scp.stop();
     * ```
     */
    #[napi]
    pub fn start(&mut self) -> napi::Result<()> {
        info!("Starting server...");
        if self.storage_backend == StorageBackendType::S3 {
            if let Some(ref s3_config) = self.s3_config {
                info!("Using S3 storage backend");
                info!("S3 Bucket: {}", s3_config.bucket);
                if let Some(ref endpoint) = s3_config.endpoint {
                    info!("S3 Endpoint: {}", endpoint);
                } else {
                    info!("S3 Endpoint: Not specified");
                }
                // S3 connectivity check at server startup
                let config = s3_config.clone();
                RUNTIME.block_on(async move {
                    let bucket = build_s3_bucket(&config);
                    check_s3_connectivity(&bucket).await;
                });
            } else {
                error!("S3 storage backend selected, but no S3 config provided!");
                return Err(napi::Error::from_reason("S3 config required for S3 backend"));
            }
        } else {
            info!("Using Filesystem storage backend");
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        
        let args = StoreScp {
            verbose: self.verbose,
            calling_ae_title: self.calling_ae_title.clone(),
            strict: self.strict,
            max_pdu_length: self.max_pdu_length,
            port: self.port,
            out_dir: self.out_dir.clone(),
            study_timeout: self.study_timeout,
            storage_backend: self.storage_backend.clone(),
            s3_config: self.s3_config.clone(),
            store_with_file_meta: self.store_with_file_meta,
            extract_tags: self.extract_tags.clone(),
            extract_custom_tags: self.extract_custom_tags.clone(),
            abstract_syntax_mode: self.abstract_syntax_mode.clone(),
            abstract_syntaxes: self.abstract_syntaxes.clone(),
            transfer_syntax_mode: self.transfer_syntax_mode.clone(),
            transfer_syntaxes: self.transfer_syntaxes.clone(),
            on_before_store: self.on_before_store.clone(),
            shutdown_tx: None,
        };

        RUNTIME.spawn(async move {
            if let Err(e) = run(args, shutdown_rx).await {
                error!("Server error: {:?}", e);
            }
            info!("Server stopped");
        });

        self.shutdown_tx = Some(shutdown_tx);
        Ok(())
    }

    /**
     * Stop the DICOM C-STORE SCP server and close all connections.
     * 
     * Initiates a graceful shutdown of the server. All active connections will be
     * terminated and the server will stop accepting new connections.
     * 
     * @example
     * ```typescript
     * const scp = new StoreScp({ port: 11111 });
     * scp.start();
     * 
     * // Later, when you want to stop the server
     * scp.stop();
     * console.log('Server stopped');
     * ```
     */
    #[napi]
    pub fn stop(&mut self) -> napi::Result<()> {
        info!("Stopping server...");
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        info!("Shutdown signal sent");
        Ok(())
    }

    /**
     * Register callback for server started events
     */
    #[napi]
    pub fn on_server_started(&self, handler: ThreadsafeFunction<ScpEventData, ()>) {
        let mut receiver = EVENT_CHANNEL.0.subscribe();
        RUNTIME.spawn(async move {
            loop {
                if let Ok((StoreScpEvent::OnServerStarted, data)) = receiver.recv().await {
                    handler.call(Ok(data), ThreadsafeFunctionCallMode::NonBlocking);
                }
            }
        });
    }

    /**
     * Register callback for new connection events
     */
    #[napi]
    pub fn on_connection(&self, handler: ThreadsafeFunction<ScpEventData, ()>) {
        let mut receiver = EVENT_CHANNEL.0.subscribe();
        RUNTIME.spawn(async move {
            loop {
                if let Ok((StoreScpEvent::OnConnection, data)) = receiver.recv().await {
                    handler.call(Ok(data), ThreadsafeFunctionCallMode::NonBlocking);
                }
            }
        });
    }

    /**
     * Register callback for file stored events
     * 
     * Called when a DICOM file has been successfully received and stored.
     * The event data includes file path, SOP UIDs, and extracted DICOM tags.
     */
    #[napi]
    pub fn on_file_stored(&self, handler: ThreadsafeFunction<ScpEventData, ()>) {
        let mut receiver = EVENT_CHANNEL.0.subscribe();
        RUNTIME.spawn(async move {
            loop {
                if let Ok((StoreScpEvent::OnFileStored, data)) = receiver.recv().await {
                    handler.call(Ok(data), ThreadsafeFunctionCallMode::NonBlocking);
                }
            }
        });
    }

    /**
     * Register callback for study completed events
     * 
     * Called when all files for a study have been received (after study timeout).
     * Includes the complete study hierarchy with all series and instances.
     */
    #[napi]
    pub fn on_study_completed(&self, handler: ThreadsafeFunction<ScpEventData, ()>) {
        let mut receiver = EVENT_CHANNEL.0.subscribe();
        RUNTIME.spawn(async move {
            loop {
                if let Ok((StoreScpEvent::OnStudyCompleted, data)) = receiver.recv().await {
                    handler.call(Ok(data), ThreadsafeFunctionCallMode::NonBlocking);
                }
            }
        });
    }

    /**
     * Register callback for error events
     */
    #[napi]
    pub fn on_error(&self, handler: ThreadsafeFunction<ScpEventData, ()>) {
        let mut receiver = EVENT_CHANNEL.0.subscribe();
        RUNTIME.spawn(async move {
            loop {
                if let Ok((StoreScpEvent::OnError, data)) = receiver.recv().await {
                    handler.call(Ok(data), ThreadsafeFunctionCallMode::NonBlocking);
                }
            }
        });
    }

    /**
     * Register a callback to modify DICOM tags before files are saved.
     * 
     * This callback is invoked **asynchronously** for each received DICOM file, allowing you
     * to modify tags before the file is written to disk. The callback receives an error parameter
     * (always null unless there's an internal error) and the extracted tags as a JSON string.
     * 
     * **Important:** 
     * - You must configure `extractTags` to specify which tags should be extracted
     * - Only tags specified in `extractTags` will be available to modify
     * - The callback follows error-first pattern: `(err, tagsJson) => Promise<string>`
     * - Tags are passed as JSON string, must be parsed with JSON.parse()
     * - Must return a Promise that resolves to a JSON string (use JSON.stringify())
     * - Must call this method BEFORE `start()`
     * - File storage waits for the Promise to resolve before saving
     * 
     * **Common Use Cases:**
     * - **Anonymization**: Remove or replace patient-identifying information (with async lookups)
     * - **Tag Enrichment**: Add institution-specific metadata (with database queries)
     * - **Validation**: Verify required tags are present (throw error to reject file)
     * - **Normalization**: Standardize tag formats across different sources
     * 
     * @param callback - Error-first async function that receives tags JSON and returns Promise of modified tags JSON
     * 
     * @example
     * ```typescript
     * // Async anonymization with database lookup
     * const scp = new StoreScp({
     *   port: 11115,
     *   outDir: './anonymized',
     *   storeWithFileMeta: true, // Important for re-reading files
     *   extractTags: ['PatientName', 'PatientID', 'PatientBirthDate', 'StudyDescription']
     * });
     * 
     * scp.onBeforeStore(async (error, tagsJson) => {
     *   if (error) throw error;
     *   
     *   const tags = JSON.parse(tagsJson);
     *   
     *   // Async database lookup for anonymous ID
     *   const anonId = await db.getOrCreateAnonId(tags.PatientID);
     *   
     *   const modified = {
     *     ...tags,
     *     PatientName: 'ANONYMOUS^PATIENT',
     *     PatientID: anonId,
     *     PatientBirthDate: '',
     *     StudyDescription: tags.StudyDescription 
     *       ? `ANONYMIZED - ${tags.StudyDescription}` 
     *       : 'ANONYMIZED STUDY'
     *   };
     *   
     *   return JSON.stringify(modified);
     * });
     * 
     * scp.start();
     * ```
     * 
     * @example
     * ```typescript
     * // Async validation with external service
     * scp.onBeforeStore(async (error, tagsJson) => {
     *   if (error) throw error;
     *   
     *   const tags = JSON.parse(tagsJson);
     *   
     *   if (!tags.PatientID || !tags.StudyInstanceUID) {
     *     throw new Error('Missing required patient or study identifiers');
     *   }
     *   
     *   // Validate against external service
     *   const isValid = await validationService.checkPatientId(tags.PatientID);
     *   if (!isValid) {
     *     throw new Error('Invalid PatientID - not found in registry');
     *   }
     *   
     *   return JSON.stringify(tags); // No modifications, just validation
     * });
     * ```
     * 
     * @example
     * ```typescript
     * // Async tag enrichment with API call
     * scp.onBeforeStore(async (error, tagsJson) => {
     *   if (error) throw error;
     *   
     *   const tags = JSON.parse(tagsJson);
     *   
     *   // Fetch additional metadata from external API
     *   const metadata = await fetchPatientMetadata(tags.PatientID);
     *   
     *   const modified = {
     *     ...tags,
     *     PatientName: tags.PatientName?.toUpperCase() || '',
     *     PatientSex: metadata.sex || 'O',
     *     InstitutionName: metadata.institution || 'UNKNOWN'
     *   };
     *   
     *   return JSON.stringify(modified);
     * });
     * ```
     */
    #[napi(ts_args_type = "callback: (err: Error | null, tagsJson: string) => Promise<string>")]
    pub fn on_before_store(&mut self, callback: ThreadsafeFunction<String, napi::bindgen_prelude::Promise<String>>) {
        self.on_before_store = Some(Arc::new(callback));
    }

    fn emit_event(event: StoreScpEvent, data: ScpEventData) {
        let _ = EVENT_CHANNEL.0.send((event, data));
    }
}

pub(crate) fn create_cstore_response(
    message_id: u16,
    sop_class_uid: &str,
    sop_instance_uid: &str,
) -> InMemDicomObject<StandardDataDictionary> {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            dicom_value!(Str, sop_class_uid),
        ),
        DataElement::new(tags::COMMAND_FIELD, VR::US, dicom_value!(U16, [0x8001])),
        DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            dicom_value!(U16, [message_id]),
        ),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [0x0101]),
        ),
        DataElement::new(tags::STATUS, VR::US, dicom_value!(U16, [0x0000])),
        DataElement::new(
            tags::AFFECTED_SOP_INSTANCE_UID,
            VR::UI,
            dicom_value!(Str, sop_instance_uid),
        ),
    ])
}

pub(crate) fn create_cecho_response(message_id: u16) -> InMemDicomObject<StandardDataDictionary> {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(tags::COMMAND_FIELD, VR::US, dicom_value!(U16, [0x8030])),
        DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            dicom_value!(U16, [message_id]),
        ),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [0x0101]),
        ),
        DataElement::new(tags::STATUS, VR::US, dicom_value!(U16, [0x0000])),
    ])
}

/// SOP Class configuration object
#[napi(object)]
pub struct SopClassConfig {
    /// CT imaging SOP classes
    pub ct: Vec<String>,
    /// MR imaging SOP classes
    pub mr: Vec<String>,
    /// Ultrasound imaging SOP classes
    pub ultrasound: Vec<String>,
    /// PET and nuclear medicine SOP classes
    pub pet: Vec<String>,
    /// X-Ray and CR imaging SOP classes
    pub xray: Vec<String>,
    /// Mammography SOP classes
    pub mammography: Vec<String>,
    /// Secondary capture SOP classes
    pub secondary_capture: Vec<String>,
    /// Radiation therapy SOP classes
    pub radiation_therapy: Vec<String>,
    /// Document and presentation SOP classes
    pub documents: Vec<String>,
    /// Structured report SOP classes
    pub structured_reports: Vec<String>,
    /// All imaging modalities (CT, MR, US, PET, XR)
    pub all_imaging: Vec<String>,
    /// All storage SOP classes
    pub all: Vec<String>,
}

/**
 * Get a list of common SOP Class UIDs (Abstract Syntaxes).
 * 
 * Use these to configure which types of DICOM objects your SCP accepts.
 * 
 * @returns Object containing categorized SOP Class UID lists
 * 
 * @example
 * ```typescript
 * import { StoreScp, getCommonSopClasses } from '@nuxthealth/node-dicom';
 * 
 * const sopClasses = getCommonSopClasses();
 * 
 * // Accept only CT and MR images
 * const scp = new StoreScp({
 *   port: 11111,
 *   abstractSyntaxMode: 'Custom',
 *   abstractSyntaxes: [...sopClasses.ct, ...sopClasses.mr]
 * });
 * 
 * // Accept all imaging modalities
 * const scp2 = new StoreScp({
 *   port: 11112,
 *   abstractSyntaxMode: 'Custom',
 *   abstractSyntaxes: sopClasses.allImaging
 * });
 * ```
 */
#[napi]
pub fn get_common_sop_classes() -> SopClassConfig {
    let ct = vec![
        "CTImageStorage".to_string(),
        "EnhancedCTImageStorage".to_string(),
    ];
    
    let mr = vec![
        "MRImageStorage".to_string(),
        "EnhancedMRImageStorage".to_string(),
    ];
    
    let ultrasound = vec![
        "UltrasoundImageStorage".to_string(),
        "UltrasoundMultiFrameImageStorage".to_string(),
    ];
    
    let pet = vec![
        "PositronEmissionTomographyImageStorage".to_string(),
        "EnhancedPETImageStorage".to_string(),
        "NuclearMedicineImageStorage".to_string(),
    ];
    
    let xray = vec![
        "ComputedRadiographyImageStorage".to_string(),
        "DigitalXRayImageStorageForPresentation".to_string(),
        "DigitalXRayImageStorageForProcessing".to_string(),
    ];
    
    let mammography = vec![
        "DigitalMammographyXRayImageStorageForPresentation".to_string(),
        "DigitalMammographyXRayImageStorageForProcessing".to_string(),
        "BreastTomosynthesisImageStorage".to_string(),
        "BreastProjectionXRayImageStorageForPresentation".to_string(),
        "BreastProjectionXRayImageStorageForProcessing".to_string(),
    ];
    
    let secondary_capture = vec![
        "SecondaryCaptureImageStorage".to_string(),
        "MultiFrameGrayscaleByteSecondaryCaptureImageStorage".to_string(),
        "MultiFrameGrayscaleWordSecondaryCaptureImageStorage".to_string(),
        "MultiFrameTrueColorSecondaryCaptureImageStorage".to_string(),
    ];
    
    let radiation_therapy = vec![
        "RTImageStorage".to_string(),
        "RTDoseStorage".to_string(),
        "RTStructureSetStorage".to_string(),
        "RTPlanStorage".to_string(),
    ];
    
    let documents = vec![
        "EncapsulatedPDFStorage".to_string(),
        "EncapsulatedCDAStorage".to_string(),
        "EncapsulatedSTLStorage".to_string(),
        "GrayscaleSoftcopyPresentationStateStorage".to_string(),
    ];
    
    let structured_reports = vec![
        "BasicTextSRStorage".to_string(),
        "EnhancedSRStorage".to_string(),
        "ComprehensiveSRStorage".to_string(),
    ];
    
    let mut all_imaging = Vec::new();
    all_imaging.extend_from_slice(&ct);
    all_imaging.extend_from_slice(&mr);
    all_imaging.extend_from_slice(&ultrasound);
    all_imaging.extend_from_slice(&pet);
    all_imaging.extend_from_slice(&xray);
    all_imaging.extend_from_slice(&mammography);
    
    let mut all = all_imaging.clone();
    all.extend_from_slice(&secondary_capture);
    all.extend_from_slice(&radiation_therapy);
    all.extend_from_slice(&documents);
    all.extend_from_slice(&structured_reports);
    all.push("Verification".to_string());
    
    SopClassConfig {
        ct,
        mr,
        ultrasound,
        pet,
        xray,
        mammography,
        secondary_capture,
        radiation_therapy,
        documents,
        structured_reports,
        all_imaging,
        all,
    }
}

/// Transfer Syntax configuration object
#[napi(object)]
pub struct TransferSyntaxConfig {
    /// Uncompressed transfer syntaxes
    pub uncompressed: Vec<String>,
    /// JPEG transfer syntaxes
    pub jpeg: Vec<String>,
    /// JPEG-LS transfer syntaxes
    pub jpeg_ls: Vec<String>,
    /// JPEG 2000 transfer syntaxes
    pub jpeg2000: Vec<String>,
    /// RLE transfer syntax
    pub rle: Vec<String>,
    /// MPEG video transfer syntaxes
    pub mpeg: Vec<String>,
    /// All compressed transfer syntaxes
    pub all_compressed: Vec<String>,
    /// All transfer syntaxes
    pub all: Vec<String>,
}

/**
 * Get a list of common Transfer Syntax UIDs.
 * 
 * Use these to configure which encodings/compressions your SCP accepts.
 * 
 * @returns Object containing categorized Transfer Syntax UID lists
 * 
 * @example
 * ```typescript
 * import { StoreScp, getCommonTransferSyntaxes } from '@nuxthealth/node-dicom';
 * 
 * const transferSyntaxes = getCommonTransferSyntaxes();
 * 
 * // Accept uncompressed and JPEG only
 * const scp = new StoreScp({
 *   port: 11111,
 *   transferSyntaxMode: 'Custom',
 *   transferSyntaxes: [...transferSyntaxes.uncompressed, ...transferSyntaxes.jpeg]
 * });
 * ```
 */
#[napi]
pub fn get_common_transfer_syntaxes() -> TransferSyntaxConfig {
    let uncompressed = vec![
        "ImplicitVRLittleEndian".to_string(),
        "ExplicitVRLittleEndian".to_string(),
        "ExplicitVRBigEndian".to_string(),
    ];
    
    let jpeg = vec![
        "JPEGBaseline".to_string(),
        "JPEGExtended".to_string(),
        "JPEGLossless".to_string(),
        "JPEGLosslessNonHierarchical".to_string(),
    ];
    
    let jpeg_ls = vec![
        "JPEGLSLossless".to_string(),
        "JPEGLSLossy".to_string(),
    ];
    
    let jpeg2000 = vec![
        "JPEG2000Lossless".to_string(),
        "JPEG2000".to_string(),
    ];
    
    let rle = vec![
        "RLELossless".to_string(),
    ];
    
    let mpeg = vec![
        "MPEG2MainProfile".to_string(),
        "MPEG2MainProfileHighLevel".to_string(),
        "MPEG4AVCH264HighProfile".to_string(),
        "MPEG4AVCH264BDCompatibleHighProfile".to_string(),
    ];
    
    let mut all_compressed = Vec::new();
    all_compressed.extend_from_slice(&jpeg);
    all_compressed.extend_from_slice(&jpeg_ls);
    all_compressed.extend_from_slice(&jpeg2000);
    all_compressed.extend_from_slice(&rle);
    all_compressed.extend_from_slice(&mpeg);
    
    let mut all = uncompressed.clone();
    all.extend_from_slice(&all_compressed);
    all.push("DeflatedExplicitVRLittleEndian".to_string());
    
    TransferSyntaxConfig {
        uncompressed,
        jpeg,
        jpeg_ls,
        jpeg2000,
        rle,
        mpeg,
        all_compressed,
        all,
    }
}
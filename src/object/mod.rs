use std::path::{Path, PathBuf};
use std::ffi::OsStr;
use std::sync::Mutex;
use std::collections::HashMap;
use dicom_dictionary_std::tags;
use dicom_core::header::Tag;
use dicom_object::{ open_file, DefaultDicomObject};
use snafu::prelude::*;
use napi::JsError;
use s3::Bucket;

#[cfg(feature = "transcode")]
use dicom_pixeldata::{DecodedPixelData, PixelDecoder};

use crate::utils::{extract_tags_flat, CustomTag, S3Config, build_s3_bucket, s3_get_object, s3_put_object};

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(whatever, display("{}", message))]
    Other {
        message: String,
        #[snafu(source(from(Box<dyn std::error::Error + 'static>, Some)))]
        source: Option<Box<dyn std::error::Error + 'static>>,
    },
}

/// Storage backend type
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq)]
pub enum StorageBackend {
    /// Local filesystem storage
    Filesystem,
    /// S3-compatible object storage
    S3,
}

/// Storage configuration for DicomFile
#[derive(Debug, Clone)]
#[napi(object)]
pub struct StorageConfig {
    /// Storage backend type
    pub backend: StorageBackend,
    /// Root directory for filesystem storage (relative or absolute path)
    pub root_dir: Option<String>,
    /// S3 configuration (required if backend is S3)
    pub s3_config: Option<S3Config>,
}

#[napi(object)]
pub struct DicomFileMeta {
    /// Storage SOP Class UID
    pub sop_class_uid: String,
    /// Storage SOP Instance UID
    pub sop_instance_uid: String,
}

/// Options for pixel data processing
#[napi(object)]
pub struct PixelDataOptions {
    /// Output file path
    pub output_path: String,
    
    /// Output format
    pub format: Option<PixelDataFormat>,
    
    /// Decode/decompress pixel data (requires transcode feature)
    pub decode: Option<bool>,
    
    /// Convert to 8-bit grayscale (requires decode=true)
    pub convert_to_8bit: Option<bool>,
    
    /// Apply VOI LUT (Value of Interest Lookup Table) for windowing
    pub apply_voi_lut: Option<bool>,
    
    /// Window center for manual windowing (overrides VOI LUT from file)
    pub window_center: Option<f64>,
    
    /// Window width for manual windowing (overrides VOI LUT from file)
    pub window_width: Option<f64>,
    
    /// Frame number to extract (0-based, for multi-frame images)
    pub frame_number: Option<u32>,
    
    /// Extract all frames as separate files (output_path will be used as template: path_{frame}.ext)
    pub extract_all_frames: Option<bool>,
}

/// Output format for pixel data
#[napi(string_enum)]
pub enum PixelDataFormat {
    /// Raw binary data (no processing)
    Raw,
    /// PNG image (requires decode=true)
    Png,
    /// JPEG image (requires decode=true)
    Jpeg,
    /// BMP image (requires decode=true)
    Bmp,
    /// JSON metadata about pixel data
    Json,
}

/// Metadata about a DICOM tag's data type
#[napi(object)]
pub struct TagDataInfo {
    /// DICOM Value Representation (e.g. "OB", "OW", "LO", "UI")
    pub vr: String,
    /// True when the tag holds raw binary bytes (OB/OW/OF/OD/OL/OV/UN)
    pub is_binary: bool,
    /// True when the tag is the pixel data element (7FE0,0010)
    pub is_image: bool,
    /// MIME type for encapsulated document tags (0042,0011), otherwise null
    pub mime_type: Option<String>,
    /// Number of raw bytes in the tag payload
    pub byte_length: u32,
}

/// Encapsulated non-image document stored in a DICOM file
#[napi(object)]
pub struct EncapsulatedDocumentData {
    /// MIME type from tag (0042,0012) e.g. "application/pdf" or "text/plain"
    pub mime_type: String,
    /// Raw document bytes
    pub data: napi::bindgen_prelude::Buffer,
    /// Number of bytes in the document
    pub byte_length: u32,
    /// Content date from DICOM header, if present
    pub document_title: Option<String>,
}

/// Options for encoded image buffer output (PNG/JPEG/BMP)
#[napi(object)]
pub struct PixelDataImageBufferOptions {
    /// Output format (defaults to PNG)
    pub format: Option<PixelDataFormat>,

    /// Apply VOI LUT (Value of Interest Lookup Table) for windowing
    pub apply_voi_lut: Option<bool>,

    /// Window center for manual windowing (overrides VOI LUT from file)
    pub window_center: Option<f64>,

    /// Window width for manual windowing (overrides VOI LUT from file)
    pub window_width: Option<f64>,

    /// Frame number to extract (0-based, for multi-frame images)
    pub frame_number: Option<u32>,

    /// Convert to 8-bit grayscale
    pub convert_to_8bit: Option<bool>,

    /// JPEG quality (1-100), ignored for PNG/BMP
    pub quality: Option<u8>,
}

/// Options for processing pixel data in-memory (return as Buffer)
#[napi(object)]
pub struct PixelDataProcessingOptions {
    /// Specific frame number to extract (0-based, for multi-frame images)
    pub frame_number: Option<u32>,
    
    /// Apply VOI LUT (Value of Interest Lookup Table) for windowing
    pub apply_voi_lut: Option<bool>,
    
    /// Window center for manual windowing (overrides VOI LUT from file)
    pub window_center: Option<f64>,
    
    /// Window width for manual windowing (overrides VOI LUT from file)
    pub window_width: Option<f64>,
    
    /// Convert to 8-bit grayscale (applies windowing then scales to 0-255)
    pub convert_to_8bit: Option<bool>,
}

/// Pixel data information
#[napi(object)]
pub struct PixelDataInfo {
    /// Width in pixels
    pub width: u32,
    
    /// Height in pixels
    pub height: u32,
    
    /// Number of frames
    pub frames: u32,
    
    /// Bits allocated per pixel
    pub bits_allocated: u16,
    
    /// Bits stored per pixel
    pub bits_stored: u16,
    
    /// High bit
    pub high_bit: u16,
    
    /// Pixel representation (0=unsigned, 1=signed)
    pub pixel_representation: u16,
    
    /// Samples per pixel (1=grayscale, 3=RGB)
    pub samples_per_pixel: u16,
    
    /// Photometric interpretation
    pub photometric_interpretation: String,
    
    /// Transfer syntax UID
    pub transfer_syntax_uid: String,
    
    /// Whether pixel data is compressed
    pub is_compressed: bool,
    
    /// Total pixel data size in bytes
    pub data_size: u32,
    
    /// Rescale intercept (for Hounsfield units in CT)
    pub rescale_intercept: Option<f64>,
    
    /// Rescale slope
    pub rescale_slope: Option<f64>,
    
    /// Window center from file
    pub window_center: Option<f64>,
    
    /// Window width from file
    pub window_width: Option<f64>,
}



#[napi]
pub struct DicomFile{
    /// DICOM object (wrapped in Mutex for thread-safe async operations)
    dicom_file: Mutex<Option<DefaultDicomObject>>,
    /// Storage configuration
    storage_config: StorageConfig,
    /// S3 bucket instance (if using S3)
    s3_bucket: Option<Bucket>,
}

#[napi]
impl DicomFile {

    /**
     * Create a new DicomFile instance.
     * 
     * The instance is initially empty. Call `open()` to load a DICOM file.
     * 
     * @param storageConfig - Optional storage configuration for S3 or filesystem with root directory
     * 
     * @example
     * ```typescript
     * // Default filesystem storage (current directory)
     * const file1 = new DicomFile();
     * 
     * // Filesystem with root directory
     * const file2 = new DicomFile({
     *   backend: 'Filesystem',
     *   rootDir: '/data/dicom'
     * });
     * 
     * // S3 storage
     * const file3 = new DicomFile({
     *   backend: 'S3',
     *   s3Config: {
     *     bucket: 'my-dicom-bucket',
     *     accessKey: 'ACCESS_KEY',
     *     secretKey: 'SECRET_KEY',
     *     endpoint: 'http://localhost:9000'
     *   }
     * });
     * ```
     */
    #[napi(constructor)]
    pub fn new(storage_config: Option<StorageConfig>) -> Result<Self, JsError> {
        let config = storage_config.unwrap_or(StorageConfig {
            backend: StorageBackend::Filesystem,
            root_dir: None,
            s3_config: None,
        });
        
        // Validate and build S3 bucket if S3 backend is used
        let s3_bucket = if config.backend == StorageBackend::S3 {
            let s3_cfg = config.s3_config.as_ref()
                .ok_or_else(|| JsError::from(napi::Error::from_reason(
                    "S3 backend requires s3_config to be provided".to_string()
                )))?;
            
            Some(build_s3_bucket(s3_cfg))
        } else {
            None
        };
        
        Ok(DicomFile {
            dicom_file: Mutex::new(None),
            storage_config: config,
            s3_bucket,
        })
    }

    // Helper method to resolve file path with root_dir
    fn resolve_path(&self, path: &str) -> PathBuf {
        if let Some(root_dir) = &self.storage_config.root_dir {
            let root = PathBuf::from(root_dir);
            root.join(path)
        } else {
            PathBuf::from(path)
        }
    }

    // Helper method to read from S3
    async fn read_from_s3(&self, path: &str) -> Result<Vec<u8>, napi::Error> {
        let bucket = self.s3_bucket.as_ref()
            .ok_or_else(|| napi::Error::from_reason("S3 bucket not initialized".to_string()))?;
        
        let result = s3_get_object(bucket, path).await;
        match result {
            Ok(data) => Ok(data),
            Err(_e) => Err(napi::Error::from_reason(format!("Failed to read from S3: {}", path)))
        }
    }

    // Helper method to write to S3
    async fn write_to_s3(&self, path: &str, data: &[u8]) -> Result<(), napi::Error> {
        let bucket = self.s3_bucket.as_ref()
            .ok_or_else(|| napi::Error::from_reason("S3 bucket not initialized".to_string()))?;
        
        let result = s3_put_object(bucket, path, data).await;
        match result {
            Ok(()) => Ok(()),
            Err(_e) => Err(napi::Error::from_reason(format!("Failed to write to S3: {}", path)))
        }
    }

    /**
     * Check if a file is a valid DICOM file and extract its metadata.
     * 
     * This is a lightweight operation that only reads the file meta information
     * without loading the entire dataset. Useful for quickly validating files
     * or extracting SOPInstanceUID without a full file open.
     * 
     * @param path - Absolute or relative path to the DICOM file
     * @returns DicomFileMeta containing SOP Class UID and SOP Instance UID
     * @throws Error if the file is not a valid DICOM file or DICOMDIR
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * const meta = file.check('/path/to/file.dcm');
     * console.log(meta.sopInstanceUid);
     * ```
     */
    #[napi]
    pub fn check(&self, path: String) -> Result<DicomFileMeta, JsError> {
        let file = PathBuf::from(path);
        Self::check_file(file.as_path())
            .map_err(|e| JsError::from(napi::Error::from_reason(e.to_string())))
    }

    /**
     * Open and load a DICOM file into memory.
     * 
     * Reads the entire DICOM dataset and makes it available for operations like
     * `extract()`, `saveRawPixelData()`, and `dump()`. Any previously opened file
     * is automatically closed. 
     * 
     * **Storage Backend:** Automatically uses S3 or filesystem based on the
     * `StorageConfig` provided in the constructor. The same `open()` method works
     * for both backends.
     * 
     * **File Format:** Handles DICOM files both with and without file meta header.
     * Files with meta header (standard .dcm files) and dataset-only files are both supported.
     * 
     * @param path - Path to the DICOM file (filesystem path when using Filesystem backend, or S3 key when using S3 backend)
     * @returns Success message if the file was opened successfully
     * @throws Error if the file cannot be opened or is not a valid DICOM file
     * 
     * @example
     * ```typescript
     * // Filesystem backend (default)
     * const file1 = new DicomFile();
     * await file1.open('/path/to/file.dcm');
     * 
     * // Filesystem with root directory
     * const file2 = new DicomFile({ 
     *   backend: 'Filesystem', 
     *   rootDir: '/data/dicom' 
     * });
     * await file2.open('subfolder/file.dcm'); // Resolves to /data/dicom/subfolder/file.dcm
     * 
     * // S3 backend - same open() method, different config
     * const file3 = new DicomFile({ 
     *   backend: 'S3', 
     *   s3Config: {
     *     bucket: 'my-dicom-bucket',
     *     accessKey: 'ACCESS_KEY',
     *     secretKey: 'SECRET_KEY'
     *   }
     * });
     * await file3.open('folder/file.dcm'); // Reads from S3 bucket
     * ```
     */
    #[napi]
    pub async fn open(&self, path: String) -> napi::Result<String> {
        use dicom_object::OpenFileOptions;
        use dicom_object::file::ReadPreamble;
        use dicom_object::{InMemDicomObject, FileMetaTableBuilder};
        use dicom_dictionary_std::uids;
        
        match self.storage_config.backend {
            StorageBackend::S3 => {
                // Read from S3
                let data = self.read_from_s3(&path).await?;
                
                // Try to parse with auto-detection first (standard DICOM with meta)
                let result = OpenFileOptions::new()
                    .read_preamble(ReadPreamble::Auto)
                    .from_reader(&data[..]);
                
                let dicom_file = match result {
                    Ok(file) => {
                        *self.dicom_file.lock().unwrap() = Some(file);
                        return Ok(format!("File opened successfully from S3 (with meta header): {}", path));
                    },
                    Err(_) => {
                        // Failed with Auto, try dataset-only (no meta header)
                        // First read as InMemDicomObject to get dataset
                        let mem_obj = InMemDicomObject::read_dataset_with_ts(&data[..], 
                            &dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased())
                            .map_err(|e| napi::Error::from_reason(format!("Failed to parse DICOM dataset from S3: {}", e)))?;
                        
                        // Extract necessary info to build meta header
                        let sop_class_uid = mem_obj.element(tags::SOP_CLASS_UID)
                            .ok()
                            .and_then(|e| e.to_str().ok())
                            .map(|s| s.trim().to_string())
                            .unwrap_or_else(|| uids::SECONDARY_CAPTURE_IMAGE_STORAGE.to_string());
                        
                        let sop_instance_uid = mem_obj.element(tags::SOP_INSTANCE_UID)
                            .ok()
                            .and_then(|e| e.to_str().ok())
                            .map(|s| s.trim().to_string())
                            .ok_or_else(|| napi::Error::from_reason("SOP Instance UID not found in dataset".to_string()))?;
                        
                        let transfer_syntax = mem_obj.element(tags::TRANSFER_SYNTAX_UID)
                            .ok()
                            .and_then(|e| e.to_str().ok())
                            .map(|s| s.trim().to_string())
                            .unwrap_or_else(|| uids::IMPLICIT_VR_LITTLE_ENDIAN.to_string());
                        
                        // Build proper file meta information
                        let meta = FileMetaTableBuilder::new()
                            .media_storage_sop_class_uid(sop_class_uid)
                            .media_storage_sop_instance_uid(sop_instance_uid)
                            .transfer_syntax(transfer_syntax)
                            .implementation_class_uid("1.2.826.0.1.3680043.9.7433.1.1")
                            .implementation_version_name("node-dicom-rs")
                            .build()
                            .map_err(|e| napi::Error::from_reason(format!("Failed to build meta information: {}", e)))?;
                        
                        // Create FileDicomObject with proper meta
                        mem_obj.with_exact_meta(meta)
                    }
                };
                
                *self.dicom_file.lock().unwrap() = Some(dicom_file);
                Ok(format!("File opened successfully from S3 (dataset-only, meta created): {}", path))
            },
            StorageBackend::Filesystem => {
                // Read from filesystem with auto-detection first
                let resolved_path = self.resolve_path(&path);
                let result = OpenFileOptions::new()
                    .read_preamble(ReadPreamble::Auto)
                    .open_file(&resolved_path);
                
                let dicom_file = match result {
                    Ok(file) => {
                        *self.dicom_file.lock().unwrap() = Some(file);
                        return Ok(format!("File opened successfully (with meta header): {}", resolved_path.display()));
                    },
                    Err(_) => {
                        // Failed with Auto, try dataset-only (no meta header)
                        let file_data = std::fs::read(&resolved_path)
                            .map_err(|e| napi::Error::from_reason(format!("Failed to read file: {}", e)))?;
                        
                        // Read as InMemDicomObject to get dataset
                        let mem_obj = InMemDicomObject::read_dataset_with_ts(&file_data[..], 
                            &dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN.erased())
                            .map_err(|e| napi::Error::from_reason(format!("Failed to parse DICOM dataset: {}", e)))?;
                        
                        // Extract necessary info to build meta header
                        let sop_class_uid = mem_obj.element(tags::SOP_CLASS_UID)
                            .ok()
                            .and_then(|e| e.to_str().ok())
                            .map(|s| s.trim().to_string())
                            .unwrap_or_else(|| uids::SECONDARY_CAPTURE_IMAGE_STORAGE.to_string());
                        
                        let sop_instance_uid = mem_obj.element(tags::SOP_INSTANCE_UID)
                            .ok()
                            .and_then(|e| e.to_str().ok())
                            .map(|s| s.trim().to_string())
                            .ok_or_else(|| napi::Error::from_reason("SOP Instance UID not found in dataset".to_string()))?;
                        
                        let transfer_syntax = mem_obj.element(tags::TRANSFER_SYNTAX_UID)
                            .ok()
                            .and_then(|e| e.to_str().ok())
                            .map(|s| s.trim().to_string())
                            .unwrap_or_else(|| uids::IMPLICIT_VR_LITTLE_ENDIAN.to_string());
                        
                        // Build proper file meta information
                        let meta = FileMetaTableBuilder::new()
                            .media_storage_sop_class_uid(sop_class_uid)
                            .media_storage_sop_instance_uid(sop_instance_uid)
                            .transfer_syntax(transfer_syntax)
                            .implementation_class_uid("1.2.826.0.1.3680043.9.7433.1.1")
                            .implementation_version_name("node-dicom-rs")
                            .build()
                            .map_err(|e| napi::Error::from_reason(format!("Failed to build meta information: {}", e)))?;
                        
                        // Create FileDicomObject with proper meta
                        mem_obj.with_exact_meta(meta)
                    }
                };
                
                *self.dicom_file.lock().unwrap() = Some(dicom_file);
                Ok(format!("File opened successfully (dataset-only, meta created): {}", resolved_path.display()))
            }
        }
    }

    /**
     * Open and load a DICOM JSON file into memory.
     * 
     * Reads a DICOM file in JSON format (as specified by DICOM Part 18) and converts it
     * to an internal DICOM object representation. After opening, all standard operations
     * like `extract()`, `dump()`, and `saveAsDicom()` are available.
     * 
     * **Storage Backend:** Automatically uses S3 or filesystem based on the
     * `StorageConfig` provided in the constructor.
     * 
     * @param path - Path to the DICOM JSON file (filesystem path when using Filesystem backend, or S3 key when using S3 backend)
     * @returns Success message if the file was opened successfully
     * @throws Error if the file cannot be opened or is not valid DICOM JSON
     * 
     * @example
     * ```typescript
     * // Filesystem
     * const file = new DicomFile();
     * await file.openJson('/path/to/file.json');
     * const data = file.extract(['PatientName', 'StudyDate']);
     * 
     * // S3 backend
     * const fileS3 = new DicomFile({ backend: 'S3', s3Config: {...} });
     * await fileS3.openJson('folder/file.json');
     * ```
     */
    #[napi]
    pub async fn open_json(&self, path: String) -> napi::Result<String> {
        // Read JSON content from S3 or filesystem
        let json_content = match self.storage_config.backend {
            StorageBackend::S3 => {
                let data = self.read_from_s3(&path).await?;
                String::from_utf8(data)
                    .map_err(|e| napi::Error::from_reason(format!("Invalid UTF-8 in JSON file: {}", e)))?
            },
            StorageBackend::Filesystem => {
                let resolved_path = self.resolve_path(&path);
                std::fs::read_to_string(&resolved_path)
                    .map_err(|e| napi::Error::from_reason(format!("Failed to read JSON file: {}", e)))?
            }
        };
        
        self.parse_and_set_json(json_content, &path)
    }
    
    fn parse_and_set_json(&self, json_content: String, path: &str) -> napi::Result<String> {
        let json_content_ref = &json_content;
        
        // dicom_json::from_str returns InMemDicomObject directly
        let mem_obj = dicom_json::from_str::<dicom_object::InMemDicomObject>(json_content_ref)
            .map_err(|e| napi::Error::from_reason(format!("Failed to parse DICOM JSON: {}", e)))?;
        
        // Create file meta information for DefaultDicomObject
        use dicom_object::FileMetaTable;
        use dicom_dictionary_std::uids;
        
        // Extract or create necessary meta information
        let sop_class_uid = mem_obj.element(tags::SOP_CLASS_UID)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uids::SECONDARY_CAPTURE_IMAGE_STORAGE.to_string());
        
        let sop_instance_uid = mem_obj.element(tags::SOP_INSTANCE_UID)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "1.2.3.4.5.6.7.8.9".to_string());
        
        // Create file meta table
        let meta = FileMetaTable {
            information_group_length: 0, // Will be calculated on write
            information_version: [0, 1],
            media_storage_sop_class_uid: sop_class_uid,
            media_storage_sop_instance_uid: sop_instance_uid,
            transfer_syntax: uids::EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
            implementation_class_uid: "1.2.826.0.1.3680043.9.7433.1.1".to_string(),
            implementation_version_name: Some("node-dicom-rs".to_string()),
            source_application_entity_title: None,
            sending_application_entity_title: None,
            receiving_application_entity_title: None,
            private_information_creator_uid: None,
            private_information: None,
        };
        
        // Create FileDicomObject with meta and copy data from mem_obj
        use dicom_object::FileDicomObject;
        let mut dicom_obj = FileDicomObject::new_empty_with_dict_and_meta(
            dicom_object::StandardDataDictionary,
            meta
        );
        
        // Copy all elements from mem_obj to dicom_obj
        for elem in mem_obj.into_iter() {
            let _ = dicom_obj.put(elem);
        }
        
        *self.dicom_file.lock().unwrap() = Some(dicom_obj);
        Ok(format!("DICOM JSON file opened successfully: {}", path))
    }

    /**
     * Print a detailed dump of the DICOM file structure to stdout.
     * 
     * Displays all DICOM elements with their tags, VRs, and values in a human-readable format.
     * Useful for debugging and inspecting DICOM file contents.
     * 
     * @throws Error if no file is currently opened
     */
    #[napi]
    pub fn dump(&self) -> Result<(), JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }
        let dicom_ref = self.dicom_file.lock().unwrap();
        dicom_dump::dump_file(dicom_ref.as_ref().unwrap())
            .map_err(|e| JsError::from(napi::Error::from_reason(e.to_string())))
    }

    /**
     * Extract and save raw pixel data to a file.
     * 
     * Extracts the raw pixel data bytes from the DICOM file's PixelData element (7FE0,0010)
     * and writes them directly to a binary file. The data is saved as-is without any
     * decompression or conversion. Useful for extracting raw image data for custom processing.
     * 
     * Note: This does not decode or decompress the pixel data. For compressed transfer syntaxes
     * (e.g., JPEG, JPEG 2000), the output will be the compressed bitstream.
     * 
     * @param path - Output path where the raw pixel data will be saved
     * @returns Success message with the number of bytes written
     * @throws Error if no file is opened, pixel data is missing, or file write fails
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * file.open('image.dcm');
     * file.saveRawPixelData('output.raw');
     * ```
     */
    #[napi]
    pub fn save_raw_pixel_data(&self, path: String) -> Result<String, JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }
        
        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();
        let pixel_data = obj.element(tags::PIXEL_DATA)
            .map_err(|e| napi::Error::from_reason(format!("Pixel data not found: {}", e)))?;
        
        let data = pixel_data.to_bytes()
            .map_err(|e| napi::Error::from_reason(format!("Failed to read pixel data: {}", e)))?;
        
        std::fs::write(&path, &data)
            .map_err(|e| napi::Error::from_reason(format!("Failed to write file: {}", e)))?;
        
        Ok(format!("Pixel data saved successfully ({} bytes)", data.len()))
    }

    /**
     * Extract DICOM tags and return as flat key-value structure.
     * 
     * Extracts specified DICOM tags and returns them as a JSON string with a simple
     * flat structure (all tags at root level). Supports any tag name from the DICOM 
     * standard or hex format (e.g., "00100010").
     * 
     * ## Tag Name Formats
     * 
     * Tags can be specified in multiple formats:
     * - Standard name: "PatientName", "StudyDate", "Modality"
     * - Hex format: "00100010", "00080020", "00080060"
     * - Any valid DICOM tag from StandardDataDictionary
     * 
     * ## Custom Tags
     * 
     * Custom tags allow extraction of private or vendor-specific tags with user-defined names:
     * ```typescript
     * import { createCustomTag } from '@nuxthealth/node-dicom';
     * 
     * file.extract(
     *   ['PatientName'],
     *   [createCustomTag('00091001', 'VendorPrivateTag')]
     * );
     * ```
     * 
     * @param tagNames - Array of DICOM tag names or hex values to extract. Supports 300+ autocomplete suggestions.
     * @param customTags - Optional array of custom tag specifications for private/vendor tags
     * @returns JSON string containing extracted tags as flat key-value pairs
     * @throws Error if no file is opened or JSON serialization fails
     * 
     * @example
     * ```typescript
     * // Extract standard tags
     * const json = file.extract(['PatientName', 'StudyDate', 'Modality']);
     * const data = JSON.parse(json);
     * // { "PatientName": "DOE^JOHN", "StudyDate": "20240101", "Modality": "CT" }
     * 
     * // Use predefined tag sets
     * import { getCommonTagSets, combineTags } from '@nuxthealth/node-dicom';
     * const tags = getCommonTagSets();
     * const allTags = combineTags([tags.patientBasic, tags.studyBasic, tags.ct]);
     * const extracted = file.extract(allTags);
     * ```
     */
    #[napi(
        ts_args_type = "tagNames: Array<'AccessionNumber' | 'AcquisitionDate' | 'AcquisitionDateTime' | 'AcquisitionNumber' | 'AcquisitionTime' | 'ActualCardiacTriggerTimePriorToRPeak' | 'ActualFrameDuration' | 'AdditionalPatientHistory' | 'AdmissionID' | 'AdmittingDiagnosesDescription' | 'AnatomicalOrientationType' | 'AnatomicRegionSequence' | 'AnodeTargetMaterial' | 'BeamLimitingDeviceAngle' | 'BitsAllocated' | 'BitsStored' | 'BluePaletteColorLookupTableDescriptor' | 'BodyPartExamined' | 'BodyPartThickness' | 'BranchOfService' | 'BurnedInAnnotation' | 'ChannelSensitivity' | 'CineRate' | 'CollimatorType' | 'Columns' | 'CompressionForce' | 'ContentDate' | 'ContentTime' | 'ContrastBolusAgent' | 'ContrastBolusIngredient' | 'ContrastBolusIngredientConcentration' | 'ContrastBolusRoute' | 'ContrastBolusStartTime' | 'ContrastBolusStopTime' | 'ContrastBolusTotalDose' | 'ContrastBolusVolume' | 'ContrastFlowDuration' | 'ContrastFlowRate' | 'ConvolutionKernel' | 'CorrectedImage' | 'CountsSource' | 'DataCollectionDiameter' | 'DecayCorrection' | 'DeidentificationMethod' | 'DerivationDescription' | 'DetectorTemperature' | 'DeviceSerialNumber' | 'DistanceSourceToDetector' | 'DistanceSourceToPatient' | 'EchoTime' | 'EthnicGroup' | 'Exposure' | 'ExposureInMicroAmpereSeconds' | 'ExposureTime' | 'FilterType' | 'FlipAngle' | 'FocalSpots' | 'FrameDelay' | 'FrameIncrementPointer' | 'FrameOfReferenceUID' | 'FrameTime' | 'GantryAngle' | 'GeneratorPower' | 'GraphicAnnotationSequence' | 'GreenPaletteColorLookupTableDescriptor' | 'HeartRate' | 'HighBit' | 'ImageComments' | 'ImageLaterality' | 'ImageOrientationPatient' | 'ImagePositionPatient' | 'ImagerPixelSpacing' | 'ImageTriggerDelay' | 'ImageType' | 'ImagingFrequency' | 'ImplementationClassUID' | 'ImplementationVersionName' | 'InstanceCreationDate' | 'InstanceCreationTime' | 'InstanceNumber' | 'InstitutionName' | 'IntensifierSize' | 'IssuerOfAdmissionID' | 'KVP' | 'LargestImagePixelValue' | 'LargestPixelValueInSeries' | 'Laterality' | 'LossyImageCompression' | 'LossyImageCompressionMethod' | 'LossyImageCompressionRatio' | 'MagneticFieldStrength' | 'Manufacturer' | 'ManufacturerModelName' | 'MedicalRecordLocator' | 'MilitaryRank' | 'Modality' | 'MultiplexGroupTimeOffset' | 'NameOfPhysiciansReadingStudy' | 'NominalCardiacTriggerDelayTime' | 'NominalInterval' | 'NumberOfFrames' | 'NumberOfSlices' | 'NumberOfTemporalPositions' | 'NumberOfWaveformChannels' | 'NumberOfWaveformSamples' | 'Occupation' | 'OperatorsName' | 'OtherPatientIDs' | 'OtherPatientNames' | 'OverlayBitPosition' | 'OverlayBitsAllocated' | 'OverlayColumns' | 'OverlayData' | 'OverlayOrigin' | 'OverlayRows' | 'OverlayType' | 'PaddleDescription' | 'PatientAge' | 'PatientBirthDate' | 'PatientBreedDescription' | 'PatientComments' | 'PatientID' | 'PatientIdentityRemoved' | 'PatientName' | 'PatientPosition' | 'PatientSex' | 'PatientSize' | 'PatientSpeciesDescription' | 'PatientSupportAngle' | 'PatientTelephoneNumbers' | 'PatientWeight' | 'PerformedProcedureStepDescription' | 'PerformedProcedureStepID' | 'PerformedProcedureStepStartDate' | 'PerformedProcedureStepStartTime' | 'PerformedProtocolCodeSequence' | 'PerformingPhysicianName' | 'PhotometricInterpretation' | 'PhysiciansOfRecord' | 'PixelAspectRatio' | 'PixelPaddingRangeLimit' | 'PixelPaddingValue' | 'PixelRepresentation' | 'PixelSpacing' | 'PlanarConfiguration' | 'PositionerPrimaryAngle' | 'PositionerSecondaryAngle' | 'PositionReferenceIndicator' | 'PreferredPlaybackSequencing' | 'PresentationIntentType' | 'PresentationLUTShape' | 'PrimaryAnatomicStructureSequence' | 'PrivateInformationCreatorUID' | 'ProtocolName' | 'QualityControlImage' | 'RadiationMachineName' | 'RadiationSetting' | 'RadionuclideTotalDose' | 'RadiopharmaceuticalInformationSequence' | 'RadiopharmaceuticalStartDateTime' | 'RadiopharmaceuticalStartTime' | 'RadiopharmaceuticalVolume' | 'ReasonForTheRequestedProcedure' | 'ReceivingApplicationEntityTitle' | 'RecognizableVisualFeatures' | 'RecommendedDisplayFrameRate' | 'ReconstructionDiameter' | 'ReconstructionTargetCenterPatient' | 'RedPaletteColorLookupTableDescriptor' | 'ReferencedBeamNumber' | 'ReferencedImageSequence' | 'ReferencedPatientPhotoSequence' | 'ReferencedPerformedProcedureStepSequence' | 'ReferencedRTPlanSequence' | 'ReferencedSOPClassUID' | 'ReferencedSOPInstanceUID' | 'ReferencedStudySequence' | 'ReferringPhysicianName' | 'RepetitionTime' | 'RequestAttributesSequence' | 'RequestedContrastAgent' | 'RequestedProcedureDescription' | 'RequestedProcedureID' | 'RequestingPhysician' | 'RescaleIntercept' | 'RescaleSlope' | 'RescaleType' | 'ResponsibleOrganization' | 'ResponsiblePerson' | 'ResponsiblePersonRole' | 'Rows' | 'RTImageDescription' | 'RTImageLabel' | 'SamplesPerPixel' | 'SamplingFrequency' | 'ScanningSequence' | 'SendingApplicationEntityTitle' | 'SeriesDate' | 'SeriesDescription' | 'SeriesInstanceUID' | 'SeriesNumber' | 'SeriesTime' | 'SeriesType' | 'SliceLocation' | 'SliceThickness' | 'SmallestImagePixelValue' | 'SmallestPixelValueInSeries' | 'SoftwareVersions' | 'SOPClassUID' | 'SOPInstanceUID' | 'SoundPathLength' | 'SourceApplicationEntityTitle' | 'SourceImageSequence' | 'SpacingBetweenSlices' | 'SpecificCharacterSet' | 'StationName' | 'StudyComments' | 'StudyDate' | 'StudyDescription' | 'StudyID' | 'StudyInstanceUID' | 'StudyTime' | 'TableHeight' | 'TableTopLateralPosition' | 'TableTopLongitudinalPosition' | 'TableTopVerticalPosition' | 'TableType' | 'TemporalPositionIdentifier' | 'TemporalResolution' | 'TextObjectSequence' | 'TimezoneOffsetFromUTC' | 'TransducerFrequency' | 'TransducerType' | 'TransferSyntaxUID' | 'TriggerTime' | 'TriggerTimeOffset' | 'UltrasoundColorDataPresent' | 'Units' | 'VOILUTFunction' | 'WaveformOriginality' | 'WaveformSequence' | 'WindowCenter' | 'WindowCenterWidthExplanation' | 'WindowWidth' | 'XRayTubeCurrent' | (string & {})>, customTags?: Array<CustomTag>"
    )]
    pub fn extract(
        &self,
        tag_names: Vec<String>, 
        custom_tags: Option<Vec<CustomTag>>
    ) -> Result<HashMap<String, String>, JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }
        
        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();
        let custom = custom_tags.unwrap_or_default();
        
        // Always use flat extraction for consistency
        let result = extract_tags_flat(obj, &tag_names, &custom);
        
        Ok(result)
    }

    /**
     * Update DICOM tag values in the currently opened file.
     * 
     * Modifies one or more DICOM tag values in memory. Changes are not persisted to disk
     * until you call `saveAsDicom()`. Useful for anonymization, correcting metadata,
     * or updating values before saving. Supports standard tag names and hex format.
     * 
     * **Important Notes:**
     * - Changes are made in-memory only
     * - Call `saveAsDicom()` to persist changes
     * - Cannot modify meta information tags (file preamble)
     * - Cannot modify pixel data with this method
     * - For new tags, appropriate VR (Value Representation) is auto-detected
     * 
     * @param updates - Object with tag names as keys and new values as strings
     * @returns Success message with number of tags updated
     * @throws Error if no file is opened or tag update fails
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * await file.open('original.dcm');
     * 
     * // Update multiple tags
     * file.updateTags({
     *     PatientName: 'ANONYMOUS',
     *     PatientID: 'ANON001',
     *     StudyDescription: 'Anonymized Study',
     *     SeriesDescription: 'Anonymized Series'
     * });
     * 
     * // Save changes
     * await file.saveAsDicom('anonymized.dcm');
     * file.close();
     * ```
     * 
     * @example
     * ```typescript
     * // Anonymization workflow
     * const file = new DicomFile();
     * await file.open('patient-scan.dcm');
     * 
     * file.updateTags({
     *     PatientName: 'ANONYMOUS',
     *     PatientID: crypto.randomUUID(),
     *     PatientBirthDate: '',
     *     PatientSex: '',
     *     PatientAge: '',
     *     InstitutionName: 'ANONYMIZED',
     *     ReferringPhysicianName: '',
     *     PerformingPhysicianName: ''
     * });
     * 
     * await file.saveAsDicom('anonymized-scan.dcm');
     * file.close();
     * ```
     * 
     * @example
     * ```typescript
     * // Update using hex tag format
     * file.updateTags({
     *     '00100010': 'DOE^JANE',        // PatientName
     *     '00100020': 'PAT12345',         // PatientID
     *     '00080020': '20240101'          // StudyDate
     * });
     * ```
     */
    #[napi]
    pub fn update_tags(&self, updates: HashMap<String, String>) -> Result<String, JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }
        
        use dicom_dictionary_std::StandardDataDictionary;
        use dicom_core::DataDictionary;
        use dicom_core::VR;
        use dicom_object::mem::InMemElement;
        use dicom_core::PrimitiveValue;
        
        let dict = StandardDataDictionary;
        let mut dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_mut().unwrap();
        let mut updated_count = 0;
        
        for (tag_name, value) in updates {
            // Try to resolve tag from name or hex format
            let tag = if tag_name.starts_with("(") && tag_name.ends_with(")") {
                // Format: (GGGG,EEEE)
                let inner = &tag_name[1..tag_name.len()-1];
                let parts: Vec<&str> = inner.split(',').collect();
                if parts.len() == 2 {
                    let group = u16::from_str_radix(parts[0].trim(), 16)
                        .map_err(|_| JsError::from(napi::Error::from_reason(format!("Invalid tag format: {}", tag_name))))?;
                    let element = u16::from_str_radix(parts[1].trim(), 16)
                        .map_err(|_| JsError::from(napi::Error::from_reason(format!("Invalid tag format: {}", tag_name))))?;
                    Tag(group, element)
                } else {
                    return Err(JsError::from(napi::Error::from_reason(format!("Invalid tag format: {}", tag_name))));
                }
            } else if tag_name.len() == 8 && tag_name.chars().all(|c| c.is_ascii_hexdigit()) {
                // Format: GGGGEEEE
                let group = u16::from_str_radix(&tag_name[0..4], 16)
                    .map_err(|_| JsError::from(napi::Error::from_reason(format!("Invalid hex tag: {}", tag_name))))?;
                let element = u16::from_str_radix(&tag_name[4..8], 16)
                    .map_err(|_| JsError::from(napi::Error::from_reason(format!("Invalid hex tag: {}", tag_name))))?;
                Tag(group, element)
            } else {
                // Try to find by name
                let entry = dict.by_name(&tag_name)
                    .ok_or_else(|| JsError::from(napi::Error::from_reason(format!("Unknown tag name: {}", tag_name))))?;
                entry.tag.inner()
            };
            
            // Don't allow modifying meta information tags
            if tag.0 == 0x0002 {
                return Err(JsError::from(napi::Error::from_reason(
                    format!("Cannot modify file meta information tag: {}", tag_name)
                )));
            }
            
            // Don't allow modifying pixel data with this method
            if tag == tags::PIXEL_DATA {
                return Err(JsError::from(napi::Error::from_reason(
                    "Cannot modify pixel data with updateTags(). Use pixel processing methods instead.".to_string()
                )));
            }
            
            // Get VR for this tag (use existing or look up in dictionary)
            let vr = obj.element(tag)
                .ok()
                .map(|e| e.vr())
                .or_else(|| dict.by_tag(tag).map(|entry| entry.vr.relaxed()))
                .unwrap_or(VR::LO); // Default to LO (Long String) if unknown
            
            // Create new element with the value
            let element = if value.is_empty() {
                // Empty value - create element with no data
                InMemElement::new(tag, vr, PrimitiveValue::Empty)
            } else {
                // Non-empty value - create appropriate primitive value based on VR
                let prim_value = match vr {
                    VR::AE | VR::AS | VR::CS | VR::DA | VR::DS | VR::DT | VR::IS | VR::LO | 
                    VR::LT | VR::PN | VR::SH | VR::ST | VR::TM | VR::UC | VR::UI | VR::UR | VR::UT => {
                        PrimitiveValue::Str(value.clone())
                    },
                    _ => {
                        // For other VRs, try to parse as string or store as bytes
                        PrimitiveValue::Str(value.clone())
                    }
                };
                
                InMemElement::new(tag, vr, prim_value)
            };
            
            // Put the element (replaces existing or adds new)
            obj.put(element);
            updated_count += 1;
        }
        
        Ok(format!("Successfully updated {} tag(s). Call saveAsDicom() to persist changes.", updated_count))
    }

    /**
     * Get comprehensive information about pixel data in the DICOM file.
     * 
     * Extracts metadata about the image dimensions, bit depth, photometric interpretation,
     * compression status, and windowing parameters without decoding the actual pixel data.
     * 
     * @returns PixelDataInfo object with detailed pixel data metadata
     * @throws Error if no file is opened or pixel data is missing
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * file.open('ct_scan.dcm');
     * const info = file.getPixelDataInfo();
     * console.log(`${info.width}x${info.height}, ${info.frames} frames`);
     * console.log(`Bits: ${info.bitsStored}, Compressed: ${info.isCompressed}`);
     * ```
     */
    #[napi]
    pub fn get_pixel_data_info(&self) -> napi::Result<PixelDataInfo> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(napi::Error::from_reason("File not opened. Call open() first.".to_string()));
        }
        
        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();
        
        // Get required attributes
        let rows = obj.element(tags::ROWS)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read Rows: {}", e)))?  
            .to_int::<u32>()
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert Rows: {}", e)))?;
        
        let columns = obj.element(tags::COLUMNS)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read Columns: {}", e)))?  
            .to_int::<u32>()
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert Columns: {}", e)))?;
        
        let bits_allocated = obj.element(tags::BITS_ALLOCATED)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read BitsAllocated: {}", e)))?  
            .to_int::<u16>()
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert BitsAllocated: {}", e)))?;
        
        let bits_stored = obj.element(tags::BITS_STORED)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read BitsStored: {}", e)))?  
            .to_int::<u16>()
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert BitsStored: {}", e)))?;
        
        let high_bit = obj.element(tags::HIGH_BIT)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read HighBit: {}", e)))?  
            .to_int::<u16>()
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert HighBit: {}", e)))?;
        
        let pixel_representation = obj.element(tags::PIXEL_REPRESENTATION)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read PixelRepresentation: {}", e)))?  
            .to_int::<u16>()
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert PixelRepresentation: {}", e)))?;
        
        let samples_per_pixel = obj.element(tags::SAMPLES_PER_PIXEL)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read SamplesPerPixel: {}", e)))?  
            .to_int::<u16>()
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert SamplesPerPixel: {}", e)))?;
        
        let photometric_interpretation = obj.element(tags::PHOTOMETRIC_INTERPRETATION)
            .map_err(|e| napi::Error::from_reason(format!("Failed to read PhotometricInterpretation: {}", e)))?  
            .to_str()
            .map(|s| s.to_string())
            .map_err(|e| napi::Error::from_reason(format!("Failed to convert PhotometricInterpretation: {}", e)))?;
        
        // Optional attributes
        let frames = obj.element(tags::NUMBER_OF_FRAMES)
            .ok()
            .and_then(|e| e.to_int::<u32>().ok())
            .unwrap_or(1);
        
        let transfer_syntax_uid = obj.meta().transfer_syntax.trim_end_matches('\0').to_string();
        
        // Check if compressed (common compressed transfer syntaxes)
        let is_compressed = !transfer_syntax_uid.starts_with("1.2.840.10008.1.2.1") // Explicit VR Little Endian
            && !transfer_syntax_uid.starts_with("1.2.840.10008.1.2.2") // Explicit VR Big Endian
            && transfer_syntax_uid != "1.2.840.10008.1.2"; // Implicit VR Little Endian
        
        // Get pixel data size
        let pixel_data = obj.element(tags::PIXEL_DATA)
            .map_err(|e| napi::Error::from_reason(format!("Pixel data not found: {}", e)))?;
        
        let data_size = pixel_data.to_bytes()
            .map(|b| b.len() as u32)
            .unwrap_or(0);
        
        // Optional windowing parameters
        let rescale_intercept = obj.element(tags::RESCALE_INTERCEPT)
            .ok()
            .and_then(|e| e.to_float64().ok());
        
        let rescale_slope = obj.element(tags::RESCALE_SLOPE)
            .ok()
            .and_then(|e| e.to_float64().ok());
        
        let window_center = obj.element(tags::WINDOW_CENTER)
            .ok()
            .and_then(|e| e.to_float64().ok());
        
        let window_width = obj.element(tags::WINDOW_WIDTH)
            .ok()
            .and_then(|e| e.to_float64().ok());
        
        Ok(PixelDataInfo {
            width: columns,
            height: rows,
            frames,
            bits_allocated,
            bits_stored,
            high_bit,
            pixel_representation,
            samples_per_pixel,
            photometric_interpretation,
            transfer_syntax_uid,
            is_compressed,
            data_size,
            rescale_intercept,
            rescale_slope,
            window_center,
            window_width,
        })
    }

    /**
     * Process and extract pixel data with flexible options.
     * 
     * Advanced pixel data processing supporting:
     * - Raw extraction or decoded/decompressed output
    * - Multiple output formats (Raw, PNG, JPEG, BMP, JSON)
     * - Frame extraction (single or all frames)
     * - Windowing and 8-bit conversion
     * - VOI LUT application
     * 
     * @param options - Processing options
     * @returns Success message with processing details
     * @throws Error if processing fails or required features are not available
     * 
     * @example
     * ```typescript
     * // Extract raw pixel data
     * await file.processPixelData({
     *   outputPath: 'output.raw',
     *   format: 'Raw'
     * });
     * 
     * // Decode and save as PNG with windowing
     * await file.processPixelData({
     *   outputPath: 'output.png',
     *   format: 'Png',
     *   decode: true,
     *   applyVoiLut: true,
     *   convertTo8bit: true
     * });
     * 
     * // Extract specific frame
     * await file.processPixelData({
     *   outputPath: 'frame_5.raw',
     *   format: 'Raw',
     *   frameNumber: 5
     * });
     * 
     * // Get metadata as JSON
     * await file.processPixelData({
     *   outputPath: 'info.json',
     *   format: 'Json'
     * });
     * ```
     */
    #[napi]
    pub async fn process_pixel_data(&self, options: PixelDataOptions) -> napi::Result<String> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(napi::Error::from_reason("File not opened. Call open() first.".to_string()));
        }
        
        let format = options.format.unwrap_or(PixelDataFormat::Raw);
        let decode = options.decode.unwrap_or(false);
        
        // Handle JSON format - just return metadata
        if matches!(format, PixelDataFormat::Json) {
            let info = self.get_pixel_data_info()?;
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "width": info.width,
                "height": info.height,
                "frames": info.frames,
                "bitsAllocated": info.bits_allocated,
                "bitsStored": info.bits_stored,
                "highBit": info.high_bit,
                "pixelRepresentation": info.pixel_representation,
                "samplesPerPixel": info.samples_per_pixel,
                "photometricInterpretation": info.photometric_interpretation,
                "transferSyntaxUid": info.transfer_syntax_uid,
                "isCompressed": info.is_compressed,
                "dataSizeBytes": info.data_size,
                "rescaleIntercept": info.rescale_intercept,
                "rescaleSlope": info.rescale_slope,
                "windowCenter": info.window_center,
                "windowWidth": info.window_width,
            }))
            .map_err(|e| napi::Error::from_reason(format!("Failed to serialize JSON: {}", e)))?;
            
            std::fs::write(&options.output_path, json)
                .map_err(|e| napi::Error::from_reason(format!("Failed to write JSON file: {}", e)))?;
            
            return Ok(format!("Pixel data metadata saved to {}", options.output_path));
        }
        
        // Handle raw format without decoding
        if matches!(format, PixelDataFormat::Raw) && !decode {
            let dicom_ref = self.dicom_file.lock().unwrap();
            let obj = dicom_ref.as_ref().unwrap();
            
            let pixel_data = obj.element(tags::PIXEL_DATA)
                .map_err(|e| napi::Error::from_reason(format!("Pixel data not found: {}", e)))?;
            
            let data = pixel_data.to_bytes()
                .map_err(|e| napi::Error::from_reason(format!("Failed to read pixel data: {}", e)))?;
            
            std::fs::write(&options.output_path, &data)
                .map_err(|e| napi::Error::from_reason(format!("Failed to write file: {}", e)))?;
            
            return Ok(format!("Raw pixel data saved to {} ({} bytes)", options.output_path, data.len()));
        }
        
        // Image rendering required (PNG/JPEG/BMP)
        if matches!(format, PixelDataFormat::Png)
            || matches!(format, PixelDataFormat::Jpeg)
            || matches!(format, PixelDataFormat::Bmp)
        {
            let (output, format_str) = self
                .render_image_buffer(
                    format,
                    options.frame_number,
                    options.apply_voi_lut,
                    options.window_center,
                    options.window_width,
                    options.convert_to_8bit,
                    Some(90),
                )?;

            std::fs::write(&options.output_path, &output)
                .map_err(|e| napi::Error::from_reason(format!("Failed to write file: {}", e)))?;

            return Ok(format!(
                "{} image saved to {} ({} bytes)",
                format_str,
                options.output_path,
                output.len()
            ));
        }
        
        // Fallback: decoded raw format
        #[cfg(not(feature = "transcode"))]
        {
            return Err(napi::Error::from_reason(
                "Pixel data decoding requires the 'transcode' feature. Rebuild with --features transcode".to_string()
            ));
        }
        
        #[cfg(feature = "transcode")]
        {
            // Get pixel data info first
            let info = self.get_pixel_data_info()?;
            
            let dicom_ref = self.dicom_file.lock().unwrap();
            let obj = dicom_ref.as_ref().unwrap();
            
            // Get pixel data (decode if compressed, otherwise get raw)
            let bytes = if info.is_compressed {
                let decoded = obj.decode_pixel_data()
                    .map_err(|e| napi::Error::from_reason(format!("Failed to decode pixel data: {}", e)))?;
                decoded.to_vec()
                    .map_err(|e| napi::Error::from_reason(format!("Failed to convert pixel data: {}", e)))?
            } else {
                let pixel_data = obj.element(tags::PIXEL_DATA)
                    .map_err(|e| napi::Error::from_reason(format!("Pixel data not found: {}", e)))?;
                pixel_data.to_bytes()
                    .map_err(|e| napi::Error::from_reason(format!("Failed to read pixel data: {}", e)))?
                    .to_vec()
            };
            
            // Handle frame extraction if requested
            if let Some(frame_num) = options.frame_number {
                if frame_num >= info.frames {
                    return Err(napi::Error::from_reason(
                        format!("Frame number {} out of range (0-{})", frame_num, info.frames - 1)
                    ));
                }
                // TODO: Extract specific frame for raw format
                return Err(napi::Error::from_reason(
                    "Frame extraction for raw decoded format not yet implemented. Use PNG/JPEG format instead.".to_string()
                ));
            }
            
            // Save decoded data
            std::fs::write(&options.output_path, &bytes)
                .map_err(|e| napi::Error::from_reason(format!("Failed to write file: {}", e)))?;
            
            Ok(format!(
                "Decoded pixel data saved to {} ({} bytes, {}x{}, {} frames)",
                options.output_path,
                bytes.len(),
                info.width,
                info.height,
                info.frames
            ))
        }
    }

    /**
     * Save the currently opened DICOM file as JSON format.
     * 
     * Converts the DICOM object to JSON representation according to DICOM Part 18
     * standard and saves it to the specified path.
     * 
     * **Storage Backend:** Automatically uses S3 or filesystem based on the
     * `StorageConfig` provided in the constructor.
     * 
     * @param path - Output path for the JSON file (filesystem path when using Filesystem backend, or S3 key when using S3 backend)
     * @param pretty - Pretty print the JSON (default: true)
     * @returns Success message with file size
     * @throws Error if no file is opened or JSON conversion fails
     * 
     * @example
     * ```typescript
     * // Filesystem
     * const file = new DicomFile();
     * await file.open('image.dcm');
     * await file.saveAsJson('output.json', true);
     * 
     * // S3 backend
     * const fileS3 = new DicomFile({ backend: 'S3', s3Config: {...} });
     * await fileS3.open('input.dcm');
     * await fileS3.saveAsJson('output.json', true); // Saves to S3
     * ```
     */
    #[napi]
    pub async fn save_as_json(&self, path: String, pretty: Option<bool>) -> napi::Result<String> {
        // Use helper method to get JSON string
        let json_string = self.dicom_to_json_string(pretty)?;
        
        match self.storage_config.backend {
            StorageBackend::S3 => {
                self.write_to_s3(&path, json_string.as_bytes()).await?;
                Ok(format!("DICOM saved as JSON to S3: {} ({} bytes)", path, json_string.len()))
            },
            StorageBackend::Filesystem => {
                let resolved_path = self.resolve_path(&path);
                std::fs::write(&resolved_path, &json_string)
                    .map_err(|e| napi::Error::from_reason(format!("Failed to write JSON file: {}", e)))?;
                Ok(format!("DICOM saved as JSON to {} ({} bytes)", resolved_path.display(), json_string.len()))
            }
        }
    }    /**
     * Save the currently opened DICOM file (regardless of original format) as standard DICOM.
     * 
     * Writes the DICOM object as a standard .dcm file with proper file meta information.
     * Useful for converting DICOM JSON back to binary DICOM format, or saving modified files.
     * 
     * **Storage Backend:** Automatically uses S3 or filesystem based on the
     * `StorageConfig` provided in the constructor.
     * 
     * @param path - Output path for the DICOM file (filesystem path when using Filesystem backend, or S3 key when using S3 backend)
     * @returns Success message
     * @throws Error if no file is opened or write fails
     * 
     * @example
     * ```typescript
     * // Convert JSON to DICOM (filesystem)
     * const file = new DicomFile();
     * await file.openJson('input.json');
     * await file.saveAsDicom('output.dcm');
     * 
     * // Modify and save DICOM file
     * await file.open('original.dcm');
     * file.updateTags({ PatientName: 'ANONYMOUS' });
     * await file.saveAsDicom('anonymized.dcm');
     * 
     * // S3 backend
     * const fileS3 = new DicomFile({ backend: 'S3', s3Config: {...} });
     * await fileS3.openJson('input.json');
     * await fileS3.saveAsDicom('output.dcm'); // Saves to S3
     * ```
     */
    #[napi]
    pub async fn save_as_dicom(&self, path: String) -> napi::Result<String> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(napi::Error::from_reason("File not opened. Call open() first.".to_string()));
        }
        
        match self.storage_config.backend {
            StorageBackend::S3 => {
                // Write to buffer without holding borrow across await
                let buffer = {
                    let dicom_ref = self.dicom_file.lock().unwrap();
                    let obj = dicom_ref.as_ref().unwrap();
                    let mut buf = Vec::new();
                    obj.write_all(&mut buf)
                        .map_err(|e| napi::Error::from_reason(format!("Failed to write DICOM to buffer: {}", e)))?;
                    buf
                }; // borrow dropped here
                
                // Upload to S3
                self.write_to_s3(&path, &buffer).await?;
                Ok(format!("DICOM file saved to S3: {} ({} bytes)", path, buffer.len()))
            },
            StorageBackend::Filesystem => {
                let resolved_path = self.resolve_path(&path);
                let dicom_ref = self.dicom_file.lock().unwrap();
                let obj = dicom_ref.as_ref().unwrap();
                obj.write_to_file(&resolved_path)
                    .map_err(|e| napi::Error::from_reason(format!("Failed to write DICOM file: {}", e)))?;
                Ok(format!("DICOM file saved to {}", resolved_path.display()))
            }
        }
    }

    /**
     * Get the DICOM file as a JSON string.
     * 
     * Converts the entire DICOM dataset to JSON format (DICOM Part 18 JSON Model).
     * Returns the JSON string directly without writing to a file. Use `saveAsJson()`
     * if you want to save to a file instead.
     * 
     * @param pretty - Whether to format the JSON with indentation (default: true)
     * @returns JSON string representing the DICOM file
     * @throws Error if no file is opened or serialization fails
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * await file.open('scan.dcm');
     * 
     * // Get pretty-printed JSON
     * const json = file.toJson(true);
     * const obj = JSON.parse(json);
     * console.log(obj);
     * 
     * // Get compact JSON
     * const compactJson = file.toJson(false);
     * 
     * file.close();
     * ```
     */
    #[napi]
    pub fn to_json(&self, pretty: Option<bool>) -> Result<String, JsError> {
        self.dicom_to_json_string(pretty)
            .map_err(|e| JsError::from(e))
    }

    /**
     * Get raw pixel data as a Buffer.
     * 
     * Extracts the raw pixel data bytes from the DICOM file's PixelData element (7FE0,0010).
     * Returns the data as-is without any decoding or decompression. For compressed transfer
     * syntaxes, the data will be in its compressed form.
     * 
     * To save to a file instead, use `saveRawPixelData()`.
     * 
     * @returns Buffer containing the raw pixel data bytes
     * @throws Error if no file is opened or pixel data not found
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * await file.open('image.dcm');
     * 
     * const pixelBuffer = file.getPixelData();
     * console.log(`Pixel data size: ${pixelBuffer.length} bytes`);
     * 
     * // Process the buffer
     * processPixelData(pixelBuffer);
     * 
     * file.close();
     * ```
     */
    #[napi]
    pub fn get_pixel_data(&self) -> Result<napi::bindgen_prelude::Buffer, JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }
        
        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();
        
        let pixel_data = obj.element(tags::PIXEL_DATA)
            .map_err(|e| JsError::from(napi::Error::from_reason(format!("Pixel data not found: {}", e))))?;
        
        let data = pixel_data.to_bytes()
            .map_err(|e| JsError::from(napi::Error::from_reason(format!("Failed to read pixel data: {}", e))))?;
        
        Ok(data.to_vec().into())
    }

    /**
     * Decode and get pixel data as a Buffer.
     * 
     * Decodes compressed or encapsulated pixel data and returns it as raw uncompressed bytes.
     * Requires the 'transcode' feature to be enabled at build time. For uncompressed data,
     * use `getPixelData()` instead for better performance.
     * 
     * @returns Buffer containing decoded pixel data
     * @throws Error if no file is opened, transcode feature not enabled, or decoding fails
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * await file.open('compressed-image.dcm');
     * 
     * const info = file.getPixelDataInfo();
     * if (info.isCompressed) {
     *     // Decode compressed data
     *     const decodedBuffer = file.getDecodedPixelData();
     *     console.log(`Decoded size: ${decodedBuffer.length} bytes`);
     * }
     * 
     * file.close();
     * ```
     */
    #[napi]
    pub fn get_decoded_pixel_data(&self) -> Result<napi::bindgen_prelude::Buffer, JsError> {
        #[cfg(not(feature = "transcode"))]
        {
            return Err(JsError::from(napi::Error::from_reason(
                "Pixel data decoding requires the 'transcode' feature. Rebuild with --features transcode".to_string()
            )));
        }
        
        #[cfg(feature = "transcode")]
        {
            if self.dicom_file.lock().unwrap().is_none() {
                return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
            }
            
            let dicom_ref = self.dicom_file.lock().unwrap();
            let obj = dicom_ref.as_ref().unwrap();
            
            let decoded = obj.decode_pixel_data()
                .map_err(|e| JsError::from(napi::Error::from_reason(format!("Failed to decode pixel data: {}", e))))?;
            
            let bytes = decoded.to_vec()
                .map_err(|e| JsError::from(napi::Error::from_reason(format!("Failed to convert pixel data: {}", e))))?;
            
            Ok(bytes.into())
        }
    }

    /**
     * Get decoded and processed pixel data as a Buffer with advanced options.
     * 
     * This method combines decoding with optional processing steps like frame extraction,
     * windowing (VOI LUT), and 8-bit conversion. Returns processed pixel data in-memory
     * without file I/O. Uses the shared image processing utility for consistent behavior
     * across WADO-RS and DicomFile APIs.
     * 
     * **Processing Pipeline:**
     * 1. Decode/decompress pixel data
     * 2. Apply rescale (slope/intercept) if present
     * 3. Apply windowing/VOI LUT (if requested)
     * 4. Extract specific frame (if frameNumber specified)
     * 5. Convert to 8-bit (if requested)
     * 
     * @param options - Processing options (all optional)
     * @returns Buffer containing processed pixel data (raw bytes, not encoded image)
     * @throws Error if no file is opened or processing fails
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * await file.open('ct-scan.dcm');
     * 
     * // Get decoded data with default windowing from file
     * const windowed = file.getProcessedPixelData({
     *     applyVoiLut: true,
     *     convertTo8bit: true
     * });
     * 
     * // Custom window for bone visualization (CT)
     * const boneWindow = file.getProcessedPixelData({
     *     windowCenter: 300,
     *     windowWidth: 1500,
     *     convertTo8bit: true
     * });
     * 
     * // Extract specific frame from multi-frame image
     * const frame5 = file.getProcessedPixelData({
     *     frameNumber: 5
     * });
     * 
     * // Complete processing pipeline
     * const processed = file.getProcessedPixelData({
     *     frameNumber: 0,
     *     applyVoiLut: true,
     *     windowCenter: 40,    // Soft tissue window
     *     windowWidth: 400,
     *     convertTo8bit: true
     * });
     * 
     * file.close();
     * ```
     */
    #[napi]
    pub fn get_processed_pixel_data(&self, options: Option<PixelDataProcessingOptions>) -> Result<napi::bindgen_prelude::Buffer, JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }
        
        let opts = options.unwrap_or(PixelDataProcessingOptions {
            frame_number: None,
            apply_voi_lut: None,
            window_center: None,
            window_width: None,
            convert_to_8bit: None,
        });

        let (bmp_data, _) = self
            .render_image_buffer(
            PixelDataFormat::Bmp,
            opts.frame_number,
            opts.apply_voi_lut,
            opts.window_center,
            opts.window_width,
            opts.convert_to_8bit,
            None,
        )
            .map_err(JsError::from)?;
        
        // BMP file format: 14-byte file header + 40-byte DIB header + pixel data
        // Skip the headers to get raw pixel data
        const BMP_HEADER_SIZE: usize = 14 + 40; // File header + DIB header (BITMAPINFOHEADER)
        
        if bmp_data.len() <= BMP_HEADER_SIZE {
            return Err(JsError::from(napi::Error::from_reason(
                "Invalid BMP data: too small".to_string()
            )));
        }
        
        // Extract just the pixel data (skip BMP headers)
        let pixel_data = &bmp_data[BMP_HEADER_SIZE..];
        
        Ok(pixel_data.to_vec().into())
    }

    /**
     * Get encoded image bytes as Buffer (PNG/JPEG/BMP).
     *
     * This returns a fully encoded image in-memory and can be used for HTTP responses,
     * database blobs, or custom storage without writing to disk first.
     *
     * @param options - Optional image buffer options
     * @returns Buffer with encoded image bytes
     */
    #[napi]
    pub fn get_image_buffer(
        &self,
        options: Option<PixelDataImageBufferOptions>,
    ) -> Result<napi::bindgen_prelude::Buffer, JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason(
                "File not opened. Call open() first.".to_string(),
            )));
        }

        let opts = options.unwrap_or(PixelDataImageBufferOptions {
            format: Some(PixelDataFormat::Png),
            apply_voi_lut: None,
            window_center: None,
            window_width: None,
            frame_number: None,
            convert_to_8bit: None,
            quality: Some(90),
        });

        let format = opts.format.unwrap_or(PixelDataFormat::Png);

        let (bytes, _) = self
            .render_image_buffer(
                format,
                opts.frame_number,
                opts.apply_voi_lut,
                opts.window_center,
                opts.window_width,
                opts.convert_to_8bit,
                opts.quality,
            )
            .map_err(JsError::from)?;

        Ok(bytes.into())
    }

    fn render_image_buffer(
        &self,
        format: PixelDataFormat,
        frame_number: Option<u32>,
        apply_voi_lut: Option<bool>,
        window_center: Option<f64>,
        window_width: Option<f64>,
        convert_to_8bit: Option<bool>,
        quality: Option<u8>,
    ) -> napi::Result<(Vec<u8>, &'static str)> {
        use crate::utils::image_processing::{render_dicom_object, ImageOutputFormat, ImageRenderOptions};

        let (output_format, format_label) = match format {
            PixelDataFormat::Png => (ImageOutputFormat::Png, "PNG"),
            PixelDataFormat::Jpeg => (ImageOutputFormat::Jpeg, "JPEG"),
            PixelDataFormat::Bmp => (ImageOutputFormat::Bmp, "BMP"),
            PixelDataFormat::Raw => {
                return Err(napi::Error::from_reason(
                    "Raw format is not an encoded image. Use getPixelData() or processPixelData({ format: 'Raw' })."
                        .to_string(),
                ));
            }
            PixelDataFormat::Json => {
                return Err(napi::Error::from_reason(
                    "Json format is metadata, not image data. Use toJson() or processPixelData({ format: 'Json' })."
                        .to_string(),
                ));
            }
        };

        let clamped_quality = quality.map(|q| q.clamp(1, 100));

        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();

        let render_opts = ImageRenderOptions {
            width: None,
            height: None,
            quality: clamped_quality,
            window_center: window_center.map(|c| c as f32),
            window_width: window_width.map(|w| w as f32),
            apply_voi_lut,
            rescale_intercept: None,
            rescale_slope: None,
            convert_to_8bit,
            frame_number,
            format: output_format,
        };

        let output = render_dicom_object(obj, &render_opts)
            .map_err(|e| napi::Error::from_reason(format!("Rendering failed: {}", e)))?;

        Ok((output, format_label))
    }

    // Helper method to convert DICOM to JSON string (used by both to_json and save_as_json)
    fn dicom_to_json_string(&self, pretty: Option<bool>) -> napi::Result<String> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(napi::Error::from_reason("File not opened. Call open() first.".to_string()));
        }
        
        let pretty_print = pretty.unwrap_or(true);
        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();
        
        if pretty_print {
            dicom_json::to_string_pretty(obj)
                .map_err(|e| napi::Error::from_reason(format!("Failed to convert to JSON: {}", e)))
        } else {
            dicom_json::to_string(obj)
                .map_err(|e| napi::Error::from_reason(format!("Failed to convert to JSON: {}", e)))
        }
    }

    /**
     * Close the currently opened DICOM file and free memory.
     * 
     * Releases the DICOM dataset from memory. After closing, you must call `open()`
     * again before performing any operations that require file data. It's good practice
     * to close files when done to free resources, though the file will be automatically
     * closed when the instance is dropped.
     * 
     * @example
     * ```typescript
     * const file = new DicomFile();
     * file.open('file1.dcm');
     * // ... work with file1
     * file.close();
     * 
     * file.open('file2.dcm');  // Can reuse same instance
     * // ... work with file2
     * file.close();
     * ```
     */
    #[napi]
    pub fn close(&self) {
        *self.dicom_file.lock().unwrap() = None;
    }

    /**
     * Get metadata about any DICOM tag's data type without reading its bytes.
     *
     * Tells you the VR (Value Representation), whether the tag holds binary data,
     * whether it is image pixel data, and the MIME type (for encapsulated documents).
     * Use this to decide whether to call `getTagBytes()` or `extract()`.
     *
     * @param tagName - Tag name (e.g. "EncapsulatedDocument"), hex "00420011", or "(0042,0011)"
     * @returns TagDataInfo with type metadata
     * @throws Error if no file is opened or the tag is not found
     *
     * @example
     * ```typescript
     * const info = file.getTagInfo('EncapsulatedDocument');
     * // { vr: 'OB', isBinary: true, isImage: false, mimeType: 'application/pdf', byteLength: 123456 }
     * if (info.isBinary && !info.isImage) {
     *   const buf = file.getTagBytes('EncapsulatedDocument');
     * }
     * ```
     */
    #[napi]
    pub fn get_tag_info(&self, tag_name: String) -> Result<TagDataInfo, JsError> {
        use crate::utils::dicom_tags::parse_tag;
        use dicom_core::value::Value;

        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }

        let tag = parse_tag(&tag_name)
            .map_err(|e| JsError::from(napi::Error::from_reason(e)))?;

        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();

        let elem = obj.element(tag)
            .map_err(|e| JsError::from(napi::Error::from_reason(format!("Tag not found: {}", e))))?;

        let vr = elem.vr();
        let vr_str = format!("{:?}", vr);

        // VRs that carry raw binary payloads (not string-representable)
        let is_binary = matches!(vr,
            dicom_core::VR::OB | dicom_core::VR::OW | dicom_core::VR::OF |
            dicom_core::VR::OD | dicom_core::VR::OL | dicom_core::VR::OV |
            dicom_core::VR::UN
        );

        // PixelData tag (7FE0,0010) is only a real image when the mandatory image
        // attributes BitsAllocated, Rows and Columns are also present.
        // If PixelData holds a text/binary blob (non-image SOP classes) those tags
        // are absent, so is_image will be false.
        let is_image = tag == tags::PIXEL_DATA
            && obj.element(tags::BITS_ALLOCATED).is_ok()
            && obj.element(tags::ROWS).is_ok()
            && obj.element(tags::COLUMNS).is_ok();

        // Byte length from the element value
        let byte_length = elem.to_bytes()
            .map(|b| b.len() as u32)
            .unwrap_or(0);

        // MIME type for encapsulated documents (0042,0012 = MIMETypeOfEncapsulatedDocument)
        let mime_type = if tag == Tag(0x0042, 0x0011) {
            // This IS the document tag; look up its MIME sibling
            obj.element(Tag(0x0042, 0x0012))
                .ok()
                .and_then(|e| e.to_str().ok())
                .map(|s| s.trim().to_string())
        } else {
            None
        };

        Ok(TagDataInfo {
            vr: vr_str,
            is_binary,
            is_image,
            mime_type,
            byte_length,
        })
    }

    /**
     * Get the raw bytes of any DICOM tag as a Buffer.
     *
     * Binary-safe alternative to `extract()`. Works for any VR including OB/OW/UN
     * (encapsulated PDF, ZIP, text blobs, private binary tags, etc.).
     * Use `getTagInfo()` first to inspect the VR and MIME type, then consume the buffer
     * according to your application's needs.
     *
     * @param tagName - Tag name, hex, or (GGGG,EEEE) format
     * @returns Buffer with the raw tag payload  
     * @throws Error if no file is opened or the tag is not found
     *
     * @example
     * ```typescript
     * // Read an embedded PDF
     * const buf = file.getTagBytes('EncapsulatedDocument'); // tag 0042,0011
     * fs.writeFileSync('embedded.pdf', buf);
     *
     * // Read a private binary tag
     * const raw = file.getTagBytes('00091010');
     * ```
     */
    #[napi]
    pub fn get_tag_bytes(&self, tag_name: String) -> Result<napi::bindgen_prelude::Buffer, JsError> {
        use crate::utils::dicom_tags::parse_tag;

        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }

        let tag = parse_tag(&tag_name)
            .map_err(|e| JsError::from(napi::Error::from_reason(e)))?;

        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();

        let elem = obj.element(tag)
            .map_err(|e| JsError::from(napi::Error::from_reason(format!("Tag not found: {}", e))))?;

        let bytes = elem.to_bytes()
            .map_err(|e| JsError::from(napi::Error::from_reason(format!("Failed to read tag bytes: {}", e))))?;

        Ok(bytes.to_vec().into())
    }

    /**
     * Extract an encapsulated document (PDF, text, etc.) from the DICOM file.
     *
     * Reads both the MIME type (tag 0042,0012) and the binary payload
     * (tag 0042,0011 – EncapsulatedDocument) together and returns them as a
     * typed object. This is the idiomatic way to handle DICOM-encapsulated PDFs,
     * structured reports, or any non-image content stored in DICOM.
     *
     * @returns EncapsulatedDocumentData with mimeType and data Buffer
     * @throws Error if no file is opened or the document tag is missing
     *
     * @example
     * ```typescript
     * const doc = file.getEncapsulatedDocument();
     * console.log(doc.mimeType); // 'application/pdf'
     * fs.writeFileSync('report.pdf', doc.data);
     *
     * // Or serve over HTTP
     * res.setHeader('Content-Type', doc.mimeType);
     * res.end(doc.data);
     * ```
     */
    #[napi]
    pub fn get_encapsulated_document(&self) -> Result<EncapsulatedDocumentData, JsError> {
        if self.dicom_file.lock().unwrap().is_none() {
            return Err(JsError::from(napi::Error::from_reason("File not opened. Call open() first.".to_string())));
        }

        let dicom_ref = self.dicom_file.lock().unwrap();
        let obj = dicom_ref.as_ref().unwrap();

        // (0042,0011) EncapsulatedDocument
        let doc_elem = obj.element(Tag(0x0042, 0x0011))
            .map_err(|_| JsError::from(napi::Error::from_reason(
                "EncapsulatedDocument tag (0042,0011) not found. This DICOM file does not contain an encapsulated document.".to_string()
            )))?;

        let data = doc_elem.to_bytes()
            .map_err(|e| JsError::from(napi::Error::from_reason(format!("Failed to read encapsulated document bytes: {}", e))))?;

        // (0042,0012) MIMETypeOfEncapsulatedDocument
        let mime_type = obj.element(Tag(0x0042, 0x0012))
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        // (0008,0016) SOPClassUID – useful for context
        let document_title = obj.element(tags::CONTENT_DATE)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.trim().to_string());

        Ok(EncapsulatedDocumentData {
            mime_type,
            data: data.to_vec().into(),
            byte_length: data.len() as u32,
            document_title,
        })
    }

    fn check_file(file: &Path) -> Result<DicomFileMeta, Error> {
        // Ignore DICOMDIR files until better support is added
        let _ = (file.file_name() != Some(OsStr::new("DICOMDIR")))
            .then_some(false)
            .whatever_context("DICOMDIR file not supported")?;
        let dicom_file = dicom_object::OpenFileOptions::new()
            .read_until(Tag(0x0001, 0x000))
            .open_file(file)
            .with_whatever_context(|_| format!("Could not open DICOM file {}", file.display()))?;

        let meta = dicom_file.meta();

        let storage_sop_class_uid = &meta.media_storage_sop_class_uid;
        let storage_sop_instance_uid = &meta.media_storage_sop_instance_uid;

        Ok(DicomFileMeta {
            sop_class_uid: storage_sop_class_uid.to_string(),
            sop_instance_uid: storage_sop_instance_uid.to_string(),
        })
    }
}
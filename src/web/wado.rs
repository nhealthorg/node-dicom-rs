use bytes::Bytes;
use dicom_object::open_file;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, QueryBuilder};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::ops::Deref;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::runtime::Runtime;
use warp::{http::StatusCode, reply::Response, Filter, Reply};

use crate::utils::S3Config;

lazy_static::lazy_static! {
    static ref WADO_RUNTIME: Runtime = Runtime::new().unwrap();
}

// Custom error type for warp rejections
#[derive(Debug)]
struct WadoError {
  message: String,
}

impl warp::reject::Reject for WadoError {}

/// Storage backend type for WADO-RS
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WadoStorageType {
  /// Store files on local filesystem
  Filesystem,
  /// Store files in S3-compatible object storage
  S3,
}

/// Media types supported for DICOM retrieval
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WadoMediaType {
  /// application/dicom - Full DICOM files
  Dicom,
  /// application/dicom+json - DICOM JSON metadata
  DicomJson,
  /// application/dicom+xml - DICOM XML metadata
  DicomXml,
}

/// Pixel data transcoding options
#[napi(string_enum)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WadoTranscoding {
  /// No transcoding - return as stored
  None,
  /// Transcode to JPEG baseline
  JpegBaseline,
  /// Transcode to JPEG 2000
  Jpeg2000,
  /// Transcode to PNG
  Png,
  /// Transcode to uncompressed
  Uncompressed,
}

/// Manifest storage adapter for resolver mode
#[napi(string_enum)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WadoManifestAdapter {
  /// Keep the full manifest in process memory
  Memory,
  /// Persist the full manifest in a local SQLite database
  Sqlite,
  /// Persist the full manifest in a PostgreSQL database
  Postgres,
  /// Disable manifest resolution and use the configured legacy storage layout
  None,
}

/// Manifest storage configuration
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WadoManifestConfig {
  pub adapter: WadoManifestAdapter,
  pub sqlite_path: Option<String>,
  pub postgres_url: Option<String>,
}

/// Options for ingesting a manifest batch
#[napi(object)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WadoManifestIngestOptions {
  /// Replace the complete catalog and delete entries absent from this batch
  pub replace_all: Option<bool>,
}

/// Current manifest catalog counts and ingestion state.
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WadoManifestMetrics {
  pub study_count: f64,
  pub series_count: f64,
  pub instance_count: f64,
  pub ingest_batch_count: f64,
  pub manifest_loaded: bool,
  pub started: bool,
}

/// Frame rendering quality for thumbnail/rendered endpoints
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WadoRenderingOptions {
  /// JPEG quality (1-100, default: 90)
  pub quality: Option<u8>,
  /// Output width in pixels (maintains aspect ratio if only one dimension specified)
  pub width: Option<u32>,
  /// Output height in pixels
  pub height: Option<u32>,
  /// Window center for grayscale images
  pub window_center: Option<f64>,
  /// Window width for grayscale images
  pub window_width: Option<f64>,
}

/// WADO-RS Server Configuration
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WadoServerConfig {
  /// Storage backend type
  pub storage_type: WadoStorageType,

  /// Base path for filesystem storage (required for Filesystem)
  /// Files should be organized as: {base_path}/{studyUID}/{seriesUID}/{instanceUID}.dcm
  pub base_path: Option<String>,

  /// S3 configuration (required for S3)
  pub s3_config: Option<S3Config>,

  /// Named S3 configurations used by manifest-resolved storage
  pub s3_storages: Option<Vec<WadoS3Storage>>,

  /// Manifest adapter configuration; defaults to Memory when s3Storages is provided
  pub manifest: Option<WadoManifestConfig>,

  /// Interface address to bind to (default: "0.0.0.0")
  pub bind_address: Option<String>,

  /// Enable metadata endpoint (GET .../metadata)
  pub enable_metadata: Option<bool>,

  /// Enable frame retrieval (GET .../frames/{frameList})
  pub enable_frames: Option<bool>,

  /// Enable rendered endpoint (GET .../rendered)
  pub enable_rendered: Option<bool>,

  /// Enable thumbnail endpoint (GET .../thumbnail)
  pub enable_thumbnail: Option<bool>,

  /// Enable bulkdata retrieval (GET .../bulkdata/{bulkdataUID})
  pub enable_bulkdata: Option<bool>,

  /// Default transcoding for pixel data
  pub default_transcoding: Option<WadoTranscoding>,

  /// Maximum concurrent connections
  pub max_connections: Option<u32>,

  /// Enable CORS (Cross-Origin Resource Sharing) headers
  /// Default: false
  pub enable_cors: Option<bool>,

  /// CORS allowed origins (comma-separated list of origins)
  /// Examples: "http://localhost:3000", "https://example.com,https://app.example.com"
  /// If not specified, allows all origins (*) when CORS is enabled
  pub cors_allowed_origins: Option<String>,

  /// Enable compression (gzip) for responses
  pub enable_compression: Option<bool>,

  /// Default rendering options for thumbnails
  pub thumbnail_options: Option<WadoRenderingOptions>,

  /// Enable verbose logging
  pub verbose: Option<bool>,
}

/// A server-side S3 configuration registered under a stable identifier.
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WadoS3Storage {
  pub storage_id: String,
  pub config: S3Config,
}

/// A manifest entry's trusted S3 object location.
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WadoObjectReference {
  pub storage_id: String,
  pub object_key: String,
  pub version_id: Option<String>,
}

/// Maps standard DICOM UIDs to one trusted server-side object reference.
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WadoManifestEntry {
  pub study_instance_uid: String,
  pub series_instance_uid: String,
  pub sop_instance_uid: String,
  pub object: WadoObjectReference,
}

type WadoInstanceKey = (String, String, String);

#[derive(Debug, Default, Clone)]
struct WadoManifestIndex {
  instances: HashMap<WadoInstanceKey, WadoObjectReference>,
  by_study: HashMap<String, Vec<WadoInstanceKey>>,
  by_series: HashMap<(String, String), Vec<WadoInstanceKey>>,
}

impl WadoManifestIndex {
  fn build(entries: Vec<WadoManifestEntry>, storage_ids: &HashSet<String>) -> Result<Self> {
    let mut index = Self::default();
    index
      .ingest(entries, storage_ids, true)
      .map_err(Error::from_reason)?;
    Ok(index)
  }

  fn ingest(
    &mut self,
    entries: Vec<WadoManifestEntry>,
    storage_ids: &HashSet<String>,
    replace_all: bool,
  ) -> std::result::Result<(), String> {
    if replace_all {
      self.instances.clear();
      self.by_study.clear();
      self.by_series.clear();
    }
    let mut batch_keys = HashSet::new();
    for entry in entries {
      validate_manifest_entry(&entry, storage_ids)?;

      let key = (
        entry.study_instance_uid.clone(),
        entry.series_instance_uid.clone(),
        entry.sop_instance_uid.clone(),
      );
      if !batch_keys.insert(key.clone()) {
        return Err("Manifest contains duplicate DICOM UID tuples".to_string());
      }

      if !self.instances.contains_key(&key) {
        self
          .by_study
          .entry(entry.study_instance_uid.clone())
          .or_default()
          .push(key.clone());
        self
          .by_series
          .entry((entry.study_instance_uid, entry.series_instance_uid))
          .or_default()
          .push(key.clone());
      }
      self.instances.insert(key, entry.object);
    }
    Ok(())
  }

  fn study_instances(&self, study_uid: &str) -> Vec<WadoInstanceKey> {
    self.by_study.get(study_uid).cloned().unwrap_or_default()
  }

  fn series_instances(&self, study_uid: &str, series_uid: &str) -> Vec<WadoInstanceKey> {
    self
      .by_series
      .get(&(study_uid.to_string(), series_uid.to_string()))
      .cloned()
      .unwrap_or_default()
  }
}

fn validate_manifest_entry(
  entry: &WadoManifestEntry,
  storage_ids: &HashSet<String>,
) -> std::result::Result<(), String> {
  if entry.study_instance_uid.is_empty()
    || entry.series_instance_uid.is_empty()
    || entry.sop_instance_uid.is_empty()
    || entry.object.object_key.is_empty()
  {
    return Err("Manifest entries require DICOM UIDs and an object key".to_string());
  }
  if !storage_ids.contains(&entry.object.storage_id) {
    return Err("Manifest entry references an unknown storageId".to_string());
  }
  if entry.object.version_id.as_deref() == Some("") {
    return Err("versionId must be omitted or non-empty".to_string());
  }
  Ok(())
}

fn open_manifest_database(path: &str) -> std::result::Result<Connection, String> {
  if path.trim().is_empty() {
    return Err("manifest.sqlitePath must not be empty".to_string());
  }

  if path != ":memory:" {
    if let Some(parent) = Path::new(path)
      .parent()
      .filter(|parent| !parent.as_os_str().is_empty())
    {
      std::fs::create_dir_all(parent)
        .map_err(|_| "Could not create SQLite manifest directory".to_string())?;
    }
  }

  let connection =
    Connection::open(path).map_err(|_| "Could not open SQLite manifest database".to_string())?;
  #[cfg(unix)]
  if path != ":memory:" {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
      .map_err(|_| "Could not restrict SQLite manifest file permissions".to_string())?;
  }
  connection
    .busy_timeout(std::time::Duration::from_secs(5))
    .map_err(|_| "Could not configure SQLite manifest database".to_string())?;
  connection
    .execute_batch(
      "PRAGMA journal_mode=WAL;
       PRAGMA synchronous=NORMAL;
       CREATE TABLE IF NOT EXISTS wado_manifest (
         study_uid TEXT NOT NULL,
         series_uid TEXT NOT NULL,
         sop_uid TEXT NOT NULL,
         storage_id TEXT NOT NULL,
         object_key TEXT NOT NULL,
         version_id TEXT,
         PRIMARY KEY (study_uid, series_uid, sop_uid)
       ) WITHOUT ROWID;",
    )
    .map_err(|_| "Could not initialize SQLite manifest database".to_string())?;
  Ok(connection)
}

fn ingest_sqlite_manifest(
  connection: &Arc<Mutex<Connection>>,
  entries: Vec<WadoManifestEntry>,
  storage_ids: &HashSet<String>,
  replace_all: bool,
) -> std::result::Result<(), String> {
  let mut connection = connection
    .lock()
    .map_err(|_| "Manifest database lock is unavailable".to_string())?;
  let transaction = connection
    .transaction()
    .map_err(|_| "Could not start SQLite manifest transaction".to_string())?;
  if replace_all {
    transaction
      .execute("DELETE FROM wado_manifest", [])
      .map_err(|_| "Could not clear previous SQLite manifest".to_string())?;
  }

  {
    let mut batch_keys = HashSet::new();
    let mut insert = transaction
      .prepare_cached(
        "INSERT INTO wado_manifest \
         (study_uid, series_uid, sop_uid, storage_id, object_key, version_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(study_uid, series_uid, sop_uid) DO UPDATE SET \
           storage_id = excluded.storage_id, \
           object_key = excluded.object_key, \
           version_id = excluded.version_id",
      )
      .map_err(|_| "Could not prepare SQLite manifest import".to_string())?;

    for entry in entries {
      validate_manifest_entry(&entry, storage_ids)?;
      let key = (
        entry.study_instance_uid.clone(),
        entry.series_instance_uid.clone(),
        entry.sop_instance_uid.clone(),
      );
      if !batch_keys.insert(key) {
        return Err("Manifest contains duplicate DICOM UID tuples".to_string());
      }
      insert
        .execute(params![
          entry.study_instance_uid,
          entry.series_instance_uid,
          entry.sop_instance_uid,
          entry.object.storage_id,
          entry.object.object_key,
          entry.object.version_id,
        ])
        .map_err(|error| match error {
          rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
          {
            "Manifest contains duplicate DICOM UID tuples".to_string()
          }
          _ => "Could not import SQLite manifest entry".to_string(),
        })?;
    }
  }

  transaction
    .commit()
    .map_err(|_| "Could not commit SQLite manifest replacement".to_string())
}

async fn ingest_postgres_manifest(
  pool: &PgPool,
  entries: Vec<WadoManifestEntry>,
  storage_ids: &HashSet<String>,
  replace_all: bool,
) -> std::result::Result<(), String> {
  let mut transaction = pool
    .begin()
    .await
    .map_err(|_| "Could not start PostgreSQL manifest transaction".to_string())?;
  sqlx::query(
    "CREATE TABLE IF NOT EXISTS wado_manifest (
       study_uid TEXT NOT NULL,
       series_uid TEXT NOT NULL,
       sop_uid TEXT NOT NULL,
       storage_id TEXT NOT NULL,
       object_key TEXT NOT NULL,
       version_id TEXT,
       PRIMARY KEY (study_uid, series_uid, sop_uid)
     )",
  )
  .execute(&mut *transaction)
  .await
  .map_err(|_| "Could not initialize PostgreSQL manifest database".to_string())?;

  if replace_all {
    sqlx::query("DELETE FROM wado_manifest")
      .execute(&mut *transaction)
      .await
      .map_err(|_| "Could not clear previous PostgreSQL manifest".to_string())?;
  }

  let mut batch_keys = HashSet::new();
  for entry in &entries {
    validate_manifest_entry(entry, storage_ids)?;
    let key = (
      entry.study_instance_uid.clone(),
      entry.series_instance_uid.clone(),
      entry.sop_instance_uid.clone(),
    );
    if !batch_keys.insert(key) {
      return Err("Manifest contains duplicate DICOM UID tuples".to_string());
    }
  }

  for batch in entries.chunks(1000) {
    let mut query = QueryBuilder::<Postgres>::new(
      "INSERT INTO wado_manifest \
       (study_uid, series_uid, sop_uid, storage_id, object_key, version_id) ",
    );
    query.push_values(batch, |mut row, entry| {
      row
        .push_bind(&entry.study_instance_uid)
        .push_bind(&entry.series_instance_uid)
        .push_bind(&entry.sop_instance_uid)
        .push_bind(&entry.object.storage_id)
        .push_bind(&entry.object.object_key)
        .push_bind(&entry.object.version_id);
    });
    query.push(
      " ON CONFLICT (study_uid, series_uid, sop_uid) DO UPDATE SET \
       storage_id = EXCLUDED.storage_id, \
       object_key = EXCLUDED.object_key, \
       version_id = EXCLUDED.version_id",
    );
    query
      .build()
      .execute(&mut *transaction)
      .await
      .map_err(|_| "Could not import PostgreSQL manifest entries".to_string())?;
  }

  transaction
    .commit()
    .await
    .map_err(|_| "Could not commit PostgreSQL manifest replacement".to_string())
}

#[derive(Clone)]
enum WadoManifestStore {
  Disabled,
  Memory(Arc<RwLock<Arc<WadoManifestIndex>>>),
  Sqlite(Arc<Mutex<Connection>>),
  Postgres(PgPool),
}

#[derive(Clone)]
enum WadoManifestSnapshot {
  Disabled,
  Memory(Arc<WadoManifestIndex>),
  Sqlite(Arc<Mutex<Connection>>),
  Postgres(PgPool),
}

impl WadoManifestStore {
  fn snapshot(&self) -> WadoManifestSnapshot {
    match self {
      Self::Disabled => WadoManifestSnapshot::Disabled,
      Self::Memory(index) => WadoManifestSnapshot::Memory(
        index
          .read()
          .unwrap_or_else(std::sync::PoisonError::into_inner)
          .clone(),
      ),
      Self::Sqlite(connection) => WadoManifestSnapshot::Sqlite(connection.clone()),
      Self::Postgres(pool) => WadoManifestSnapshot::Postgres(pool.clone()),
    }
  }
}

impl WadoManifestSnapshot {
  async fn metrics(&self) -> std::result::Result<(f64, f64, f64), String> {
    match self {
      Self::Disabled => Ok((0.0, 0.0, 0.0)),
      Self::Memory(index) => Ok((
        index.by_study.len() as f64,
        index.by_series.len() as f64,
        index.instances.len() as f64,
      )),
      Self::Sqlite(connection) => {
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
          let connection = connection
            .lock()
            .map_err(|_| "Manifest database lock is unavailable".to_string())?;
          connection
            .query_row(
              "SELECT \
                 (SELECT COUNT(DISTINCT study_uid) FROM wado_manifest), \
                 (SELECT COUNT(*) FROM (SELECT study_uid, series_uid \
                   FROM wado_manifest GROUP BY study_uid, series_uid)), \
                 (SELECT COUNT(*) FROM wado_manifest)",
              [],
              |row| {
                Ok((
                  row.get::<_, i64>(0)?,
                  row.get::<_, i64>(1)?,
                  row.get::<_, i64>(2)?,
                ))
              },
            )
            .map(|(studies, series, instances)| (studies as f64, series as f64, instances as f64))
            .map_err(|_| "Manifest database metrics query failed".to_string())
        })
        .await
        .map_err(|_| "Manifest database task failed".to_string())?
      }
      Self::Postgres(pool) => sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT \
           (SELECT COUNT(DISTINCT study_uid) FROM wado_manifest), \
           (SELECT COUNT(*) FROM (SELECT study_uid, series_uid \
             FROM wado_manifest GROUP BY study_uid, series_uid) AS manifest_series), \
           (SELECT COUNT(*) FROM wado_manifest)",
      )
      .fetch_one(pool)
      .await
      .map(|(studies, series, instances)| (studies as f64, series as f64, instances as f64))
      .map_err(|_| "Manifest database metrics query failed".to_string()),
    }
  }

  async fn get(
    &self,
    key: WadoInstanceKey,
  ) -> std::result::Result<Option<WadoObjectReference>, String> {
    match self {
      Self::Disabled => Err("Manifest resolver is disabled".to_string()),
      Self::Memory(index) => Ok(index.instances.get(&key).cloned()),
      Self::Sqlite(connection) => {
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
          let connection = connection
            .lock()
            .map_err(|_| "Manifest database lock is unavailable".to_string())?;
          connection
            .query_row(
              "SELECT storage_id, object_key, version_id FROM wado_manifest \
               WHERE study_uid = ?1 AND series_uid = ?2 AND sop_uid = ?3",
              params![key.0, key.1, key.2],
              |row| {
                Ok(WadoObjectReference {
                  storage_id: row.get(0)?,
                  object_key: row.get(1)?,
                  version_id: row.get(2)?,
                })
              },
            )
            .optional()
            .map_err(|_| "Manifest database lookup failed".to_string())
        })
        .await
        .map_err(|_| "Manifest database task failed".to_string())?
      }
      Self::Postgres(pool) => sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT storage_id, object_key, version_id FROM wado_manifest \
           WHERE study_uid = $1 AND series_uid = $2 AND sop_uid = $3",
      )
      .bind(&key.0)
      .bind(&key.1)
      .bind(&key.2)
      .fetch_optional(pool)
      .await
      .map(|reference| {
        reference.map(|(storage_id, object_key, version_id)| WadoObjectReference {
          storage_id,
          object_key,
          version_id,
        })
      })
      .map_err(|_| "Manifest database lookup failed".to_string()),
    }
  }

  async fn study_instances(
    &self,
    study_uid: String,
  ) -> std::result::Result<Vec<WadoInstanceKey>, String> {
    match self {
      Self::Disabled => Err("Manifest resolver is disabled".to_string()),
      Self::Memory(index) => Ok(index.study_instances(&study_uid)),
      Self::Sqlite(connection) => {
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
          let connection = connection
            .lock()
            .map_err(|_| "Manifest database lock is unavailable".to_string())?;
          let mut statement = connection
            .prepare_cached(
              "SELECT study_uid, series_uid, sop_uid FROM wado_manifest \
               WHERE study_uid = ?1 ORDER BY series_uid, sop_uid",
            )
            .map_err(|_| "Manifest database query failed".to_string())?;
          let rows = statement
            .query_map(params![study_uid], |row| {
              Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|_| "Manifest database query failed".to_string())?;
          rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "Manifest database query failed".to_string())
        })
        .await
        .map_err(|_| "Manifest database task failed".to_string())?
      }
      Self::Postgres(pool) => sqlx::query_as::<_, WadoInstanceKey>(
        "SELECT study_uid, series_uid, sop_uid FROM wado_manifest \
         WHERE study_uid = $1 ORDER BY series_uid, sop_uid",
      )
      .bind(study_uid)
      .fetch_all(pool)
      .await
      .map_err(|_| "Manifest database query failed".to_string()),
    }
  }

  async fn series_instances(
    &self,
    study_uid: String,
    series_uid: String,
  ) -> std::result::Result<Vec<WadoInstanceKey>, String> {
    match self {
      Self::Disabled => Err("Manifest resolver is disabled".to_string()),
      Self::Memory(index) => Ok(index.series_instances(&study_uid, &series_uid)),
      Self::Sqlite(connection) => {
        let connection = connection.clone();
        tokio::task::spawn_blocking(move || {
          let connection = connection
            .lock()
            .map_err(|_| "Manifest database lock is unavailable".to_string())?;
          let mut statement = connection
            .prepare_cached(
              "SELECT study_uid, series_uid, sop_uid FROM wado_manifest \
               WHERE study_uid = ?1 AND series_uid = ?2 ORDER BY sop_uid",
            )
            .map_err(|_| "Manifest database query failed".to_string())?;
          let rows = statement
            .query_map(params![study_uid, series_uid], |row| {
              Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|_| "Manifest database query failed".to_string())?;
          rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "Manifest database query failed".to_string())
        })
        .await
        .map_err(|_| "Manifest database task failed".to_string())?
      }
      Self::Postgres(pool) => sqlx::query_as::<_, WadoInstanceKey>(
        "SELECT study_uid, series_uid, sop_uid FROM wado_manifest \
         WHERE study_uid = $1 AND series_uid = $2 ORDER BY sop_uid",
      )
      .bind(study_uid)
      .bind(series_uid)
      .fetch_all(pool)
      .await
      .map_err(|_| "Manifest database query failed".to_string()),
    }
  }
}

struct WadoRuntimeConfig {
  settings: Arc<WadoServerConfig>,
  storage_buckets: Arc<HashMap<String, s3::bucket::Bucket>>,
  manifest_store: WadoManifestStore,
}

impl WadoRuntimeConfig {
  fn snapshot(&self) -> Arc<WadoRequestConfig> {
    Arc::new(WadoRequestConfig {
      settings: self.settings.clone(),
      storage_buckets: self.storage_buckets.clone(),
      manifest: self.manifest_store.snapshot(),
    })
  }
}

struct WadoRequestConfig {
  settings: Arc<WadoServerConfig>,
  storage_buckets: Arc<HashMap<String, s3::bucket::Bucket>>,
  manifest: WadoManifestSnapshot,
}

impl Deref for WadoRequestConfig {
  type Target = WadoServerConfig;

  fn deref(&self) -> &Self::Target {
    &self.settings
  }
}

impl Deref for WadoRuntimeConfig {
  type Target = WadoServerConfig;

  fn deref(&self) -> &Self::Target {
    &self.settings
  }
}

/// WADO-RS Server
#[napi]
pub struct WadoServer {
  port: u16,
  bind_address: IpAddr,
  config: WadoServerConfig,
  manifest_loaded: AtomicBool,
  ingest_batch_count: AtomicU64,
  started: AtomicBool,
  manifest_ingest_lock: tokio::sync::Mutex<()>,
  storage_ids: HashSet<String>,
  s3_buckets: Arc<HashMap<String, s3::bucket::Bucket>>,
  manifest_store: WadoManifestStore,
  shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

#[napi]
impl WadoServer {
  #[napi(constructor)]
  pub fn new(port: u16, config: WadoServerConfig) -> Result<Self> {
    let bind_address = config
      .bind_address
      .as_deref()
      .unwrap_or("0.0.0.0")
      .parse::<IpAddr>()
      .map_err(|_| Error::from_reason("bindAddress must be an IPv4 or IPv6 address"))?;

    let has_storages = config
      .s3_storages
      .as_ref()
      .is_some_and(|storages| !storages.is_empty());
    let manifest_adapter = config
      .manifest
      .as_ref()
      .map(|manifest| manifest.adapter)
      .unwrap_or(if has_storages {
        WadoManifestAdapter::Memory
      } else {
        WadoManifestAdapter::None
      });
    let sqlite_path = config
      .manifest
      .as_ref()
      .and_then(|manifest| manifest.sqlite_path.as_deref())
      .filter(|path| !path.trim().is_empty());
    let postgres_url = config
      .manifest
      .as_ref()
      .and_then(|manifest| manifest.postgres_url.as_deref())
      .filter(|url| !url.trim().is_empty());
    let resolver_enabled = manifest_adapter != WadoManifestAdapter::None;

    match manifest_adapter {
      WadoManifestAdapter::None => {
        if has_storages || sqlite_path.is_some() || postgres_url.is_some() {
          return Err(Error::from_reason(
            "manifest.adapter None cannot be combined with s3Storages or database settings",
          ));
        }
      }
      WadoManifestAdapter::Memory => {
        if !has_storages {
          return Err(Error::from_reason(
            "manifest.adapter Memory requires non-empty s3Storages",
          ));
        }
        if sqlite_path.is_some() || postgres_url.is_some() {
          return Err(Error::from_reason(
            "manifest database settings must match the selected adapter",
          ));
        }
      }
      WadoManifestAdapter::Sqlite => {
        if !has_storages {
          return Err(Error::from_reason(
            "manifest.adapter Sqlite requires non-empty s3Storages",
          ));
        }
        if sqlite_path.is_none() {
          return Err(Error::from_reason(
            "manifest.sqlitePath is required with manifest.adapter Sqlite",
          ));
        }
        if postgres_url.is_some() {
          return Err(Error::from_reason(
            "manifest.postgresUrl is only valid with manifest.adapter Postgres",
          ));
        }
      }
      WadoManifestAdapter::Postgres => {
        if !has_storages {
          return Err(Error::from_reason(
            "manifest.adapter Postgres requires non-empty s3Storages",
          ));
        }
        if postgres_url.is_none() {
          return Err(Error::from_reason(
            "manifest.postgresUrl is required with manifest.adapter Postgres",
          ));
        }
        if sqlite_path.is_some() {
          return Err(Error::from_reason(
            "manifest.sqlitePath is only valid with manifest.adapter Sqlite",
          ));
        }
      }
    }

    // Validate configuration
    match config.storage_type {
      WadoStorageType::Filesystem => {
        if resolver_enabled {
          return Err(Error::from_reason(
            "s3Storages can only be used with S3 storage",
          ));
        }
        if config.base_path.is_none() {
          return Err(Error::from_reason(
            "base_path is required for Filesystem storage",
          ));
        }
      }
      WadoStorageType::S3 => {
        if !resolver_enabled && config.s3_config.is_none() {
          return Err(Error::from_reason(
            "s3Config or non-empty s3Storages is required for S3 storage",
          ));
        }
      }
    }

    let mut storage_ids = HashSet::new();
    let mut s3_buckets = HashMap::new();
    if let Some(storages) = &config.s3_storages {
      for storage in storages {
        if storage.storage_id.is_empty() || !storage_ids.insert(storage.storage_id.clone()) {
          return Err(Error::from_reason(
            "s3Storages IDs must be unique and non-empty",
          ));
        }
        s3_buckets.insert(
          storage.storage_id.clone(),
          crate::utils::s3::build_s3_bucket(&storage.config),
        );
      }
    }

    let manifest_store = match manifest_adapter {
      WadoManifestAdapter::None => WadoManifestStore::Disabled,
      WadoManifestAdapter::Memory => WadoManifestStore::Memory(Arc::new(RwLock::new(Arc::new(
        WadoManifestIndex::default(),
      )))),
      WadoManifestAdapter::Sqlite => {
        let path = sqlite_path.unwrap();
        let connection = open_manifest_database(path).map_err(Error::from_reason)?;
        WadoManifestStore::Sqlite(Arc::new(Mutex::new(connection)))
      }
      WadoManifestAdapter::Postgres => {
        let pool = PgPoolOptions::new()
          .max_connections(16)
          .connect_lazy(postgres_url.unwrap())
          .map_err(|_| Error::from_reason("Could not configure PostgreSQL manifest database"))?;
        WadoManifestStore::Postgres(pool)
      }
    };

    Ok(Self {
      port,
      bind_address,
      config,
      manifest_loaded: AtomicBool::new(!resolver_enabled),
      ingest_batch_count: AtomicU64::new(0),
      started: AtomicBool::new(false),
      manifest_ingest_lock: tokio::sync::Mutex::new(()),
      storage_ids,
      s3_buckets: Arc::new(s3_buckets),
      manifest_store,
      shutdown_tx: None,
    })
  }

  /// Upsert a manifest batch; pass replaceAll to replace the catalog before start.
  #[napi]
  pub async fn ingest_manifest(
    &self,
    entries: Vec<WadoManifestEntry>,
    options: Option<WadoManifestIngestOptions>,
  ) -> Result<()> {
    let _ingest_guard = self.manifest_ingest_lock.lock().await;
    if matches!(self.manifest_store, WadoManifestStore::Disabled) {
      return Err(Error::from_reason("Manifest mode is disabled"));
    }
    let replace_all = options
      .and_then(|options| options.replace_all)
      .unwrap_or(false);
    if replace_all && self.started.load(Ordering::Acquire) {
      return Err(Error::from_reason(
        "Full manifest sync is only allowed before start; runtime ingestion is incremental",
      ));
    }

    match &self.manifest_store {
      WadoManifestStore::Disabled => {
        return Err(Error::from_reason("Manifest mode is disabled"));
      }
      WadoManifestStore::Memory(index) => {
        let current = index
          .read()
          .unwrap_or_else(std::sync::PoisonError::into_inner)
          .clone();
        let mut replacement = (*current).clone();
        replacement
          .ingest(entries, &self.storage_ids, replace_all)
          .map_err(Error::from_reason)?;
        let mut current = index
          .write()
          .map_err(|_| Error::from_reason("Manifest lock is unavailable"))?;
        *current = Arc::new(replacement);
      }
      WadoManifestStore::Sqlite(connection) => {
        let connection = connection.clone();
        let storage_ids = self.storage_ids.clone();
        tokio::task::spawn_blocking(move || {
          ingest_sqlite_manifest(&connection, entries, &storage_ids, replace_all)
        })
        .await
        .map_err(|_| Error::from_reason("Manifest database task failed"))?
        .map_err(Error::from_reason)?;
      }
      WadoManifestStore::Postgres(pool) => {
        ingest_postgres_manifest(pool, entries, &self.storage_ids, replace_all)
          .await
          .map_err(Error::from_reason)?;
      }
    }
    self.manifest_loaded.store(true, Ordering::Release);
    self.ingest_batch_count.fetch_add(1, Ordering::Release);
    Ok(())
  }

  /// Get current manifest counts and ingestion state.
  #[napi]
  pub async fn get_manifest_metrics(&self) -> Result<WadoManifestMetrics> {
    let _ingest_guard = self.manifest_ingest_lock.lock().await;
    let manifest_loaded = self.manifest_loaded.load(Ordering::Acquire);
    let (study_count, series_count, instance_count) = if manifest_loaded {
      self
        .manifest_store
        .snapshot()
        .metrics()
        .await
        .map_err(Error::from_reason)?
    } else {
      (0.0, 0.0, 0.0)
    };

    Ok(WadoManifestMetrics {
      study_count,
      series_count,
      instance_count,
      ingest_batch_count: self.ingest_batch_count.load(Ordering::Acquire) as f64,
      manifest_loaded,
      started: self.started.load(Ordering::Acquire),
    })
  }

  /// Start the WADO-RS server
  #[napi]
  pub fn start(&mut self) -> Result<()> {
    if !self.manifest_loaded.load(Ordering::Acquire) {
      return Err(Error::from_reason(
        "ingestManifest() must succeed before start",
      ));
    }
    self.started.store(true, Ordering::Release);

    let settings = Arc::new(self.config.clone());
    let config = Arc::new(WadoRuntimeConfig {
      settings,
      storage_buckets: self.s3_buckets.clone(),
      manifest_store: self.manifest_store.clone(),
    });
    let port = self.port;
    let bind_address = self.bind_address;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let handle = WADO_RUNTIME.spawn(async move {
      // CORS handling
      let cors = if config.enable_cors.unwrap_or(false) {
        let mut cors_builder = warp::cors()
          .allow_methods(vec!["GET", "OPTIONS"])
          .allow_headers(vec!["Content-Type", "Accept", "Authorization"]);

        if let Some(origins) = &config.cors_allowed_origins {
          // Parse comma-separated origins
          for origin in origins.split(',').map(|s| s.trim()) {
            if config.verbose.unwrap_or(false) {
              println!("CORS: Adding allowed origin: {}", origin);
            }
            cors_builder = cors_builder.allow_origin(origin);
          }
          if config.verbose.unwrap_or(false) {
            println!("CORS enabled: {} origin(s)", origins.split(',').count());
          }
        } else {
          cors_builder = cors_builder.allow_any_origin();
          if config.verbose.unwrap_or(false) {
            println!("CORS enabled: * (all origins)");
          }
        }
        cors_builder
      } else {
        if config.verbose.unwrap_or(false) {
          println!("CORS disabled");
        }
        warp::cors().allow_any_origin()
      };

      // ================================================================
      // Route 1: Retrieve Study - GET /studies/{studyUID}
      // ================================================================
      let config_retrieve_study = config.clone();
      let retrieve_study = warp::path!("studies" / String)
        .and(warp::get())
        .and(warp::header::optional::<String>("accept"))
        .and_then(move |study_uid: String, accept: Option<String>| {
          let config = config_retrieve_study.snapshot();
          async move { retrieve_study_handler(study_uid, accept, config).await }
        });

      // ================================================================
      // Route 2: Retrieve Series - GET /studies/{studyUID}/series/{seriesUID}
      // ================================================================
      let config_retrieve_series = config.clone();
      let retrieve_series = warp::path!("studies" / String / "series" / String)
        .and(warp::get())
        .and(warp::header::optional::<String>("accept"))
        .and_then(
          move |study_uid: String, series_uid: String, accept: Option<String>| {
            let config = config_retrieve_series.snapshot();
            async move { retrieve_series_handler(study_uid, series_uid, accept, config).await }
          },
        );

      // ================================================================
      // Route 3: Retrieve Instance - GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}
      // ================================================================
      let config_retrieve_instance = config.clone();
      let retrieve_instance =
        warp::path!("studies" / String / "series" / String / "instances" / String)
          .and(warp::get())
          .and(warp::header::optional::<String>("accept"))
          .and_then(
            move |study_uid: String,
                  series_uid: String,
                  instance_uid: String,
                  accept: Option<String>| {
              let config = config_retrieve_instance.snapshot();
              async move {
                retrieve_instance_handler(study_uid, series_uid, instance_uid, accept, config).await
              }
            },
          );

      // ================================================================
      // Route 4: Retrieve Metadata (Study) - GET /studies/{studyUID}/metadata
      // ================================================================
      let config_study_metadata = config.clone();
      let retrieve_study_metadata = warp::path!("studies" / String / "metadata")
        .and(warp::get())
        .and_then(move |study_uid: String| {
          let config = config_study_metadata.snapshot();
          async move {
            if !config.enable_metadata.unwrap_or(true) {
              return Err(warp::reject::not_found());
            }
            retrieve_study_metadata_handler(study_uid, config).await
          }
        });

      // ================================================================
      // Route 5: Retrieve Metadata (Series) - GET /studies/{studyUID}/series/{seriesUID}/metadata
      // ================================================================
      let config_series_metadata = config.clone();
      let retrieve_series_metadata =
        warp::path!("studies" / String / "series" / String / "metadata")
          .and(warp::get())
          .and_then(move |study_uid: String, series_uid: String| {
            let config = config_series_metadata.snapshot();
            async move {
              if !config.enable_metadata.unwrap_or(true) {
                return Err(warp::reject::not_found());
              }
              retrieve_series_metadata_handler(study_uid, series_uid, config).await
            }
          });

      // ================================================================
      // Route 6: Retrieve Instance Metadata - GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/metadata
      // ================================================================
      let config_instance_metadata = config.clone();
      let retrieve_instance_metadata =
        warp::path!("studies" / String / "series" / String / "instances" / String / "metadata")
          .and(warp::get())
          .and_then(
            move |study_uid: String, series_uid: String, instance_uid: String| {
              let config = config_instance_metadata.snapshot();
              async move {
                if !config.enable_metadata.unwrap_or(true) {
                  return Err(warp::reject::not_found());
                }
                retrieve_instance_metadata_handler(study_uid, series_uid, instance_uid, config)
                  .await
              }
            },
          );

      // ================================================================
      // Route 7: Retrieve Frames - GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/frames/{frameList}
      // ================================================================
      let config_frames = config.clone();
      let retrieve_frames = warp::path!(
        "studies" / String / "series" / String / "instances" / String / "frames" / String
      )
      .and(warp::get())
      .and(warp::header::optional::<String>("accept"))
      .and_then(
        move |study_uid: String,
              series_uid: String,
              instance_uid: String,
              frame_list: String,
              accept: Option<String>| {
          let config = config_frames.snapshot();
          async move {
            if !config.enable_frames.unwrap_or(true) {
              return Err(warp::reject::not_found());
            }
            retrieve_frames_handler(
              study_uid,
              series_uid,
              instance_uid,
              frame_list,
              accept,
              config,
            )
            .await
          }
        },
      );

      // ================================================================
      // Route 8: Retrieve Bulkdata - GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/bulkdata/{attributePath}
      // ================================================================
      let config_bulkdata = config.clone();
      let retrieve_bulkdata = warp::path!(
        "studies" / String / "series" / String / "instances" / String / "bulkdata" / String
      )
      .and(warp::get())
      .and_then(
        move |study_uid: String,
              series_uid: String,
              instance_uid: String,
              attribute_path: String| {
          let config = config_bulkdata.snapshot();
          async move {
            if !config.enable_bulkdata.unwrap_or(false) {
              return Err(warp::reject::not_found());
            }
            retrieve_bulkdata_handler(study_uid, series_uid, instance_uid, attribute_path, config)
              .await
          }
        },
      );

      // ================================================================
      // Route 9: Retrieve Rendered - GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/rendered
      // ================================================================
      let config_rendered = config.clone();
      let retrieve_rendered =
        warp::path!("studies" / String / "series" / String / "instances" / String / "rendered")
          .and(warp::get())
          .and(warp::query::<std::collections::HashMap<String, String>>())
          .and(warp::header::optional::<String>("accept"))
          .and_then(
            move |study_uid: String,
                  series_uid: String,
                  instance_uid: String,
                  params: std::collections::HashMap<String, String>,
                  accept: Option<String>| {
              let config = config_rendered.snapshot();
              async move {
                if !config.enable_rendered.unwrap_or(false) {
                  return Err(warp::reject::not_found());
                }
                retrieve_rendered_handler(
                  study_uid,
                  series_uid,
                  instance_uid,
                  params,
                  accept,
                  config,
                )
                .await
              }
            },
          );

      // ================================================================
      // Route 10: Retrieve Thumbnail - GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/thumbnail
      // ================================================================
      let config_thumbnail = config.clone();
      let retrieve_thumbnail =
        warp::path!("studies" / String / "series" / String / "instances" / String / "thumbnail")
          .and(warp::get())
          .and(warp::query::<std::collections::HashMap<String, String>>())
          .and(warp::header::optional::<String>("accept"))
          .and_then(
            move |study_uid: String,
                  series_uid: String,
                  instance_uid: String,
                  params: std::collections::HashMap<String, String>,
                  accept: Option<String>| {
              let config = config_thumbnail.snapshot();
              async move {
                if !config.enable_thumbnail.unwrap_or(false) {
                  return Err(warp::reject::not_found());
                }
                retrieve_thumbnail_handler(
                  study_uid,
                  series_uid,
                  instance_uid,
                  params,
                  accept,
                  config,
                )
                .await
              }
            },
          );

      // Combine all routes
      let routes = retrieve_study
        .or(retrieve_series)
        .or(retrieve_instance)
        .or(retrieve_study_metadata)
        .or(retrieve_series_metadata)
        .or(retrieve_instance_metadata)
        .or(retrieve_frames)
        .or(retrieve_bulkdata)
        .or(retrieve_rendered)
        .or(retrieve_thumbnail)
        .with(cors)
        .recover(handle_rejection);

      let bound = warp::serve(routes).bind((bind_address, port)).await;

      if config.verbose.unwrap_or(false) {
        println!("WADO-RS server started on port {}", port);
      }

      bound
        .graceful(async {
          shutdown_rx.await.ok();
        })
        .run()
        .await;
    });

    self.shutdown_tx = Some(shutdown_tx);

    Ok(())
  }

  /// Stop the WADO-RS server
  #[napi]
  pub fn stop(&mut self) -> Result<()> {
    if let Some(tx) = self.shutdown_tx.take() {
      let _ = tx.send(());
    }
    if self.config.verbose.unwrap_or(false) {
      println!("WADO-RS server stopped");
    }
    Ok(())
  }
}

// ============================================================================
// Storage Backend Functions
// ============================================================================

async fn load_dicom_file(
  study_uid: &str,
  series_uid: &str,
  instance_uid: &str,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Vec<u8>, String> {
  if !matches!(&config.manifest, WadoManifestSnapshot::Disabled) {
    let lookup_started = Instant::now();
    let key = (
      study_uid.to_string(),
      series_uid.to_string(),
      instance_uid.to_string(),
    );
    let object = config
      .manifest
      .get(key)
      .await?
      .ok_or_else(|| "DICOM instance not found".to_string())?;
    let bucket = config
      .storage_buckets
      .get(&object.storage_id)
      .ok_or_else(|| "DICOM instance not found".to_string())?;

    if config.verbose.unwrap_or(false) {
      println!(
        "WADO_METRIC stage=manifest_lookup duration_us={}",
        lookup_started.elapsed().as_micros()
      );
    }

    let s3_started = Instant::now();
    let result = crate::utils::s3::s3_get_object_at_version(
      bucket,
      &object.object_key,
      object.version_id.as_deref(),
    )
    .await;
    if config.verbose.unwrap_or(false) {
      println!(
        "WADO_METRIC stage=s3_get duration_us={}",
        s3_started.elapsed().as_micros()
      );
    }
    return result.map_err(|_| "DICOM instance not found or storage unavailable".to_string());
  }

  match config.storage_type {
    WadoStorageType::Filesystem => {
      let base_path = config
        .base_path
        .as_ref()
        .ok_or_else(|| "Base path not configured for filesystem storage".to_string())?;

      let file_path = format!(
        "{}/{}/{}/{}.dcm",
        base_path, study_uid, series_uid, instance_uid
      );

      if config.verbose.unwrap_or(false) {
        println!("Loading DICOM file: {}", file_path);
      }

      let mut file = File::open(&file_path)
        .await
        .map_err(|e| format!("Failed to open file {}: {}", file_path, e))?;

      let mut buffer = Vec::new();
      file
        .read_to_end(&mut buffer)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))?;

      Ok(buffer)
    }
    WadoStorageType::S3 => {
      let s3_config = config
        .s3_config
        .as_ref()
        .ok_or_else(|| "S3 config not configured for S3 storage".to_string())?;

      let bucket = crate::utils::s3::build_s3_bucket(s3_config);

      let object_path = format!("{}/{}/{}.dcm", study_uid, series_uid, instance_uid);

      if config.verbose.unwrap_or(false) {
        println!("Loading DICOM file from S3: {}", object_path);
      }

      let s3_started = Instant::now();
      let result = crate::utils::s3::s3_get_object(&bucket, &object_path).await;
      if config.verbose.unwrap_or(false) {
        println!(
          "WADO_METRIC stage=s3_get mode=legacy duration_us={}",
          s3_started.elapsed().as_micros()
        );
      }
      result.map_err(|e| format!("Failed to get object from S3: {}", e))
    }
  }
}

async fn load_dicom_metadata(
  study_uid: &str,
  series_uid: &str,
  instance_uid: &str,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<serde_json::Value, String> {
  // Load the DICOM file
  let buffer = load_dicom_file(study_uid, series_uid, instance_uid, config.clone()).await?;

  // Write to temporary file for dicom_object to read
  let temp_path = format!("/tmp/wado_temp_{}.dcm", instance_uid);
  tokio::fs::write(&temp_path, &buffer)
    .await
    .map_err(|e| format!("Failed to write temp file: {}", e))?;

  // Parse DICOM file
  let decode_started = Instant::now();
  let obj = open_file(&temp_path).map_err(|e| format!("Failed to parse DICOM file: {}", e))?;
  if config.verbose.unwrap_or(false) {
    println!(
      "WADO_METRIC stage=dicom_parse duration_us={}",
      decode_started.elapsed().as_micros()
    );
  }

  // Convert to DICOM JSON manually by extracting all elements
  let mut json_obj = serde_json::Map::new();

  for elem in obj.into_iter() {
    let tag = elem.header().tag;
    let tag_str = format!("{:08X}", tag.0);

    // Build DICOM JSON element structure
    let vr = elem.vr().to_string();

    // Try to extract string value
    let value = if let Ok(val) = elem.to_str() {
      serde_json::json!({
          "vr": vr,
          "Value": [val]
      })
    } else {
      serde_json::json!({
          "vr": vr
      })
    };

    json_obj.insert(tag_str, value);
  }

  // Clean up temp file
  let _ = tokio::fs::remove_file(&temp_path).await;

  Ok(serde_json::Value::Object(json_obj))
}

async fn list_study_instances(
  config: &WadoRequestConfig,
  study_uid: &str,
) -> std::result::Result<Vec<WadoInstanceKey>, String> {
  if !matches!(&config.manifest, WadoManifestSnapshot::Disabled) {
    let started = Instant::now();
    let instances = config
      .manifest
      .study_instances(study_uid.to_string())
      .await?;
    if config.verbose.unwrap_or(false) {
      println!(
        "WADO_METRIC stage=manifest_lookup operation=study_list duration_us={} entries={}",
        started.elapsed().as_micros(),
        instances.len()
      );
    }
    return Ok(instances);
  }

  let started = Instant::now();
  let result = match config.storage_type {
    WadoStorageType::Filesystem => {
      let base_path = config
        .base_path
        .as_ref()
        .ok_or_else(|| "Base path not configured for filesystem storage".to_string())?;
      scan_study_instances(base_path, study_uid).await
    }
    WadoStorageType::S3 => {
      let s3_config = config
        .s3_config
        .as_ref()
        .ok_or_else(|| "S3 configuration not provided".to_string())?;
      scan_study_instances_s3(s3_config, study_uid).await
    }
  };
  if config.verbose.unwrap_or(false) {
    println!(
      "WADO_METRIC stage=legacy_instance_list operation=study_list duration_us={}",
      started.elapsed().as_micros()
    );
  }
  result
}

async fn list_series_instances(
  config: &WadoRequestConfig,
  study_uid: &str,
  series_uid: &str,
) -> std::result::Result<Vec<WadoInstanceKey>, String> {
  if !matches!(&config.manifest, WadoManifestSnapshot::Disabled) {
    let started = Instant::now();
    let instances = config
      .manifest
      .series_instances(study_uid.to_string(), series_uid.to_string())
      .await?;
    if config.verbose.unwrap_or(false) {
      println!(
        "WADO_METRIC stage=manifest_lookup operation=series_list duration_us={} entries={}",
        started.elapsed().as_micros(),
        instances.len()
      );
    }
    return Ok(instances);
  }

  let started = Instant::now();
  let result = match config.storage_type {
    WadoStorageType::Filesystem => {
      let base_path = config
        .base_path
        .as_ref()
        .ok_or_else(|| "Base path not configured for filesystem storage".to_string())?;
      scan_series_instances(base_path, study_uid, series_uid).await
    }
    WadoStorageType::S3 => {
      let s3_config = config
        .s3_config
        .as_ref()
        .ok_or_else(|| "S3 configuration not provided".to_string())?;
      scan_series_instances_s3(s3_config, study_uid, series_uid).await
    }
  };
  if config.verbose.unwrap_or(false) {
    println!(
      "WADO_METRIC stage=legacy_instance_list operation=series_list duration_us={}",
      started.elapsed().as_micros()
    );
  }
  result
}

// ============================================================================
// Route Handler Functions
// ============================================================================

async fn retrieve_study_handler(
  study_uid: String,
  accept: Option<String>,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!("Retrieve study: {}, Accept: {:?}", study_uid, accept);
  }

  // Determine response format from Accept header
  let media_type = parse_accept_header(accept.as_deref());

  let instances = list_study_instances(&config, &study_uid)
    .await
    .map_err(|e| warp::reject::custom(WadoError { message: e }))?;

  if instances.is_empty() {
    return Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Study not found or contains no instances"
        })),
        StatusCode::NOT_FOUND,
      )
      .into_response(),
    );
  }

  if config.verbose.unwrap_or(false) {
    println!("Found {} instances in study {}", instances.len(), study_uid);
  }

  build_multipart_response(instances, config.clone(), media_type)
    .await
    .map_err(|e| warp::reject::custom(WadoError { message: e }))
}

async fn retrieve_series_handler(
  study_uid: String,
  series_uid: String,
  accept: Option<String>,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!(
      "Retrieve series: {}/{}, Accept: {:?}",
      study_uid, series_uid, accept
    );
  }

  // Determine response format from Accept header
  let media_type = parse_accept_header(accept.as_deref());

  let instances = list_series_instances(&config, &study_uid, &series_uid)
    .await
    .map_err(|e| warp::reject::custom(WadoError { message: e }))?;

  if instances.is_empty() {
    return Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Series not found or contains no instances"
        })),
        StatusCode::NOT_FOUND,
      )
      .into_response(),
    );
  }

  if config.verbose.unwrap_or(false) {
    println!(
      "Found {} instances in series {}/{}",
      instances.len(),
      study_uid,
      series_uid
    );
  }

  build_multipart_response(instances, config.clone(), media_type)
    .await
    .map_err(|e| warp::reject::custom(WadoError { message: e }))
}

async fn retrieve_instance_handler(
  study_uid: String,
  series_uid: String,
  instance_uid: String,
  accept: Option<String>,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!(
      "Retrieve instance: {}/{}/{}, Accept: {:?}",
      study_uid, series_uid, instance_uid, accept
    );
  }

  // Determine response format from Accept header
  let media_type = parse_accept_header(accept.as_deref());

  match media_type {
    WadoMediaType::Dicom => {
      // Return raw DICOM file
      match load_dicom_file(&study_uid, &series_uid, &instance_uid, config).await {
        Ok(buffer) => Ok(
          warp::http::Response::builder()
            .header("Content-Type", "application/dicom")
            .status(StatusCode::OK)
            .body(Bytes::from(buffer))
            .unwrap()
            .into_response(),
        ),
        Err(e) => Ok(
          warp::reply::with_status(
            warp::reply::json(&serde_json::json!({
                "error": e
            })),
            StatusCode::NOT_FOUND,
          )
          .into_response(),
        ),
      }
    }
    WadoMediaType::DicomJson => {
      // Return metadata as JSON
      match load_dicom_metadata(&study_uid, &series_uid, &instance_uid, config).await {
        Ok(json) => Ok(
          warp::http::Response::builder()
            .header("Content-Type", "application/dicom+json")
            .status(StatusCode::OK)
            .body(Bytes::from(serde_json::to_vec(&json).unwrap()))
            .unwrap()
            .into_response(),
        ),
        Err(e) => Ok(
          warp::reply::with_status(
            warp::reply::json(&serde_json::json!({
                "error": e
            })),
            StatusCode::NOT_FOUND,
          )
          .into_response(),
        ),
      }
    }
    WadoMediaType::DicomXml => Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "XML format not yet implemented"
        })),
        StatusCode::NOT_IMPLEMENTED,
      )
      .into_response(),
    ),
  }
}

async fn retrieve_study_metadata_handler(
  study_uid: String,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!("Retrieve study metadata: {}", study_uid);
  }

  let instances = list_study_instances(&config, &study_uid)
    .await
    .map_err(|e| warp::reject::custom(WadoError { message: e }))?;

  if instances.is_empty() {
    return Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Study not found or contains no instances"
        })),
        StatusCode::NOT_FOUND,
      )
      .into_response(),
    );
  }

  if config.verbose.unwrap_or(false) {
    println!("Found {} instances in study {}", instances.len(), study_uid);
  }

  // Load metadata for all instances
  let mut metadata_array = Vec::new();
  for (study_uid, series_uid, instance_uid) in instances {
    match load_dicom_metadata(&study_uid, &series_uid, &instance_uid, config.clone()).await {
      Ok(metadata) => metadata_array.push(metadata),
      Err(e) => {
        if config.verbose.unwrap_or(false) {
          println!(
            "Failed to load metadata for {}/{}/{}: {}",
            study_uid, series_uid, instance_uid, e
          );
        }
      }
    }
  }

  if metadata_array.is_empty() {
    return Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Failed to load metadata for any instance in study"
        })),
        StatusCode::INTERNAL_SERVER_ERROR,
      )
      .into_response(),
    );
  }

  Ok(
    warp::http::Response::builder()
      .header("Content-Type", "application/dicom+json")
      .status(StatusCode::OK)
      .body(Bytes::from(serde_json::to_vec(&metadata_array).unwrap()))
      .unwrap()
      .into_response(),
  )
}

async fn retrieve_series_metadata_handler(
  study_uid: String,
  series_uid: String,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!("Retrieve series metadata: {}/{}", study_uid, series_uid);
  }

  let instances = list_series_instances(&config, &study_uid, &series_uid)
    .await
    .map_err(|e| warp::reject::custom(WadoError { message: e }))?;

  if instances.is_empty() {
    return Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Series not found or contains no instances"
        })),
        StatusCode::NOT_FOUND,
      )
      .into_response(),
    );
  }

  if config.verbose.unwrap_or(false) {
    println!(
      "Found {} instances in series {}/{}",
      instances.len(),
      study_uid,
      series_uid
    );
  }

  // Load metadata for all instances
  let mut metadata_array = Vec::new();
  for (study_uid, series_uid, instance_uid) in instances {
    match load_dicom_metadata(&study_uid, &series_uid, &instance_uid, config.clone()).await {
      Ok(metadata) => metadata_array.push(metadata),
      Err(e) => {
        if config.verbose.unwrap_or(false) {
          println!(
            "Failed to load metadata for {}/{}/{}: {}",
            study_uid, series_uid, instance_uid, e
          );
        }
      }
    }
  }

  if metadata_array.is_empty() {
    return Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Failed to load metadata for any instance in series"
        })),
        StatusCode::INTERNAL_SERVER_ERROR,
      )
      .into_response(),
    );
  }

  Ok(
    warp::http::Response::builder()
      .header("Content-Type", "application/dicom+json")
      .status(StatusCode::OK)
      .body(Bytes::from(serde_json::to_vec(&metadata_array).unwrap()))
      .unwrap()
      .into_response(),
  )
}

async fn retrieve_instance_metadata_handler(
  study_uid: String,
  series_uid: String,
  instance_uid: String,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!(
      "Retrieve instance metadata: {}/{}/{}",
      study_uid, series_uid, instance_uid
    );
  }

  match load_dicom_metadata(&study_uid, &series_uid, &instance_uid, config).await {
    Ok(json) => {
      // DICOM JSON format requires array of objects
      Ok(
        warp::http::Response::builder()
          .header("Content-Type", "application/dicom+json")
          .status(StatusCode::OK)
          .body(Bytes::from(serde_json::to_vec(&vec![json]).unwrap()))
          .unwrap()
          .into_response(),
      )
    }
    Err(e) => Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": e
        })),
        StatusCode::NOT_FOUND,
      )
      .into_response(),
    ),
  }
}

async fn retrieve_frames_handler(
  study_uid: String,
  series_uid: String,
  instance_uid: String,
  frame_list: String,
  accept: Option<String>,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!(
      "Retrieve frames: {}/{}/{}/frames/{}, Accept: {:?}",
      study_uid, series_uid, instance_uid, frame_list, accept
    );
  }

  // Parse frame list
  let frame_numbers = match parse_frame_list(&frame_list) {
    Ok(frames) => frames,
    Err(e) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Invalid frame list: {}", e)
          })),
          StatusCode::BAD_REQUEST,
        )
        .into_response(),
      );
    }
  };

  // Load DICOM file
  let buffer = match load_dicom_file(&study_uid, &series_uid, &instance_uid, config.clone()).await {
    Ok(buf) => buf,
    Err(e) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Failed to load DICOM file: {}", e)
          })),
          StatusCode::NOT_FOUND,
        )
        .into_response(),
      );
    }
  };

  // Extract frames
  match extract_frames(&buffer, &frame_numbers, accept.as_deref()).await {
    Ok(response) => Ok(response),
    Err(e) => Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": format!("Failed to extract frames: {}", e)
        })),
        StatusCode::INTERNAL_SERVER_ERROR,
      )
      .into_response(),
    ),
  }
}

async fn retrieve_bulkdata_handler(
  study_uid: String,
  series_uid: String,
  instance_uid: String,
  attribute_path: String,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!(
      "Retrieve bulkdata: {}/{}/{}, attribute: {}",
      study_uid, series_uid, instance_uid, attribute_path
    );
  }

  // Parse attribute path (e.g., "7FE00010" for PixelData)
  let tag_str = attribute_path.replace("-", "");

  // Load DICOM file
  let buffer = match load_dicom_file(&study_uid, &series_uid, &instance_uid, config.clone()).await {
    Ok(buf) => buf,
    Err(e) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Failed to load DICOM file: {}", e)
          })),
          StatusCode::NOT_FOUND,
        )
        .into_response(),
      );
    }
  };

  // Parse DICOM file to extract attribute
  use dicom_object::from_reader;
  use std::io::Cursor;

  let obj = match from_reader(Cursor::new(&buffer)) {
    Ok(obj) => obj,
    Err(e) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Failed to parse DICOM file: {}", e)
          })),
          StatusCode::INTERNAL_SERVER_ERROR,
        )
        .into_response(),
      );
    }
  };

  // Parse tag from hex string (e.g., "7FE00010" -> Tag(0x7FE0, 0x0010))
  if tag_str.len() != 8 {
    return Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({
                "error": format!("Invalid tag format: {}. Expected 8 hex digits (e.g., 7FE00010)", tag_str)
            })),
            StatusCode::BAD_REQUEST,
        ).into_response());
  }

  let group = match u16::from_str_radix(&tag_str[0..4], 16) {
    Ok(g) => g,
    Err(_) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Invalid tag group: {}", &tag_str[0..4])
          })),
          StatusCode::BAD_REQUEST,
        )
        .into_response(),
      );
    }
  };

  let element = match u16::from_str_radix(&tag_str[4..8], 16) {
    Ok(e) => e,
    Err(_) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Invalid tag element: {}", &tag_str[4..8])
          })),
          StatusCode::BAD_REQUEST,
        )
        .into_response(),
      );
    }
  };

  use dicom_core::Tag;
  let tag = Tag(group, element);

  // Get element by tag
  let element_data = match obj.element(tag) {
    Ok(elem) => elem,
    Err(_) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Attribute {} not found in instance", tag_str)
          })),
          StatusCode::NOT_FOUND,
        )
        .into_response(),
      );
    }
  };

  // Extract raw bytes from element
  let bytes = match element_data.to_bytes() {
    Ok(b) => b.to_vec(),
    Err(e) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Failed to extract bytes from attribute: {}", e)
          })),
          StatusCode::INTERNAL_SERVER_ERROR,
        )
        .into_response(),
      );
    }
  };

  if config.verbose.unwrap_or(false) {
    println!("Retrieved {} bytes for attribute {}", bytes.len(), tag_str);
  }

  // Return raw bytes with appropriate content type
  Ok(
    warp::http::Response::builder()
      .header("Content-Type", "application/octet-stream")
      .status(StatusCode::OK)
      .body(Bytes::from(bytes))
      .unwrap()
      .into_response(),
  )
}

async fn retrieve_rendered_handler(
  study_uid: String,
  series_uid: String,
  instance_uid: String,
  params: std::collections::HashMap<String, String>,
  accept: Option<String>,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!(
      "Retrieve rendered: {}/{}/{}, params: {:?}",
      study_uid, series_uid, instance_uid, params
    );
  }

  // Load DICOM file
  let buffer = match load_dicom_file(&study_uid, &series_uid, &instance_uid, config.clone()).await {
    Ok(buf) => buf,
    Err(e) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Failed to load DICOM file: {}", e)
          })),
          StatusCode::NOT_FOUND,
        )
        .into_response(),
      );
    }
  };

  // Parse query parameters
  let viewport = params.get("viewport").map(|s| s.as_str());
  let quality = params
    .get("quality")
    .and_then(|s| s.parse::<u8>().ok())
    .unwrap_or(90);
  let window = params.get("window").map(|s| s.as_str());

  // Determine output format from Accept header
  let format = accept.as_deref().unwrap_or("image/jpeg").to_lowercase();

  match render_dicom_image(&buffer, viewport, Some(quality), window, &format).await {
    Ok((image_bytes, content_type)) => Ok(
      warp::http::Response::builder()
        .header("Content-Type", content_type)
        .status(StatusCode::OK)
        .body(Bytes::from(image_bytes))
        .unwrap()
        .into_response(),
    ),
    Err(e) => Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": format!("Failed to render image: {}", e)
        })),
        StatusCode::INTERNAL_SERVER_ERROR,
      )
      .into_response(),
    ),
  }
}

async fn retrieve_thumbnail_handler(
  study_uid: String,
  series_uid: String,
  instance_uid: String,
  params: std::collections::HashMap<String, String>,
  accept: Option<String>,
  config: Arc<WadoRequestConfig>,
) -> std::result::Result<Response, warp::Rejection> {
  if config.verbose.unwrap_or(false) {
    println!(
      "Retrieve thumbnail: {}/{}/{}, params: {:?}",
      study_uid, series_uid, instance_uid, params
    );
  }

  // Load DICOM file
  let buffer = match load_dicom_file(&study_uid, &series_uid, &instance_uid, config.clone()).await {
    Ok(buf) => buf,
    Err(e) => {
      return Ok(
        warp::reply::with_status(
          warp::reply::json(&serde_json::json!({
              "error": format!("Failed to load DICOM file: {}", e)
          })),
          StatusCode::NOT_FOUND,
        )
        .into_response(),
      );
    }
  };

  // Get thumbnail options from config or params
  let viewport = if let Some(vp) = params.get("viewport") {
    vp.clone()
  } else if let Some(opts) = config.thumbnail_options.as_ref() {
    format!(
      "{},{}",
      opts.width.unwrap_or(200),
      opts.height.unwrap_or(200)
    )
  } else {
    "200,200".to_string()
  };
  let viewport_str = viewport.as_str();

  let quality = params
    .get("quality")
    .and_then(|s| s.parse::<u8>().ok())
    .or(
      config
        .thumbnail_options
        .as_ref()
        .and_then(|opts| opts.quality),
    )
    .unwrap_or(80);

  // Determine output format
  let format = accept.as_deref().unwrap_or("image/jpeg").to_lowercase();

  match render_dicom_image(&buffer, Some(viewport_str), Some(quality), None, &format).await {
    Ok((image_bytes, content_type)) => Ok(
      warp::http::Response::builder()
        .header("Content-Type", content_type)
        .status(StatusCode::OK)
        .body(Bytes::from(image_bytes))
        .unwrap()
        .into_response(),
    ),
    Err(e) => Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": format!("Failed to render thumbnail: {}", e)
        })),
        StatusCode::INTERNAL_SERVER_ERROR,
      )
      .into_response(),
    ),
  }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse frame list string into vector of frame numbers
/// Formats: "1" or "1,3,5" or "1-10" or "1,3-5,7"
fn parse_frame_list(frame_list: &str) -> std::result::Result<Vec<usize>, String> {
  let mut frames = Vec::new();

  for part in frame_list.split(',') {
    let part = part.trim();

    if part.contains('-') {
      // Range: "1-10"
      let range_parts: Vec<&str> = part.split('-').collect();
      if range_parts.len() != 2 {
        return Err(format!("Invalid range format: {}", part));
      }

      let start: usize = range_parts[0]
        .parse()
        .map_err(|_| format!("Invalid start number: {}", range_parts[0]))?;
      let end: usize = range_parts[1]
        .parse()
        .map_err(|_| format!("Invalid end number: {}", range_parts[1]))?;

      if start > end {
        return Err(format!("Invalid range: start ({}) > end ({})", start, end));
      }

      for i in start..=end {
        frames.push(i);
      }
    } else {
      // Single frame: "1"
      let frame: usize = part
        .parse()
        .map_err(|_| format!("Invalid frame number: {}", part))?;
      frames.push(frame);
    }
  }

  if frames.is_empty() {
    return Err("Empty frame list".to_string());
  }

  // Remove duplicates and sort
  frames.sort_unstable();
  frames.dedup();

  Ok(frames)
}

/// Extract frames from DICOM pixel data
async fn extract_frames(
  buffer: &[u8],
  frame_numbers: &[usize],
  accept: Option<&str>,
) -> std::result::Result<Response, String> {
  // Write buffer to temporary file for dicom_object to read
  let temp_path = format!("/tmp/wado_frame_{}.dcm", uuid::Uuid::new_v4().simple());
  tokio::fs::write(&temp_path, buffer)
    .await
    .map_err(|e| format!("Failed to write temp file: {}", e))?;

  // Parse DICOM file
  let obj = open_file(&temp_path).map_err(|e| format!("Failed to parse DICOM file: {}", e))?;

  // Get number of frames
  let num_frames = obj
    .element(dicom_dictionary_std::tags::NUMBER_OF_FRAMES)
    .ok()
    .and_then(|e| e.to_int::<u32>().ok())
    .unwrap_or(1) as usize;

  // Validate frame numbers
  for &frame_num in frame_numbers {
    if frame_num < 1 || frame_num > num_frames {
      return Err(format!(
        "Frame {} out of range (1-{})",
        frame_num, num_frames
      ));
    }
  }

  #[cfg(feature = "transcode")]
  {
    // Get pixel data info
    let rows = obj
      .element(dicom_dictionary_std::tags::ROWS)
      .map_err(|e| format!("Failed to read rows: {}", e))?
      .to_int::<u16>()
      .map_err(|e| format!("Failed to convert rows: {}", e))? as usize;

    let columns = obj
      .element(dicom_dictionary_std::tags::COLUMNS)
      .map_err(|e| format!("Failed to read columns: {}", e))?
      .to_int::<u16>()
      .map_err(|e| format!("Failed to convert columns: {}", e))? as usize;

    let bits_allocated = obj
      .element(dicom_dictionary_std::tags::BITS_ALLOCATED)
      .map_err(|e| format!("Failed to read bits allocated: {}", e))?
      .to_int::<u16>()
      .map_err(|e| format!("Failed to convert bits allocated: {}", e))?
      as usize;

    let samples_per_pixel = obj
      .element(dicom_dictionary_std::tags::SAMPLES_PER_PIXEL)
      .map_err(|e| format!("Failed to read samples per pixel: {}", e))?
      .to_int::<u16>()
      .map_err(|e| format!("Failed to convert samples per pixel: {}", e))?
      as usize;

    // Calculate frame size in bytes
    let frame_size = rows * columns * samples_per_pixel * (bits_allocated / 8);

    // Get raw pixel data (works for both compressed and uncompressed)
    let all_bytes = obj
      .element(dicom_dictionary_std::tags::PIXEL_DATA)
      .map_err(|e| format!("Failed to get pixel data element: {}", e))?
      .to_bytes()
      .map_err(|e| format!("Failed to get raw pixel data: {}", e))?
      .to_vec();

    // Build multipart response
    let boundary = format!("boundary_{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::new();

    for &frame_num in frame_numbers {
      // Frame numbers are 1-indexed in DICOM
      let frame_index = frame_num - 1;

      // Calculate frame offset
      let offset = frame_index * frame_size;
      if offset + frame_size > all_bytes.len() {
        return Err(format!(
          "Frame {} exceeds pixel data size (offset: {}, frame_size: {}, total: {})",
          frame_num,
          offset,
          frame_size,
          all_bytes.len()
        ));
      }

      // Add boundary
      body.extend_from_slice(format!("\r\n--{}\r\n", boundary).as_bytes());
      body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");

      // Extract frame data
      let frame_data = &all_bytes[offset..offset + frame_size];
      body.extend_from_slice(frame_data);
    }

    // Final boundary
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    // Clean up temp file
    let _ = tokio::fs::remove_file(&temp_path).await;

    Ok(
      warp::http::Response::builder()
        .status(StatusCode::OK)
        .header(
          "Content-Type",
          format!(
            "multipart/related; type=\"application/octet-stream\"; boundary=\"{}\"",
            boundary
          ),
        )
        .body(body.into())
        .unwrap(),
    )
  }

  #[cfg(not(feature = "transcode"))]
  {
    // Clean up temp file
    let _ = tokio::fs::remove_file(&temp_path).await;

    Err("Frame extraction requires the 'transcode' feature to be enabled".to_string())
  }
}

/// Scans filesystem storage for all instances in a study
async fn scan_study_instances(
  base_path: &str,
  study_uid: &str,
) -> std::result::Result<Vec<(String, String, String)>, String> {
  let mut instances = Vec::new();
  let study_path = format!("{}/{}", base_path, study_uid);

  let mut entries = tokio::fs::read_dir(&study_path)
    .await
    .map_err(|e| format!("Failed to read study directory: {}", e))?;

  while let Some(series_entry) = entries
    .next_entry()
    .await
    .map_err(|e| format!("Failed to read series entry: {}", e))?
  {
    if !series_entry
      .file_type()
      .await
      .map_err(|e| format!("Failed to get file type: {}", e))?
      .is_dir()
    {
      continue;
    }

    let series_uid = series_entry.file_name().to_string_lossy().to_string();
    let series_path = series_entry.path();

    let mut series_entries = tokio::fs::read_dir(&series_path)
      .await
      .map_err(|e| format!("Failed to read series directory: {}", e))?;

    while let Some(instance_entry) = series_entries
      .next_entry()
      .await
      .map_err(|e| format!("Failed to read instance entry: {}", e))?
    {
      let file_name = instance_entry.file_name().to_string_lossy().to_string();
      if file_name.ends_with(".dcm") {
        let instance_uid = file_name.trim_end_matches(".dcm").to_string();
        instances.push((study_uid.to_string(), series_uid.clone(), instance_uid));
      }
    }
  }

  Ok(instances)
}

/// Scans filesystem storage for all instances in a series
async fn scan_series_instances(
  base_path: &str,
  study_uid: &str,
  series_uid: &str,
) -> std::result::Result<Vec<(String, String, String)>, String> {
  let mut instances = Vec::new();
  let series_path = format!("{}/{}/{}", base_path, study_uid, series_uid);

  let mut entries = tokio::fs::read_dir(&series_path)
    .await
    .map_err(|e| format!("Failed to read series directory: {}", e))?;

  while let Some(instance_entry) = entries
    .next_entry()
    .await
    .map_err(|e| format!("Failed to read instance entry: {}", e))?
  {
    let file_name = instance_entry.file_name().to_string_lossy().to_string();
    if file_name.ends_with(".dcm") {
      let instance_uid = file_name.trim_end_matches(".dcm").to_string();
      instances.push((study_uid.to_string(), series_uid.to_string(), instance_uid));
    }
  }

  Ok(instances)
}

/// Scans S3 storage for all instances in a study
async fn scan_study_instances_s3(
  s3_config: &crate::utils::S3Config,
  study_uid: &str,
) -> std::result::Result<Vec<(String, String, String)>, String> {
  let bucket = crate::utils::s3::build_s3_bucket(s3_config);
  let prefix = format!("{}/", study_uid);

  // List all objects with the study prefix
  let objects = crate::utils::s3::s3_list_objects(&bucket, &prefix)
    .await
    .map_err(|e| format!("Failed to list S3 objects: {}", e))?;

  let mut instances = Vec::new();

  for object_key in objects {
    // Expected format: {studyUID}/{seriesUID}/{instanceUID}.dcm
    if !object_key.ends_with(".dcm") {
      continue;
    }

    let parts: Vec<&str> = object_key.split('/').collect();
    if parts.len() >= 3 {
      let study = parts[0].to_string();
      let series = parts[1].to_string();
      let instance = parts[2].trim_end_matches(".dcm").to_string();
      instances.push((study, series, instance));
    }
  }

  Ok(instances)
}

/// Scans S3 storage for all instances in a series
async fn scan_series_instances_s3(
  s3_config: &crate::utils::S3Config,
  study_uid: &str,
  series_uid: &str,
) -> std::result::Result<Vec<(String, String, String)>, String> {
  let bucket = crate::utils::s3::build_s3_bucket(s3_config);
  let prefix = format!("{}/{}/", study_uid, series_uid);

  // List all objects with the series prefix
  let objects = crate::utils::s3::s3_list_objects(&bucket, &prefix)
    .await
    .map_err(|e| format!("Failed to list S3 objects: {}", e))?;

  let mut instances = Vec::new();

  for object_key in objects {
    // Expected format: {studyUID}/{seriesUID}/{instanceUID}.dcm
    if !object_key.ends_with(".dcm") {
      continue;
    }

    let parts: Vec<&str> = object_key.split('/').collect();
    if parts.len() >= 3 {
      let instance = parts[2].trim_end_matches(".dcm").to_string();
      instances.push((study_uid.to_string(), series_uid.to_string(), instance));
    }
  }

  Ok(instances)
}

/// Builds a multipart/related response with multiple DICOM instances
async fn build_multipart_response(
  instances: Vec<(String, String, String)>,
  config: Arc<WadoRequestConfig>,
  media_type: WadoMediaType,
) -> std::result::Result<Response, String> {
  let boundary = format!("boundary_{}", uuid::Uuid::new_v4().simple());
  let mut body = Vec::new();

  for (study_uid, series_uid, instance_uid) in instances {
    // Add boundary
    body.extend_from_slice(format!("\r\n--{}\r\n", boundary).as_bytes());

    match media_type {
      WadoMediaType::Dicom => {
        // Load DICOM file
        let dicom_data =
          load_dicom_file(&study_uid, &series_uid, &instance_uid, config.clone()).await?;

        body.extend_from_slice(b"Content-Type: application/dicom\r\n\r\n");
        body.extend_from_slice(&dicom_data);
      }
      WadoMediaType::DicomJson => {
        // Load DICOM metadata as JSON
        let metadata =
          load_dicom_metadata(&study_uid, &series_uid, &instance_uid, config.clone()).await?;

        body.extend_from_slice(b"Content-Type: application/dicom+json\r\n\r\n");
        let metadata_str = serde_json::to_string(&metadata)
          .map_err(|e| format!("Failed to serialize metadata: {}", e))?;
        body.extend_from_slice(metadata_str.as_bytes());
      }
      WadoMediaType::DicomXml => {
        return Err("DICOM XML format not yet supported".to_string());
      }
    }
  }

  // Final boundary
  body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

  let content_type = match media_type {
    WadoMediaType::Dicom => {
      format!(
        "multipart/related; type=\"application/dicom\"; boundary=\"{}\"",
        boundary
      )
    }
    WadoMediaType::DicomJson => {
      format!(
        "multipart/related; type=\"application/dicom+json\"; boundary=\"{}\"",
        boundary
      )
    }
    WadoMediaType::DicomXml => {
      format!(
        "multipart/related; type=\"application/dicom+xml\"; boundary=\"{}\"",
        boundary
      )
    }
  };

  Ok(
    warp::http::Response::builder()
      .status(StatusCode::OK)
      .header("Content-Type", content_type)
      .body(body.into())
      .unwrap(),
  )
}

fn parse_accept_header(accept: Option<&str>) -> WadoMediaType {
  match accept {
    Some(header) => {
      let lower = header.to_lowercase();
      if lower.contains("application/dicom+json") {
        WadoMediaType::DicomJson
      } else if lower.contains("application/dicom+xml") {
        WadoMediaType::DicomXml
      } else {
        WadoMediaType::Dicom
      }
    }
    None => WadoMediaType::Dicom,
  }
}

async fn handle_rejection(err: warp::Rejection) -> std::result::Result<Response, warp::Rejection> {
  if err.is_not_found() {
    Ok(
      warp::reply::with_status(
        warp::reply::json(&serde_json::json!({
            "error": "Not found"
        })),
        StatusCode::NOT_FOUND,
      )
      .into_response(),
    )
  } else {
    Err(err)
  }
}

/// Render DICOM image to standard image format (JPEG/PNG)
async fn render_dicom_image(
  buffer: &[u8],
  viewport: Option<&str>,
  quality: Option<u8>,
  window: Option<&str>,
  format: &str,
) -> std::result::Result<(Vec<u8>, &'static str), String> {
  use crate::utils::image_processing::{
    parse_viewport, parse_window, render_dicom_image as render_image, ImageOutputFormat,
    ImageRenderOptions,
  };

  // Parse viewport if provided
  let (width, height) = if let Some(vp) = viewport {
    let (w, h) = parse_viewport(vp)?;
    (Some(w), Some(h))
  } else {
    (None, None)
  };

  // Parse window if provided
  let (window_center, window_width) = if let Some(w) = window {
    let (c, w) = parse_window(w)?;
    (Some(c), Some(w))
  } else {
    (None, None)
  };

  // Create render options with VOI LUT support for WADO-RS
  // Apply VOI LUT if window parameters are not manually specified
  let options = ImageRenderOptions {
    width,
    height,
    quality,
    window_center,
    window_width,
    apply_voi_lut: Some(window_center.is_none() && window_width.is_none()),
    rescale_intercept: None, // Let utility read from file
    rescale_slope: None,     // Let utility read from file
    convert_to_8bit: None,   // Always 8-bit for rendered output
    frame_number: None,
    format: ImageOutputFormat::from_mime_type(format),
  };

  // Render the image
  let output = render_image(buffer, &options)?;
  let content_type = options.format.content_type();

  Ok((output, content_type))
}

// ============================================================================
// WADO-RS CORS Configuration Guide
// ============================================================================

/*
 * WADO-RS CORS (Cross-Origin Resource Sharing) Configuration
 *
 * This module provides CORS support for WADO-RS servers to enable web-based
 * DICOM applications to retrieve medical imaging data from different origins.
 *
 * ## DICOM Standard Reference
 * - **DICOM PS3.18 Section 8:** WADO-RS (Web Access to DICOM Objects)
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
 * Enable CORS when your WADO-RS server needs to be accessed by:
 *
 * 1. **Web-based DICOM Viewers:**
 *    - OHIF Viewer (https://ohif.org)
 *    - Cornerstone-based applications
 *    - Radiant DICOM Viewer web interface
 *    - Custom React/Vue/Angular medical imaging apps
 *
 * 2. **Single-Page Applications (SPAs):**
 *    - Frontend served from different domain than PACS/WADO server
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
 * The `WadoServerConfig` object supports the following CORS options:
 *
 * ### 1. `enableCors` (boolean, default: false)
 *
 * Master switch to enable/disable CORS support.
 *
 * ```typescript
 * // CORS disabled (default) - restrictive, internal network only
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
 *   enableCors: false
 * });
 *
 * // CORS enabled - allows cross-origin requests
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
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
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com'
 * });
 *
 * // Allow multiple origins (comma-separated)
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com,https://app.hospital.com,https://research.hospital.com'
 * });
 *
 * // Allow all origins (development only!)
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
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
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com',
 *   verbose: true  // Log CORS configuration
 * });
 * ```
 *
 * ## CORS Headers Sent by WADO-RS
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
 * const wado = new WadoServer(443, {
 *   storageType: 'S3',
 *   s3Config: { /* S3 settings */ },
 *   enableCors: true,
 *   corsAllowedOrigins: 'https://viewer.hospital.com',
 *   verbose: false
 * });
 *
 * // ❌ BAD: Insecure production configuration
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
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
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: './testdata',
 *   enableCors: true,
 *   corsAllowedOrigins: 'http://localhost:3000,http://localhost:5173',
 *   verbose: true
 * });
 * ```
 *
 * ### 3. Network Segmentation
 *
 * If your WADO server is only accessible within a hospital network,
 * you may choose to disable CORS and rely on network-level security:
 *
 * ```typescript
 * // Internal network only - no CORS needed
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: '/path/to/dicom',
 *   enableCors: false  // Network firewall provides security
 * });
 * ```
 *
 * ## Complete Usage Examples
 *
 * ### Example 1: OHIF Viewer Integration
 *
 * ```typescript
 * import { WadoServer } from '@nuxthealth/node-dicom';
 *
 * // WADO server for OHIF Viewer
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: './dicom-storage',
 *   enableCors: true,
 *   corsAllowedOrigins: 'http://localhost:3000',  // OHIF development server
 *   enableMetadata: true,
 *   enableFrames: true,
 *   enableRendered: true,
 *   enableThumbnail: true,
 *   verbose: true
 * });
 *
 * wado.start();
 * console.log('WADO-RS server ready for OHIF Viewer');
 * ```
 *
 * ### Example 2: Multi-Origin Production Setup
 *
 * ```typescript
 * // Production: Multiple hospital viewer origins
 * const wado = new WadoServer(443, {
 *   storageType: 'S3',
 *   s3Config: {
 *     endpoint: 'https://s3.amazonaws.com',
 *     region: 'us-east-1',
 *     bucket: 'hospital-dicom-prod',
 *     accessKeyId: process.env.AWS_ACCESS_KEY_ID,
 *     secretAccessKey: process.env.AWS_SECRET_ACCESS_KEY
 *   },
 *   enableCors: true,
 *   corsAllowedOrigins: [
 *     'https://radiology.hospital.com',
 *     'https://cardiology.hospital.com',
 *     'https://oncology.hospital.com'
 *   ].join(','),
 *   enableMetadata: true,
 *   enableFrames: true,
 *   enableRendered: true,
 *   enableThumbnail: true,
 *   verbose: false
 * });
 *
 * wado.start();
 * ```
 *
 * ### Example 3: Development with Hot Reload
 *
 * ```typescript
 * // Development with Vite/React hot reload
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: './testdata',
 *   enableCors: true,
 *   corsAllowedOrigins: 'http://localhost:5173,http://127.0.0.1:5173',
 *   enableMetadata: true,
 *   enableFrames: true,
 *   enableRendered: true,
 *   enableThumbnail: true,
 *   thumbnailOptions: {
 *     quality: 80,
 *     width: 200,
 *     height: 200
 *   },
 *   verbose: true
 * });
 *
 * wado.start();
 * console.log('🔥 WADO-RS server ready for hot reload development');
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
 *      -v http://localhost:8042/studies/1.2.3.4/instances
 *
 * # Test actual GET request with CORS
 * curl -H "Origin: http://localhost:3000" \
 *      -H "Accept: application/dicom" \
 *      -v http://localhost:8042/studies/1.2.3.4/series/5.6.7.8/instances/9.10.11.12
 *
 * # Check for Access-Control-Allow-Origin header in response
 * ```
 *
 * ### Using Browser DevTools
 *
 * Open browser console and test:
 *
 * ```javascript
 * // Retrieve DICOM instance
 * fetch('http://localhost:8042/studies/1.2.3.4/series/5.6.7.8/instances/9.10.11.12', {
 *   headers: { 'Accept': 'application/dicom' }
 * })
 *   .then(res => res.arrayBuffer())
 *   .then(data => console.log('DICOM Instance:', data))
 *   .catch(err => console.error('CORS Error:', err));
 *
 * // Retrieve metadata
 * fetch('http://localhost:8042/studies/1.2.3.4/metadata')
 *   .then(res => res.json())
 *   .then(data => console.log('Study Metadata:', data))
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
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: './dicom-storage',
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
 * const wado = new WadoServer(8042, {
 *   storageType: 'Filesystem',
 *   basePath: './dicom-storage',
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
 * ### Error: Large images fail to load
 *
 * **Cause:** CORS headers missing on multipart responses
 *
 * **Solution:** CORS is automatically applied to all routes including multipart responses
 *
 * ## WADO-RS Specific Considerations
 *
 * ### Multipart Responses
 *
 * WADO-RS uses multipart/related responses for retrieving multiple instances.
 * CORS headers are automatically included in these responses.
 *
 * ### Binary Data Transfer
 *
 * Large DICOM files and pixel data require proper CORS configuration to work
 * in browsers. The `Access-Control-Allow-Headers` includes all necessary
 * headers for binary data transfer.
 *
 * ### Rendered/Thumbnail Endpoints
 *
 * Image rendering endpoints (JPEG/PNG) also respect CORS configuration,
 * allowing web viewers to display thumbnails and rendered images from
 * different origins.
 *
 * ## Related Documentation
 *
 * - DICOM PS3.18 WADO-RS: https://dicom.nema.org/medical/dicom/current/output/html/part18.html#sect_8
 * - MDN CORS Guide: https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS
 * - OHIF Configuration: https://docs.ohif.org/
 * - Cornerstone WADO Image Loader: https://github.com/cornerstonejs/cornerstoneWADOImageLoader
 *
 * ## TypeScript Type Definitions
 *
 * ```typescript
 * interface WadoServerConfig {
 *   // Storage backend configuration
 *   storageType: 'Filesystem' | 'S3';
 *   basePath?: string;  // Required for Filesystem
 *   s3Config?: S3Config;  // Required for S3
 *
 *   // CORS configuration
 *   enableCors?: boolean;  // Enable CORS headers (default: false)
 *   corsAllowedOrigins?: string;  // Comma-separated list of allowed origins
 *
 *   // Feature flags
 *   enableMetadata?: boolean;
 *   enableFrames?: boolean;
 *   enableRendered?: boolean;
 *   enableThumbnail?: boolean;
 *   enableBulkdata?: boolean;
 *
 *   // Transcoding and rendering
 *   defaultTranscoding?: 'None' | 'JpegBaseline' | 'Jpeg2000' | 'Png' | 'Uncompressed';
 *   thumbnailOptions?: WadoRenderingOptions;
 *
 *   // Server options
 *   maxConnections?: number;
 *   enableCompression?: boolean;
 *   verbose?: boolean;  // Enable verbose logging
 * }
 *
 * class WadoServer {
 *   constructor(port: number, config: WadoServerConfig);
 *   start(): void;
 *   stop(): void;
 * }
 * ```
 *
 * ## Performance Considerations
 *
 * ### Preflight Caching
 *
 * Browsers cache preflight (OPTIONS) requests. For production:
 * - Preflight responses are cached for a short time
 * - Reduce unnecessary preflight requests by using simple requests when possible
 * - Consider Access-Control-Max-Age header for longer cache times
 *
 * ### Large File Transfers
 *
 * WADO-RS often serves large DICOM files (10MB+):
 * - Enable compression (`enableCompression: true`) for better performance
 * - Use HTTP/2 for parallel transfers
 * - Consider CDN for frequently accessed studies
 * - Monitor network bandwidth and adjust `maxConnections`
 *
 * ## Security Checklist
 *
 * Before deploying WADO-RS with CORS to production:
 *
 * - [ ] CORS enabled only when needed
 * - [ ] Specific origins listed (no wildcards)
 * - [ ] HTTPS enforced for all origins
 * - [ ] Authentication/authorization implemented
 * - [ ] Rate limiting configured
 * - [ ] Logging and monitoring in place
 * - [ ] Regular security audits scheduled
 * - [ ] Network firewall rules configured
 * - [ ] PHI access logged for HIPAA compliance
 *
 */

#[cfg(test)]
mod tests {
  use super::*;

  fn manifest_entry(object_key: &str) -> WadoManifestEntry {
    WadoManifestEntry {
      study_instance_uid: "1.2.3".to_string(),
      series_instance_uid: "1.2.3.4".to_string(),
      sop_instance_uid: "1.2.3.4.5".to_string(),
      object: WadoObjectReference {
        storage_id: "project-store".to_string(),
        object_key: object_key.to_string(),
        version_id: Some("revision-version".to_string()),
      },
    }
  }

  fn test_server_config() -> WadoServerConfig {
    WadoServerConfig {
      storage_type: WadoStorageType::S3,
      base_path: None,
      s3_config: None,
      s3_storages: Some(vec![]),
      manifest: Some(WadoManifestConfig {
        adapter: WadoManifestAdapter::Memory,
        sqlite_path: None,
        postgres_url: None,
      }),
      bind_address: Some("127.0.0.1".to_string()),
      enable_metadata: None,
      enable_frames: None,
      enable_rendered: None,
      enable_thumbnail: None,
      enable_bulkdata: None,
      default_transcoding: None,
      max_connections: None,
      enable_cors: None,
      cors_allowed_origins: None,
      enable_compression: None,
      thumbnail_options: None,
      verbose: None,
    }
  }

  #[test]
  fn manifests_are_isolated_per_server_scope_and_support_same_uids() {
    let storage_ids = HashSet::from(["project-store".to_string()]);
    let older_revision =
      WadoManifestIndex::build(vec![manifest_entry("opaque/revision-a")], &storage_ids).unwrap();
    let newer_revision =
      WadoManifestIndex::build(vec![manifest_entry("opaque/revision-b")], &storage_ids).unwrap();
    let uid = (
      "1.2.3".to_string(),
      "1.2.3.4".to_string(),
      "1.2.3.4.5".to_string(),
    );

    assert_eq!(
      older_revision.instances[&uid].object_key,
      "opaque/revision-a"
    );
    assert_eq!(
      newer_revision.instances[&uid].object_key,
      "opaque/revision-b"
    );
    assert_eq!(older_revision.study_instances("1.2.3"), vec![uid.clone()]);
    assert_eq!(
      newer_revision.series_instances("1.2.3", "1.2.3.4"),
      vec![uid]
    );
  }

  #[test]
  fn manifest_rejects_duplicate_uid_tuples_and_unknown_storages() {
    let storage_ids = HashSet::from(["project-store".to_string()]);
    assert!(WadoManifestIndex::build(
      vec![manifest_entry("first"), manifest_entry("second")],
      &storage_ids,
    )
    .is_err());

    let mut unknown_storage = manifest_entry("opaque/key");
    unknown_storage.object.storage_id = "unregistered".to_string();
    assert!(WadoManifestIndex::build(vec![unknown_storage], &storage_ids).is_err());
  }

  #[test]
  fn request_snapshot_keeps_old_manifest_during_atomic_replacement() {
    let storage_ids = HashSet::from(["project-store".to_string()]);
    let old_index =
      Arc::new(WadoManifestIndex::build(vec![manifest_entry("opaque/old")], &storage_ids).unwrap());
    let new_index =
      Arc::new(WadoManifestIndex::build(vec![manifest_entry("opaque/new")], &storage_ids).unwrap());
    let manifest = WadoManifestStore::Memory(Arc::new(RwLock::new(old_index)));
    let runtime = WadoRuntimeConfig {
      settings: Arc::new(test_server_config()),
      storage_buckets: Arc::new(HashMap::new()),
      manifest_store: manifest.clone(),
    };

    let request_snapshot = runtime.snapshot();
    if let WadoManifestStore::Memory(index) = &manifest {
      *index.write().unwrap() = new_index;
    }
    let next_request = runtime.snapshot();
    let uid = (
      "1.2.3".to_string(),
      "1.2.3.4".to_string(),
      "1.2.3.4.5".to_string(),
    );

    assert_eq!(
      match &request_snapshot.manifest {
        WadoManifestSnapshot::Memory(index) => index.instances[&uid].object_key.as_str(),
        _ => panic!("expected memory manifest snapshot"),
      },
      "opaque/old"
    );
    assert_eq!(
      match &next_request.manifest {
        WadoManifestSnapshot::Memory(index) => index.instances[&uid].object_key.as_str(),
        _ => panic!("expected memory manifest snapshot"),
      },
      "opaque/new"
    );
  }

  #[test]
  fn sqlite_manifest_replacement_overwrites_deletes_and_rolls_back_on_invalid_input() {
    let connection = Arc::new(Mutex::new(open_manifest_database(":memory:").unwrap()));
    let storage_ids = HashSet::from(["project-store".to_string()]);
    let mut removed_entry = manifest_entry("opaque/removed");
    removed_entry.sop_instance_uid = "1.2.3.4.6".to_string();

    ingest_sqlite_manifest(
      &connection,
      vec![manifest_entry("opaque/old"), removed_entry],
      &storage_ids,
      true,
    )
    .unwrap();
    ingest_sqlite_manifest(
      &connection,
      vec![manifest_entry("opaque/new")],
      &storage_ids,
      false,
    )
    .unwrap();

    let snapshot = WadoManifestSnapshot::Sqlite(connection.clone());
    let updated = WADO_RUNTIME
      .block_on(snapshot.get((
        "1.2.3".to_string(),
        "1.2.3.4".to_string(),
        "1.2.3.4.5".to_string(),
      )))
      .unwrap()
      .unwrap();
    assert_eq!(updated.object_key, "opaque/new");
    assert!(WADO_RUNTIME
      .block_on(snapshot.get((
        "1.2.3".to_string(),
        "1.2.3.4".to_string(),
        "1.2.3.4.6".to_string(),
      )))
      .unwrap()
      .is_some());

    ingest_sqlite_manifest(
      &connection,
      vec![manifest_entry("opaque/final")],
      &storage_ids,
      true,
    )
    .unwrap();
    assert!(WADO_RUNTIME
      .block_on(snapshot.get((
        "1.2.3".to_string(),
        "1.2.3.4".to_string(),
        "1.2.3.4.6".to_string(),
      )))
      .unwrap()
      .is_none());

    assert!(ingest_sqlite_manifest(
      &connection,
      vec![
        manifest_entry("opaque/bad-1"),
        manifest_entry("opaque/bad-2")
      ],
      &storage_ids,
      false,
    )
    .is_err());
    let unchanged = WADO_RUNTIME
      .block_on(snapshot.get((
        "1.2.3".to_string(),
        "1.2.3.4".to_string(),
        "1.2.3.4.5".to_string(),
      )))
      .unwrap()
      .unwrap();
    assert_eq!(unchanged.object_key, "opaque/final");
  }

  #[test]
  fn first_sqlite_server_batch_preserves_existing_catalog_entries() {
    let storage_ids = HashSet::from(["project-store".to_string()]);
    let connection = Arc::new(Mutex::new(open_manifest_database(":memory:").unwrap()));
    ingest_sqlite_manifest(
      &connection,
      vec![manifest_entry("opaque/existing")],
      &storage_ids,
      true,
    )
    .unwrap();

    let mut config = test_server_config();
    config.s3_storages = Some(vec![WadoS3Storage {
      storage_id: "project-store".to_string(),
      config: crate::utils::S3Config {
        bucket: "test-bucket".to_string(),
        access_key: "test-access-key".to_string(),
        secret_key: "test-secret-key".to_string(),
        endpoint: Some("http://127.0.0.1:9000".to_string()),
      },
    }]);
    let server = WadoServer {
      port: 0,
      bind_address: "127.0.0.1".parse().unwrap(),
      config,
      manifest_loaded: AtomicBool::new(false),
      ingest_batch_count: AtomicU64::new(0),
      started: AtomicBool::new(false),
      manifest_ingest_lock: tokio::sync::Mutex::new(()),
      storage_ids,
      s3_buckets: Arc::new(HashMap::new()),
      manifest_store: WadoManifestStore::Sqlite(connection),
      shutdown_tx: None,
    };

    WADO_RUNTIME
      .block_on(server.ingest_manifest(vec![manifest_entry("opaque/new")], None))
      .unwrap();
    let metrics = WADO_RUNTIME
      .block_on(server.get_manifest_metrics())
      .unwrap();
    assert_eq!(metrics.instance_count, 2.0);
    assert_eq!(metrics.ingest_batch_count, 1.0);
  }

  #[tokio::test]
  async fn resolver_miss_does_not_fall_back_to_uid_derived_s3_key() {
    let config = Arc::new(WadoRequestConfig {
      settings: Arc::new(test_server_config()),
      storage_buckets: Arc::new(HashMap::new()),
      manifest: WadoManifestSnapshot::Memory(Arc::new(WadoManifestIndex::default())),
    });

    let error = load_dicom_file("1.2.3", "1.2.3.4", "1.2.3.4.5", config)
      .await
      .unwrap_err();

    assert_eq!(error, "DICOM instance not found");
  }
}

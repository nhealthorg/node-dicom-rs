# WADO-RS Server

WADO-RS (Web Access to DICOM Objects - RESTful Services) is a DICOMweb service for retrieving DICOM studies, series, instances, frames, and metadata over HTTP. This implementation follows the [DICOM Part 18](http://dicom.nema.org/medical/dicom/current/output/html/part18.html) specification.

## Overview

The WADO-RS server provides RESTful HTTP endpoints for retrieving DICOM objects and metadata. It supports:

- Multiple storage backends (Filesystem, S3)
- Content negotiation (DICOM, DICOM JSON, DICOM XML)
- Pixel data transcoding
- Frame-level retrieval
- Metadata-only retrieval
- CORS support
- Configurable feature flags

Metadata endpoints return DICOM JSON with full eight-digit group/element tag
keys, separate values for multi-valued attributes, numeric values and nested
sequences. Pixel Data, Float Pixel Data and Double Float Pixel Data are omitted;
retrieve pixel data through the frame or instance endpoints instead. Metadata
parsing uses the downloaded bytes directly, without shared temporary files.

## Configuration

```typescript
const config = {
    storageType: 'Filesystem' | 'S3',
    
    // Storage backend configuration
    basePath?: string,              // Required for Filesystem
    s3Config?: S3Config,           // Required for S3
    s3Storages?: WadoS3Storage[],  // Named stores for manifest mode
    manifest?: {                   // Defaults based on s3Storages when omitted
        adapter: 'Memory' | 'Sqlite' | 'Postgres' | 'None',
        sqlitePath?: string,       // Required for Sqlite
        postgresUrl?: string       // Required for Postgres
    },
    bindAddress?: string,          // IP address (default: 0.0.0.0)
    
    // Feature toggles
    enableMetadata?: boolean,      // Enable metadata endpoints (default: true)
    enableFrames?: boolean,        // Enable frame retrieval (default: true)
    enableRendered?: boolean,      // Enable rendered endpoints (default: false)
    enableThumbnail?: boolean,     // Enable thumbnail generation (default: false)
    enableBulkdata?: boolean,      // Enable bulkdata retrieval (default: false)
    
    // Transcoding options
    defaultTranscoding?: 'None' | 'JpegBaseline' | 'Jpeg2000' | 'Png' | 'Uncompressed',
    
    // HTTP features
    enableCors?: boolean,          // Enable CORS headers (default: false)
    enableCompression?: boolean,   // Enable gzip compression (default: false)
    
    // Performance
    maxConnections?: number,       // Maximum concurrent connections
    
    // Rendering
    thumbnailOptions?: {
        quality?: number,          // JPEG quality 1-100 (default: 90)
        width?: number,            // Output width in pixels
        height?: number,           // Output height in pixels
        windowCenter?: number,     // Grayscale window center
        windowWidth?: number,      // Grayscale window width
    },
    
    // Logging
    verbose?: boolean,             // Enable verbose logging (default: false)
};
```

## Storage Backends

### Filesystem Storage

Files must be organized as: `{basePath}/{studyUID}/{seriesUID}/{instanceUID}.dcm`

```javascript
const config = {
    storageType: 'Filesystem',
    basePath: '/path/to/dicom/storage',
    verbose: true
};

const server = new WadoServer(8043, config);
server.start();
```

### Manifest-Resolved, Version-Pinned S3 Storage

Use `s3Storages` and `ingestManifest()` to resolve standard DICOM UIDs to opaque S3 keys and optional exact object versions. `manifest.adapter` selects the resolver index: `Memory` keeps it in process memory, `Sqlite` stores it in a local database, `Postgres` stores it in PostgreSQL, and `None` disables the resolver and preserves legacy UID-derived behavior. For backward compatibility, a non-empty `s3Storages` defaults to `Memory`; without it, the adapter defaults to `None`. Resolver mode does not fall back to UID-derived keys when an entry is missing or a version cannot be read.

```typescript
import { WadoManifestAdapter, WadoServer, WadoStorageType } from 'node-dicom-rs';

const wado = new WadoServer(8043, {
    storageType: WadoStorageType.S3,
    manifest: {
        adapter: WadoManifestAdapter.Sqlite,
        sqlitePath: '/var/lib/node-dicom/wado-manifest.sqlite'
    },
    bindAddress: '127.0.0.1',
    s3Storages: [
        {
            storageId: 'project-a',
            config: {
                bucket: 'private-dicom-a',
                endpoint: 'https://s3.example.internal',
                accessKey: process.env.S3_ACCESS_KEY!,
                secretKey: process.env.S3_SECRET_KEY!
            }
        }
    ],
    enableMetadata: true,
    enableFrames: true,
    enableRendered: true,
    enableThumbnail: true,
    enableBulkdata: true
});

await wado.ingestManifest([
    {
        studyInstanceUid: '1.2.3',
        seriesInstanceUid: '1.2.3.4',
        sopInstanceUid: '1.2.3.4.5',
        object: {
            storageId: 'project-a',
            objectKey: 'records/opaque-object-9482',
            versionId: '3Lg...exact-version'
        }
    }
]);

wado.start();
```

There is no separate begin/finish API: call `ingestManifest()` repeatedly with batches before `start()`. Every call upserts its batch; the first call has no special behavior, so importing a large catalog in batches works consistently. Existing rows in persistent SQLite or PostgreSQL catalogs are retained unless explicitly replaced. To discard an old catalog while importing in batches, pass `{ replaceAll: true }` on the first batch; that transaction clears the old catalog and inserts that batch, while later calls upsert normally. `start()` marks ingestion complete; while serving, calls may only apply incremental upserts. SQLite creates the database, parent directories, and schema automatically. PostgreSQL creates its `wado_manifest` table on first ingestion; provide `postgresUrl` using a PostgreSQL connection URL. Use a dedicated database or schema for each independent manifest scope because the adapter uses a fixed table name. Every database batch is transactional; validation or database errors roll it back. Duplicate study/series/instance UID tuples within a batch and unregistered `storageId` values are rejected. Memory publishes each batch as an atomic index snapshot; SQLite and PostgreSQL make each batch visible atomically.

Call `getManifestMetrics()` between batches to inspect the current catalog and import state:

```typescript
const metrics = await wado.getManifestMetrics();
console.log(metrics.studyCount, metrics.seriesCount, metrics.instanceCount);
```

The returned counts reflect the current catalog after successful ingests, not the cumulative number of rows sent; `ingestBatchCount` counts successful calls. `manifestLoaded` becomes true after the first successful batch, and `started` becomes true after the server starts. The call waits for an active ingest to finish so all values describe a consistent snapshot. Database-backed adapters calculate catalog counts with an aggregate query, so avoid polling excessively often for very large catalogs.

Study, series, and their metadata endpoints enumerate only matching entries in the selected manifest adapter. The composite primary key indexes study and series prefixes in SQLite and PostgreSQL; object data itself remains in S3 and is fetched only for requested instances. Instance, metadata, frame, bulkdata, rendered, and thumbnail endpoints all load through the same resolved object reference. Set `versionId` to pin an object; when omitted, the current object version is requested. An unavailable pinned version returns an error and is never retried without the version.

Each `WadoServer` instance has one manifest scope. To serve revisions that reuse the same DICOM UIDs, run a separate instance/scope for each authorized project or revision and have the authenticated application gateway route to the correct private listener. `bindAddress` can restrict the listener to loopback or another private interface; it defaults to `0.0.0.0` for backward compatibility. The service does not authenticate users itself, so do not expose a manifest-scoped listener publicly without an authorization gateway or equivalent network controls. Do not select project or revision based on untrusted client query parameters or headers.

The client-facing WADO-RS URLs and media negotiation remain standard. Storage IDs, bucket details, credentials, object keys, and version IDs are server-side metadata and are not returned in WADO responses.

#### Benchmarking

The benchmark compares end-to-end instance, instance-metadata, series, and study retrieval against the same S3 bucket. Use UID-derived object keys so both modes resolve the same objects. Store a JSON file outside version control with this shape:

```json
{
    "port": 18043,
    "iterations": 20,
    "s3Config": {
        "bucket": "private-dicom",
        "endpoint": "http://127.0.0.1:7070",
        "accessKey": "...",
        "secretKey": "..."
    },
    "entries": [
        {
            "studyInstanceUid": "1.2.3",
            "seriesInstanceUid": "1.2.3.4",
            "sopInstanceUid": "1.2.3.4.5",
            "objectKey": "1.2.3/1.2.3.4/1.2.3.4.5.dcm"
        }
    ]
}
```

Run `node playground/wado-s3-manifest-benchmark.mjs /path/to/benchmark-config.json`. The script reports median and p95 end-to-end latency; verbose WADO server output reports resolver/list lookup, S3 GET, and DICOM parse stages separately. Because the legacy path cannot pin an S3 version, omit `versionId` and keep the test objects unchanged for the duration of the comparison. Run against an otherwise idle S3-compatible service.

### S3 Storage

Uses S3-compatible object storage (AWS S3, MinIO, etc.):

```javascript
const config = {
    storageType: 'S3',
    s3Config: {
        endpoint: 'https://s3.amazonaws.com',
        region: 'us-east-1',
        bucket: 'my-dicom-bucket',
        accessKeyId: 'YOUR_ACCESS_KEY',
        secretAccessKey: 'YOUR_SECRET_KEY'
    },
    verbose: true
};

const server = new WadoServer(8043, config);
server.start();
```

## DICOM Part 18 Endpoints

### Retrieve Study

Retrieve all instances in a study:

```
GET /studies/{studyUID}
Accept: application/dicom
```

Returns multipart/related response with all DICOM instances.

**Status:** ✅ Implemented (Filesystem & S3)

### Retrieve Series

Retrieve all instances in a series:

```
GET /studies/{studyUID}/series/{seriesUID}
Accept: application/dicom
```

Returns multipart/related response with all DICOM instances in the series.

**Status:** ✅ Implemented (Filesystem & S3)

### Retrieve Instance

Retrieve a single DICOM instance:

```
GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}
Accept: application/dicom
```

**Status:** ✅ Implemented

**Example:**

```javascript
const response = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9',
    { headers: { 'Accept': 'application/dicom' } }
);

const dicomBuffer = await response.arrayBuffer();
fs.writeFileSync('retrieved.dcm', Buffer.from(dicomBuffer));
```

### Retrieve Instance Metadata

Retrieve metadata for a specific instance as DICOM JSON:

```
GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/metadata
```

Returns DICOM JSON array with one element (the instance metadata).

**Status:** ✅ Implemented

**Example:**

```javascript
const response = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9/metadata'
);

const metadata = await response.json();
console.log(metadata[0]['0020000D']); // Study Instance UID
```

### Retrieve Study/Series Metadata

Retrieve metadata for all instances in a study or series:

```
GET /studies/{studyUID}/metadata
GET /studies/{studyUID}/series/{seriesUID}/metadata
```

Returns DICOM JSON array with metadata for all instances.

**Status:** Not yet implemented

### Retrieve Frames

Retrieve specific frames from a multi-frame instance:

```
GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/frames/{frameList}
Accept: application/octet-stream
```

Frame list can be:
- Single frame: `1`
- Multiple frames: `1,3,5`
- Range: `1-10`
- Mixed: `1,3-5,7`

Returns multipart/related response with raw pixel data for requested frames.

**Status:** ✅ Implemented (Filesystem & S3)

### Retrieve Rendered

Retrieve rendered pixel data:

```
GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/rendered
Accept: image/jpeg
```

Query parameters:
- `viewport` - Output dimensions (e.g., `512,512`)
- `quality` - JPEG quality 1-100
- `window` - Window center/width (e.g., `40,400`)

**Status:** ✅ Implemented

## Image Rendering and Processing

The WADO-RS server includes comprehensive image rendering capabilities with automatic VOI LUT (Value of Interest Lookup Table) support, windowing, and format conversion.

### Automatic VOI LUT Support

When retrieving rendered images without manual windowing parameters, the server automatically applies VOI LUT from the DICOM file:

```javascript
// Automatic VOI LUT from file metadata
const response = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9/rendered'
);

const imageBlob = await response.blob();
// Image rendered with WindowCenter/WindowWidth from DICOM file
```

**Automatic Processing:**
- Reads `WindowCenter` and `WindowWidth` from DICOM file
- Applies `RescaleIntercept` and `RescaleSlope` for Hounsfield units (CT)
- Converts to 8-bit display range (0-255)
- Returns optimally windowed image

### Manual Windowing

Override automatic VOI LUT with custom window parameters:

```javascript
// Soft tissue window (CT)
const softTissue = await fetch(
    'http://localhost:8043/studies/.../instances/.../rendered?window=40,400'
);

// Lung window (CT)
const lung = await fetch(
    'http://localhost:8043/studies/.../instances/.../rendered?window=-600,1500'
);

// Bone window (CT)
const bone = await fetch(
    'http://localhost:8043/studies/.../instances/.../rendered?window=300,1500'
);
```

**Common CT Windowing Presets:**
- **Soft Tissue**: `window=40,400` - Optimal for abdomen, pelvis
- **Lung**: `window=-600,1500` - Optimal for chest/lungs
- **Bone**: `window=300,1500` - Optimal for skeletal structures
- **Brain**: `window=40,80` - Optimal for head CT

### Viewport Transformation

Resize images while maintaining aspect ratio:

```javascript
// Resize to 512x512 viewport
const resized = await fetch(
    'http://localhost:8043/studies/.../instances/.../rendered?viewport=512,512'
);

// Combined: resize + custom windowing
const combined = await fetch(
    'http://localhost:8043/studies/.../instances/.../rendered?viewport=512,512&window=40,400'
);
```

**Viewport Features:**
- Maintains original aspect ratio
- High-quality Lanczos3 resampling
- Supports any output dimensions

### Image Quality Control

Control JPEG compression quality:

```javascript
// High quality (90)
const highQuality = await fetch(
    'http://localhost:8043/studies/.../instances/.../rendered?quality=90'
);

// Medium quality (75, smaller file)
const mediumQuality = await fetch(
    'http://localhost:8043/studies/.../instances/.../rendered?quality=75'
);
```

**Quality Guidelines:**
- `90-100`: Diagnostic quality, larger files
- `75-89`: Good quality, balanced size
- `60-74`: Lower quality, smaller files

### Format Support

Request different image formats via Accept header:

```javascript
// JPEG (default)
const jpeg = await fetch(url, {
    headers: { 'Accept': 'image/jpeg' }
});

// PNG (lossless)
const png = await fetch(url, {
    headers: { 'Accept': 'image/png' }
});
```

### Complete Rendering Example

```javascript
import { WadoServer } from 'node-dicom-rs';

const server = new WadoServer(8043, {
    storageType: 'Filesystem',
    basePath: './dicom-storage',
    enableRendered: true,
    verbose: true
});

server.start();

// Example 1: Automatic VOI LUT
const auto = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9/rendered'
);
await saveImage(auto, 'automatic-windowing.jpg');

// Example 2: CT soft tissue window with resizing
const ct = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9/rendered?window=40,400&viewport=512,512&quality=90'
);
await saveImage(ct, 'ct-soft-tissue.jpg');

// Example 3: PNG format with automatic windowing
const png = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9/rendered',
    { headers: { 'Accept': 'image/png' } }
);
await saveImage(png, 'lossless.png');

async function saveImage(response, filename) {
    const buffer = await response.arrayBuffer();
    fs.writeFileSync(filename, Buffer.from(buffer));
}
```

### Retrieve Thumbnail

Retrieve thumbnail image:

```
GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}/thumbnail
Accept: image/jpeg
```

**Status:** Not yet implemented

## Content Negotiation

The server supports content negotiation via the `Accept` header:

### DICOM Format (application/dicom)

Default format. Returns raw DICOM files.

```javascript
fetch(url, {
    headers: { 'Accept': 'application/dicom' }
});
```

### DICOM JSON (application/dicom+json)

Returns metadata as DICOM JSON:

```javascript
fetch(url, {
    headers: { 'Accept': 'application/dicom+json' }
});
```

**Example response:**

```json
{
  "00080005": {
    "vr": "CS",
    "Value": ["ISO_IR 100"]
  },
  "00080020": {
    "vr": "DA",
    "Value": ["20240101"]
  },
  "00100010": {
    "vr": "PN",
    "Value": [{"Alphabetic": "Doe^John"}]
  }
}
```

### DICOM XML (application/dicom+xml)

Returns metadata as DICOM XML.

**Status:** Not yet implemented

## Feature Flags

Control which endpoints are enabled:

```javascript
const config = {
    storageType: 'Filesystem',
    basePath: '/dicom/storage',
    
    // Enable/disable specific features
    enableMetadata: true,      // Metadata endpoints
    enableFrames: false,       // Frame retrieval
    enableRendered: false,     // Rendered images
    enableThumbnail: false,    // Thumbnail generation
    enableBulkdata: false,     // Bulkdata retrieval
};
```

Disabled endpoints return `404 Not Found`.

## Complete Example

```javascript
import { WadoServer } from 'node-dicom-rs';

// Configure server
const config = {
    storageType: 'Filesystem',
    basePath: './dicom-storage',
    enableMetadata: true,
    enableFrames: true,
    enableCors: true,
    verbose: true
};

// Start server
const server = new WadoServer(8043, config);
server.start();

// Retrieve DICOM instance
const response = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9',
    {
        headers: {
            'Accept': 'application/dicom'
        }
    }
);

if (response.ok) {
    const buffer = await response.arrayBuffer();
    console.log(`Retrieved DICOM file: ${buffer.byteLength} bytes`);
}

// Retrieve metadata as DICOM JSON
const metadataResponse = await fetch(
    'http://localhost:8043/studies/1.2.3/series/4.5.6/instances/7.8.9/metadata'
);

const metadata = await metadataResponse.json();
console.log('Metadata:', metadata);

// Stop server
server.stop();
```

## Error Handling

### 404 Not Found

Returned when:
- Resource does not exist
- Feature is disabled via configuration

```json
{
  "error": "Not found"
}
```

### 501 Not Implemented

Returned for endpoints not yet implemented:

```json
{
  "error": "Study retrieval not yet implemented",
  "studyUID": "1.2.3",
  "note": "This requires scanning storage for all series/instances in the study"
}
```

### 400 Bad Request

Returned for invalid requests or configuration errors.

## Performance Considerations

### Thread Pool

WADO-RS file I/O uses the Node.js libuv thread pool (default: 4 threads). For high-throughput scenarios, increase the thread pool:

```bash
UV_THREADPOOL_SIZE=16 node server.js
```

### Storage Backend

- **Filesystem:** Fast for local storage, simple deployment
- **S3:** Scales horizontally, suitable for cloud deployments

### Caching

Consider implementing caching at the application level for frequently accessed instances.

### Load Balancing

WADO-RS is stateless and can be load balanced across multiple instances.

## Integration with Nuxt/Nitro

The WADO-RS server runs on a separate port but can be integrated with Nuxt/Nitro:

```javascript
// server/api/dicom/wado.ts
import { WadoServer } from 'node-dicom-rs';

let wadoServer: WadoServer | null = null;

export default defineEventHandler(async (event) => {
    // Start WADO server on first request
    if (!wadoServer) {
        wadoServer = new WadoServer(8043, {
            storageType: 'Filesystem',
            basePath: './storage/dicom'
        });
        wadoServer.start();
    }
    
    // Proxy requests to WADO server or return info
    return {
        wadoUrl: 'http://localhost:8043',
        status: 'running'
    };
});
```

## Architecture

The WADO-RS server architecture:

```
┌─────────────────┐
│   Node.js App   │
│  (Nuxt/Nitro)   │
└────────┬────────┘
         │
         │ Start/Stop
         ▼
┌─────────────────┐
│  WADO-RS Server │
│   (Rust/Warp)   │
└────────┬────────┘
         │
         ├─────────────┐
         │             │
         ▼             ▼
┌─────────────┐  ┌──────────┐
│ Filesystem  │  │    S3    │
│   Storage   │  │ Storage  │
└─────────────┘  └──────────┘
```

**Key Points:**
- Warp HTTP server runs on Tokio thread pool (separate from Node.js event loop)
- File I/O uses libuv thread pool (shared with Node.js)
- DICOM parsing happens in Rust (high performance)
- Minimal overhead for serving files

## Roadmap

### Implemented ✅

- [x] Basic instance retrieval - GET /studies/{uid}/series/{uid}/instances/{uid}
- [x] Instance metadata retrieval - GET .../instances/{uid}/metadata
- [x] Study metadata retrieval - GET /studies/{uid}/metadata
- [x] Series metadata retrieval - GET /studies/{uid}/series/{uid}/metadata
- [x] Content negotiation (Accept header handling)
- [x] DICOM format (application/dicom)
- [x] DICOM JSON format (application/dicom+json)
- [x] Filesystem storage backend with hierarchical file organization
- [x] S3 storage backend (S3-compatible object storage)
- [x] Frame extraction and retrieval - GET .../instances/{uid}/frames/{frameList}
- [x] Rendered endpoint with windowing - GET .../instances/{uid}/rendered
- [x] Thumbnail endpoint - GET .../instances/{uid}/thumbnail
- [x] Image rendering with automatic VOI LUT support
- [x] Custom windowing parameters (window center/width)
- [x] Viewport transformation (resizing with aspect ratio)
- [x] Quality control for JPEG compression
- [x] Bulkdata retrieval - GET .../instances/{uid}/bulkdata/{attributePath}
- [x] Query parameter support (viewport, quality, window)
- [x] Feature flags for endpoint control
- [x] CORS support with configurable origins

### Planned 🔄

- [ ] Study/series retrieval with multipart responses (scans entire storage)
- [ ] Pixel data transcoding (JPEG/JPEG2000/PNG conversion)
- [ ] Compression support (gzip for responses)
- [ ] DICOM XML format (application/dicom+xml)
- [ ] ETag/caching support
- [ ] Proper HTTP caching headers
- [ ] Streaming responses for large files
- [ ] Authentication and authorization

## References

- [DICOM Part 18 - Web Services](http://dicom.nema.org/medical/dicom/current/output/html/part18.html)
- [DICOMweb Standard](https://www.dicomstandard.org/using/dicomweb)
- [WADO-RS Specification](http://dicom.nema.org/medical/dicom/current/output/html/part18.html#sect_10.4)

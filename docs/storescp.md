# StoreScp - DICOM C-STORE SCP Server

The `StoreScp` class implements a DICOM C-STORE Service Class Provider (SCP) server that receives DICOM files over the network.

## Basic Usage

```typescript
import { StoreScp } from '@nuxthealth/node-dicom';

const receiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'MY-SCP',
    outDir: './dicom-storage',
    verbose: true
});

receiver.onFileStored((err, event) => {
    if (err) {
        console.error('Error:', err);
        return;
    }
    console.log('File received:', event.data?.file);
});

receiver.start();
```

## Configuration Options

The `StoreScp` constructor accepts a configuration object with the following options:

### Required Options

#### port

**Type:** `number` (required)

The TCP port number on which the SCP server will listen for incoming DICOM connections.

```typescript
// Standard DICOM port
port: 104

// Common alternative ports
port: 4446
port: 11112
port: 8042  // Orthanc default
```

**Default Port:** DICOM standard is `104`, but requires root/admin privileges on Linux/macOS. Most development and production deployments use ports above 1024.

**Common Issues:**
- **Permission denied**: Port 104 requires elevated privileges (`sudo` on Linux/macOS)
- **Port already in use**: Another service is using the port
- **Firewall blocking**: Ensure firewall allows incoming connections

**Guidelines:**
- Development: Use `4446` or `11112`
- Production: Use `104` (with proper permissions) or your organization's standard
- Docker: Map container port to host (e.g., `11112:104`)

### Optional Options

#### callingAeTitle

**Type:** `string` (optional)  
**Default:** `'STORE-SCP'`

Your Application Entity (AE) Title - identifies this SCP server to remote SCU clients.

```typescript
callingAeTitle: 'HOSPITAL-PACS'
callingAeTitle: 'RESEARCH-SCP'
callingAeTitle: 'ARCHIVE-01'
```

**Constraints:**
- Maximum 16 characters
- Usually uppercase
- No spaces (use hyphens or underscores)
- Should be descriptive of the server's purpose

**Use Cases:**
- Remote SCU clients may require specific AE titles for routing
- Helps with logging and identifying which server received data
- Some sending systems validate AE titles before transmitting

**Important:** This is YOUR AE title (the server), not the remote client's.

#### outDir

**Type:** `string` (optional, required for Filesystem storage)  
**Default:** None

The local filesystem directory where received DICOM files will be stored.

```typescript
// Relative path
outDir: './dicom-storage'

// Absolute path
outDir: '/var/dicom/archive'

// Per-environment
outDir: process.env.DICOM_STORAGE_PATH || './dicom-storage'
```

**Directory Structure:**

Files are automatically organized in a hierarchy:
```
{outDir}/
  {StudyInstanceUID}/
    {SeriesInstanceUID}/
      {SOPInstanceUID}.dcm
```

Example:
```
./dicom-storage/
  1.2.840.113619.2.55.3.../
    1.2.840.113619.2.55.3.../
      1.2.840.113619.2.55.3....dcm
      1.2.840.113619.2.55.3....dcm
```

**Important Notes:**
- Directory will be created automatically if it doesn't exist
- Ensure sufficient disk space for expected volume
- Consider mount points for large archives
- Required when `storageBackend: 'Filesystem'`
- Ignored when using S3 storage

#### storageBackend

**Type:** `'Filesystem' | 'S3'` (optional)  
**Default:** `'Filesystem'`

The storage backend to use for saving received DICOM files.

```typescript
// Local filesystem storage
storageBackend: 'Filesystem'

// S3-compatible object storage
storageBackend: 'S3'
```

**Filesystem Storage:**
- ✅ Simple setup, no dependencies
- ✅ Fast for local development
- ✅ Good for small to medium volumes
- ❌ Limited scalability
- ❌ Requires disk space management
- ❌ No built-in redundancy

**S3 Storage:**
- ✅ Highly scalable
- ✅ Built-in redundancy
- ✅ Works with AWS S3, MinIO, etc.
- ✅ No local disk space concerns
- ❌ Requires S3 configuration
- ❌ Network latency for storage operations
- ❌ Additional costs (cloud storage)

**When to Use:**
- **Development**: Filesystem
- **Production (small scale)**: Filesystem with proper backup
- **Production (large scale)**: S3 for unlimited scalability
- **Multi-server deployment**: S3 for shared storage

#### s3Config

**Type:** `object` (required when `storageBackend: 'S3'`)

S3 storage configuration. Required when using S3 storage backend.

```typescript
s3Config: {
    bucket: string,       // S3 bucket name (required)
    accessKey: string,    // AWS access key ID (required)
    secretKey: string,    // AWS secret access key (required)
    endpoint: string,     // S3 endpoint URL (required)
    region?: string       // AWS region (optional)
}
```

**AWS S3 Example:**
```typescript
s3Config: {
    bucket: 'hospital-dicom-archive',
    accessKey: process.env.AWS_ACCESS_KEY_ID,
    secretKey: process.env.AWS_SECRET_ACCESS_KEY,
    endpoint: 'https://s3.amazonaws.com',
    region: 'us-east-1'
}
```

**MinIO Example:**
```typescript
s3Config: {
    bucket: 'dicom',
    accessKey: 'minioadmin',
    secretKey: 'minioadmin',
    endpoint: 'http://localhost:9000',
    region: 'us-east-1'  // MinIO requires a region, any value works
}
```

**DigitalOcean Spaces Example:**
```typescript
s3Config: {
    bucket: 'my-dicom-space',
    accessKey: process.env.DO_SPACES_KEY,
    secretKey: process.env.DO_SPACES_SECRET,
    endpoint: 'https://nyc3.digitaloceanspaces.com',
    region: 'nyc3'
}
```

**Security Best Practices:**
- Never hardcode credentials in source code
- Use environment variables or secret management
- Use IAM roles when running on AWS EC2
- Restrict S3 bucket access with appropriate policies
- Enable bucket encryption at rest

#### storeWithFileMeta

**Type:** `boolean` (optional)  
**Default:** `true`

Whether to include DICOM File Meta Information header when storing files.

```typescript
// With file meta (default) - complete DICOM file
storeWithFileMeta: true

// Without file meta - just dataset (advanced use case)
storeWithFileMeta: false
```

**With File Meta (true - default):**
- ✅ Stores complete DICOM Part-10 compliant file
- ✅ Includes preamble, "DICM" marker, and meta header
- ✅ Contains Transfer Syntax UID, SOP Class UID, etc.
- ✅ Compatible with DICOM viewers and standard tools
- ✅ Can be reopened with `DicomFile.open()`
- ✅ Ready for archival storage
- **Recommended for most use cases**

**Without File Meta (false):**
- Stores only the DICOM dataset (no preamble or meta)
- Smaller file size (~150-200 bytes less per file)
- Only suitable for specialized PACS internal storage
- **Not compatible** with standard DICOM viewers
- **Not compatible** with most DICOM tools
- Use only if you have specific requirements for dataset-only format

**When to Use `false`:**
- Custom PACS system that prefers dataset-only format
- Temporary storage for immediate network retransmission
- Storage space is extremely critical
- You understand DICOM file structure and have specific needs

**When to Use `true` (recommended):**
- General archival storage
- Files will be viewed with DICOM viewers
- Files will be reopened or processed later
- Standard DICOM file format compliance is needed
- **Use this unless you have a specific reason not to**

#### strict

**Type:** `boolean` (optional)  
**Default:** `false`

Enforce strict DICOM protocol compliance for PDU length limits.

```typescript
// Relaxed mode (default) - accept larger PDUs
strict: false

// Strict mode - enforce maximum PDU length
strict: true
```

**Relaxed Mode (false, default):**
- Accepts PDUs slightly over the negotiated maximum
- More compatible with non-compliant implementations
- Recommended for interoperability

**Strict Mode (true):**
- Rejects PDUs that exceed negotiated maximum
- Strictly follows DICOM standard
- May cause connection failures with non-compliant senders

**When to Enable:**
- Testing DICOM standard compliance
- Security-critical environments
- When specifically requested

**When to Disable (default):**
- Maximum compatibility
- Working with diverse DICOM implementations
- Production environments

#### maxPduLength

**Type:** `number` (optional)  
**Default:** `16384` (16 KB)

Maximum Protocol Data Unit (PDU) size in bytes that the server will accept.

```typescript
// Small PDU (default)
maxPduLength: 16384    // 16 KB

// Medium PDU
maxPduLength: 32768    // 32 KB

// Large PDU
maxPduLength: 65536    // 64 KB

// Maximum PDU
maxPduLength: 131072   // 128 KB
```

**Range:** Typically `16384` to `131072` bytes

**Guidelines:**
- **Default (16 KB)**: Works with all implementations
- **32 KB**: Good balance for most networks
- **64-128 KB**: Fast networks, high-performance needs
- Larger PDU = faster transfers (if network supports it)
- Must be compatible with sending SCU

**Performance Impact:**

Larger PDU can improve throughput:
```typescript
// Receiving 1000 files from same sender
maxPduLength: 16384   // Baseline
maxPduLength: 32768   // ~30% faster
maxPduLength: 65536   // ~45% faster (on fast LAN)
```

**Important Notes:**
- Remote SCU must support the PDU size
- Some older implementations only support 16 KB
- Network must handle larger packets without fragmentation

#### verbose

**Type:** `boolean` (optional)  
**Default:** `false`

Enable detailed logging of DICOM protocol operations and server activity.

```typescript
// Production: minimal logging
verbose: false

// Development/debugging: detailed logging
verbose: true
```

**When enabled, logs include:**
- Server startup and listening status
- Incoming association requests
- AE title validation
- Presentation context negotiation
- Transfer syntax acceptance/rejection
- File reception progress
- Storage operations
- Study completion events
- Error details and stack traces

**Example output:**
```
[StoreScp] Server listening on 0.0.0.0:4446
[StoreScp] Association request from 192.168.1.50
[StoreScp] Calling AE: SENDER-SCU, Called AE: HOSPITAL-PACS
[StoreScp] Presentation context: CT Image Storage - Accepted
[StoreScp] Transfer syntax: Implicit VR Little Endian - Accepted
[StoreScp] Receiving file: 1.2.840.113619...
[StoreScp] Stored: ./dicom-storage/1.2.840.../1.2.840.../1.2.840....dcm
```

**Use Cases:**
- Development and testing
- Troubleshooting connection issues
- Understanding why files are rejected
- Debugging storage backend issues
- Monitoring association activity

**Note:** Verbose output may contain sensitive information (AE titles, patient data in tags). Don't enable in production unless necessary for troubleshooting.

#### extractTags

**Type:** `string[]` (optional)  
**Default:** `[]` (no tags extracted)

List of DICOM tag names to extract from received files and include in events.

```typescript
extractTags: [
    // Patient tags
    'PatientName',
    'PatientID',
    'PatientBirthDate',
    'PatientSex',
    
    // Study tags
    'StudyDate',
    'StudyTime',
    'StudyDescription',
    'AccessionNumber',
    
    // Series tags
    'Modality',
    'SeriesNumber',
    'SeriesDescription',
    'BodyPartExamined',
    
    // Instance tags
    'InstanceNumber',
    'SliceLocation',
    'SliceThickness',
    
    // Equipment tags
    'Manufacturer',
    'ManufacturerModelName',
    'SoftwareVersions'
]
```

**Available Tag Names:**

Use standard DICOM tag names from the helper:
```typescript
import { getAvailableTagNames } from '@nuxthealth/node-dicom';

const tagNames = getAvailableTagNames();
console.log(tagNames);  // Lists all ~500 available tags
```

**Common Tag Categories:**
- **Patient**: PatientName, PatientID, PatientBirthDate, PatientSex, PatientAge
- **Study**: StudyDate, StudyTime, StudyDescription, AccessionNumber, StudyID
- **Series**: Modality, SeriesNumber, SeriesDescription, BodyPartExamined
- **Instance**: InstanceNumber, SliceLocation, ImagePositionPatient, WindowCenter
- **Equipment**: Manufacturer, ManufacturerModelName, DeviceSerialNumber

**Performance Considerations:**
- Each tag requires parsing and extraction
- More tags = slightly slower processing
- Extract only tags you actually need
- For archival without processing, leave empty

**Extracted tags appear in events:**
```typescript
receiver.onFileStored((err, event) => {
    console.log(event.data?.tags?.PatientName);
    console.log(event.data?.tags?.StudyDate);
});
```

#### extractCustomTags

**Type:** `CustomTag[]` (optional)  
**Default:** `[]` (no custom tags)

List of private or vendor-specific DICOM tags to extract with user-defined names.

```typescript
import { createCustomTag } from '@nuxthealth/node-dicom';

extractCustomTags: [
    createCustomTag('00091001', 'VendorField1'),
    createCustomTag('00091002', 'VendorField2'),
    { tag: '00191010', name: 'PrivateTag1' }  // Or inline
]
```

**Tag Format:**
- Tag group and element as 8-character hex string
- Example: `'00091001'` = Group 0009, Element 1001
- Must be valid DICOM tag hex format

**Use Cases:**
- Extracting proprietary vendor tags (e.g., Siemens, GE private tags)
- Custom institutional tags
- Research protocol-specific tags
- Non-standard extensions

**Example with vendor tags:**
```typescript
extractCustomTags: [
    // GE private tags
    createCustomTag('00091001', 'GE_PrivateCreator'),
    createCustomTag('00091002', 'GE_SeriesType'),
    
    // Siemens private tags
    createCustomTag('00191010', 'Siemens_Protocol'),
    createCustomTag('00291010', 'Siemens_Technique')
]
```

**Extracted custom tags appear in events:**
```typescript
receiver.onFileStored((err, event) => {
    console.log(event.data?.tags?.VendorField1);
    console.log(event.data?.tags?.GE_PrivateCreator);
});
```

**Important Notes:**
- Tag names must be unique
- Invalid tags are silently ignored
- Check vendor documentation for private tag meanings
- Not all files will have private tags

#### studyTimeout

**Type:** `number` (optional)  
**Default:** `30` (seconds)

Number of seconds to wait after the last file of a study before triggering the `OnStudyCompleted` event.

```typescript
// Quick timeout for testing
studyTimeout: 10

// Default timeout
studyTimeout: 30

// Long timeout for slow senders
studyTimeout: 60

// Very long timeout for batch uploads
studyTimeout: 300  // 5 minutes
```

**How It Works:**

1. First file of study arrives → timer starts
2. Each new file in the study → timer resets
3. No new files for `studyTimeout` seconds → `OnStudyCompleted` fires

**Guidelines:**
- **Fast modalities (CR, DR)**: 10-30 seconds
- **CT/MR with few slices**: 30-60 seconds
- **CT/MR with many slices**: 60-120 seconds
- **Batch/delayed uploads**: 120-300 seconds
- **High latency networks**: Increase timeout

**Factors to Consider:**
- Network speed between sender and receiver
- Number of instances in typical studies
- Sending system behavior (sequential vs concurrent)
- Whether studies are sent in batches

**Example Scenarios:**
```typescript
// Emergency radiology - fast turnaround needed
studyTimeout: 15

// General radiology - balanced
studyTimeout: 30

// Research/batch processing - can wait
studyTimeout: 120
```

**Important Notes:**
- Timeout is per study (tracked by StudyInstanceUID)
- Multiple studies can have independent timers
- Very short timeouts may cause premature triggers
- Very long timeouts delay downstream processing

**Important Notes:**
- Timeout is per study (tracked by StudyInstanceUID)
- Multiple studies can have independent timers
- Very short timeouts may cause premature triggers
- Very long timeouts delay downstream processing

#### abstractSyntaxMode

**Type:** `'AllStorage' | 'All' | 'Custom'` (optional)  
**Default:** `'AllStorage'`

Controls which SOP Class types (DICOM object types) the server will accept.

```typescript
// Accept all Storage SOP Classes (~200 classes)
abstractSyntaxMode: 'AllStorage'

// Accept ALL SOP Classes including non-storage (Verification, Query/Retrieve, etc.)
abstractSyntaxMode: 'All'

// Accept only specific SOP Classes (defined in abstractSyntaxes)
abstractSyntaxMode: 'Custom'
```

**AllStorage Mode:**
- ✅ Accepts all imaging and storage SOP classes
- ✅ Works with any modality (CT, MR, US, etc.)
- ✅ Includes specialized storage (SR, RT, etc.)
- ✅ Recommended for general-purpose PACS
- ❌ Cannot filter by specific modality

**All Mode:**
- ✅ Accepts everything (storage + non-storage)
- ✅ Maximum compatibility
- ❌ May accept unexpected SOP classes
- ❌ Rarely needed for storage servers

**Custom Mode:**
- ✅ Fine-grained control over accepted types
- ✅ Can limit to specific modalities only
- ✅ Better security and validation
- ⚠️ Requires defining `abstractSyntaxes` list

**When to Use Each:**

```typescript
// General PACS - accept everything
abstractSyntaxMode: 'AllStorage'

// Specialized archive - only CT and MR
abstractSyntaxMode: 'Custom'
abstractSyntaxes: [...sopClasses.ct, ...sopClasses.mr]

// Modality-specific - only ultrasound
abstractSyntaxMode: 'Custom'
abstractSyntaxes: sopClasses.ultrasound

// Research project - only specific SOP classes
abstractSyntaxMode: 'Custom'
abstractSyntaxes: ['1.2.840.10008.5.1.4.1.1.2']  // CT Image Storage
```

#### abstractSyntaxes

**Type:** `string[]` (required when `abstractSyntaxMode: 'Custom'`)

List of SOP Class UIDs to accept when in Custom mode.

```typescript
import { getCommonSopClasses } from '@nuxthealth/node-dicom';

const sopClasses = getCommonSopClasses();

// Accept only CT and MR
abstractSyntaxes: [
    ...sopClasses.ct,
    ...sopClasses.mr
]

// Accept all imaging
abstractSyntaxes: sopClasses.allImaging

// Accept specific classes by UID
abstractSyntaxes: [
    '1.2.840.10008.5.1.4.1.1.2',    // CT Image Storage
    '1.2.840.10008.5.1.4.1.1.4',    // MR Image Storage
    '1.2.840.10008.5.1.4.1.1.6.1'   // Ultrasound Image Storage
]
```

**Available SOP Class Helper Categories:**

```typescript
const sopClasses = getCommonSopClasses();

// Imaging modalities
sopClasses.ct              // CT (2 classes)
sopClasses.mr              // MR (2 classes)
sopClasses.ultrasound      // US (2 classes)
sopClasses.pet             // PET (3 classes)
sopClasses.xray            // X-Ray (3 classes)
sopClasses.mammography     // Mammography (5 classes)

// Specialized
sopClasses.secondaryCapture     // Screen captures (4 classes)
sopClasses.radiationTherapy     // RT planning (4 classes)
sopClasses.documents            // Encapsulated PDFs, etc. (4 classes)
sopClasses.structuredReports    // SR documents (3 classes)

// Combinations
sopClasses.allImaging      // All imaging (17 classes)
sopClasses.all             // Everything (33 classes)
```

**Common Patterns:**

```typescript
// Radiology department - all imaging
abstractSyntaxes: sopClasses.allImaging

// CT scanner - CT only
abstractSyntaxes: sopClasses.ct

// Multi-modality - CT, MR, US
abstractSyntaxes: [
    ...sopClasses.ct,
    ...sopClasses.mr,
    ...sopClasses.ultrasound
]

// Oncology - PET/CT with RT planning
abstractSyntaxes: [
    ...sopClasses.pet,
    ...sopClasses.ct,
    ...sopClasses.radiationTherapy
]

// Breast imaging - mammography + US
abstractSyntaxes: [
    ...sopClasses.mammography,
    ...sopClasses.ultrasound
]
```

**SOP Class Rejection:**

If a sender tries to send an unsupported SOP class:
- Association is established
- Presentation context is rejected
- Sender receives rejection status
- Connection remains open for supported SOP classes

**Finding SOP Class UIDs:**

```typescript
// List all available classes
const sopClasses = getCommonSopClasses();
console.log(JSON.stringify(sopClasses, null, 2));

// Or look up in DICOM standard
// Part 4: Service-Object Pair (SOP) Class Definitions
```

#### transferSyntaxMode

**Type:** `'All' | 'UncompressedOnly' | 'Custom'` (optional)  
**Default:** `'All'`

Controls which Transfer Syntaxes (compression/encoding formats) the server will accept.

```typescript
// Accept all transfer syntaxes (compressed + uncompressed)
transferSyntaxMode: 'All'

// Accept only uncompressed (Implicit/Explicit VR Little Endian)
transferSyntaxMode: 'UncompressedOnly'

// Accept specific transfer syntaxes (defined in transferSyntaxes)
transferSyntaxMode: 'Custom'
```

**All Mode (default):**
- ✅ Maximum compatibility
- ✅ Accepts compressed and uncompressed
- ✅ No transcoding needed
- ✅ Recommended for general use
- ❌ May receive unexpected encodings

**UncompressedOnly Mode:**
- ✅ Only accepts standard uncompressed formats
- ✅ Fastest processing (no decompression)
- ✅ Consistent format for storage
- ❌ Sender must transcode if needed
- ❌ May reject valid compressed images

**Custom Mode:**
- ✅ Fine-grained control over accepted formats
- ✅ Can limit to specific compression types
- ✅ Better for specialized workflows
- ⚠️ Requires defining `transferSyntaxes` list

**When to Use Each:**

```typescript
// General PACS - accept everything
transferSyntaxMode: 'All'

// Fast processing - uncompressed only
transferSyntaxMode: 'UncompressedOnly'

// Specific formats - uncompressed + JPEG 2000
transferSyntaxMode: 'Custom'
transferSyntaxes: [
    ...transferSyntaxes.uncompressed,
    ...transferSyntaxes.jpeg2000
]
```

#### transferSyntaxes

**Type:** `string[]` (required when `transferSyntaxMode: 'Custom'`)

List of Transfer Syntax UIDs to accept when in Custom mode.

```typescript
import { getCommonTransferSyntaxes } from '@nuxthealth/node-dicom';

const transferSyntaxes = getCommonTransferSyntaxes();

// Accept uncompressed + JPEG
transferSyntaxes: [
    ...transferSyntaxes.uncompressed,
    ...transferSyntaxes.jpeg
]

// Accept all compressed formats
transferSyntaxes: transferSyntaxes.allCompressed

// Accept specific syntaxes by UID
transferSyntaxes: [
    '1.2.840.10008.1.2',      // Implicit VR Little Endian
    '1.2.840.10008.1.2.1',    // Explicit VR Little Endian
    '1.2.840.10008.1.2.4.90'  // JPEG 2000 Lossless
]
```

**Available Transfer Syntax Helper Categories:**

```typescript
const transferSyntaxes = getCommonTransferSyntaxes();

// Uncompressed
transferSyntaxes.uncompressed  // Implicit/Explicit VR (3 syntaxes)

// Compressed formats
transferSyntaxes.jpeg          // JPEG variants (4 syntaxes)
transferSyntaxes.jpegLs        // JPEG-LS (2 syntaxes)
transferSyntaxes.jpeg2000      // JPEG 2000 (2 syntaxes)
transferSyntaxes.rle           // RLE Lossless (1 syntax)
transferSyntaxes.mpeg          // MPEG video (4 syntaxes)

// Combinations
transferSyntaxes.allCompressed // All compressed (13 syntaxes)
transferSyntaxes.all           // Everything (17 syntaxes)
```

**Common Transfer Syntaxes Table:**

| Name | UID | Description | Use Case |
|------|-----|-------------|----------|
| Implicit VR Little Endian | 1.2.840.10008.1.2 | Uncompressed, implicit | Default, widest compatibility |
| Explicit VR Little Endian | 1.2.840.10008.1.2.1 | Uncompressed, explicit | Standard uncompressed |
| Deflated Explicit VR | 1.2.840.10008.1.2.1.99 | ZIP compression | Lossless compression |
| JPEG Baseline | 1.2.840.10008.1.2.4.50 | JPEG lossy 8-bit | Web/preview |
| JPEG Lossless | 1.2.840.10008.1.2.4.70 | JPEG lossless | Diagnostic quality |
| JPEG 2000 Lossless | 1.2.840.10008.1.2.4.90 | JP2 lossless | High quality |
| JPEG 2000 Lossy | 1.2.840.10008.1.2.4.91 | JP2 lossy | Compressed archives |
| RLE Lossless | 1.2.840.10008.1.2.5 | RLE compression | Medical images |

**Common Patterns:**

```typescript
// Accept all formats (default)
transferSyntaxMode: 'All'

// Only uncompressed for fast processing
transferSyntaxMode: 'UncompressedOnly'

// Uncompressed + lossless compression
transferSyntaxes: [
    ...transferSyntaxes.uncompressed,
    '1.2.840.10008.1.2.4.70',  // JPEG Lossless
    '1.2.840.10008.1.2.4.90'   // JPEG 2000 Lossless
]

// Web-friendly formats
transferSyntaxes: [
    ...transferSyntaxes.uncompressed,
    ...transferSyntaxes.jpeg
]
```

**Transfer Syntax Rejection:**

If a sender tries to use an unsupported transfer syntax:
- Association is established
- Presentation context is rejected
- Sender may retry with different transfer syntax (if capable)
- Some senders will transcode automatically

**Performance Considerations:**
- Uncompressed = fastest processing
- Compressed = smaller storage, slower processing
- Automatic decompression happens transparently
- Some compressed formats require additional libraries

**Performance Considerations:**
- Uncompressed = fastest processing
- Compressed = smaller storage, slower processing
- Automatic decompression happens transparently
- Some compressed formats require additional libraries

### Complete Configuration Examples

```typescript
import { StoreScp, getCommonSopClasses, getCommonTransferSyntaxes } from '@nuxthealth/node-dicom';

const sopClasses = getCommonSopClasses();
const transferSyntaxes = getCommonTransferSyntaxes();

// Minimal configuration (development)
const minimalReceiver = new StoreScp({
    port: 4446,
    outDir: './dicom-storage'
});

// Standard configuration (production)
const standardReceiver = new StoreScp({
    port: 11112,
    callingAeTitle: 'HOSPITAL-PACS',
    outDir: '/var/dicom/archive',
    storageBackend: 'Filesystem',
    verbose: false,
    extractTags: [
        'PatientName', 'PatientID',
        'StudyDate', 'StudyDescription',
        'Modality', 'SeriesDescription'
    ],
    studyTimeout: 30
});

// High-performance configuration (S3 storage)
const s3Receiver = new StoreScp({
    port: 104,
    callingAeTitle: 'CLOUD-PACS',
    storageBackend: 'S3',
    s3Config: {
        bucket: process.env.S3_BUCKET,
        accessKey: process.env.AWS_ACCESS_KEY_ID,
        secretKey: process.env.AWS_SECRET_ACCESS_KEY,
        endpoint: 'https://s3.amazonaws.com',
        region: 'us-east-1'
    },
    maxPduLength: 65536,
    extractTags: [
        'PatientName', 'PatientID', 'PatientBirthDate',
        'StudyDate', 'StudyDescription', 'AccessionNumber',
        'Modality', 'SeriesDescription', 'SeriesNumber',
        'InstanceNumber'
    ],
    studyTimeout: 60,
    verbose: false
});

// Modality-specific (CT/MR only)
const ctMrReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'CT-MR-SCP',
    outDir: './dicom-storage',
    abstractSyntaxMode: 'Custom',
    abstractSyntaxes: [
        ...sopClasses.ct,
        ...sopClasses.mr
    ],
    transferSyntaxMode: 'All',
    extractTags: [
        'PatientName', 'PatientID',
        'StudyDate', 'StudyDescription',
        'Modality', 'SeriesDescription',
        'SliceThickness', 'KVP', 'Exposure'
    ],
    studyTimeout: 45
});

// Uncompressed-only (fast processing)
const uncompressedReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'FAST-SCP',
    outDir: './dicom-storage',
    transferSyntaxMode: 'UncompressedOnly',
    maxPduLength: 65536,
    extractTags: ['PatientName', 'PatientID', 'StudyDate', 'Modality'],
    studyTimeout: 20,
    verbose: false
});

// Custom transfer syntaxes (uncompressed + JPEG 2000)
const customTransferReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'CUSTOM-SCP',
    outDir: './dicom-storage',
    transferSyntaxMode: 'Custom',
    transferSyntaxes: [
        ...transferSyntaxes.uncompressed,
        ...transferSyntaxes.jpeg2000
    ],
    extractTags: ['PatientName', 'StudyDate', 'Modality']
});

// Research configuration (with custom tags)
const researchReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'RESEARCH-SCP',
    outDir: './research-data',
    extractTags: [
        'PatientID', 'StudyDate', 'Modality',
        'SeriesDescription', 'InstanceNumber'
    ],
    extractCustomTags: [
        { tag: '00091001', name: 'ProtocolName' },
        { tag: '00191010', name: 'SequenceVariant' }
    ],
    abstractSyntaxMode: 'Custom',
    abstractSyntaxes: sopClasses.allImaging,
    studyTimeout: 90,
    verbose: true
});

// Debugging configuration
const debugReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'DEBUG-SCP',
    outDir: './debug-storage',
    storeWithFileMeta: true,  // Preserve exact file structure
    verbose: true,             // See all protocol details
    strict: false,             // Relaxed for compatibility
    maxPduLength: 16384,
    extractTags: ['PatientName', 'PatientID', 'StudyDate', 'Modality'],
    studyTimeout: 60
});
```

### Configuration Best Practices

1. **Start simple, add complexity as needed**
   ```typescript
   // Good starting point
   const receiver = new StoreScp({
       port: 4446,
       outDir: './dicom-storage'
   });
   ```

2. **Extract only needed tags**
   ```typescript
   // Don't extract everything
   extractTags: ['PatientName', 'StudyDate', 'Modality']  // Only what you need
   ```

3. **Use environment variables for sensitive data**
   ```typescript
   s3Config: {
       bucket: process.env.S3_BUCKET,
       accessKey: process.env.AWS_ACCESS_KEY_ID,
       secretKey: process.env.AWS_SECRET_ACCESS_KEY,
       endpoint: process.env.S3_ENDPOINT
   }
   ```

4. **Set appropriate timeouts**
   ```typescript
   // Fast modalities (CR, DR)
   studyTimeout: 15
   
   // Standard cross-sectional (CT, MR)
   studyTimeout: 30
   
   // Large studies or slow networks
   studyTimeout: 60
   ```

5. **Use S3 for production scale**
   ```typescript
   // Development
   storageBackend: 'Filesystem'
   
   // Production with high volume
   storageBackend: 'S3'
   ```

6. **Enable verbose only when needed**
   ```typescript
   // Production
   verbose: false
   
   // Development/troubleshooting
   verbose: true
   ```

7. **Restrict SOP classes for specialized systems**
   ```typescript
   // General PACS - accept all
   abstractSyntaxMode: 'AllStorage'
   
   // Specialized (e.g., CT archive)
   abstractSyntaxMode: 'Custom'
   abstractSyntaxes: sopClasses.ct
   ```

8. **Consider storage format needs**
   ```typescript
   // Standard archival (default, recommended)
   storeWithFileMeta: true
   
   // Dataset-only for specialized PACS storage
   storeWithFileMeta: false
   ```

### Troubleshooting Configuration Issues

**Server Won't Start**
```typescript
// Check: Port permissions (104 requires sudo)
port: 4446  // Use port > 1024

// Check: Port already in use
// Run: netstat -an | grep 4446
```

**Files Not Stored**
```typescript
// Check: outDir exists and is writable
outDir: './dicom-storage'  // Verify permissions

// Check: Disk space available
// Run: df -h
```

**Connection Rejected**
```typescript
// Check: Calling AE title matches sender expectations
callingAeTitle: 'EXPECTED-NAME'

// Enable verbose to see rejection reason
verbose: true
```

**Wrong SOP Classes Accepted/Rejected**
```typescript
// Verify mode and syntaxes
abstractSyntaxMode: 'Custom'
abstractSyntaxes: [...sopClasses.ct]  // Check this matches intent

// Enable verbose to see negotiation
verbose: true
```

**Study Completion Not Firing**
```typescript
// Increase timeout if studies are large
studyTimeout: 60  // or higher

// Check: Are files from same StudyInstanceUID?
// Enable verbose to see study grouping
verbose: true
```

**S3 Storage Failing**
```typescript
// Verify credentials
s3Config: {
    bucket: 'correct-bucket-name',
    accessKey: process.env.AWS_ACCESS_KEY_ID,  // Check env vars set
    secretKey: process.env.AWS_SECRET_ACCESS_KEY,
    endpoint: 'https://s3.amazonaws.com',  // Correct endpoint
    region: 'us-east-1'  // Correct region
}

// Test S3 access separately
// Enable verbose to see S3 errors
verbose: true
```

**Tags Not Extracted**
```typescript
// Check: Tag names are correct
extractTags: ['PatientName']  // Not 'patientname'

// Check: Tags exist in files
// Some tags are optional in DICOM

// Use getAvailableTagNames() to see valid names
import { getAvailableTagNames } from '@nuxthealth/node-dicom';
console.log(getAvailableTagNames());
```

## Helper Functions

node-dicom-rs provides helper functions to simplify common configuration tasks.

### getCommonTagSets()

Get predefined sets of commonly used DICOM tags organized by category.

**Available Sets:**
- `patientBasic` - Essential patient demographics (7 tags)
- `studyBasic` - Study-level metadata (7 tags)
- `seriesBasic` - Series-level metadata (8 tags)
- `instanceBasic` - Instance identifiers (5 tags)
- `imagePixelInfo` - Image dimensions and characteristics (9 tags)
- `equipment` - Device and institution tags (6 tags)
- `ct` - CT-specific parameters (6 tags)
- `mr` - MR-specific parameters (6 tags)
- `ultrasound` - Ultrasound-specific tags (6 tags)
- `petNm` - PET/Nuclear Medicine tags (11 tags)
- `xa` - X-Ray Angiography tags (8 tags)
- `rt` - Radiation Therapy tags (11 tags)
- `default` - Comprehensive set (42 tags combining patient, study, series, instance, pixel, equipment)

**Example:**

```typescript
import { StoreScp, getCommonTagSets } from '@nuxthealth/node-dicom';

const tagSets = getCommonTagSets();

const receiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'MY-SCP',
    outDir: './dicom-storage',
    extractTags: tagSets.default  // Extract 42 common tags
});

receiver.onFileStored((err, event) => {
    const tags = event.data?.tags;
    if (tags) {
        console.log('Patient:', tags.PatientName);
        console.log('Study:', tags.StudyDescription);
        console.log('Modality:', tags.Modality);
    }
});
```

**Modality-Specific Configuration:**

```typescript
import { StoreScp, getCommonTagSets, combineTags } from '@nuxthealth/node-dicom';

const tagSets = getCommonTagSets();

// CT receiver with CT-specific parameters
const ctReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'CT-SCP',
    outDir: './ct-storage',
    extractTags: combineTags([
        tagSets.default,
        tagSets.ct
    ])
});

ctReceiver.onFileStored((err, event) => {
    const tags = event.data?.tags;
    if (tags) {
        console.log('CT Parameters:');
        console.log('  kVp:', tags.KVP);
        console.log('  Tube Current:', tags.XRayTubeCurrent);
        console.log('  Kernel:', tags.ConvolutionKernel);
    }
});

// MR receiver with MR-specific parameters
const mrReceiver = new StoreScp({
    port: 4447,
    callingAeTitle: 'MR-SCP',
    outDir: './mr-storage',
    extractTags: combineTags([
        tagSets.default,
        tagSets.mr
    ])
});

mrReceiver.onFileStored((err, event) => {
    const tags = event.data?.tags;
    if (tags) {
        console.log('MR Parameters:');
        console.log('  TR:', tags.RepetitionTime);
        console.log('  TE:', tags.EchoTime);
        console.log('  Field Strength:', tags.MagneticFieldStrength);
    }
});
```

### combineTags()

Combine multiple tag arrays into a single deduplicated array.

**Example:**

```typescript
import { getCommonTagSets, combineTags } from '@nuxthealth/node-dicom';

const tagSets = getCommonTagSets();

// Combine predefined sets
const workflowTags = combineTags([
    tagSets.patientBasic,
    tagSets.studyBasic,
    tagSets.seriesBasic,
    ['WindowCenter', 'WindowWidth'],           // Display params
    ['RescaleIntercept', 'RescaleSlope']      // Rescale params
]);

const receiver = new StoreScp({
    port: 4446,
    outDir: './storage',
    extractTags: workflowTags
});
```

### getCommonSopClasses()

Get predefined sets of SOP Class UIDs organized by modality.

**Available Sets:**
- `ct` - CT Image Storage
- `mr` - MR Image Storage  
- `us` - Ultrasound Image Storage
- `pet` - PET Image Storage
- `nm` - Nuclear Medicine Image Storage
- `xa` - X-Ray Angiographic Image Storage
- `dx` - Digital X-Ray Image Storage
- `mg` - Digital Mammography X-Ray Image Storage
- `sc` - Secondary Capture Image Storage
- `rt` - RT Image, Dose, Structure Set, Plan Storage
- `all` - All above SOP classes

**Example:**

```typescript
import { StoreScp, getCommonSopClasses } from '@nuxthealth/node-dicom';

const sopClasses = getCommonSopClasses();

// Accept only CT images
const ctReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'CT-SCP',
    outDir: './ct-storage',
    abstractSyntaxMode: 'Custom',
    abstractSyntaxes: sopClasses.ct
});

// Accept CT and MR
const crossModalityReceiver = new StoreScp({
    port: 4447,
    callingAeTitle: 'CROSS-SCP',
    outDir: './multimodal-storage',
    abstractSyntaxMode: 'Custom',
    abstractSyntaxes: [...sopClasses.ct, ...sopClasses.mr]
});
```

### getCommonTransferSyntaxes()

Get predefined sets of Transfer Syntax UIDs for compression and encoding.

**Available Sets:**
- `uncompressed` - Explicit/Implicit VR Little/Big Endian
- `jpegLossy` - JPEG Lossy compressions
- `jpegLossless` - JPEG Lossless compressions
- `jpeg2000Lossy` - JPEG 2000 Lossy
- `jpeg2000Lossless` - JPEG 2000 Lossless
- `rle` - RLE Lossless
- `all` - All common transfer syntaxes

**Example:**

```typescript
import { StoreScp, getCommonTransferSyntaxes } from '@nuxthealth/node-dicom';

const transferSyntaxes = getCommonTransferSyntaxes();

// Accept only uncompressed images
const uncompressedReceiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'UNCOMP-SCP',
    outDir: './uncompressed',
    transferSyntaxMode: 'Custom',
    transferSyntaxes: transferSyntaxes.uncompressed
});

// Accept lossless only
const losslessReceiver = new StoreScp({
    port: 4447,
    callingAeTitle: 'LOSSLESS-SCP',
    outDir: './lossless-storage',
    transferSyntaxMode: 'Custom',
    transferSyntaxes: [
        ...transferSyntaxes.uncompressed,
        ...transferSyntaxes.jpegLossless,
        ...transferSyntaxes.jpeg2000Lossless,
        ...transferSyntaxes.rle
    ]
});
```

### getAvailableTagNames()

Get a list of 300+ commonly used DICOM tag names for validation and discovery.

**Example:**

```typescript
import { getAvailableTagNames } from '@nuxthealth/node-dicom';

const allTags = getAvailableTagNames();
console.log(`Total available: ${allTags.length} tags`);

// Validate user input
const userTags = ['PatientName', 'InvalidTag', 'StudyDate'];
const validTags = userTags.filter(tag => allTags.includes(tag));
console.log('Valid tags:', validTags); // ['PatientName', 'StudyDate']

// Find specific tags
const patientTags = allTags.filter(tag => 
    tag.toLowerCase().includes('patient')
);
console.log('Patient tags:', patientTags);
```

### createCustomTag()

Create custom tag specifications for private or vendor-specific DICOM tags.

**Example:**

```typescript
import { StoreScp, createCustomTag } from '@nuxthealth/node-dicom';

const customTags = [
    createCustomTag('00091001', 'VendorID'),
    createCustomTag('00091010', 'ScannerMode'),
    createCustomTag('00431001', 'ImageQuality')
];

const receiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'MY-SCP',
    outDir: './received',
    extractTags: ['PatientName', 'StudyDate', 'Modality'],
    extractCustomTags: customTags
});

receiver.onFileStored((err, event) => {
    const tags = event.data?.tags;
    if (tags) {
        console.log('Standard:', tags.PatientName);
        console.log('Custom:', tags.VendorID, tags.ScannerMode);
    }
});
```

## Events and Callbacks

### onBeforeStore (Callback)

The `onBeforeStore` callback allows you to intercept and modify DICOM tags **before** files are saved to disk. This is a powerful feature for anonymization, validation, tag normalization, and audit logging.

**Important:** This is an **async callback** (registered with a method), not an event listener. It follows the error-first callback pattern and uses JSON for data exchange.

```typescript
// Register the async callback BEFORE calling start()
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Can now use await for async operations!
  const anonId = await database.getOrCreateAnonymousId(tags.PatientID);
  
  const modified = {
    ...tags,
    PatientName: 'ANONYMOUS^PATIENT',
    PatientID: anonId
  };
  
  return JSON.stringify(modified);
});

receiver.start();
```

#### Callback Signature

```typescript
type OnBeforeStoreCallback = (err: Error | null, tagsJson: string) => Promise<string>;
```

**Parameters:**
- `err`: Error object (typically null unless internal error occurs)
- `tagsJson`: JSON string containing the extracted DICOM tags

**Returns:**
- **Promise** that resolves to JSON string of modified tags

**Important Notes:**
1. The callback follows **error-first pattern**: `async (err, tagsJson) => Promise<string>`
2. Tags are provided as **JSON string** - must use `JSON.parse()` to read and `JSON.stringify()` to return
3. The callback is **async** - it can use `await` for database queries, API calls, etc.
4. File storage waits for the Promise to resolve before saving the file
5. The **server is fully asynchronous** - multiple files are processed in parallel, each with independent callback execution
6. Only tags specified in `extractTags` configuration are available
7. You must return a Promise that resolves to a JSON string of modified tags
8. If `extractTags` is empty or not configured, the callback won't be invoked
9. Must call `onBeforeStore()` **before** calling `start()`
10. Promise rejections are logged and prevent the file from being saved

**Critical Limitations:**
1. **Only return patient demographic tags** (PatientName, PatientID, PatientBirthDate, PatientSex) from your callback
   - ❌ Do NOT return pixel-related tags (BitsAllocated, Rows, Columns, PixelSpacing, etc.)
   - ❌ Do NOT return technical image metadata tags
   - ✅ Only return the specific tags you actually modified
   - Returning unmodified technical tags can corrupt the DICOM file structure and make images unreadable
2. **Use case**: This callback is designed for **anonymization and patient data modification only**
   - Not intended for modifying image pixel data or technical parameters
   - Modifying pixel-related metadata will break image viewers (e.g., CornerstoneJS)
3. **Best practice**: Extract only the tags you need to modify in `extractTags`, return only those modified tags from the callback

#### Configuration Requirements

To use `onBeforeStore`, you must:

1. **Configure `extractTags`**: Specify which tags to extract and modify
   ```typescript
   extractTags: ['PatientName', 'PatientID', 'StudyDescription', 'PatientBirthDate']
   ```

2. **Enable `storeWithFileMeta`**: Ensure files are saved with DICOM meta header (recommended)
   ```typescript
   storeWithFileMeta: true
   ```

3. **Register the callback** before calling `listen()`

#### Use Cases

##### 1. Anonymization

```javascript
// Simple in-memory mapping
const patientMapping = new Map();
let anonymousCounter = 1000;

receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Generate or retrieve anonymous ID
  let anonymousID = patientMapping.get(tags.PatientID);
  if (!anonymousID) {
    anonymousID = `ANON_${String(anonymousCounter++).padStart(4, '0')}`;
    patientMapping.set(tags.PatientID, anonymousID);
  }

  // IMPORTANT: Only return the tags you actually modified!
  // Do NOT return all tags with spread operator (...tags)
  // This prevents corruption of pixel data and technical metadata
  const modified = {
    PatientName: 'ANONYMOUS^PATIENT',
    PatientID: anonymousID,
    PatientBirthDate: '',
    PatientSex: ''
  };
  
  return JSON.stringify(modified);
});

// Advanced: Async database lookup
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Use await for database operations
  const anonId = await db.query(
    'SELECT anon_id FROM patient_mapping WHERE real_id = ?',
    [tags.PatientID]
  ).then(result => {
    if (result.length > 0) {
      return result[0].anon_id;
    } else {
      const newId = `ANON_${Date.now()}`;
      db.execute(
        'INSERT INTO patient_mapping (real_id, anon_id) VALUES (?, ?)',
        [tags.PatientID, newId]
      );
      return newId;
    }
  });

  // Async audit logging
  // Async audit logging
  await db.execute(
    'INSERT INTO audit_log (timestamp, original_id, anon_id, study_uid) VALUES (?, ?, ?, ?)',
    [new Date(), tags.PatientID, anonId, tags.StudyInstanceUID]
  );

  // Only return modified patient tags - not all tags
  const modified = {
    PatientName: 'ANONYMOUS^PATIENT',
    PatientID: anonId,
    PatientBirthDate: '',
    PatientSex: ''
  };
  
  return JSON.stringify(modified);
});
```

##### 2. Validation

```javascript
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Validate required fields
  if (!tags.PatientID || !tags.StudyInstanceUID) {
    throw new Error('Missing required tags: PatientID or StudyInstanceUID');
  }
  
  // Validate format
  if (!/^\d+$/.test(tags.PatientID)) {
    throw new Error(`Invalid PatientID format: ${tags.PatientID}`);
  }
  
  // Validate modality
  const allowedModalities = ['CT', 'MR', 'US', 'CR', 'DR'];
  if (!allowedModalities.includes(tags.Modality)) {
    throw new Error(`Unsupported modality: ${tags.Modality}`);
  }
  
  // Async validation against external service
  const isValid = await fetch(`https://api.hospital.com/validate/patient/${tags.PatientID}`)
    .then(res => res.json())
    .then(data => data.valid);
  
  if (!isValid) {
    throw new Error(`Patient ID not found in hospital registry: ${tags.PatientID}`);
  }
  
  return JSON.stringify(tags);
});
```

##### 3. Tag Normalization

```javascript
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Fetch additional metadata from database
  const metadata = await db.query(
    'SELECT facility_code, department FROM facilities WHERE patient_id = ?',
    [tags.PatientID]
  ).then(result => result[0] || {});

  // Only return the tags you actually modified
  const modified = {
    // Normalize patient name to uppercase
    PatientName: tags.PatientName?.toUpperCase() || '',
    // Ensure consistent date format (YYYYMMDD)
    PatientBirthDate: tags.PatientBirthDate?.replace(/[^0-9]/g, '') || '',
    // Add facility prefix to patient ID
    PatientID: tags.PatientID ? `${metadata.facility_code || 'UNK'}_${tags.PatientID}` : '',
    // Enrich with department info
    InstitutionName: metadata.department || 'UNKNOWN'
  };
  
  return JSON.stringify(modified);
});
```

##### 4. Audit Logging

```javascript
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Async logging to database
  await db.execute(
    `INSERT INTO dicom_audit_log 
     (timestamp, patient_id, study_uid, modality, study_date, study_description, source_ip) 
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
    [
      new Date(),
      tags.PatientID,
      tags.StudyInstanceUID,
      tags.Modality,
      tags.StudyDate,
      tags.StudyDescription,
      'remote-source-ip'  // Could be passed via context
    ]
  );
  
  // Async notification to monitoring service
  await fetch('https://monitoring.hospital.com/api/dicom-received', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      timestamp: new Date().toISOString(),
      patientID: tags.PatientID,
      studyUID: tags.StudyInstanceUID,
      modality: tags.Modality
    })
  });
  
  // Return unmodified tags
  return JSON.stringify(tags);
});
```

#### Complete Example: Anonymization Server

```typescript
import { StoreScp } from '@nuxthealth/node-dicom';

// Simulate async database
class AnonymizationDB {
  private mapping = new Map();
  private counter = 1000;
  
  async getOrCreateAnonId(realId: string): Promise<string> {
    // Simulate database delay
    await new Promise(resolve => setTimeout(resolve, 10));
    
    let anonId = this.mapping.get(realId);
    if (!anonId) {
      anonId = `ANON_${String(this.counter++).padStart(4, '0')}`;
      this.mapping.set(realId, anonId);
    }
    return anonId;
  }
  
  async logAnonymization(realId: string, anonId: string): Promise<void> {
    // Simulate async audit logging
    await new Promise(resolve => setTimeout(resolve, 5));
    console.log(`[AUDIT] ${new Date().toISOString()}: ${realId} → ${anonId}`);
  }
}

const db = new AnonymizationDB();

const receiver = new StoreScp({
  port: 11115,
  callingAeTitle: 'ANON-SCP',
  outDir: './anonymized-storage',
  storeWithFileMeta: true,
  extractTags: [
    'PatientName',
    'PatientID',
    'PatientBirthDate',
    'PatientSex',
    'StudyDescription',
    'StudyInstanceUID',
    'Modality'
  ],
  verbose: true
});

// Register async anonymization callback
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  console.log(`Anonymizing: ${tags.PatientName} (${tags.PatientID})`);
  
  // Async database lookup
  const anonymousID = await db.getOrCreateAnonId(tags.PatientID);
  
  // Async audit logging
  await db.logAnonymization(tags.PatientID, anonymousID);

  const modified = {
    ...tags,
  // Only return modified patient demographic tags
  const modified = {
    PatientName: 'ANONYMOUS^PATIENT',
    PatientID: anonymousID,
    PatientBirthDate: '',
    PatientSex: ''
  };
  
  return JSON.stringify(modified);
});eiver.onServerStarted((err, event) => {
  if (err) {
    console.error('Server start error:', err);
    return;
  }
  console.log('✓ Anonymization server started:', event.message);
});

receiver.onFileStored((err, event) => {
  if (err) {
    console.error('Storage error:', err);
    return;
  }
  const data = event.data;
  console.log(`✓ Anonymized file stored: ${data.file}`);
  console.log(`  Patient: ${data.tags?.PatientName} (${data.tags?.PatientID})`);
});

receiver.start();
```

#### Error Handling

If the callback throws an error or the Promise rejects:
- The file will **not** be saved to disk
- The sending SCU receives a DICOM error status
- The association remains open for subsequent files
- Error is logged if `verbose: true`

```typescript
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Validation example
  if (!tags.PatientID) {
    throw new Error('PatientID is required');
  }
  
  // Async validation with external service
  const patient = await fetch(`https://api.hospital.com/patients/${tags.PatientID}`)
    .then(res => {
      if (!res.ok) throw new Error(`Patient not found: ${tags.PatientID}`);
      return res.json();
    });
  
  // Business rule example
  if (tags.Modality === 'CT' && !tags.StudyDescription) {
    throw new Error('StudyDescription required for CT studies');
  }
  
  return JSON.stringify(tags);
});
```

#### Performance Considerations

1. **Async operations**: The callback now supports async/await for database queries, API calls, and other I/O operations
2. **Per-file processing**: Each file's callback waits for the Promise to resolve before saving
3. **Parallel processing**: The server handles multiple files asynchronously in parallel, each with its own callback invocation
4. **What's Efficient**:
   - ✅ Database queries (with connection pooling)
   - ✅ API calls (with reasonable timeouts)
   - ✅ Async I/O operations (logging, caching)
   - ✅ String manipulation and validation
   - ✅ In-memory lookups (Map, object access)
   - ⚠️ Slow external services (consider timeouts and retries)
   - ⚠️ Heavy computation (consider offloading to workers)
   - ❌ Blocking operations (avoid synchronous file I/O)
5. **Best Practices**:
   - Use connection pooling for database operations
   - Set reasonable timeouts for external API calls
   - Cache frequently accessed data when possible
   - Use `Promise.all()` for parallel async operations
   - Handle Promise rejections to prevent file rejection
6. **Thread safety**: The callback is called from async Rust context via ThreadsafeFunction - perfectly safe for concurrent execution
7. **Tag extraction overhead**: Only extract tags you need to modify
8. **Memory**: Each callback invocation clones the tags HashMap (typically <1KB per file)

**Example: Optimized async operations**
```typescript
receiver.onBeforeStore(async (error, tagsJson) => {
  if (error) throw error;
  
  const tags = JSON.parse(tagsJson);
  
  // Parallel async operations
  const [anonId, metadata, isAuthorized] = await Promise.all([
    db.getAnonymousId(tags.PatientID),
    api.getStudyMetadata(tags.StudyInstanceUID),
    authService.checkAccess(tags.PatientID)
  ]);
  
  if (!isAuthorized) {
    throw new Error('Unauthorized patient access');
  }
  
  const modified = {
    ...tags,
    PatientID: anonId,
    InstitutionName: metadata.institution
  };
  
  return JSON.stringify(modified);
});
```


### OnServerStarted (Event)

Triggered when the server starts listening.

```typescript
receiver.onServerStarted((err, event) => {
    if (err) {
        console.error('Error:', err);
        return;
    }
    console.log('Server started:', event.message);
});
```

### OnFileStored (Event)

Triggered when each DICOM file is received and stored.

```typescript
receiver.onFileStored((err, event) => {
    if (err) {
        console.error('Error:', err);
        return;
    }
    
    const data = event.data;
    if (!data) return;
    
    console.log('File:', data.file);
    console.log('SOP Instance UID:', data.sopInstanceUid);
    console.log('SOP Class UID:', data.sopClassUid);
    console.log('Transfer Syntax:', data.transferSyntaxUid);
    console.log('Study UID:', data.studyInstanceUid);
    console.log('Series UID:', data.seriesInstanceUid);
    
    // Tags are always flat for simple, direct access
    if (data.tags) {
        console.log('Patient:', data.tags.PatientName);
        console.log('Study Date:', data.tags.StudyDate);
        console.log('Modality:', data.tags.Modality);
    }
});
```

Event data structure (flat tags):
```typescript
{
    file: "path/to/file.dcm",
    sopInstanceUid: "1.2.3...",
    sopClassUid: "1.2.840...",
    transferSyntaxUid: "1.2.840...",
    studyInstanceUid: "1.2.3...",
    seriesInstanceUid: "1.2.3...",
    tags: {                        // All extracted tags in flat structure
        PatientName: "DOE^JOHN",
        PatientID: "12345",
        StudyDate: "20231201",
        StudyDescription: "CT Chest",
        Modality: "CT",
        SeriesDescription: "Chest with contrast",
        InstanceNumber: "1",
        SliceThickness: "5.0",
        Manufacturer: "GE"         // Equipment tags also included
    }
}
```

### OnStudyCompleted (Event)

Triggered when no new files are received for a study after the timeout period.

```typescript
receiver.onStudyCompleted((err, event) => {
    if (err) {
        console.error('Error:', err);
        return;
    }
    
    const study = event.data?.study;
    if (!study) return;
    
    console.log(`Study ${study.studyInstanceUid} completed`);
    console.log(`Patient: ${study.tags?.PatientName}`);
    console.log(`Study Date: ${study.tags?.StudyDate}`);
    console.log(`${study.series.length} series`);
    
    for (const series of study.series) {
        console.log(`  Series ${series.seriesInstanceUid}`);
        console.log(`  Modality: ${series.tags?.Modality}`);
        console.log(`  ${series.instances.length} instances`);
        
        for (const instance of series.instances) {
            console.log(`    Instance ${instance.tags?.InstanceNumber}: ${instance.file}`);
        }
    }
});
```

Event data structure (hierarchical with flat tags at each level):
```typescript
{
    studyInstanceUid: "1.2.3...",
    tags: {                        // Patient + Study level tags only
        PatientName: "DOE^JOHN",
        PatientID: "12345",
        StudyDate: "20231201",
        StudyDescription: "CT Chest",
        AccessionNumber: "A12345"
    },
    series: [
        {
            seriesInstanceUid: "1.2.3...",
            tags: {                // Series level tags only
                Modality: "CT",
                SeriesNumber: "1",
                SeriesDescription: "Chest with contrast",
                BodyPartExamined: "CHEST"
            },
            instances: [
                {
                    sopInstanceUid: "1.2.3...",
                    sopClassUid: "1.2.840...",
                    transferSyntaxUid: "1.2.840...",
                    file: "path/to/file.dcm",
                    tags: {        // Instance + Equipment level tags only
                        InstanceNumber: "1",
                        SliceLocation: "100.0",
                        SliceThickness: "5.0",
                        Manufacturer: "GE",              // Equipment tag
                        ManufacturerModelName: "CT750",  // Equipment tag
                        SoftwareVersions: "1.0"          // Equipment tag
                    }
                }
            ]
        }
    ]
}
```

**Tag Distribution:**
- **Study level**: Patient demographics + Study metadata (no duplication across series/instances)
- **Series level**: Series-specific tags (no duplication across instances)
- **Instance level**: Instance-specific data + Equipment/device information

This hierarchical structure avoids data duplication while keeping tags flat at each level for easy access.

## Storage Backends

### Filesystem Storage

```typescript
const receiver = new StoreScp({
    port: 4446,
    storageBackend: 'Filesystem',
    outDir: './dicom-storage'
});
```

Files are stored in hierarchy: `{outDir}/{StudyInstanceUID}/{SeriesInstanceUID}/{SOPInstanceUID}.dcm`

### S3 Storage

```typescript
const receiver = new StoreScp({
    port: 4446,
    storageBackend: 'S3',
    s3Config: {
        bucket: 'dicom-archive',
        accessKey: 'YOUR_ACCESS_KEY',
        secretKey: 'YOUR_SECRET_KEY',
        endpoint: 'https://s3.amazonaws.com',  // Or MinIO/other S3-compatible
        region: 'us-east-1'                    // Optional
    }
});
```

Files are stored with the same path structure in the S3 bucket.

## Complete Example

```typescript
import { StoreScp, getCommonSopClasses, getCommonTransferSyntaxes } from '@nuxthealth/node-dicom';

const sopClasses = getCommonSopClasses();
const transferSyntaxes = getCommonTransferSyntaxes();

const receiver = new StoreScp({
    // Network
    port: 4446,
    callingAeTitle: 'HOSPITAL-SCP',
    maxPduLength: 32768,
    
    // Storage
    storageBackend: 'Filesystem',
    outDir: './dicom-archive',
    
    // Tag Extraction
    extractTags: [
        'PatientName', 'PatientID', 'PatientBirthDate',
        'StudyDate', 'StudyDescription', 'AccessionNumber',
        'Modality', 'SeriesDescription', 'SeriesNumber',
        'InstanceNumber', 'SliceThickness'
    ],
    
    // SOP Classes (only accept CT and MR)
    abstractSyntaxMode: 'Custom',
    abstractSyntaxes: [...sopClasses.ct, ...sopClasses.mr],
    
    // Transfer Syntaxes (accept all)
    transferSyntaxMode: 'All',
    
    // Study completion
    studyTimeout: 60,
    
    verbose: true
});

receiver.onServerStarted((err, event) => {
    if (err) return console.error('Error:', err);
    console.log(`✓ SCP Server started:`, event.message);
});

receiver.onFileStored((err, event) => {
    if (err) return console.error('Error:', err);
    const data = event.data;
    if (!data) return;
    
    console.log(`✓ Received: ${data.file}`);
    console.log(`  Patient: ${data.tags?.PatientName}`);
    console.log(`  Study: ${data.tags?.StudyDescription}`);
    console.log(`  Modality: ${data.tags?.Modality}`);
});

receiver.onStudyCompleted((err, event) => {
    if (err) return console.error('Error:', err);
    const study = event.data?.study;
    if (!study) return;
    
    const totalInstances = study.series.reduce((sum, s) => sum + s.instances.length, 0);
    console.log(`✓ Study ${study.studyInstanceUid} completed`);
    console.log(`  Patient: ${study.tags?.PatientName}`);
    console.log(`  ${study.series.length} series, ${totalInstances} instances`);
});

receiver.start();
```

## Tips

1. **Tag extraction is always flat**: `OnFileStored` provides flat tags for simple access; `OnStudyCompleted` provides hierarchical organization with flat tags at each level
2. **Use onBeforeStore for real-time processing**: Anonymize, validate, or normalize tags before files are saved to disk
3. **Configure SOP classes**: Limit to only the modalities you need for better control and security
4. **Set appropriate timeout**: Adjust `studyTimeout` based on your typical scan duration and network speed
5. **Use S3 for production scale**: Filesystem is good for development, S3 for unlimited scalable storage
6. **Extract only needed tags**: Don't extract unnecessary tags to reduce memory usage and processing time
7. **Start with defaults**: Begin with simple configuration and add complexity only as needed
8. **Enable verbose for debugging**: Use `verbose: true` during development to see detailed protocol information
9. **Test with real data**: Always test with actual DICOM files from your modalities before production
10. **Monitor disk space**: With Filesystem storage, ensure adequate space and set up monitoring/alerts
11. **Secure your credentials**: Never hardcode S3 credentials - use environment variables or secret management
12. **Enable storeWithFileMeta when using onBeforeStore**: Ensures proper DICOM file structure after tag modifications

# node-dicom-rs

High-performance Node.js bindings for DICOM (Digital Imaging and Communications in Medicine) operations, powered by Rust and [dicom-rs](https://github.com/Enet4/dicom-rs).

## Features

- **StoreScp**: Receive DICOM files over the network with C-STORE SCP server
- **StoreScu**: Send DICOM files to remote PACS systems
- **DicomFile**: Read, parse, and manipulate DICOM files with full metadata extraction
- **Storage Backends**: Filesystem and S3-compatible object storage support
- **TypeScript Support**: Full TypeScript definitions with autocomplete for 300+ DICOM tags
- **Event-driven API**: Consistent callback-based events with typed data structures

## Installation

```bash
npm install @nuxthealth/node-dicom
```

## Quick Start

### Receiving DICOM Files (StoreScp)

```typescript
import { StoreScp } from '@nuxthealth/node-dicom';

const receiver = new StoreScp({
    port: 4446,
    callingAeTitle: 'MY-SCP',
    outDir: './dicom-storage',
    verbose: true,
    extractTags: ['PatientName', 'StudyDate', 'Modality']
});

receiver.onFileStored((err, event) => {
    if (err) return console.error('Error:', err);
    const data = event.data;
    if (!data) return;
    
    console.log('File received:', data.file);
    if (data.tags) {
        console.log('Patient:', data.tags.PatientName);
        console.log('Study Date:', data.tags.StudyDate);
        console.log('Modality:', data.tags.Modality);
    }
});

receiver.onStudyCompleted((err, event) => {
    if (err) return console.error('Error:', err);
    const study = event.data?.study;
    if (!study) return;
    
    console.log(`Study ${study.studyInstanceUid} complete`);
    console.log(`${study.series.length} series, total instances: ${study.series.reduce((sum, s) => sum + s.instances.length, 0)}`);
});

receiver.start();
```

### Sending DICOM Files (StoreScu)

```typescript
import { StoreScu } from '@nuxthealth/node-dicom';

const sender = new StoreScu({
    addr: '192.168.1.100:104',
    callingAeTitle: 'MY-SCU',
    calledAeTitle: 'REMOTE-SCP',
    verbose: true,
    throttleDelayMs: 100  // Optional: Rate limiting - delay 100ms between each file
});

// Add files
sender.addFile('./path/to/file.dcm');
sender.addFolder('./dicom-folder');

// Send with progress tracking
const result = await sender.send({
    onFileSent: (err, event) => {
        console.log('✓ File sent:', event.data?.sopInstanceUid);
    },
    onFileError: (err, event) => {
        console.error('✗ Error:', event.message, event.data?.error);
    },
    onTransferCompleted: (err, event) => {
        const data = event.data;
        if (data) {
            console.log(`Transfer complete! ${data.successful}/${data.totalFiles} files in ${data.durationSeconds.toFixed(2)}s`);
        }
    }
});

console.log('Result:', result);
```

### Working with DICOM Files

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';

const file = new DicomFile();
await file.open('./scan.dcm');
// Automatically handles both standard DICOM files with meta headers
// and dataset-only files (without meta) - creates meta on-the-fly if needed

// Extract specific tags (always returns flat structure)
const data = file.extract(['PatientName', 'StudyDate', 'Modality']);
console.log('Patient:', data.PatientName);
console.log('Study Date:', data.StudyDate);
console.log('Modality:', data.Modality);

// Get DICOM as JSON (without saving to file)
const json = file.toJson(true);
const obj = JSON.parse(json);

// Get pixel data info
const pixelInfo = file.getPixelDataInfo();
console.log(`Image: ${pixelInfo.width}x${pixelInfo.height}, ${pixelInfo.frames} frames`);

// Get pixel data as Buffer (without saving to file)
const pixelBuffer = file.getPixelData();
console.log(`Pixel data: ${pixelBuffer.length} bytes`);

// For compressed data, decode it
if (pixelInfo.isCompressed) {
    const decodedBuffer = file.getDecodedPixelData();
    processImage(decodedBuffer, pixelInfo);
}

// NEW! Get processed pixel data with windowing, frame extraction, 8-bit conversion
const displayReady = file.getProcessedPixelData({
    applyVoiLut: true,      // Use WindowCenter/Width from file
    convertTo8bit: true      // Convert to 8-bit for display (0-255)
});

// Custom window settings for different tissue types
const boneWindow = file.getProcessedPixelData({
    windowCenter: 300,       // Bone window
    windowWidth: 1500,
    convertTo8bit: true
});

// Or save pixel data to file (synchronous)
file.saveRawPixelData('./output.raw');

file.close();
```

### Update DICOM Tags 🆕

Modify tag values for anonymization or corrections:

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';
import crypto from 'crypto';

const file = new DicomFile();
await file.open('scan.dcm');

// Update tags (changes in memory only)
file.updateTags({
    PatientName: 'ANONYMOUS',
    PatientID: crypto.randomUUID(),
    PatientBirthDate: '',
    InstitutionName: 'ANONYMIZED'
});

// Save changes to new file
await file.saveAsDicom('anonymized.dcm');
file.close();
```

### DICOMweb Services

node-dicom-rs provides DICOMweb servers for querying and retrieving DICOM objects over HTTP.

#### QIDO-RS Server (Query)

QIDO-RS allows clients to search for DICOM studies, series, and instances:

```javascript
import { QidoServer } from '@nuxthealth/node-dicom';

const qidoServer = new QidoServer(8080);
qidoServer.start();

// Server is now listening on http://localhost:8080
// Endpoints:
//   GET /studies - Search for studies
//   GET /series - Search for series
//   GET /instances - Search for instances

// Stop when done
qidoServer.stop();
```

For more details, see the [QIDO-RS Guide](./docs/qido-rs.md).

#### WADO-RS Server (Retrieval)

WADO-RS provides standardized retrieval of DICOM files:

```javascript
import { WadoServer } from '@nuxthealth/node-dicom';

const wadoConfig = {
  storageType: 'filesystem',
  basePath: '/path/to/dicom/files'
};

const wadoServer = new WadoServer(8081, wadoConfig);
wadoServer.start();

// Server is now listening on http://localhost:8081
// Endpoints:
//   GET /studies/{studyUID}
//   GET /studies/{studyUID}/series/{seriesUID}
//   GET /studies/{studyUID}/series/{seriesUID}/instances/{instanceUID}
//   GET /studies/{studyUID}/metadata

// Stop when done
wadoServer.stop();
```

For filesystem storage, organize files as: `{basePath}/{studyUID}/{seriesUID}/{instanceUID}.dcm`

For more details, see the [QIDO-RS Guide](./docs/wado-rs.md).

## Documentation

For detailed documentation, see:

- **[StoreScp Guide](./docs/storescp.md)** - Receiving DICOM files, tag extraction, storage backends, async tag modification
- **[StoreScu Guide](./docs/storescu.md)** - Sending DICOM files, transfer syntaxes, batch operations
- **[DicomFile Guide](./docs/dicomfile.md)** - Reading files, extracting metadata, pixel data operations
- **[QIDO-RS Guide](./docs/qido-rs.md)** - Query service for searching DICOM studies, series, and instances
- **[WADO-RS Guide](./docs/wado-rs.md)** - Retrieval service for accessing DICOM objects over HTTP

## Key Features

### Tag Extraction

Extract DICOM metadata with ease:

```typescript
// DicomFile: Always returns flat structure
const data = file.extract(['PatientName', 'StudyDate', 'Modality']);
console.log('Patient:', data.PatientName);

// StoreScp: Flat tags for OnFileStored
receiver.onFileStored((err, event) => {
    const tags = event.data?.tags;
    console.log('Patient:', tags?.PatientName);
});

// StoreScp: Hierarchical with flat tags at each level for OnStudyCompleted
receiver.onStudyCompleted((err, event) => {
    const study = event.data?.study;
    console.log('Study tags:', study?.tags); // Patient + Study level
    study?.series.forEach(series => {
        console.log('Series tags:', series.tags); // Series level
        series.instances.forEach(instance => {
            console.log('Instance tags:', instance.tags); // Instance + Equipment level
        });
    });
});
```

### Tag Modification Before Storage 🆕

Modify DICOM tags asynchronously before files are saved using the `onBeforeStore` callback:

```typescript
const receiver = new StoreScp({
    port: 4446,
    outDir: './dicom-storage',
    extractTags: ['PatientName', 'PatientID', 'PatientBirthDate', 'StudyDescription']
});

// Anonymize incoming files before storage (with async database lookup)
receiver.onBeforeStore(async (error, tagsJson) => {
    if (error) throw error;
    
    const tags = JSON.parse(tagsJson);
    
    // Async database lookup for persistent anonymization
    const anonId = await db.getOrCreateAnonId(tags.PatientID);
    
    const modified = {
        ...tags,
        PatientName: 'ANONYMOUS',
        PatientID: anonId,
        PatientBirthDate: '', // Remove PHI
        StudyDescription: tags.StudyDescription ? 
            `ANONYMIZED - ${tags.StudyDescription}` : 
            'ANONYMIZED STUDY'
    };
    
    return JSON.stringify(modified);
});

receiver.start();
```

**Key Features:**
- **Asynchronous**: Supports async/await for database operations and API calls
- **Error-First Pattern**: Callback receives `(error, tagsJson)` parameters
- **Pre-Storage**: Modifications applied BEFORE writing to disk
- **Tag-Safe**: Only modifies extracted tags (specified in `extractTags`)
- **JSON Format**: Tags passed as JSON string, must parse and stringify
- **Flexible**: Use for anonymization, validation, enrichment, or standardization

**Use Cases:**
- Real-time anonymization with persistent database mappings
- Adding institution-specific metadata from external APIs
- Tag validation against external services
- Format standardization
- PHI removal for GDPR/HIPAA compliance

See [demos](./playground/README.md) for complete examples.

### TypeScript Autocomplete

Full autocomplete support for 300+ DICOM tags:

```typescript
const data = file.extract([
    'PatientName',      // Autocomplete suggests all standard tags
    'StudyDate',
    'Modality',
    'SeriesDescription'
]);
```

### Storage Backends

Store received DICOM files to filesystem or S3:

```typescript
// S3 Storage
const receiver = new StoreScp({
    port: 4446,
    storageBackend: 'S3',
    s3Config: {
        bucket: 'dicom-archive',
        accessKey: 'YOUR_KEY',
        secretKey: 'YOUR_SECRET',
        endpoint: 'https://s3.amazonaws.com'
    }
});
```

### Configurable SCP Acceptance

Control which DICOM types your SCP accepts:

```typescript
import { getCommonSopClasses, getCommonTransferSyntaxes } from '@nuxthealth/node-dicom';

const sopClasses = getCommonSopClasses();
const transferSyntaxes = getCommonTransferSyntaxes();

const receiver = new StoreScp({
    port: 4446,
    abstractSyntaxMode: 'Custom',
    abstractSyntaxes: [...sopClasses.ct, ...sopClasses.mr], // Only CT and MR
    transferSyntaxMode: 'UncompressedOnly' // Only uncompressed
});
```

## Examples

Check the `playground/` directory for more examples:

- Basic SCP receiver
- SCU sender with batch processing
- File metadata extraction
- S3 storage integration
- Custom tag extraction

## Performance

Built with Rust for maximum performance:
- Fast DICOM parsing and validation
- Efficient memory usage for large files
- Native async/await support
- Zero-copy operations where possible

## Credits

- Built on [dicom-rs](https://github.com/Enet4/dicom-rs) by Eduardo Pinho [@Enet4](https://github.com/Enet4)
- Uses [napi-rs](https://napi.rs/) for Rust ↔ Node.js bindings

## License

See LICENSE file for details.
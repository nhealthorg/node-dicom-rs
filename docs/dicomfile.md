# DicomFile - Reading and Manipulating DICOM Files

The `DicomFile` class provides comprehensive methods to read, parse, extract data, and manipulate DICOM files. It supports both filesystem and S3 storage backends, handles compressed and uncompressed pixel data, and provides convenient tag extraction with TypeScript autocomplete.

## Quick Reference

### Core Operations

| Method | Type | Return Type | Description |
|--------|------|-------------|-------------|
| `new DicomFile(config?)` | Constructor | `DicomFile` | Create new instance with optional S3/filesystem config |
| `open(path)` | Async | `Promise<string>` | Open DICOM file from filesystem or S3 |
| `openJson(path)` | Async | `Promise<string>` | Open DICOM JSON file (Part 18 format) |
| `close()` | Sync | `void` | Close file and free memory |
| `check(path)` | Static Sync | `DicomFileMeta` | Validate DICOM file without fully opening |

### Metadata Extraction

| Method | Type | Return Type | Description |
|--------|------|-------------|-------------|
| `extract(tags, customTags?)` | Sync | `Record<string, string>` | Extract specific tags as flat object |
| `updateTags(updates)` | Sync | `string` | Update tag values in memory (call saveAsDicom to persist) |
| `toJson(pretty?)` | Sync | `string` | Get entire DICOM as JSON string (no file I/O) |
| `dump()` | Sync | `void` | Print formatted DICOM structure to stdout |
| `getPixelDataInfo()` | Sync | `PixelDataInfo` | Get comprehensive pixel data metadata |

### Pixel Data Access

| Method | Type | Return Type | Description |
|--------|------|-------------|-------------|
| `getPixelData()` | Sync | `Buffer` | Get raw pixel data as Buffer (may be compressed) |
| `getDecodedPixelData()` | Sync | `Buffer` | Get decoded/decompressed pixel data (requires transcode feature) |
| `getProcessedPixelData(options?)` | Sync | `Buffer` | Get decoded + processed pixel data with windowing, frame extraction, 8-bit conversion |
| `getImageBuffer(options?)` | Sync | `Buffer` | Get encoded PNG/JPEG/BMP image as in-memory buffer (no file I/O) |
| `saveRawPixelData(path)` | Sync | `string` | Save raw pixel data to file |
| `processPixelData(options)` | Async | `Promise<string>` | Advanced: process and save with format conversion |

### Binary Tag & Document Access

| Method | Type | Return Type | Description |
|--------|------|-------------|-------------|
| `getTagInfo(tagName)` | Sync | `TagDataInfo` | Inspect any tag's VR, binary flag, image flag, MIME type, and byte length |
| `getTagBytes(tagName)` | Sync | `Buffer` | Binary-safe raw bytes for any tag (OB/OW/UN/etc.) |
| `getEncapsulatedDocument()` | Sync | `EncapsulatedDocumentData` | Extract encapsulated PDF/text document with MIME type and data Buffer |

### File Operations

| Method | Type | Return Type | Description |
|--------|------|-------------|-------------|
| `saveAsJson(path, pretty?)` | Async | `Promise<string>` | Save as DICOM JSON file |
| `saveAsDicom(path)` | Async | `Promise<string>` | Save as binary DICOM file |

**Method Type Conventions:**
- **Sync methods**: Execute immediately, return results directly, use for metadata operations
- **Async methods**: Return Promises, use for I/O operations (file/S3 reads and writes)
- **Static methods**: Call on class itself (e.g., `DicomFile.check()`), don't require instance

## Basic Usage

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';

// Create instance and open file
const file = new DicomFile();
await file.open('./scan.dcm');

// Extract metadata - always returns flat structure
const data = file.extract(['PatientName', 'StudyDate', 'Modality']);

console.log('Patient:', data.PatientName);
console.log('Study Date:', data.StudyDate);
console.log('Modality:', data.Modality);

// Don't forget to close
file.close();
```

## Creating Instances

### Default Configuration

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';

// Uses filesystem with current working directory as root
const file = new DicomFile();
```

### Filesystem with Custom Root

```typescript
const file = new DicomFile({
    backend: 'Filesystem',
    filesystemConfig: {
        rootDir: '/path/to/dicom/archive'
    }
});

// Now all paths are relative to /path/to/dicom/archive
await file.open('studies/study1/series1/image1.dcm');
// Opens: /path/to/dicom/archive/studies/study1/series1/image1.dcm
```

### S3 Storage Backend

```typescript
const file = new DicomFile({
    backend: 'S3',
    s3Config: {
        bucket: 'my-dicom-bucket',
        accessKey: process.env.AWS_ACCESS_KEY,
        secretKey: process.env.AWS_SECRET_KEY,
        endpoint: 'https://s3.amazonaws.com',
        region: 'us-east-1'
    }
});

// All operations now use S3
await file.open('studies/patient123/ct-scan.dcm');
await file.saveAsJson('output/metadata.json', true);

// Note: S3 backend also supports intelligent format detection
// Handles both standard DICOM files and dataset-only files seamlessly
```

## Opening Files

### From Filesystem

```typescript
const file = new DicomFile();
await file.open('/path/to/scan.dcm');

// Operations...

file.close();
```

**Intelligent Format Detection:**

The `open()` method automatically detects and handles both standard DICOM files and dataset-only files:

```typescript
const file = new DicomFile();

// Standard DICOM file with meta header - opens directly
await file.open('./standard-dicom.dcm');

// Dataset-only file (no meta header) - automatically creates meta on-the-fly
await file.open('./dataset-only.dcm');
// Creates proper FileMetaTable from dataset information:
//   - Extracts SOP Class UID and SOP Instance UID from dataset
//   - Determines Transfer Syntax UID (or defaults to Implicit VR Little Endian)
//   - Builds complete file meta information for full DICOM compatibility

// Works seamlessly with both formats from StoreSCP
// (regardless of storeWithFileMeta setting)
file.close();
```

**How it works:**
1. **First attempt**: Try to open as standard DICOM file with meta header
2. **Fallback**: If that fails, parse as dataset-only and create meta information from:
   - `SOPClassUID` tag (required, defaults to Secondary Capture if missing)
   - `SOPInstanceUID` tag (required, must exist in dataset)
   - `TransferSyntaxUID` tag (optional, defaults to Implicit VR Little Endian)
3. **Result**: Full DICOM file object ready for all operations

This is particularly useful when working with StoreSCP-received files that may have been stored 
without meta headers (when `storeWithFileMeta: false` is configured), or when processing 
dataset-only files from other sources.

### From DICOM JSON

DICOM files can be stored as JSON following the DICOM Part 18 JSON Model:

```typescript
const file = new DicomFile();
await file.openJson('/path/to/file.json');

// Work with it like any DICOM file
const data = file.extract(['PatientName', 'StudyDate']);

// Can even save back as binary DICOM
await file.saveAsDicom('output.dcm');

file.close();
```

### Validating Without Full Open

Use the static `check()` method to validate DICOM files and get basic metadata without loading the entire file into memory. This is much faster than `open()` and useful for batch validation:

```typescript
// Static method - no instance needed
const info = DicomFile.check('/path/to/scan.dcm');

console.log('SOP Instance UID:', info.sopInstanceUid);
console.log('SOP Class UID:', info.sopClassUid);

// Use for batch validation
const files = [/* ... */];
const validFiles = files.filter(f => {
    try {
        DicomFile.check(f);
        return true;
    } catch {
        return false;
    }
});
```

**When to use `check()` vs `open()`:**
- Use `check()`: Quick validation, batch processing, only need SOP UIDs
- Use `open()`: Full metadata extraction, pixel data access, file manipulation

## Extracting Metadata

The `extract()` method provides efficient tag extraction with TypeScript autocomplete for 300+ standard DICOM tags.

### Basic Tag Extraction

```typescript
const file = new DicomFile();
await file.open('./scan.dcm');

// Extract specific tags - returns flat object
const data = file.extract([
    'PatientName',
    'PatientID',
    'StudyDate',
    'StudyTime',
    'Modality',
    'SeriesDescription'
]);

console.log(data);
// {
//   PatientName: "DOE^JOHN",
//   PatientID: "12345",
//   StudyDate: "20231201",
//   StudyTime: "143022",
//   Modality: "CT",
//   SeriesDescription: "Chest w/o contrast"
// }

file.close();
```

### TypeScript Autocomplete

The extract method provides autocomplete for all standard DICOM tags:

```typescript
const data = file.extract([
    'Patient',  // Type 'Patient' and get autocomplete for:
                // PatientName, PatientID, PatientBirthDate, PatientSex, etc.
    
    'Study',    // Type 'Study' and get:
                // StudyDate, StudyTime, StudyDescription, StudyInstanceUID, etc.
    
    'Modality', // All 300+ standard tags available
    'SOPClassUID',
    'SeriesNumber',
    'InstanceNumber',
    'Rows',
    'Columns',
    // ... and many more
]);
```

### Alternative Tag Formats

Tags can be specified in multiple formats:

```typescript
// All three extract the same tag (PatientName)
const data = file.extract([
    'PatientName',     // Standard name (recommended)
    '00100010',        // Hex format (group + element)
    '(0010,0010)'      // DICOM format with parentheses
]);

// Mix formats as needed
const mixed = file.extract([
    'PatientName',     // Standard
    '00080020',        // Hex for StudyDate
    '(0008,0060)'      // DICOM format for Modality
]);
```

### Custom Private Tags

Extract private or vendor-specific tags by specifying the hex tag and a custom name:

```typescript
import { createCustomTag } from '@nuxthealth/node-dicom';

const data = file.extract(
    // Standard tags
    ['PatientName', 'StudyDate', 'Modality'],
    
    // Custom private tags
    [
        createCustomTag('00091001', 'GE_PrivateCreator'),
        createCustomTag('00091010', 'GE_ScannerMode'),
        createCustomTag('(0019,100A)', 'Siemens_Sequence')
    ]
);

console.log(data);
// {
//   PatientName: "...",
//   StudyDate: "...",
//   Modality: "...",
//   GE_PrivateCreator: "GEMS_PETD_01",
//   GE_ScannerMode: "HELICAL",
//   Siemens_Sequence: "..."
// }
```

### Using Predefined Tag Sets

Use the helper functions to extract common tag groups without typing each tag name:

```typescript
import { DicomFile, getCommonTagSets, combineTags } from '@nuxthealth/node-dicom';

const file = new DicomFile();
await file.open('./scan.dcm');

const tags = getCommonTagSets();

// Extract patient demographics
const patientData = file.extract(tags.patientBasic);
// Returns: PatientName, PatientID, PatientBirthDate, PatientSex

// Extract study information
const studyData = file.extract(tags.studyBasic);
// Returns: StudyDate, StudyTime, StudyDescription, StudyInstanceUID, StudyID

// Combine multiple tag sets
const comprehensiveData = file.extract(
    combineTags([
        tags.patientBasic,
        tags.studyBasic,
        tags.seriesBasic,
        tags.imagePixel,
        tags.ct  // Modality-specific tags
    ])
);

file.close();
```

**Available Tag Sets:**
- `patientBasic` - Core patient demographics (Name, ID, Birth Date, Sex)
- `patientExtended` - Additional patient info (Age, Size, Weight, Comments)
- `studyBasic` - Study identifiers and metadata
- `studyExtended` - Referring physician, accession number
- `seriesBasic` - Series number, description, modality, date/time
- `instanceBasic` - Instance number, creation date/time, SOP UIDs
- `imagePixel` - Image dimensions, bit depth, photometric interpretation
- `imageGeometry` - Pixel spacing, slice thickness, orientation, position
- `ct` - CT-specific (KVP, exposure, reconstruction diameter, kernel)
- `mr` - MR-specific (echo time, repetition time, flip angle, field strength)
- `us` - Ultrasound-specific tags
- `pet` - PET-specific tags
- `equipment` - Manufacturer, model, software versions, station name

### Flat Structure

**Important**: `DicomFile.extract()` always returns a flat object structure for simple, direct access to tag values:

```typescript
const data = file.extract(['PatientName', 'StudyDate', 'SeriesNumber']);
// {
//   PatientName: "DOE^JOHN",
//   StudyDate: "20231201",
//   SeriesNumber: "3"
// }
```

For hierarchical organization by study/series/instance, use the `StoreScp` `OnStudyCompleted` event which provides a structured tree.

## Helper Functions

The library provides several helper functions to simplify common DICOM operations, especially for tag extraction and configuration.

### getCommonTagSets()

Get predefined sets of commonly used DICOM tags organized by category. This eliminates the need to manually type tag names and provides well-tested tag combinations.

**Returns:** Object containing 13 different tag sets

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
- `default` - Comprehensive set (42 tags: patient, study, series, instance, pixel info, equipment)

**Example:**

```typescript
import { DicomFile, getCommonTagSets } from '@nuxthealth/node-dicom';

const file = new DicomFile();
await file.open('./scan.dcm');

const tagSets = getCommonTagSets();

// Extract patient demographics only
const patientData = file.extract(tagSets.patientBasic);
console.log('Patient:', patientData.PatientName);
console.log('ID:', patientData.PatientID);
console.log('Birth Date:', patientData.PatientBirthDate);

// Extract study metadata
const studyData = file.extract(tagSets.studyBasic);
console.log('Study:', studyData.StudyDescription);
console.log('Date:', studyData.StudyDate);

// Extract comprehensive metadata (42 common tags)
const allData = file.extract(tagSets.default);

file.close();
```

**Modality-Specific Extraction:**

```typescript
const tagSets = getCommonTagSets();

// CT workflow
const ctData = file.extract(tagSets.ct);
console.log('CT Parameters:');
console.log('  kVp:', ctData.KVP);
console.log('  Exposure Time:', ctData.ExposureTime);
console.log('  Tube Current:', ctData.XRayTubeCurrent);
console.log('  Convolution Kernel:', ctData.ConvolutionKernel);

// MR workflow
const mrData = file.extract(tagSets.mr);
console.log('MR Parameters:');
console.log('  TR:', mrData.RepetitionTime);
console.log('  TE:', mrData.EchoTime);
console.log('  Flip Angle:', mrData.FlipAngle);
console.log('  Field Strength:', mrData.MagneticFieldStrength);

// PET workflow
const petData = file.extract(tagSets.petNm);
console.log('PET Parameters:');
console.log('  Units:', petData.Units);
console.log('  Decay Correction:', petData.DecayCorrection);
```

### combineTags()

Combine multiple tag arrays into a single deduplicated array. Useful for building custom tag sets from predefined groups.

**Parameters:**
- `tagArrays: string[][]` - Array of tag name arrays to combine

**Returns:** `string[]` - Single array containing all unique tag names

**Example:**

```typescript
import { DicomFile, getCommonTagSets, combineTags } from '@nuxthealth/node-dicom';

const file = new DicomFile();
await file.open('./scan.dcm');

const tagSets = getCommonTagSets();

// Combine predefined sets with custom tags
const workflowTags = combineTags([
    tagSets.patientBasic,
    tagSets.studyBasic,
    tagSets.seriesBasic,
    ['WindowCenter', 'WindowWidth'],           // Display params
    ['RescaleIntercept', 'RescaleSlope'],     // Rescale params
    tagSets.ct                                 // CT-specific
]);

// Extract all tags at once (duplicates automatically removed)
const data = file.extract(workflowTags);

file.close();
```

**Build Reusable Configurations:**

```typescript
// config.ts - Define once, use everywhere
import { getCommonTagSets, combineTags } from '@nuxthealth/node-dicom';

const tagSets = getCommonTagSets();

export const ANONYMIZATION_TAGS = combineTags([
    tagSets.studyBasic,
    tagSets.seriesBasic,
    tagSets.instanceBasic
    // Excludes patient demographics
]);

export const ROUTING_TAGS = combineTags([
    ['StudyInstanceUID', 'SeriesInstanceUID', 'SOPInstanceUID'],
    ['Modality', 'StationName'],
    ['StudyDate', 'StudyTime']
]);

export const QA_TAGS = combineTags([
    tagSets.imagePixelInfo,
    ['WindowCenter', 'WindowWidth', 'ImageType'],
    ['BurnedInAnnotation', 'LossyImageCompression']
]);

// Use in your application
import { QA_TAGS } from './config';
const qaData = file.extract(QA_TAGS);
```

### getAvailableTagNames()

Get a comprehensive list of 300+ commonly used DICOM tag names for validation and discovery.

**Returns:** `string[]` - Array of standard DICOM tag names

**Example:**

```typescript
import { getAvailableTagNames } from '@nuxthealth/node-dicom';

// Get all available tags
const allTags = getAvailableTagNames();
console.log(`Total available: ${allTags.length} tags`);
// Output: Total available: 300+ tags

// Check if specific tag is available
const hasTag = allTags.includes('WindowCenter');
console.log('WindowCenter available:', hasTag); // true

// Validate user input
const userTags = ['PatientName', 'InvalidTag', 'StudyDate'];
const validTags = userTags.filter(tag => allTags.includes(tag));
console.log('Valid tags:', validTags); // ['PatientName', 'StudyDate']

// Find patient-related tags
const patientTags = allTags.filter(tag => 
    tag.toLowerCase().includes('patient')
);
console.log('Patient tags:', patientTags);
// ['PatientName', 'PatientID', 'PatientBirthDate', ...]

// Find all UID tags
const uidTags = allTags.filter(tag => tag.endsWith('UID'));
console.log('UID tags:', uidTags);
// ['StudyInstanceUID', 'SeriesInstanceUID', 'SOPInstanceUID', ...]
```

### createCustomTag()

Create custom tag specifications for private or vendor-specific DICOM tags.

**Parameters:**
- `tag: string` - DICOM tag in hex format (e.g., "00091001" or "(0009,1001)")
- `name: string` - Human-readable name for this tag

**Returns:** `CustomTag` object for use in extraction

**Example:**

```typescript
import { DicomFile, createCustomTag, getCommonTagSets } from '@nuxthealth/node-dicom';

const file = new DicomFile();
await file.open('./scan-with-private-tags.dcm');

// Define custom tags
const customTags = [
    createCustomTag('00091001', 'VendorSpecificID'),
    createCustomTag('00431027', 'ScannerMode'),
    createCustomTag('(0019,100A)', 'ProcessingFlags')
];

// Extract with custom tags
const data = file.extract(
    ['PatientName', 'StudyDate', 'Modality'],
    customTags
);

console.log('Standard:', data.PatientName, data.Modality);
console.log('Custom:', data.VendorSpecificID, data.ScannerMode);

file.close();
```

**Vendor-Specific Tag Libraries:**

```typescript
// vendor-tags.ts
import { createCustomTag } from '@nuxthealth/node-dicom';

export const GE_TAGS = [
    createCustomTag('00091001', 'GE_PrivateCreator'),
    createCustomTag('00091027', 'GE_ScanOptions'),
    createCustomTag('00431001', 'GE_ImageFiltering'),
    createCustomTag('00431010', 'GE_ReconstructionParams')
];

export const SIEMENS_TAGS = [
    createCustomTag('00191008', 'Siemens_ImagingMode'),
    createCustomTag('00191009', 'Siemens_SequenceInfo'),
    createCustomTag('00191010', 'Siemens_CoilID'),
    createCustomTag('0029100C', 'Siemens_CoilString')
];

export const PHILIPS_TAGS = [
    createCustomTag('20011001', 'Philips_ScanMode'),
    createCustomTag('20011003', 'Philips_ContrastEnhancement'),
    createCustomTag('20051080', 'Philips_ReconstructionParams')
];

// Dynamic selection
export function getVendorTags(manufacturer: string) {
    const vendor = manufacturer.toLowerCase();
    if (vendor.includes('ge')) return GE_TAGS;
    if (vendor.includes('siemens')) return SIEMENS_TAGS;
    if (vendor.includes('philips')) return PHILIPS_TAGS;
    return [];
}

// Usage
import { getVendorTags } from './vendor-tags';

const mfgData = file.extract(['Manufacturer']);
const vendorTags = getVendorTags(mfgData.Manufacturer);
const allData = file.extract(tagSets.default, vendorTags);

## Inspecting DICOM Files

### Dump to Console

Print a formatted view of the entire DICOM structure to stdout for quick inspection and debugging:

```typescript
const file = new DicomFile();
await file.open('./scan.dcm');

// Print all DICOM elements with tags, VRs, and values
file.dump();

// Output example:
// (0008,0005) CS SpecificCharacterSet: ISO_IR 100
// (0008,0008) CS ImageType: ORIGINAL\PRIMARY\AXIAL
// (0008,0016) UI SOPClassUID: 1.2.840.10008.5.1.4.1.1.2
// (0008,0018) UI SOPInstanceUID: 1.2.840.113619.2.55...
// (0010,0010) PN PatientName: DOE^JOHN
// (0010,0020) LO PatientID: 12345
// ...

file.close();
```

**Use cases:**
- Quick file inspection during development
- Debugging tag extraction issues
- Verifying file contents
- Finding private tags

**Note**: Output goes to stdout, not returned as string. Useful for CLI tools and debugging.

### Get Complete DICOM as JSON

Convert the entire DICOM file to JSON format without saving to disk:

```typescript
const file = new DicomFile();
await file.open('./scan.dcm');

// Get pretty-printed JSON (default)
const json = file.toJson();
const obj = JSON.parse(json);

// Access any tag by DICOM tag format
console.log(obj['(0010,0010)']);  // Patient Name
console.log(obj['(0008,0060)']);  // Modality
console.log(obj['(0020,000D)']);  // Study Instance UID

// Get compact JSON (no whitespace)
const compactJson = file.toJson(false);

file.close();
```

**JSON Format**: Follows DICOM Part 18 JSON Model standard with tags in `(GGGG,EEEE)` format.

**When to use:**
- Sending DICOM data over REST APIs
- Storing metadata in document databases (MongoDB, CouchDB)
- Creating web-friendly representations
- Converting between formats
- Quick inspection without file I/O

**Performance**: Synchronous operation, much faster than `saveAsJson()` for in-memory use.

### Comparison: `toJson()` vs `extract()`

```typescript
// toJson() - Complete DICOM dataset as JSON string
const fullJson = file.toJson();
const parsed = JSON.parse(fullJson);
// Contains ALL tags in DICOM Part 18 format
// Good for: full backups, format conversion, complete metadata

// extract() - Specific tags as flat object
const specific = file.extract(['PatientName', 'StudyDate']);
// Contains ONLY requested tags with simple key-value pairs
// Good for: targeted extraction, performance, type-safe access
```

## Updating DICOM Tags

### Basic Tag Updates

The `updateTags()` method modifies DICOM tag values in memory. Changes must be saved with `saveAsDicom()` to persist.

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';

const file = new DicomFile();
await file.open('original.dcm');

// Update single or multiple tags
file.updateTags({
    PatientName: 'DOE^JANE',
    PatientID: 'PAT12345',
    StudyDescription: 'Updated Study'
});

// Save changes to new file
await file.saveAsDicom('modified.dcm');
file.close();
```

### DICOM Anonymization

Complete example for anonymizing patient data:

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';
import crypto from 'crypto';

async function anonymizeDicom(inputPath: string, outputPath: string) {
    const file = new DicomFile();
    await file.open(inputPath);
    
    // Remove/anonymize patient identifying information
    file.updateTags({
        // Patient Module
        PatientName: 'ANONYMOUS',
        PatientID: crypto.randomUUID(),
        PatientBirthDate: '',
        PatientSex: '',
        PatientAge: '',
        PatientWeight: '',
        PatientSize: '',
        PatientComments: '',
        OtherPatientIDs: '',
        OtherPatientNames: '',
        
        // Physician/Institution
        InstitutionName: 'ANONYMIZED',
        ReferringPhysicianName: '',
        PerformingPhysicianName: '',
        NameOfPhysiciansReadingStudy: '',
        OperatorsName: '',
        
        // Study/Series Descriptions (optional - may contain PHI)
        StudyDescription: 'ANONYMIZED',
        SeriesDescription: 'ANONYMIZED',
        
        // Dates (optional - can shift instead of removing)
        StudyDate: '',
        SeriesDate: '',
        AcquisitionDate: '',
        ContentDate: ''
    });
    
    await file.saveAsDicom(outputPath);
    file.close();
    
    console.log(`Anonymized: ${inputPath} → ${outputPath}`);
}

// Use it
await anonymizeDicom('patient-scan.dcm', 'anonymous-scan.dcm');
```

### Date Shifting (Preserve Temporal Relationships)

Instead of removing dates, shift them by a consistent offset:

```typescript
function shiftDate(dateStr: string, dayOffset: number): string {
    if (!dateStr || dateStr.length !== 8) return '';
    
    const year = parseInt(dateStr.substring(0, 4));
    const month = parseInt(dateStr.substring(4, 6)) - 1;
    const day = parseInt(dateStr.substring(6, 8));
    
    const date = new Date(year, month, day);
    date.setDate(date.getDate() + dayOffset);
    
    const newYear = date.getFullYear().toString();
    const newMonth = (date.getMonth() + 1).toString().padStart(2, '0');
    const newDay = date.getDate().toString().padStart(2, '0');
    
    return `${newYear}${newMonth}${newDay}`;
}

const file = new DicomFile();
await file.open('scan.dcm');

// Extract original dates
const data = file.extract(['StudyDate', 'SeriesDate', 'AcquisitionDate']);

// Shift all dates by -365 days (1 year back)
file.updateTags({
    StudyDate: shiftDate(data.StudyDate, -365),
    SeriesDate: shiftDate(data.SeriesDate, -365),
    AcquisitionDate: shiftDate(data.AcquisitionDate, -365)
});

await file.saveAsDicom('date-shifted.dcm');
file.close();
```

### Update Using Hex Tag Format

```typescript
const file = new DicomFile();
await file.open('scan.dcm');

// Use hex format for tag identifiers
file.updateTags({
    '00100010': 'DOE^JOHN',        // PatientName
    '00100020': 'PAT12345',         // PatientID
    '00080020': '20240101',         // StudyDate
    '00080030': '120000'            // StudyTime
});

await file.saveAsDicom('updated.dcm');
file.close();
```

### Batch Processing Multiple Files

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';
import { readdir } from 'fs/promises';
import path from 'path';

async function batchAnonymize(inputDir: string, outputDir: string) {
    const files = await readdir(inputDir);
    const dcmFiles = files.filter(f => f.endsWith('.dcm'));
    
    for (const filename of dcmFiles) {
        const file = new DicomFile();
        const inputPath = path.join(inputDir, filename);
        const outputPath = path.join(outputDir, filename);
        
        try {
            await file.open(inputPath);
            
            file.updateTags({
                PatientName: 'ANONYMOUS',
                PatientID: crypto.randomUUID(),
                PatientBirthDate: '',
                InstitutionName: 'ANONYMIZED'
            });
            
            await file.saveAsDicom(outputPath);
            console.log(`✓ ${filename}`);
        } catch (err) {
            console.error(`✗ ${filename}:`, err.message);
        } finally {
            file.close();
        }
    }
}

await batchAnonymize('./dicom-input', './dicom-output');
```

### Important Notes

**Restrictions:**
- Cannot modify file meta information tags (group 0x0002)
- Cannot modify pixel data - use pixel processing methods instead
- Changes are in-memory only until `saveAsDicom()` is called

**Value Representation (VR):**
- VR is automatically determined from DICOM data dictionary
- For existing tags, original VR is preserved
- For new tags, appropriate VR is inferred
- Empty strings clear tag values

**Tag Formats Supported:**
- Standard names: `'PatientName'`, `'StudyDate'`
- Hex format: `'00100010'`, `'00080020'`
- Tag notation: `'(0010,0010)'`, `'(0008,0020)'`


## Working with Pixel Data

DICOM files contain medical images as pixel data, which can be compressed or uncompressed. The `DicomFile` class provides multiple methods for accessing and processing this data.

### Understanding Pixel Data

First, get information about the pixel data format:

```typescript
const file = new DicomFile();
await file.open('./image.dcm');

const info = file.getPixelDataInfo();

console.log('Image Dimensions:', info.width, 'x', info.height);
console.log('Number of Frames:', info.frames);
console.log('Bits Allocated:', info.bitsAllocated);
console.log('Bits Stored:', info.bitsStored);
console.log('Samples per Pixel:', info.samplesPerPixel);
console.log('Photometric Interpretation:', info.photometricInterpretation);
console.log('Compressed:', info.isCompressed);
console.log('Transfer Syntax UID:', info.transferSyntaxUID);

// Optional windowing parameters (common in CT)
if (info.windowCenter && info.windowWidth) {
    console.log('Window Center/Width:', info.windowCenter, '/', info.windowWidth);
}

// Optional rescale parameters (for Hounsfield units)
if (info.rescaleIntercept && info.rescaleSlope) {
    console.log('Rescale Slope/Intercept:', info.rescaleSlope, '/', info.rescaleIntercept);
}

file.close();
```

**PixelDataInfo Properties:**
- `width`, `height` - Image dimensions in pixels
- `frames` - Number of frames (1 for single images, >1 for cine/3D)
- `bitsAllocated` - Bits per pixel value (typically 8 or 16)
- `bitsStored` - Actual significant bits
- `highBit` - Most significant bit position
- `pixelRepresentation` - 0=unsigned, 1=signed
- `samplesPerPixel` - 1=grayscale, 3=RGB
- `photometricInterpretation` - Color space (MONOCHROME1, MONOCHROME2, RGB, etc.)
- `transferSyntaxUID` - Encoding format
- `isCompressed` - Whether pixel data is compressed
- `dataSize` - Size of pixel data in bytes
- `windowCenter`, `windowWidth` - Display windowing (optional)
- `rescaleSlope`, `rescaleIntercept` - Value transformation (optional)

### Get Pixel Data as Buffer

Extract raw pixel data directly as a Node.js Buffer for in-memory processing:

```typescript
const file = new DicomFile();
await file.open('./image.dcm');

// Get raw pixel data (may be compressed)
const pixelBuffer = file.getPixelData();
console.log(`Pixel data size: ${pixelBuffer.length} bytes`);

// Get image info
const info = file.getPixelDataInfo();
console.log(`Dimensions: ${info.width} x ${info.height}`);

// Process the buffer
if (!info.isCompressed) {
    // For uncompressed data, calculate expected size
    const bytesPerPixel = Math.ceil(info.bitsAllocated / 8);
    const expectedSize = info.width * info.height * info.frames * 
                         info.samplesPerPixel * bytesPerPixel;
    console.log(`Expected size: ${expectedSize} bytes`);
    
    // Access pixel values
    processRawPixels(pixelBuffer, info);
}

file.close();
```

**Important Notes:**
- Returns data as-is from the DICOM file
- For compressed images, returns compressed bitstream
- For uncompressed images, returns raw pixel values
- No decompression or format conversion
- Fast, synchronous operation

**Use cases:**
- Custom image processing pipelines
- Extracting data without file I/O
- Memory-efficient workflows
- Integration with image processing libraries

### Get Decoded Pixel Data

Automatically decompress and decode pixel data:

```typescript
const file = new DicomFile();
await file.open('./compressed-ct.dcm');

const info = file.getPixelDataInfo();
console.log(`Transfer Syntax: ${info.transferSyntaxUID}`);
console.log(`Compressed: ${info.isCompressed}`);

if (info.isCompressed) {
    // Automatically decompress
    const decodedBuffer = file.getDecodedPixelData();
    
    console.log(`Original (compressed): ${info.dataSize} bytes`);
    console.log(`Decoded (uncompressed): ${decodedBuffer.length} bytes`);
    
    // Now work with uncompressed pixel values
    renderImage(decodedBuffer, info.width, info.height, info.bitsAllocated);
} else {
    // Already uncompressed, use getPixelData() for efficiency
    const pixelBuffer = file.getPixelData();
    renderImage(pixelBuffer, info.width, info.height, info.bitsAllocated);
}

file.close();
```

**Requirements:**
- Requires the `transcode` feature to be enabled at build time
- Throws error if feature not available
- Supports common compression formats (JPEG, JPEG 2000, RLE)

**Common Transfer Syntaxes:**
- `1.2.840.10008.1.2` - Implicit VR Little Endian (uncompressed)
- `1.2.840.10008.1.2.1` - Explicit VR Little Endian (uncompressed)
- `1.2.840.10008.1.2.4.50` - JPEG Baseline (compressed)
- `1.2.840.10008.1.2.4.90` - JPEG 2000 (compressed)
- `1.2.840.10008.1.2.5` - RLE Lossless (compressed)

### Get Processed Pixel Data

Get decoded pixel data with advanced processing options - windowing, frame extraction, and 8-bit conversion, all in-memory:

```typescript
const file = new DicomFile();
await file.open('./ct-scan.dcm');

// Example 1: Apply windowing from file metadata + convert to 8-bit
const displayReady = file.getProcessedPixelData({
    applyVoiLut: true,      // Use WindowCenter/Width from file
    convertTo8bit: true      // Convert to 8-bit for display (0-255)
});
// Perfect for rendering to canvas or display

// Example 2: Custom window for specific tissue visualization
const boneWindow = file.getProcessedPixelData({
    windowCenter: 300,       // HU center for bone
    windowWidth: 1500,       // HU width for bone
    convertTo8bit: true
});

const softTissueWindow = file.getProcessedPixelData({
    windowCenter: 40,        // HU center for soft tissue
    windowWidth: 400,        // HU width for soft tissue
    convertTo8bit: true
});

// Example 3: Extract specific frame from multi-frame image
const frame5 = file.getProcessedPixelData({
    frameNumber: 5           // 0-based frame index
});

// Example 4: Complete processing pipeline
const processed = file.getProcessedPixelData({
    frameNumber: 0,          // First frame
    windowCenter: 40,        // Custom window
    windowWidth: 400,
    convertTo8bit: true      // Ready for display
});

// Can render directly
renderToCanvas(processed, info.width, info.height);

file.close();
```

**Processing Pipeline:**
1. **Decode**: Decompress pixel data (if compressed)
2. **Frame Extraction**: Extract specific frame (if multi-frame)
3. **Windowing/VOI LUT**: Apply window level/width for contrast adjustment
4. **8-bit Conversion**: Scale to 0-255 range for display

**Options:**
- `frameNumber` - Extract specific frame (0-based index)
- `applyVoiLut` - Use WindowCenter/Width from file metadata
- `windowCenter` - Custom window center (overrides file metadata)
- `windowWidth` - Custom window width (overrides file metadata)
- `convertTo8bit` - Convert to 8-bit grayscale (0-255)

**Windowing Parameters:**

Common CT windowing presets:
```typescript
// Brain window
const brain = file.getProcessedPixelData({
    windowCenter: 40,
    windowWidth: 80,
    convertTo8bit: true
});

// Lung window
const lung = file.getProcessedPixelData({
    windowCenter: -600,
    windowWidth: 1500,
    convertTo8bit: true
});

// Bone window
const bone = file.getProcessedPixelData({
    windowCenter: 300,
    windowWidth: 1500,
    convertTo8bit: true
});

// Soft tissue window
const softTissue = file.getProcessedPixelData({
    windowCenter: 40,
    windowWidth: 400,
    convertTo8bit: true
});
```

**Requirements:**
- Requires the `transcode` feature
- Works with 16-bit images (most medical imaging)
- Automatically applies rescale slope/intercept if present (Hounsfield units)

**Use Cases:**
- **Web viewers**: Get display-ready 8-bit images
- **Multi-window display**: Generate multiple windowed versions
- **Frame extraction**: Extract frames from cine loops or 3D volumes
- **Custom visualization**: Apply specific window settings for different tissues
- **Real-time adjustment**: Fast in-memory processing without file I/O

### Save Pixel Data to File

Save raw pixel data directly to a file (synchronous operation):

```typescript
const file = new DicomFile();
await file.open('./image.dcm');

// Save pixel data to file
const result = file.saveRawPixelData('./output/pixels.raw');
console.log(result);  // "Pixel data saved successfully (2097152 bytes)"

file.close();
```

**Characteristics:**
- Synchronous operation (returns immediately)
- Saves raw pixel data as-is
- No decompression or format conversion
- For compressed data, saves the compressed bitstream
- Returns success message with byte count

**Use cases:**
- Exporting pixel data for external processing
- Creating raw image files for analysis tools
- Batch extraction workflows
- Archiving pixel data separately from DICOM headers

### Method Comparison

| Method | Returns | File I/O | Decompression | Processing | Use Case |
|--------|---------|----------|---------------|------------|----------|
| `getPixelData()` | `Buffer` | No | No | No | Raw data extraction, fastest |
| `getDecodedPixelData()` | `Buffer` | No | Yes* | No | Decompress compressed images |
| `getProcessedPixelData()` | `Buffer` | No | Yes* | Yes | Windowing, frame extraction, 8-bit conversion |
| `getImageBuffer()` | `Buffer` | No | Yes* | Yes | Encoded PNG/JPEG/BMP bytes in-memory, no disk I/O |
| `saveRawPixelData()` | `string` | Yes | No | No | Export raw data to file |
| `processPixelData()` | `Promise<string>` | Yes | Yes* | Yes | Advanced processing with file output |

\* Requires `transcode` feature

### Advanced Image Rendering with processPixelData()

The `processPixelData()` method provides comprehensive image rendering capabilities, converting DICOM pixel data to standard image formats (JPEG, PNG, BMP) with full VOI LUT support, windowing, and format conversion.

**Method Signature:**

```typescript
await file.processPixelData(options: {
    outputPath: string,           // Output file path
    format: 'Jpeg' | 'Png' | 'Bmp',  // Output format
    decode?: boolean,             // Decompress if needed (default: false)
    width?: number,               // Output width (maintains aspect ratio)
    height?: number,              // Output height (maintains aspect ratio)
    quality?: number,             // JPEG quality 1-100 (default: 90)
    windowCenter?: number,        // Manual window center
    windowWidth?: number,         // Manual window width
    applyVoiLut?: boolean,       // Use WindowCenter/Width from file
    rescaleIntercept?: number,   // Override rescale intercept
    rescaleSlope?: number,       // Override rescale slope
    convertTo8bit?: boolean,     // Force 8-bit output
    frameNumber?: number         // Extract specific frame (0-based)
}): Promise<string>
```

**Basic Image Rendering:**

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';

const file = new DicomFile();
await file.open('./ct-scan.dcm');

// Default JPEG rendering with automatic VOI LUT
await file.processPixelData({
    outputPath: 'output.jpg',
    format: 'Jpeg',
    decode: true,
    applyVoiLut: true  // Use WindowCenter/Width from file
});

// High-quality PNG
await file.processPixelData({
    outputPath: 'output.png',
    format: 'Png',
    decode: true,
    applyVoiLut: true
});

file.close();
```

**Manual Windowing for CT:**

```typescript
const file = new DicomFile();
await file.open('./ct-abdomen.dcm');

// Soft tissue window (C=40, W=400)
await file.processPixelData({
    outputPath: 'soft-tissue.jpg',
    format: 'Jpeg',
    decode: true,
    windowCenter: 40,
    windowWidth: 400,
    quality: 90
});

// Lung window (C=-600, W=1500)
await file.processPixelData({
    outputPath: 'lung.jpg',
    format: 'Jpeg',
    decode: true,
    windowCenter: -600,
    windowWidth: 1500
});

// Bone window (C=300, W=1500)
await file.processPixelData({
    outputPath: 'bone.jpg',
    format: 'Jpeg',
    decode: true,
    windowCenter: 300,
    windowWidth: 1500
});

file.close();
```

**Common CT Windowing Presets:**

| Preset | Center | Width | Use Case |
|--------|--------|-------|----------|
| Soft Tissue | 40 | 400 | Abdomen, pelvis, general soft tissue |
| Lung | -600 | 1500 | Chest, lung parenchyma |
| Bone | 300 | 1500 | Skeletal structures, fractures |
| Brain | 40 | 80 | Head CT, intracranial structures |
| Liver | 60 | 160 | Liver parenchyma |

**Viewport Transformation:**

```typescript
// Resize to specific dimensions (maintains aspect ratio)
await file.processPixelData({
    outputPath: 'thumbnail.jpg',
    format: 'Jpeg',
    decode: true,
    width: 256,
    height: 256,
    applyVoiLut: true
});

// Web-friendly size with custom windowing
await file.processPixelData({
    outputPath: 'web-preview.jpg',
    format: 'Jpeg',
    decode: true,
    width: 512,
    height: 512,
    windowCenter: 40,
    windowWidth: 400,
    quality: 85
});
```

**Frame Extraction from Multi-frame Images:**

```typescript
const file = new DicomFile();
await file.open('./cine-loop.dcm');

const info = file.getPixelDataInfo();
console.log(`Total frames: ${info.frames}`);

// Extract specific frames
for (let i = 0; i < info.frames; i++) {
    await file.processPixelData({
        outputPath: `frame-${i.toString().padStart(4, '0')}.jpg`,
        format: 'Jpeg',
        decode: true,
        frameNumber: i,
        applyVoiLut: true,
        quality: 90
    });
}

file.close();
```

**8-bit Conversion for Display:**

```typescript
// Convert 16-bit DICOM to 8-bit image
await file.processPixelData({
    outputPath: '8bit-display.jpg',
    format: 'Jpeg',
    decode: true,
    convertTo8bit: true,
    applyVoiLut: true
});
```

**Batch Processing Multiple Files:**

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';
import { readdir } from 'fs/promises';
import path from 'path';

async function batchRender(inputDir: string, outputDir: string) {
    const files = await readdir(inputDir);
    const dcmFiles = files.filter(f => f.endsWith('.dcm'));
    
    for (const filename of dcmFiles) {
        const file = new DicomFile();
        const inputPath = path.join(inputDir, filename);
        const outputPath = path.join(outputDir, filename.replace('.dcm', '.jpg'));
        
        try {
            await file.open(inputPath);
            
            await file.processPixelData({
                outputPath,
                format: 'Jpeg',
                decode: true,
                applyVoiLut: true,
                quality: 90
            });
            
            console.log(`✓ ${filename}`);
        } catch (err) {
            console.error(`✗ ${filename}:`, err.message);
        } finally {
            file.close();
        }
    }
}

await batchRender('./dicom-input', './jpeg-output');
```

**Complete Rendering Workflow:**

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';

async function renderWithPresets(dicomPath: string, outputDir: string) {
    const file = new DicomFile();
    await file.open(dicomPath);
    
    const info = file.getPixelDataInfo();
    const basename = path.basename(dicomPath, '.dcm');
    
    console.log('Rendering:', dicomPath);
    console.log(`  Dimensions: ${info.width}x${info.height}`);
    console.log(`  Frames: ${info.frames}`);
    console.log(`  Compressed: ${info.isCompressed}`);
    
    // Automatic VOI LUT
    await file.processPixelData({
        outputPath: path.join(outputDir, `${basename}-auto.jpg`),
        format: 'Jpeg',
        decode: true,
        applyVoiLut: true,
        quality: 90
    });
    
    // CT presets
    if (info.windowCenter && info.rescaleIntercept !== undefined) {
        const presets = [
            { name: 'soft-tissue', center: 40, width: 400 },
            { name: 'lung', center: -600, width: 1500 },
            { name: 'bone', center: 300, width: 1500 }
        ];
        
        for (const preset of presets) {
            await file.processPixelData({
                outputPath: path.join(outputDir, `${basename}-${preset.name}.jpg`),
                format: 'Jpeg',
                decode: true,
                windowCenter: preset.center,
                windowWidth: preset.width,
                quality: 90
            });
        }
    }
    
    // Thumbnail
    await file.processPixelData({
        outputPath: path.join(outputDir, `${basename}-thumb.jpg`),
        format: 'Jpeg',
        decode: true,
        width: 256,
        height: 256,
        applyVoiLut: true,
        quality: 85
    });
    
    file.close();
    console.log('✓ Rendering complete');
}

await renderWithPresets('./ct-scan.dcm', './output');
```

**Image Processing Features:**

- **VOI LUT Support**: Automatic application of WindowCenter/WindowWidth from DICOM files
- **Rescale Parameters**: Support for RescaleIntercept and RescaleSlope (Hounsfield units in CT)
- **Frame Extraction**: Extract specific frames from multi-frame DICOM images
- **8-bit Conversion**: Automatic windowing and conversion to 8-bit for display
- **Signed/Unsigned Pixels**: Proper handling of pixel representation
- **Format Conversion**: JPEG, PNG, or BMP output
- **Quality Control**: Configurable JPEG compression quality
- **Viewport Transformation**: Resize with aspect ratio preservation using high-quality Lanczos3 resampling

**Important Notes:**

- Requires `transcode` feature to be enabled at build time
- Manual windowing parameters override automatic VOI LUT
- Viewport transformation maintains aspect ratio (image may not fill entire viewport)
- JPEG quality affects file size and image quality (90 recommended for diagnostic use)
- PNG format is lossless but produces larger files

### Complete Pixel Data Example

```typescript
import { DicomFile } from '@nuxthealth/node-dicom';

async function analyzePixelData(filePath: string) {
    const file = new DicomFile();
    
    try {
        await file.open(filePath);
        
        // Step 1: Get metadata
        const info = file.getPixelDataInfo();
        console.log('=== Image Information ===');
        console.log(`Dimensions: ${info.width} x ${info.height}`);
        console.log(`Frames: ${info.frames}`);
        console.log(`Bits: ${info.bitsAllocated}/${info.bitsStored}`);
        console.log(`Photometric: ${info.photometricInterpretation}`);
        console.log(`Compressed: ${info.isCompressed}`);
        console.log(`Transfer Syntax: ${info.transferSyntaxUID}`);
        
        // Step 2: Get pixel data based on needs
        
        // Option A: Raw data (fastest)
        const rawBuffer = file.getPixelData();
        console.log(`\n=== Raw Data ===`);
        console.log(`Size: ${rawBuffer.length} bytes`);
        
        // Option B: Decoded data (if compressed)
        if (info.isCompressed) {
            const decodedBuffer = file.getDecodedPixelData();
            console.log(`\n=== Decoded Data ===`);
            console.log(`Compressed: ${info.dataSize} bytes`);
            console.log(`Decompressed: ${decodedBuffer.length} bytes`);
        }
        
        // Option C: Processed data with windowing (NEW!)
        if (info.windowCenter && info.windowWidth) {
            console.log(`\n=== Windowed Display ===`);
            console.log(`Using file windowing: C=${info.windowCenter} W=${info.windowWidth}`);
            
            // Get display-ready 8-bit image
            const displayBuffer = file.getProcessedPixelData({
                applyVoiLut: true,
                convertTo8bit: true
            });
            console.log(`Display buffer: ${displayBuffer.length} bytes (8-bit)`);
            
            // Render to canvas or create image
            renderToCanvas(displayBuffer, info.width, info.height);
        }
        
        // Option D: Multiple windowing presets for CT
        if (info.rescaleIntercept !== undefined) {
            console.log(`\n=== CT Windowing Presets ===`);
            
            const presets = [
                { name: 'Soft Tissue', center: 40, width: 400 },
                { name: 'Lung', center: -600, width: 1500 },
                { name: 'Bone', center: 300, width: 1500 },
                { name: 'Brain', center: 40, width: 80 }
            ];
            
            for (const preset of presets) {
                const windowed = file.getProcessedPixelData({
                    windowCenter: preset.center,
                    windowWidth: preset.width,
                    convertTo8bit: true
                });
                console.log(`${preset.name} window: ${windowed.length} bytes`);
                saveAsImage(`${preset.name.toLowerCase()}.png`, windowed, info.width, info.height);
            }
        }
        
        // Option E: Frame extraction from multi-frame
        if (info.frames > 1) {
            console.log(`\n=== Frame Extraction ===`);
            console.log(`Total frames: ${info.frames}`);
            
            // Extract middle frame
            const middleFrame = Math.floor(info.frames / 2);
            const frameBuffer = file.getProcessedPixelData({
                frameNumber: middleFrame,
                applyVoiLut: true,
                convertTo8bit: true
            });
            console.log(`Extracted frame ${middleFrame}: ${frameBuffer.length} bytes`);
        }
        
        // Step 3: Calculate statistics on raw data
        console.log('\n=== Pixel Statistics ===');
        const decodedForStats = info.isCompressed 
            ? file.getDecodedPixelData() 
            : file.getPixelData();
        
        const bytesPerPixel = Math.ceil(info.bitsAllocated / 8);
        const pixelCount = info.width * info.height * info.frames;
        
        let min = Infinity;
        let max = -Infinity;
        let sum = 0;
        
        // Read pixel values (16-bit example)
        if (info.bitsAllocated === 16) {
            for (let i = 0; i < pixelCount; i++) {
                const value = decodedForStats.readUInt16LE(i * 2);
                if (value < min) min = value;
                if (value > max) max = value;
                sum += value;
            }
            
            const mean = sum / pixelCount;
            console.log(`Min: ${min}, Max: ${max}, Mean: ${mean.toFixed(2)}`);
            
            // Apply rescale for Hounsfield units
            if (info.rescaleSlope && info.rescaleIntercept) {
                const huMin = min * info.rescaleSlope + info.rescaleIntercept;
                const huMax = max * info.rescaleSlope + info.rescaleIntercept;
                const huMean = mean * info.rescaleSlope + info.rescaleIntercept;
                console.log(`Hounsfield Units: ${huMin.toFixed(1)} to ${huMax.toFixed(1)} HU (mean: ${huMean.toFixed(1)})`);
            }
        }
        
    } finally {
        file.close();
    }
}

// Helper function to render to HTML canvas
function renderToCanvas(buffer: Buffer, width: number, height: number) {
    // Assuming browser environment with canvas
    const canvas = document.getElementById('dicomCanvas') as HTMLCanvasElement;
    canvas.width = width;
    canvas.height = height;
    
    const ctx = canvas.getContext('2d')!;
    const imageData = ctx.createImageData(width, height);
    
    // Convert grayscale to RGBA
    for (let i = 0; i < buffer.length; i++) {
        const pixelValue = buffer[i];
        imageData.data[i * 4] = pixelValue;     // R
        imageData.data[i * 4 + 1] = pixelValue; // G
        imageData.data[i * 4 + 2] = pixelValue; // B
        imageData.data[i * 4 + 3] = 255;        // A
    }
    
    ctx.putImageData(imageData, 0, 0);
}

// Usage
await analyzePixelData('./ct-scan.dcm');
```

### Get Encoded Image as Buffer

`getImageBuffer()` returns a fully encoded PNG, JPEG, or BMP image as a Node.js `Buffer` without writing anything to disk. This is convenient for serving images over HTTP, storing blobs in a database, or piping to downstream processing without temporary files.

**Method Signature:**

```typescript
file.getImageBuffer(options?: {
    format?: 'Png' | 'Jpeg' | 'Bmp';  // default: 'Png'
    applyVoiLut?: boolean;             // use WindowCenter/Width from file
    windowCenter?: number;             // manual window center (overrides VOI LUT)
    windowWidth?: number;              // manual window width (overrides VOI LUT)
    frameNumber?: number;              // 0-based frame index for multi-frame
    convertTo8bit?: boolean;           // scale to 0-255 for display
    quality?: number;                  // JPEG quality 1-100 (default: 90), ignored for PNG/BMP
}): Buffer
```

**Basic Usage:**

```typescript
const file = new DicomFile();
await file.open('./ct-scan.dcm');

// Default: PNG with no windowing
const pngBuffer = file.getImageBuffer();
console.log(`PNG size: ${pngBuffer.length} bytes`);

// JPEG with VOI LUT from the file header
const jpegBuffer = file.getImageBuffer({
    format: 'Jpeg',
    applyVoiLut: true,
    convertTo8bit: true,
    quality: 90
});

// BMP displayed with custom bone window
const bmpBuffer = file.getImageBuffer({
    format: 'Bmp',
    windowCenter: 300,
    windowWidth: 1500,
    convertTo8bit: true
});

file.close();
```

**HTTP Response (Express / H3):**

```typescript
import express from 'express';
import { DicomFile } from '@nuxthealth/node-dicom';

app.get('/image/:instanceUid', async (req, res) => {
    const file = new DicomFile();
    await file.open(`/data/${req.params.instanceUid}.dcm`);

    const buf = file.getImageBuffer({
        format: 'Jpeg',
        applyVoiLut: true,
        convertTo8bit: true,
        quality: 85
    });

    file.close();

    res.setHeader('Content-Type', 'image/jpeg');
    res.setHeader('Content-Length', buf.length);
    res.end(buf);
});
```

**Multi-frame: serve a specific frame:**

```typescript
const info = file.getPixelDataInfo();

for (let i = 0; i < info.frames; i++) {
    const frame = file.getImageBuffer({
        format: 'Png',
        frameNumber: i,
        applyVoiLut: true,
        convertTo8bit: true
    });
    await uploadToStorage(`frames/${i}.png`, frame);
}
```

**Requirements:** Requires the `transcode` feature. Throws for `Raw` or `Json` format values (use `getPixelData()` or `processPixelData()` instead).

---

## Working with Binary Tags and Encapsulated Documents

Standard `extract()` calls use `to_str()` internally and are not safe for binary payloads (OB, OW, UN, etc.). Three dedicated methods cover this use case.

### Inspecting Tag Metadata: `getTagInfo()`

Returns type metadata for any tag without reading all its bytes. Use this to determine whether to call `extract()` (for string-representable VRs) or `getTagBytes()` / `getEncapsulatedDocument()` (for binary payloads).

**Return type `TagDataInfo`:**

| Field | Type | Description |
|-------|------|-------------|
| `vr` | `string` | DICOM Value Representation (e.g. `"OB"`, `"LO"`) |
| `isBinary` | `boolean` | `true` for OB/OW/OF/OD/OL/OV/UN |
| `isImage` | `boolean` | `true` only when the tag is PixelData (7FE0,0010) **and** `BitsAllocated`, `Rows`, and `Columns` are also present — i.e. a real image object. `false` when PixelData holds a non-image payload such as raw bytes or text. |
| `mimeType` | `string \| null` | MIME type from (0042,0012), populated when tag is (0042,0011) |
| `byteLength` | `number` | Raw byte count of the payload |

```typescript
const file = new DicomFile();
await file.open('./report.dcm');

const info = file.getTagInfo('EncapsulatedDocument');
console.log(info.vr);        // 'OB'
console.log(info.isBinary);  // true
console.log(info.isImage);   // false
console.log(info.mimeType);  // 'application/pdf'
console.log(info.byteLength); // e.g. 245760

// Decision logic
if (info.isBinary && !info.isImage) {
    const bytes = file.getTagBytes('EncapsulatedDocument');
    // do something with bytes
}

file.close();
```

Tag name formats accepted (same as `extract()`):
- Standard name: `'EncapsulatedDocument'`
- Hex string: `'00420011'`
- DICOM notation: `'(0042,0011)'`

### Reading Raw Binary Bytes: `getTagBytes()`

Binary-safe alternative to `extract()`. Returns the raw tag payload as a `Buffer` for any VR, including compressed private tags, embedded documents, or unknown binary content (UN).

```typescript
const file = new DicomFile();
await file.open('./scan-with-private.dcm');

// Read an embedded PDF directly
const pdfBytes = file.getTagBytes('EncapsulatedDocument');
fs.writeFileSync('embedded.pdf', pdfBytes);

// Read a private binary tag by hex address
const vendorBlob = file.getTagBytes('00991001');
processVendorData(vendorBlob);

file.close();
```

**Notes:**
- Works for any DICOM tag regardless of VR — including non-binary ones.
- For multi-fragment pixel data, returns the raw concatenated bytes (same as `getPixelData()`).
- Does not decompress or decode anything.

### Extracting Encapsulated Documents: `getEncapsulatedDocument()`

The idiomatic API for DICOM-encapsulated PDFs, CDA structured reports, and text documents. Reads both the binary payload (tag `0042,0011 – EncapsulatedDocument`) and the MIME type (tag `0042,0012 – MIMETypeOfEncapsulatedDocument`) in one call.

**Return type `EncapsulatedDocumentData`:**

| Field | Type | Description |
|-------|------|-------------|
| `mimeType` | `string` | MIME type (defaults to `"application/octet-stream"` if tag absent) |
| `data` | `Buffer` | Raw document bytes |
| `byteLength` | `number` | Byte count of the document |
| `documentTitle` | `string \| null` | Content date from DICOM header (tag 0008,0023), if present |

```typescript
const file = new DicomFile();
await file.open('./structured-report.dcm');

const doc = file.getEncapsulatedDocument();

console.log(doc.mimeType);    // 'application/pdf'
console.log(doc.byteLength);  // e.g. 102400

// Save to disk
fs.writeFileSync('report.pdf', doc.data);

file.close();
```

**HTTP response example:**

```typescript
app.get('/report/:uid', async (req, res) => {
    const file = new DicomFile();
    await file.open(`/data/${req.params.uid}.dcm`);
    const doc = file.getEncapsulatedDocument();
    file.close();

    res.setHeader('Content-Type', doc.mimeType);
    res.setHeader('Content-Disposition', 'inline; filename="report.pdf"');
    res.end(doc.data);
});
```

**Distinguishing image vs. document files:**

```typescript
async function classifyDicom(path: string) {
    const file = new DicomFile();
    await file.open(path);

    try {
        const pixInfo = file.getTagInfo('PixelData');
        if (pixInfo.isImage) {
            console.log('This is an image file');
            const img = file.getImageBuffer({ format: 'Jpeg', applyVoiLut: true, convertTo8bit: true });
            // handle image...
            return;
        }
    } catch {
        // no pixel data tag
    }

    try {
        const doc = file.getEncapsulatedDocument();
        console.log(`Encapsulated document: ${doc.mimeType} (${doc.byteLength} bytes)`);
        // handle document...
    } catch {
        console.log('No encapsulated document found');
    }

    file.close();
}
```

### Special Case: Text or Binary Blob Stored in the PixelData Tag

Some non-standard or vendor-specific DICOM files store raw text or binary blobs directly in the PixelData element (7FE0,0010) instead of actual image pixel data. This happens when, for example, a proprietary system places a report attachment or a custom binary payload into that tag using a non-image SOP class.

**How `isImage` detects this:**

`getTagInfo('PixelData')` checks for the presence of the three mandatory image attributes alongside PixelData:
- `BitsAllocated` (0028,0100)
- `Rows` (0028,0010)
- `Columns` (0028,0011)

A real image always has all three. A PixelData tag used to store a blob or text will lack them, so `isImage` is `false` even though `isBinary` is `true`.

```typescript
const file = new DicomFile();
await file.open('./custom-blob.dcm');

const info = file.getTagInfo('PixelData');
console.log(info.vr);         // 'OB' or 'OW'
console.log(info.isBinary);   // true  — raw bytes
console.log(info.isImage);    // false — no BitsAllocated/Rows/Columns
console.log(info.byteLength); // e.g. 4096

if (info.isBinary && !info.isImage) {
    const buf = file.getTagBytes('PixelData');

    // If you know it is UTF-8 text:
    const text = buf.toString('utf8');
    console.log(text);

    // Or save the raw bytes:
    fs.writeFileSync('payload.bin', buf);
}

file.close();
```

**Important notes:**
- `getImageBuffer()` and `processPixelData()` will still attempt to decode the bytes as an image and will throw. Always gate on `isImage` before calling those methods.
- `getTagBytes('PixelData')` and `getPixelData()` are identical in behaviour — both return the raw bytes regardless of content.

---

## Saving Files

The `DicomFile` class can save DICOM data in multiple formats to filesystem or S3.

### Save as DICOM Binary

Save the currently opened DICOM file (regardless of how it was opened) as a standard binary DICOM file:

```typescript
const file = new DicomFile();

// Can open from JSON and save as DICOM
await file.openJson('input.json');
await file.saveAsDicom('output.dcm');

// Or open DICOM and save to new location
await file.open('input.dcm');
await file.saveAsDicom('modified.dcm');

file.close();
```

**Use cases:**
- Converting DICOM JSON back to binary format
- Creating copies or backups
- Saving after modifications
- Migrating between storage backends

**S3 Support:**
```typescript
const s3File = new DicomFile({
    backend: 'S3',
    s3Config: {
        bucket: 'dicom-archive',
        accessKey: process.env.AWS_ACCESS_KEY,
        secretKey: process.env.AWS_SECRET_KEY,
        endpoint: 'https://s3.amazonaws.com',
        region: 'us-east-1'
    }
});

// Open from S3, save to S3
await s3File.open('input/scan.dcm');
await s3File.saveAsDicom('output/scan.dcm');
```

### Save as JSON

Convert and save DICOM file as JSON (DICOM Part 18 format):

```typescript
const file = new DicomFile();
await file.open('image.dcm');

// Pretty-printed JSON (default, recommended for humans)
await file.saveAsJson('output.json', true);

// Compact JSON (smaller file size)
await file.saveAsJson('output-compact.json', false);

file.close();
```

**JSON Format**: Follows DICOM Part 18 JSON Model specification

**Comparison with `toJson()`:**

| Method | File I/O | Async | Use Case |
|--------|----------|-------|----------|
| `saveAsJson()` | Yes | Yes | Save JSON to disk/S3 |
| `toJson()` | No | No | Get JSON string in memory |

```typescript
// toJson() - In-memory string
const jsonString = file.toJson();
sendOverApi(jsonString);  // Fast, no file I/O

// saveAsJson() - Write to file
await file.saveAsJson('output.json', true);  // Saves to disk
```

**S3 Example:**
```typescript
const s3File = new DicomFile({
    backend: 'S3',
    s3Config: { /* ... */ }
});

await s3File.open('dicom/patient123/scan.dcm');
await s3File.saveAsJson('json/patient123/metadata.json', true);
```

## Complete Example: DICOM Metadata Extractor

A comprehensive example showing file validation, metadata extraction, pixel data analysis, and batch processing:

```typescript
import { DicomFile, getCommonTagSets, combineTags } from '@nuxthealth/node-dicom';
import * as fs from 'fs';
import * as path from 'path';

interface DicomMetadata {
    filePath: string;
    sopInstanceUid: string;
    sopClassUid: string;
    metadata: Record<string, string>;
    pixelInfo?: {
        width: number;
        height: number;
        frames: number;
        compressed: boolean;
        size: number;
    };
}

/**
 * Extract comprehensive metadata from a single DICOM file
 */
async function extractMetadata(filePath: string): Promise<DicomMetadata | null> {
    // Step 1: Quick validation without full open
    let basicInfo;
    try {
        basicInfo = DicomFile.check(filePath);
    } catch (error) {
        console.error(`Invalid DICOM file ${filePath}:`, error.message);
        return null;
    }
    
    console.log(`Processing: ${filePath}`);
    console.log(`  SOP Instance UID: ${basicInfo.sopInstanceUid}`);
    
    // Step 2: Open file and extract metadata
    const file = new DicomFile();
    
    try {
        await file.open(filePath);
        
        // Get comprehensive tag sets
        const tags = getCommonTagSets();
        const allTags = combineTags([
            tags.patientBasic,
            tags.patientExtended,
            tags.studyBasic,
            tags.seriesBasic,
            tags.instanceBasic,
            tags.imagePixel,
            tags.imageGeometry,
            tags.equipment
        ]);
        
        // Extract all metadata
        const metadata = file.extract(allTags);
        
        // Try to get pixel data info
        let pixelInfo;
        try {
            const info = file.getPixelDataInfo();
            pixelInfo = {
                width: info.width,
                height: info.height,
                frames: info.frames,
                compressed: info.isCompressed,
                size: info.dataSize
            };
            console.log(`  Image: ${info.width}x${info.height}, ${info.frames} frames, ${info.isCompressed ? 'compressed' : 'uncompressed'}`);
        } catch {
            console.log(`  No pixel data`);
        }
        
        return {
            filePath,
            sopInstanceUid: basicInfo.sopInstanceUid,
            sopClassUid: basicInfo.sopClassUid,
            metadata,
            pixelInfo
        };
        
    } catch (error) {
        console.error(`Error processing ${filePath}:`, error.message);
        return null;
    } finally {
        file.close();
    }
}

/**
 * Recursively process all DICOM files in a directory
 */
async function processDirectory(dirPath: string): Promise<DicomMetadata[]> {
    const results: DicomMetadata[] = [];
    const entries = fs.readdirSync(dirPath, { withFileTypes: true });
    
    for (const entry of entries) {
        const fullPath = path.join(dirPath, entry.name);
        
        if (entry.isDirectory()) {
            // Recurse into subdirectories
            const subResults = await processDirectory(fullPath);
            results.push(...subResults);
        } else if (entry.name.endsWith('.dcm')) {
            // Process DICOM file
            const metadata = await extractMetadata(fullPath);
            if (metadata) {
                results.push(metadata);
            }
        }
    }
    
    return results;
}

/**
 * Organize extracted metadata by study hierarchy
 */
interface StudyGroup {
    studyInstanceUid: string;
    patientName: string;
    studyDate: string;
    studyDescription: string;
    series: Map<string, DicomMetadata[]>;
}

function organizeByStudy(metadata: DicomMetadata[]): Map<string, StudyGroup> {
    const studies = new Map<string, StudyGroup>();
    
    for (const item of metadata) {
        const studyUid = item.metadata.StudyInstanceUID || 'unknown';
        const seriesUid = item.metadata.SeriesInstanceUID || 'unknown';
        
        if (!studies.has(studyUid)) {
            studies.set(studyUid, {
                studyInstanceUid: studyUid,
                patientName: item.metadata.PatientName || 'Unknown',
                studyDate: item.metadata.StudyDate || 'Unknown',
                studyDescription: item.metadata.StudyDescription || 'Unknown',
                series: new Map()
            });
        }
        
        const study = studies.get(studyUid)!;
        
        if (!study.series.has(seriesUid)) {
            study.series.set(seriesUid, []);
        }
        
        study.series.get(seriesUid)!.push(item);
    }
    
    return studies;
}

/**
 * Main execution
 */
async function main() {
    const inputDir = process.argv[2] || './dicom-studies';
    const outputFile = process.argv[3] || './metadata.json';
    
    console.log('=== DICOM Metadata Extractor ===');
    console.log(`Input directory: ${inputDir}`);
    console.log(`Output file: ${outputFile}\n`);
    
    // Process all files
    console.log('Processing files...\n');
    const startTime = Date.now();
    const metadata = await processDirectory(inputDir);
    const duration = ((Date.now() - startTime) / 1000).toFixed(2);
    
    console.log(`\n=== Processing Complete ===`);
    console.log(`Processed ${metadata.length} files in ${duration} seconds`);
    
    // Organize by study
    const studies = organizeByStudy(metadata);
    
    console.log(`\nFound ${studies.size} studies:`);
    studies.forEach((study, studyUid) => {
        const totalInstances = Array.from(study.series.values())
            .reduce((sum, instances) => sum + instances.length, 0);
        
        console.log(`\nStudy: ${study.studyDescription}`);
        console.log(`  Patient: ${study.patientName}`);
        console.log(`  Date: ${study.studyDate}`);
        console.log(`  UID: ${studyUid}`);
        console.log(`  Series: ${study.series.size}`);
        console.log(`  Instances: ${totalInstances}`);
        
        study.series.forEach((instances, seriesUid) => {
            const first = instances[0];
            console.log(`    - ${first.metadata.SeriesDescription || 'Unknown'} (${instances.length} images)`);
        });
    });
    
    // Save results
    const output = {
        processedAt: new Date().toISOString(),
        totalFiles: metadata.length,
        totalStudies: studies.size,
        studies: Array.from(studies.entries()).map(([uid, study]) => ({
            studyInstanceUid: uid,
            patientName: study.patientName,
            studyDate: study.studyDate,
            studyDescription: study.studyDescription,
            seriesCount: study.series.size,
            series: Array.from(study.series.entries()).map(([seriesUid, instances]) => ({
                seriesInstanceUid: seriesUid,
                seriesDescription: instances[0].metadata.SeriesDescription,
                seriesNumber: instances[0].metadata.SeriesNumber,
                modality: instances[0].metadata.Modality,
                instanceCount: instances.length,
                instances: instances.map(i => ({
                    sopInstanceUid: i.sopInstanceUid,
                    instanceNumber: i.metadata.InstanceNumber,
                    filePath: i.filePath,
                    pixelInfo: i.pixelInfo
                }))
            }))
        }))
    };
    
    fs.writeFileSync(outputFile, JSON.stringify(output, null, 2));
    console.log(`\nMetadata saved to: ${outputFile}`);
}

// Run
main().catch(console.error);
```

**Usage:**

```bash
# Process directory and save metadata
npx tsx metadata-extractor.ts ./dicom-archive ./output.json

# Output example:
# === DICOM Metadata Extractor ===
# Input directory: ./dicom-archive
# Output file: ./output.json
#
# Processing files...
#
# Processing: ./dicom-archive/study1/series1/image001.dcm
#   SOP Instance UID: 1.2.840.113619...
#   Image: 512x512, 1 frames, compressed
#
# === Processing Complete ===
# Processed 150 files in 3.45 seconds
#
# Found 3 studies:
#
# Study: Chest CT with Contrast
#   Patient: DOE^JOHN
#   Date: 20231201
#   UID: 1.2.840.113619...
#   Series: 2
#   Instances: 75
#     - Chest Scout (2 images)
#     - Chest Axial (73 images)
```

## Best Practices

### Memory Management

```typescript
// Always close files when done
const file = new DicomFile();
try {
    await file.open('scan.dcm');
    // ... work with file
} finally {
    file.close();  // Releases memory
}

// Or reuse the same instance
const file = new DicomFile();
for (const filePath of fileList) {
    await file.open(filePath);
    // ... process
    file.close();  // Free memory before next file
}
```

### Error Handling

```typescript
// Validate before opening
try {
    const info = DicomFile.check(filePath);
    console.log('Valid DICOM:', info.sopInstanceUid);
} catch (error) {
    console.error('Invalid DICOM file:', error.message);
    return;
}

// Handle missing tags gracefully
const data = file.extract(['PatientName', 'StudyDate', 'RareTag']);
const patientName = data.PatientName || 'Unknown';
const studyDate = data.StudyDate || 'N/A';
```

### Performance Optimization

```typescript
// Use check() for batch validation (faster than open())
const validFiles = files.filter(f => {
    try {
        DicomFile.check(f);
        return true;
    } catch {
        return false;
    }
});

// Extract only needed tags
const data = file.extract(['PatientID', 'StudyInstanceUID']);
// Better than extracting everything

// Use predefined tag sets
const tags = getCommonTagSets();
const data = file.extract(tags.patientBasic);
// More maintainable than manual lists

// For pixel data, choose the right method
if (needInMemory) {
    const buffer = file.getPixelData();  // Fast, no I/O
} else {
    file.saveRawPixelData('output.raw');  // Direct to file
}
```

### S3 Best Practices

```typescript
// Reuse S3 configuration for multiple files
const s3Config = {
    backend: 'S3' as const,
    s3Config: {
        bucket: process.env.DICOM_BUCKET!,
        accessKey: process.env.AWS_ACCESS_KEY!,
        secretKey: process.env.AWS_SECRET_KEY!,
        endpoint: process.env.S3_ENDPOINT!,
        region: process.env.AWS_REGION!
    }
};

const file = new DicomFile(s3Config);

// Process multiple files
for (const key of s3Keys) {
    await file.open(key);
    // ... process
    file.close();
}
```

### Working with Large Datasets

```typescript
// Process files in batches to control memory
async function processBatch(files: string[], batchSize: number = 10) {
    for (let i = 0; i < files.length; i += batchSize) {
        const batch = files.slice(i, i + batchSize);
        await Promise.all(batch.map(processFile));
        
        // Give GC a chance
        if (i % 100 === 0) {
            await new Promise(resolve => setTimeout(resolve, 100));
        }
    }
}

async function processFile(filePath: string) {
    const file = new DicomFile();
    try {
        await file.open(filePath);
        const data = file.extract(['PatientID', 'StudyInstanceUID']);
        // Store minimal data
        await saveToDatabase(data);
    } finally {
        file.close();  // Critical for large batches
    }
}
```

### Pixel Data Considerations

```typescript
const info = file.getPixelDataInfo();

// Check if compressed before decoding
if (info.isCompressed) {
    // Requires transcode feature
    try {
        const decoded = file.getDecodedPixelData();
    } catch (error) {
        console.error('Decompression not available:', error.message);
        // Fall back to raw data
        const raw = file.getPixelData();
    }
}

// Be aware of memory usage for multi-frame images
const bytesPerPixel = Math.ceil(info.bitsAllocated / 8);
const totalBytes = info.width * info.height * info.frames * 
                   info.samplesPerPixel * bytesPerPixel;
console.log(`Pixel data will use ~${(totalBytes / 1024 / 1024).toFixed(2)} MB`);

if (totalBytes > 100 * 1024 * 1024) {
    console.warn('Large pixel data - consider streaming or frame-by-frame processing');
}
```

## Tips and Common Patterns

1. **Always close files**: Call `close()` when done, especially in loops, to free memory
2. **Use `check()` first**: Validate files before opening for better error handling and performance
3. **Leverage predefined tag sets**: Use `getCommonTagSets()` instead of manually listing tags
4. **Handle missing tags**: Not all DICOM files have all tags - always check for undefined
5. **Choose the right pixel method**: 
   - `getPixelData()` for raw extraction (fastest, no processing)
   - `getDecodedPixelData()` for decompression only
   - `getProcessedPixelData()` for windowing, frame extraction, 8-bit conversion (NEW!)
   - `saveRawPixelData()` for direct file export
   - `processPixelData()` for advanced processing with file output
6. **Reuse instances**: Create one `DicomFile` instance and reuse it for multiple files
7. **Batch processing**: Process large datasets in batches to manage memory
8. **S3 configuration**: Store S3 config in environment variables, not in code

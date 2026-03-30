# FindScu - DICOM C-FIND SCU Client

The `FindScu` class implements a DICOM C-FIND Service Class User (SCU) client for querying DICOM archives. It supports Study Root, Patient Root, and Modality Worklist query/retrieve information models.

## Table of Contents

- [Basic Usage](#basic-usage)
- [Configuration Options](#configuration-options)
- [Query Methods](#query-methods)
  - [Manual Queries](#manual-queries)
  - [QueryBuilder (Recommended)](#querybuilder-recommended)
- [Query Models](#query-models)
- [Event Callbacks](#event-callbacks)
- [Common Query Patterns](#common-query-patterns)
- [Advanced Features](#advanced-features)

## Basic Usage

```typescript
import { FindScu, QueryBuilder } from '@nuxthealth/node-dicom';

// Create a finder instance
const finder = new FindScu({
    addr: '192.168.1.100:4242',
    callingAeTitle: 'MY-SCU',
    calledAeTitle: 'PACS-SCP',
    verbose: true
});

// Method 1: Using QueryBuilder (type-safe, recommended)
const query = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .studyDateRange('20240101', '20240131')
    .modality('CT')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);

// Method 2: Manual query (flexible)
const results2 = await finder.find(
    {
        PatientName: 'DOE^JOHN',
        StudyDate: '20240101-20240131',
        Modality: 'CT'
    },
    'StudyRoot'
);

console.log(`Found ${results.length} studies`);
results.forEach(result => {
    console.log('Patient:', result.attributes.PatientName);
    console.log('Study Date:', result.attributes.StudyDate);
    console.log('Study UID:', result.attributes.StudyInstanceUID);
});
```

## Configuration Options

### Constructor: `new FindScu(options)`

#### Required Options

##### addr

**Type:** `string` (required)

The network address of the DICOM C-FIND SCP (PACS server) in format `host:port`.

```typescript
addr: '192.168.1.100:4242'
addr: 'pacs.hospital.org:104'
addr: '127.0.0.1:4242'
```

#### Optional Options

##### callingAeTitle

**Type:** `string` (optional)  
**Default:** `"FIND-SCU"`

The Application Entity (AE) title of this SCU client.

```typescript
callingAeTitle: 'WORKSTATION-01'
```

##### calledAeTitle

**Type:** `string` (optional)  
**Default:** `"ANY-SCP"`

The AE title of the remote SCP server. If not specified, extracted from `addr` if present.

```typescript
calledAeTitle: 'ORTHANC'
```

##### maxPduLength

**Type:** `number` (optional)  
**Default:** `16384`  
**Range:** `4096` to `131072`

Maximum Protocol Data Unit (PDU) length in bytes for network communication.

```typescript
maxPduLength: 32768  // 32 KB
```

##### verbose

**Type:** `boolean` (optional)  
**Default:** `false`

Enable detailed logging for debugging.

```typescript
verbose: true
```

## Query Methods

### Manual Queries

Use the `find()` method for manual queries when you need maximum flexibility:

```typescript
find(
    query: object,
    queryModel?: 'StudyRoot' | 'PatientRoot' | 'ModalityWorklist',
    onResult?: (err, result) => void,
    onCompleted?: (err, completed) => void
): Promise<FindResult[]>
```

**Example:**

```typescript
const results = await finder.find(
    {
        // Query parameters as DICOM tag names
        PatientID: 'PAT12345',
        StudyDate: '20240101-20240131',
        Modality: 'CT'
    },
    'StudyRoot',  // Query model (optional, defaults to StudyRoot)
    (err, result) => {
        // Callback for each result
        if (!err) {
            console.log('Match found:', result.data?.StudyInstanceUID);
        }
    },
    (err, completed) => {
        // Callback when complete
        if (!err) {
            console.log(completed.message);  // "C-FIND completed: 5 result(s) in 0.23s"
        }
    }
);
```

### QueryBuilder (Recommended)

The `QueryBuilder` provides a type-safe, fluent API for constructing queries without needing to know DICOM tag names.

```typescript
findWithQuery(
    query: QueryBuilder,
    onResult?: (err, result) => void,
    onCompleted?: (err, completed) => void
): Promise<FindResult[]>
```

**Example:**

```typescript
import { QueryBuilder } from '@nuxthealth/node-dicom';

const query = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .studyDateRange('20240101', '20240131')
    .modality('CT')
    .studyDescription('*Abdomen*')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(
    query,
    (err, result) => {
        if (!err && result.data) {
            console.log('Study found:', result.data.StudyInstanceUID);
        }
    },
    (err, completed) => {
        if (!err) {
            console.log(`Query completed: ${completed.data?.totalResults} results`);
        }
    }
);
```

## Query Models

DICOM C-FIND supports three query/retrieve information models:

### Study Root (Default)

Query and retrieve at the study level. This is the most commonly used model.

```typescript
// Manual query
const results = await finder.find({
    PatientName: 'DOE^*',
    StudyDate: '20240101-'
}, 'StudyRoot');

// QueryBuilder
const query = QueryBuilder.study()
    .patientName('DOE^*')
    .studyDateFrom('20240101')
    .includeAllReturnAttributes();
```

### Patient Root

Query and retrieve at the patient level.

```typescript
// Manual query
const results = await finder.find({
    PatientID: 'PAT12345',
    PatientName: '',
    PatientBirthDate: '',
    PatientSex: ''
}, 'PatientRoot');

// QueryBuilder
const query = QueryBuilder.patient()
    .patientId('PAT12345')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);
```

### Modality Worklist

Query scheduled procedures from the worklist.

```typescript
// Manual query
const results = await finder.find({
    ScheduledProcedureStepStartDate: '20240315',
    Modality: 'MR'
}, 'ModalityWorklist');

// QueryBuilder
const query = QueryBuilder.modalityWorklist()
    .scheduledProcedureStepStartDate('20240315')
    .modality('MR')
    .includeAllReturnAttributes();
```

## Event Callbacks

### onResult

Callback invoked for each matching result found during the query.

**Signature:** `(err: Error | null, result: FindResultEvent) => void`

```typescript
{
    message: string,           // "Match #1", "Match #2", etc.
    data?: {                   // DICOM attributes for this match
        PatientName: string,
        StudyInstanceUID: string,
        StudyDate: string,
        // ... other attributes
    }
}
```

**Example:**

```typescript
await finder.find(
    { PatientName: '*' },
    'StudyRoot',
    (err, result) => {
        if (err) {
            console.error('Error:', err);
            return;
        }
        
        const data = result.data;
        if (data) {
            console.log('Patient:', data.PatientName);
            console.log('Study UID:', data.StudyInstanceUID);
            console.log('Date:', data.StudyDate);
        }
    }
);
```

### onCompleted

Callback invoked when the entire query completes.

**Signature:** `(err: Error | null, completed: FindCompletedEvent) => void`

```typescript
{
    message: string,           // "C-FIND completed: 10 result(s) in 0.45s"
    data?: {
        totalResults: number,     // Total number of matches
        durationSeconds: number   // Query execution time
    }
}
```

**Example:**

```typescript
await finder.find(
    { Modality: 'CT' },
    'StudyRoot',
    null,  // No per-result callback
    (err, completed) => {
        if (err) {
            console.error('Query failed:', err);
            return;
        }
        
        console.log(completed.message);
        if (completed.data) {
            console.log(`Found ${completed.data.totalResults} studies`);
            console.log(`Query took ${completed.data.durationSeconds.toFixed(2)}s`);
        }
    }
);
```

## Common Query Patterns

### Search by Patient Name

```typescript
// Exact match
const query1 = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .includeAllReturnAttributes();

// Wildcard search (lastName starts with DOE)
const query2 = QueryBuilder.study()
    .patientName('DOE^*')
    .includeAllReturnAttributes();

// Any patient
const query3 = QueryBuilder.study()
    .patientName('*')
    .includeAllReturnAttributes();
```

### Search by Date Range

```typescript
// Specific date range
const query1 = QueryBuilder.study()
    .studyDateRange('20240101', '20240131')  // January 2024
    .includeAllReturnAttributes();

// From date onwards
const query2 = QueryBuilder.study()
    .studyDateFrom('20240101')  // All studies from Jan 1, 2024
    .includeAllReturnAttributes();

// Up to date
const query3 = QueryBuilder.study()
    .studyDateTo('20231231')  // All studies until Dec 31, 2023
    .includeAllReturnAttributes();
```

### Search by Modality

```typescript
const query = QueryBuilder.study()
    .modality('CT')  // CT scans only
    .includeAllReturnAttributes();

// Multiple queries for different modalities
const ctStudies = await finder.findWithQuery(
    QueryBuilder.study().modality('CT').includeAllReturnAttributes()
);
const mrStudies = await finder.findWithQuery(
    QueryBuilder.study().modality('MR').includeAllReturnAttributes()
);
```

### Complex Multi-Criteria Query

```typescript
const query = QueryBuilder.study()
    .patientName('DOE^*')
    .studyDateRange('20240101', '20240331')  // Q1 2024
    .modality('CT')
    .studyDescription('*Abdomen*')
    .accessionNumber('')  // Include accession number in results
    .referringPhysicianName('')  // Include physician in results
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);
```

### Search by Patient ID

```typescript
const query = QueryBuilder.study()
    .patientId('PAT12345')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);
```

### Patient Demographics Query

```typescript
const query = QueryBuilder.patient()
    .patientName('*')
    .patientBirthDate('')
    .patientSex('')
    .includeAllReturnAttributes();

const patients = await finder.findWithQuery(query);
patients.forEach(p => {
    const attrs = p.attributes;
    console.log(`${attrs.PatientName} (${attrs.PatientID})`);
    console.log(`  DOB: ${attrs.PatientBirthDate}, Sex: ${attrs.PatientSex}`);
});
```

## Advanced Features

### Using Hex Tag Codes

You can also query using DICOM tag hex codes instead of names:

```typescript
const results = await finder.find(
    {
        '00100010': '',  // PatientName
        '0020000D': '',  // StudyInstanceUID
        '00080020': '',  // StudyDate
        '00081030': ''   // StudyDescription
    },
    'StudyRoot'
);
```

### Wildcard Patterns

DICOM supports two wildcards in queries:

- `*` - Matches any number of characters
- `?` - Matches exactly one character

```typescript
const query = QueryBuilder.study()
    .patientName('SM??H^J*')  // Matches SMITH^JOHN, SMYTH^JANE, etc.
    .studyDescription('*CT*')  // Contains "CT" anywhere
    .includeAllReturnAttributes();
```

### Empty String vs Omitted

- **Empty string** (`''`): Include this attribute in the response
- **Omitted**: Don't include this attribute in the response

```typescript
// Only return PatientName, StudyDate, and Modality
const results = await finder.find({
    PatientName: '',
    StudyDate: '',
    Modality: ''
}, 'StudyRoot');

// Return all standard attributes
const query = QueryBuilder.study()
    .patientName('*')
    .includeAllReturnAttributes();  // Adds all common return attributes
```

### Progress Tracking with Callbacks

Track query progress in real-time:

```typescript
let matchCount = 0;

const results = await finder.find(
    { Modality: 'CT' },
    'StudyRoot',
    (err, result) => {
        if (!err) {
            matchCount++;
            console.log(`Match ${matchCount}: ${result.data?.PatientName}`);
        }
    },
    (err, completed) => {
        if (!err) {
            console.log(`Query complete! Found ${matchCount} matches in ${completed.data?.durationSeconds}s`);
        }
    }
);
```

### Error Handling

```typescript
try {
    const results = await finder.find({
        PatientID: 'PAT12345'
    }, 'StudyRoot');
    
    if (results.length === 0) {
        console.log('No studies found for this patient');
    }
} catch (error) {
    console.error('Query failed:', error.message);
    
    // Common errors:
    // - Connection refused (PACS not running)
    // - Association rejected (AE title mismatch)
    // - Timeout (network issues)
    // - Invalid query parameters
}
```

### Performance Tips

1. **Be specific**: Add more criteria to reduce results
2. **Use date ranges**: Limit searches to relevant time periods
3. **Avoid wildcards at start**: `*SMITH` is slower than `SMITH*`
4. **Include only needed attributes**: Don't use `includeAllReturnAttributes()` if you only need a few fields

```typescript
// Good - specific query
const query = QueryBuilder.study()
    .patientId('PAT12345')
    .studyDateRange('20240101', '20240131')
    .includeAllReturnAttributes();

// Less efficient - very broad query
const query2 = QueryBuilder.study()
    .patientName('*')  // Returns everything
    .includeAllReturnAttributes();
```

## Return Value Structure

### FindResult

Each result returned is a `FindResult` object:

```typescript
interface FindResult {
    attributes: {
        [tagName: string]: string
    }
}
```

**Example:**

```typescript
const results = await finder.findWithQuery(query);

results.forEach(result => {
    // Access attributes by tag name
    console.log(result.attributes.PatientName);
    console.log(result.attributes.StudyDate);
    console.log(result.attributes.StudyInstanceUID);
    console.log(result.attributes.Modality);
    
    // Or iterate all attributes
    for (const [tag, value] of Object.entries(result.attributes)) {
        console.log(`${tag}: ${value}`);
    }
});
```

## See Also

- [QueryBuilder API Reference](./querybuilder.md) - Detailed QueryBuilder documentation
- [StoreScu Documentation](./storescu.md) - Sending DICOM files
- [StoreScp Documentation](./storescp.md) - Receiving DICOM files
- [DICOM Standard Part 4](https://dicom.nema.org/medical/dicom/current/output/html/part04.html) - C-FIND specification

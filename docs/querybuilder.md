# QueryBuilder - Type-Safe DICOM Query Construction

The `QueryBuilder` class provides a fluent, type-safe API for constructing DICOM C-FIND queries without needing to memorize DICOM tag names or query syntax.

## Table of Contents

- [Overview](#overview)
- [Benefits](#benefits)
- [Getting Started](#getting-started)
- [Query Models](#query-models)
- [Patient Methods](#patient-methods)
- [Study Methods](#study-methods)
- [Series Methods](#series-methods)
- [Modality Worklist Methods](#modality-worklist-methods)
- [Helper Methods](#helper-methods)
- [Method Chaining](#method-chaining)
- [Complete Examples](#complete-examples)

## Overview

QueryBuilder eliminates the need to remember DICOM tag names and provides IntelliSense/autocomplete support in your IDE.

**Before (manual queries):**
```typescript
const results = await finder.find({
    '00100010': '',           // What tag is this?
    '0020000D': '',           // And this?
    '00080020': '20240101-'   // Date format correct?
}, 'StudyRoot');
```

**After (QueryBuilder):**
```typescript
const query = QueryBuilder.study()
    .patientName('')           // Autocomplete available
    .studyInstanceUid('')      // Clear and readable
    .studyDateFrom('20240101') // Helper method handles format
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);
```

## Benefits

### 1. Type Safety
Catch errors at compile time instead of runtime.

```typescript
// TypeScript will error on invalid method names
const query = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .studyDat('20240101');  // ❌ TypeScript error - method doesn't exist
```

### 2. Autocomplete Support
Your IDE provides suggestions for available methods.

```typescript
const query = QueryBuilder.study()
    .st  // ← IDE shows: studyDate, studyTime, studyDescription, etc.
```

### 3. No DICOM Knowledge Required
Use intuitive method names instead of hex tag codes.

```typescript
// Don't need to know that PatientName is (0010,0010)
query.patientName('DOE^JOHN');

// Don't need to know that StudyDate is (0008,0020)
query.studyDate('20240101');
```

### 4. Helper Methods
Date range helpers and attribute inclusion utilities.

```typescript
// Instead of manual date range syntax
query.studyDateRange('20240101', '20240131');  // Easy!

// Instead of manually adding all return attributes
query.includeAllReturnAttributes();  // One call!
```

### 5. Self-Documenting Code
Queries are readable and maintainable.

```typescript
// Clear intent
const query = QueryBuilder.study()
    .patientName('DOE^*')
    .studyDateRange('20240101', '20240331')
    .modality('CT')
    .studyDescription('*Abdomen*')
    .includeAllReturnAttributes();
```

## Getting Started

### Create a Query

Use factory methods to create a query for the desired information model:

```typescript
import { QueryBuilder } from '@nuxthealth/node-dicom';

// Study Root query (most common)
const studyQuery = QueryBuilder.study();

// Patient Root query
const patientQuery = QueryBuilder.patient();

// Modality Worklist query
const worklistQuery = QueryBuilder.modalityWorklist();
```

### Add Criteria

Chain methods to add query criteria:

```typescript
const query = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .studyDate('20240101')
    .modality('CT');
```

### Execute the Query

Use `findWithQuery()` to execute:

```typescript
import { FindScu } from '@nuxthealth/node-dicom';

const finder = new FindScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'MY-SCU',
    calledAeTitle: 'ORTHANC'
});

const results = await finder.findWithQuery(query);
```

## Query Models

### Study Root Query

Query studies in the PACS. This is the most commonly used model.

```typescript
const query = QueryBuilder.study()
    .patientName('*')
    .studyDateRange('20240101', '20240131')
    .modality('CT')
    .includeAllReturnAttributes();
```

**Returns:** Study-level information including patient demographics, study details, and series information.

### Patient Root Query

Query patients in the PACS.

```typescript
const query = QueryBuilder.patient()
    .patientId('PAT12345')
    .patientName('')
    .patientBirthDate('')
    .patientSex('')
    .includeAllReturnAttributes();
```

**Returns:** Patient-level information without study details.

### Modality Worklist Query

Query the modality worklist for scheduled procedures.

```typescript
const query = QueryBuilder.modalityWorklist()
    .scheduledProcedureStepStartDate('20240315')
    .scheduledStationAeTitle('CT-ROOM-1')
    .modality('CT')
    .includeAllReturnAttributes();
```

**Returns:** Scheduled procedure information from the worklist.

## Patient Methods

Methods for querying patient demographics.

### patientName(value: string)

Set the patient's name to search for.

**DICOM Tag:** `(0010,0010)` - PatientName

**Format:** `LastName^FirstName^MiddleName^Prefix^Suffix`

```typescript
// Exact match
query.patientName('DOE^JOHN');

// Wildcard - all patients with last name starting with DOE
query.patientName('DOE^*');

// Include PatientName in results
query.patientName('');

// Any patient name
query.patientName('*');
```

### patientId(value: string)

Set the patient ID to search for.

**DICOM Tag:** `(0010,0020)` - PatientID

```typescript
// Exact match
query.patientId('PAT12345');

// Wildcard pattern
query.patientId('PAT*');

// Include PatientID in results
query.patientId('');
```

### patientBirthDate(value: string)

Set the patient's birth date.

**DICOM Tag:** `(0010,0030)` - PatientBirthDate

**Format:** `YYYYMMDD`

```typescript
// Exact date
query.patientBirthDate('19800515');

// Include birth date in results
query.patientBirthDate('');
```

### patientSex(value: string)

Set the patient's sex.

**DICOM Tag:** `(0010,0040)` - PatientSex

**Values:** `M` (male), `F` (female), `O` (other)

```typescript
query.patientSex('M');
query.patientSex('F');
query.patientSex('');  // Include in results
```

## Study Methods

Methods for querying study-level information.

### studyInstanceUid(value: string)

Set the study instance UID.

**DICOM Tag:** `(0020,000D)` - StudyInstanceUID

```typescript
// Exact UID
query.studyInstanceUid('1.2.840.123456.7.8.9');

// Include UID in results
query.studyInstanceUid('');
```

### studyDate(value: string)

Set the study date.

**DICOM Tag:** `(0008,0020)` - StudyDate

**Format:** `YYYYMMDD` or `YYYYMMDD-YYYYMMDD` for ranges

```typescript
// Single date
query.studyDate('20240101');

// Date range
query.studyDate('20240101-20240131');

// From date onwards
query.studyDate('20240101-');

// Up to date
query.studyDate('-20240131');

// Include study date in results
query.studyDate('');
```

### studyDateRange(fromDate: string, toDate: string)

Set a date range (convenience method).

**DICOM Tag:** `(0008,0020)` - StudyDate

```typescript
// January 2024
query.studyDateRange('20240101', '20240131');

// Q1 2024
query.studyDateRange('20240101', '20240331');
```

**Note:** This is equivalent to `.studyDate('20240101-20240131')` but more readable.

### studyDateFrom(date: string)

Set the start date for studies (from this date onwards).

**DICOM Tag:** `(0008,0020)` - StudyDate

```typescript
// All studies from Jan 1, 2024 onwards
query.studyDateFrom('20240101');
```

**Note:** Equivalent to `.studyDate('20240101-')`

### studyDateTo(date: string)

Set the end date for studies (up to and including this date).

**DICOM Tag:** `(0008,0020)` - StudyDate

```typescript
// All studies up to Dec 31, 2023
query.studyDateTo('20231231');
```

**Note:** Equivalent to `.studyDate('-20231231')`

### studyTime(value: string)

Set the study time.

**DICOM Tag:** `(0008,0030)` - StudyTime

**Format:** `HHMMSS.FFFFFF` (fractional seconds optional)

```typescript
// Specific time
query.studyTime('143000');  // 14:30:00

// Time range
query.studyTime('080000-170000');  // 08:00:00 to 17:00:00

// Include study time in results
query.studyTime('');
```

### accessionNumber(value: string)

Set the accession number.

**DICOM Tag:** `(0008,0050)` - AccessionNumber

```typescript
// Exact accession number
query.accessionNumber('ACC12345');

// Include accession number in results
query.accessionNumber('');
```

### studyDescription(value: string)

Set the study description.

**DICOM Tag:** `(0008,1030)` - StudyDescription

```typescript
// Exact description
query.studyDescription('CT Abdomen');

// Wildcard search
query.studyDescription('*Abdomen*');

// Include description in results
query.studyDescription('');
```

### studyId(value: string)

Set the study ID.

**DICOM Tag:** `(0020,0010)` - StudyID

```typescript
query.studyId('STUDY-001');
query.studyId('');  // Include in results
```

### modality(value: string)

Set the modality type.

**DICOM Tag:** `(0008,0060)` - Modality

**Common values:** `CT`, `MR`, `CR`, `DX`, `US`, `XA`, `NM`, `PT`, `OT`

```typescript
// Specific modality
query.modality('CT');
query.modality('MR');

// Include modality in results
query.modality('');
```

### referringPhysicianName(value: string)

Set the referring physician's name.

**DICOM Tag:** `(0008,0090)` - ReferringPhysicianName

**Format:** `LastName^FirstName^MiddleName^Prefix^Suffix`

```typescript
query.referringPhysicianName('SMITH^JANE');
query.referringPhysicianName('SMITH^*');  // Wildcard
query.referringPhysicianName('');  // Include in results
```

## Series Methods

Methods for querying series-level information (Study Root only).

### seriesInstanceUid(value: string)

Set the series instance UID.

**DICOM Tag:** `(0020,000E)` - SeriesInstanceUID

```typescript
query.seriesInstanceUid('1.2.840.123456.7.8.9.10');
query.seriesInstanceUid('');  // Include in results
```

### seriesNumber(value: string)

Set the series number.

**DICOM Tag:** `(0020,0011)` - SeriesNumber

```typescript
query.seriesNumber('1');
query.seriesNumber('');  // Include in results
```

### seriesDescription(value: string)

Set the series description.

**DICOM Tag:** `(0008,103E)` - SeriesDescription

```typescript
query.seriesDescription('Axial');
query.seriesDescription('*Portal*');  // Wildcard
query.seriesDescription('');  // Include in results
```

## Modality Worklist Methods

Methods specific to modality worklist queries.

### scheduledStationAeTitle(value: string)

Set the scheduled station AE title.

**DICOM Tag:** `(0040,0001)` - ScheduledStationAETitle

```typescript
query.scheduledStationAeTitle('CT-ROOM-1');
query.scheduledStationAeTitle('');  // Include in results
```

### scheduledProcedureStepStartDate(value: string)

Set the scheduled procedure step start date.

**DICOM Tag:** `(0040,0002)` - ScheduledProcedureStepStartDate

**Format:** `YYYYMMDD`

```typescript
query.scheduledProcedureStepStartDate('20240315');
query.scheduledProcedureStepStartDate('');  // Include in results
```

### scheduledProcedureStepStartTime(value: string)

Set the scheduled procedure step start time.

**DICOM Tag:** `(0040,0003)` - ScheduledProcedureStepStartTime

**Format:** `HHMMSS`

```typescript
query.scheduledProcedureStepStartTime('143000');
query.scheduledProcedureStepStartTime('');  // Include in results
```

### scheduledPerformingPhysicianName(value: string)

Set the scheduled performing physician's name.

**DICOM Tag:** `(0040,0006)` - ScheduledPerformingPhysicianName

**Format:** `LastName^FirstName`

```typescript
query.scheduledPerformingPhysicianName('SMITH^JOHN');
query.scheduledPerformingPhysicianName('');  // Include in results
```

## Helper Methods

### includeAllReturnAttributes()

Automatically adds all standard return attributes for the query model.

```typescript
const query = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .includeAllReturnAttributes();  // Adds all common study return tags
```

**Study Root adds:**
- PatientName, PatientID, PatientBirthDate, PatientSex
- StudyInstanceUID, StudyDate, StudyTime, StudyDescription, StudyID
- AccessionNumber, Modality, ReferringPhysicianName
- NumberOfStudyRelatedSeries, NumberOfStudyRelatedInstances

**Patient Root adds:**
- PatientName, PatientID, PatientBirthDate, PatientSex

**Modality Worklist adds:**
- PatientName, PatientID, PatientBirthDate, PatientSex
- StudyInstanceUID
- ScheduledStationAETitle, ScheduledProcedureStepStartDate, ScheduledProcedureStepStartTime
- Modality, ScheduledPerformingPhysicianName

**Without this helper:**
```typescript
const query = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .patientId('')
    .patientBirthDate('')
    .patientSex('')
    .studyInstanceUid('')
    .studyDate('')
    .studyTime('')
    .studyDescription('')
    .studyId('')
    .accessionNumber('')
    .modality('')
    .referringPhysicianName('');
```

**With this helper:**
```typescript
const query = QueryBuilder.study()
    .patientName('DOE^JOHN')
    .includeAllReturnAttributes();  // Much simpler!
```

## Method Chaining

All methods return the QueryBuilder instance, allowing for fluent chaining.

```typescript
const query = QueryBuilder.study()
    .patientName('DOE^*')
    .patientId('')
    .studyDateRange('20240101', '20240331')
    .modality('CT')
    .studyDescription('*Abdomen*')
    .accessionNumber('')
    .referringPhysicianName('')
    .includeAllReturnAttributes();
```

You can also split across multiple lines:

```typescript
const query = QueryBuilder.study();
query.patientName('DOE^JOHN');
query.studyDateFrom('20240101');
query.modality('CT');
query.includeAllReturnAttributes();
```

## Complete Examples

### Example 1: Simple Study Search

Find all CT studies for a specific patient:

```typescript
import { FindScu, QueryBuilder } from '@nuxthealth/node-dicom';

const finder = new FindScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'MY-SCU',
    calledAeTitle: 'ORTHANC'
});

const query = QueryBuilder.study()
    .patientId('PAT12345')
    .modality('CT')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);

console.log(`Found ${results.length} studies`);
results.forEach(r => {
    console.log('Study Date:', r.attributes.StudyDate);
    console.log('Description:', r.attributes.StudyDescription);
});
```

### Example 2: Date Range Search

Find all studies in Q1 2024:

```typescript
const query = QueryBuilder.study()
    .patientName('*')
    .studyDateRange('20240101', '20240331')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);
```

### Example 3: Complex Multi-Criteria

Find CT abdomen studies for patients with last name starting with "Fischer":

```typescript
const query = QueryBuilder.study()
    .patientName('Fischer^*')
    .studyDateRange('20240101', '20241231')
    .modality('CT')
    .studyDescription('*Abdomen*')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);

results.forEach(r => {
    const attrs = r.attributes;
    console.log(`Patient: ${attrs.PatientName} (${attrs.PatientID})`);
    console.log(`Study: ${attrs.StudyDescription}`);
    console.log(`Date: ${attrs.StudyDate}`);
    console.log(`UID: ${attrs.StudyInstanceUID}`);
    console.log('---');
});
```

### Example 4: Patient Demographics

Query patient information without study details:

```typescript
const query = QueryBuilder.patient()
    .patientName('*')
    .includeAllReturnAttributes();

const patients = await finder.findWithQuery(query);

patients.forEach(p => {
    const attrs = p.attributes;
    console.log(`${attrs.PatientName} (ID: ${attrs.PatientID})`);
    console.log(`  DOB: ${attrs.PatientBirthDate}`);
    console.log(`  Sex: ${attrs.PatientSex}`);
});
```

### Example 5: Modality Worklist

Query today's scheduled CT procedures:

```typescript
const today = new Date().toISOString().slice(0, 10).replace(/-/g, '');  // YYYYMMDD

const query = QueryBuilder.modalityWorklist()
    .scheduledProcedureStepStartDate(today)
    .modality('CT')
    .includeAllReturnAttributes();

const scheduled = await finder.findWithQuery(query);

scheduled.forEach(proc => {
    const attrs = proc.attributes;
    console.log(`Patient: ${attrs.PatientName}`);
    console.log(`Time: ${attrs.ScheduledProcedureStepStartTime}`);
    console.log(`Station: ${attrs.ScheduledStationAETitle}`);
    console.log(`Physician: ${attrs.ScheduledPerformingPhysicianName}`);
    console.log('---');
});
```

### Example 6: Specific Study Retrieval

Retrieve a specific study by UID:

```typescript
const query = QueryBuilder.study()
    .studyInstanceUid('1.2.840.123456.7.8.9')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);

if (results.length > 0) {
    const study = results[0].attributes;
    console.log('Study found:');
    console.log('  Patient:', study.PatientName);
    console.log('  Date:', study.StudyDate);
    console.log('  Description:', study.StudyDescription);
    console.log('  Modality:', study.Modality);
} else {
    console.log('Study not found');
}
```

### Example 7: Comparison - Old vs New API

**Old way (manual queries):**
```typescript
const results = await finder.find(
    {
        'PatientName': 'Fischer^*',
        'PatientID': '',
        'PatientBirthDate': '',
        'PatientSex': '',
        'StudyInstanceUID': '',
        'StudyDate': '20240101-20241231',
        'StudyTime': '',
        'StudyDescription': '*Abdomen*',
        'Modality': 'CT',
        'AccessionNumber': '',
        'ReferringPhysicianName': ''
    },
    'StudyRoot'
);
```

**New way (QueryBuilder):**
```typescript
const query = QueryBuilder.study()
    .patientName('Fischer^*')
    .studyDateRange('20240101', '20241231')
    .studyDescription('*Abdomen*')
    .modality('CT')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(query);
```

### Example 8: With Progress Callbacks

Track query progress:

```typescript
const query = QueryBuilder.study()
    .modality('CT')
    .studyDateFrom('20240101')
    .includeAllReturnAttributes();

const results = await finder.findWithQuery(
    query,
    (err, result) => {
        if (!err && result.data) {
            console.log('Found study:', result.data.StudyInstanceUID);
        }
    },
    (err, completed) => {
        if (!err && completed.data) {
            console.log(`Query completed: ${completed.data.totalResults} results in ${completed.data.durationSeconds}s`);
        }
    }
);
```

## Integration with FindScu

QueryBuilder is designed to work seamlessly with FindScu:

```typescript
import { FindScu, QueryBuilder } from '@nuxthealth/node-dicom';

// Create finder
const finder = new FindScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'MY-SCU',
    calledAeTitle: 'ORTHANC',
    verbose: true
});

// Build query
const query = QueryBuilder.study()
    .patientName('DOE^*')
    .studyDateRange('20240101', '20240331')
    .includeAllReturnAttributes();

// Execute query
const results = await finder.findWithQuery(query);

// Process results
console.log(`Found ${results.length} studies`);
```

## API Reference

### Factory Methods

- `QueryBuilder.study()` → Creates a Study Root query
- `QueryBuilder.patient()` → Creates a Patient Root query
- `QueryBuilder.modalityWorklist()` → Creates a Modality Worklist query

### Getter Methods

- `query.queryModel()` → Returns the query model enum value
- `query.params()` → Returns the underlying query parameters as an object

### All Chainable Methods

Every method listed in this documentation returns `this` (the QueryBuilder instance), enabling method chaining.

## See Also

- [FindScu Documentation](./findscu.md) - DICOM C-FIND client documentation
- [DICOM Standard Part 4](https://dicom.nema.org/medical/dicom/current/output/html/part04.html) - Query/Retrieve specifications

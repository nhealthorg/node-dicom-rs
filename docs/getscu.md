# GetScu - DICOM C-GET SCU Documentation

## Overview

The `GetScu` class provides a DICOM C-GET Service Class User (SCU) for retrieving studies, series, or instances from a PACS over the same association that issued the query.

**Key Concept**: Unlike `MoveScu`, C-GET does not forward data to another AE title. The remote PACS sends C-STORE requests back over the same connection, and `GetScu` stores those incoming objects either on the local filesystem or in S3-compatible object storage.

## Basic Usage

```javascript
import { GetScu } from '@nuxthealth/node-dicom';

const getScu = new GetScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'GET-SCU',
  calledAeTitle: 'ORTHANC',
  outDir: './retrieved',
  storageBackend: 'Filesystem',
  verbose: true
});

const result = await getScu.getStudy({
  query: {
    StudyInstanceUID: '1.2.3.4.5',
    QueryRetrieveLevel: 'STUDY'
  },
  queryModel: 'StudyRoot',
  onSubOperation: (err, event) => {
    if (err || !event.data) {
      return;
    }

    const total = event.data.completed + event.data.remaining;
    console.log(`Progress: ${event.data.completed}/${total}`);
    if (event.data.file) {
      console.log(`Stored: ${event.data.file}`);
    }
  },
  onCompleted: (err, event) => {
    if (err || !event.data) {
      return;
    }

    console.log(`Completed in ${event.data.durationSeconds.toFixed(2)}s`);
  }
});

console.log(`Retrieved ${result.completed} of ${result.total} instances`);
```

## Constructor Options

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `addr` | string | Yes | - | PACS address in format `host:port` or `AE@host:port` |
| `callingAeTitle` | string | No | `GET-SCU` | Local AE title |
| `calledAeTitle` | string | No | `ANY-SCP` | Remote PACS AE title |
| `maxPduLength` | number | No | `16384` | Maximum PDU length |
| `verbose` | boolean | No | `false` | Enable verbose logging |
| `outDir` | string | No | - | Base directory for filesystem storage |
| `storageBackend` | `'Filesystem'\|'S3'` | No | `Filesystem` | Storage backend for received files |
| `s3Config` | object | No | - | S3 configuration (required when `storageBackend` is `S3`) |

## getStudy() API

```typescript
getStudy(options: {
  query: Record<string, string>
  queryModel?: 'StudyRoot' | 'PatientRoot'
  onSubOperation?: (err: Error | null, event: GetSubOperationEvent) => void
  onCompleted?: (err: Error | null, event: GetCompletedEvent) => void
}): Promise<GetResult>
```

### Options

- `query`: DICOM matching keys used for the C-GET request.
- `queryModel`: Either `StudyRoot` or `PatientRoot`. Defaults to `StudyRoot`.
- `onSubOperation`: Progress callback during retrieval and storage.
- `onCompleted`: Final callback with counts and duration.

## Storage Backends

### Filesystem

```javascript
const getScu = new GetScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'GET-SCU',
  calledAeTitle: 'ORTHANC',
  outDir: './retrieved',
  storageBackend: 'Filesystem'
});

await getScu.getStudy({
  query: {
    StudyInstanceUID: '1.2.3.4.5',
    QueryRetrieveLevel: 'STUDY'
  }
});
```

Retrieved files are stored as:

```text
./retrieved/
  <StudyInstanceUID>/
    <SeriesInstanceUID>/
      <SOPInstanceUID>.dcm
```

### S3

```javascript
const getScu = new GetScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'GET-SCU',
  calledAeTitle: 'ORTHANC',
  storageBackend: 'S3',
  s3Config: {
    bucket: 'dicom-archive',
    accessKey: process.env.S3_ACCESS_KEY,
    secretKey: process.env.S3_SECRET_KEY,
    endpoint: 'http://127.0.0.1:9000'
  }
});

await getScu.getStudy({
  query: {
    StudyInstanceUID: '1.2.3.4.5',
```

Objects are uploaded with the same key layout:

```text
<StudyInstanceUID>/<SeriesInstanceUID>/<SOPInstanceUID>.dcm
```

## Query Models

### Study Root

```javascript
const getScu = new GetScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'GET-SCU',
  calledAeTitle: 'ORTHANC',
  outDir: './retrieved'
});

await getScu.getStudy({
  query: {
    StudyInstanceUID: '1.2.3.4.5',
    QueryRetrieveLevel: 'STUDY'
  },
  queryModel: 'StudyRoot'
});
```

### Patient Root

```javascript
const getScu = new GetScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'GET-SCU',
  calledAeTitle: 'ORTHANC',
  outDir: './retrieved'
});

await getScu.getStudy({
  query: {
    PatientID: 'PAT12345',
    QueryRetrieveLevel: 'PATIENT'
  },
  queryModel: 'PatientRoot'
});
```

## Events

### onSubOperation

`onSubOperation` is called while the remote PACS is still sending instances.

```javascript
onSubOperation: (err, event) => {
  if (err || !event.data) {
    return;
  }

  const total = event.data.completed + event.data.remaining;
  console.log(`Completed ${event.data.completed}/${total}`);
  console.log(`Failed ${event.data.failed}, warnings ${event.data.warning}`);

  if (event.data.file) {
    console.log(`Last stored file: ${event.data.file}`);
  }
}
```

`GetSubOperationEvent.data` contains:

- `remaining`
- `completed`
- `failed`
- `warning`
- `file`
- `sopInstanceUid`

### onCompleted

```javascript
onCompleted: (err, event) => {
  if (err || !event.data) {
    return;
  }

  console.log(`Retrieved ${event.data.completed} objects in ${event.data.durationSeconds.toFixed(2)}s`);
}
```

`GetCompletedEvent.data` contains:

- `total`
- `completed`
- `failed`
- `warning`
- `durationSeconds`

## C-GET vs C-MOVE

Use `GetScu` when:

- You want to retrieve objects directly into your Node.js process.
- You want the library to store received objects to local disk or S3.
- You do not want to configure a separate destination AE in the source PACS.

Use `MoveScu` when:

- The source PACS should push instances to another DICOM node.
- You already have a Store SCP destination configured.
- You want retrieval delegated to another AE title.

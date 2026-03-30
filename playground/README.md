# Playground - Demo Examples

Simple, focused demos for each node-dicom-rs service.

## Setup

Download test DICOM files first:

```bash
./downloadTestData.sh
```

This downloads sample DICOM CT scan data to `./testdata/`.

## Demos

### 1. DicomFile - Read and manipulate DICOM files

```bash
node dicomfile-demo.mjs
```

Demonstrates:
- Opening DICOM files
- Extracting tags
- Getting pixel data info
- Processing pixels for display
- Updating tags (anonymization)
- Saving modified files

### 2. StoreScp - Receive DICOM files

```bash
node storescp-demo.mjs
```

Demonstrates:
- Starting C-STORE SCP server
- Receiving DICOM files
- Extracting metadata
- Study completion detection

Keep this running while sending files with StoreScu.

### 2b. StoreScp with onBeforeStore - Async tag modification

```bash
node onBeforeStore.mjs
```

Demonstrates:
- Async tag modification before storage
- Error-first callback pattern
- JSON tag serialization
- Simulated database lookup
- Real-time anonymization

Keep this running and send files to see tag modifications in action.

### 3. StoreScu - Send DICOM files

```bash
node storescu-demo.mjs
```

Demonstrates:
- Sending DICOM files via C-STORE
- Progress tracking
- Error handling
- Transfer completion

Requires StoreScp demo to be running first.

### 4. GetScu - Retrieve and store DICOM files

```bash
node getscu-demo.mjs
```

Demonstrates:
- Retrieving a study with C-GET
- Writing received instances to the local filesystem
- Progress tracking during retrieval
- Equivalent S3 backend configuration

### 5. DICOMweb - Query and retrieve servers

```bash
node dicomweb-demo.mjs
```

Demonstrates:
- QIDO-RS query server (port 8042)
- WADO-RS retrieval server (port 8043)
- RESTful DICOM access

Test with curl:
```bash
# Query all studies
curl http://localhost:8042/studies

# Retrieve a study
curl http://localhost:8043/studies/1.3.6.1.4.1.9328.50.2.125354
```

## File Structure

```
playground/
├── README.md                    # This file
├── downloadTestData.sh          # Download sample DICOM data
├── dicomfile-demo.mjs          # DicomFile demo
├── storescp-demo.mjs           # StoreScp demo  
├── test-onBeforeStore.mjs      # StoreScp with async tag modification demo
├── storescu-demo.mjs           # StoreScu demo
├── getscu-demo.mjs             # GetScu demo
├── getscu-simple-test.mjs      # GetScu minimal retrieval test
├── dicomweb-demo.mjs           # QIDO-RS + WADO-RS demo
├── testdata/                   # Downloaded test DICOM files
├── test-received/              # Files received by StoreScp demos
├── test-get-received/          # Files received by GetScu demo
└── test-output-onbeforestore/  # Files with modified tags from onBeforeStore demo
```

## Tips

- Run `downloadTestData.sh` first to get sample data
- StoreScu requires StoreScp to be running
- GetScu requires a remote PACS with C-GET enabled
- DICOMweb servers read from `testdata/` directory
- All demos use minimal configuration for clarity
- Check each demo file's header comments for prerequisites

# Docker Test Environment

This directory contains a Docker Compose setup for testing DICOM and S3 functionality.

## Services

### Orthanc PACS (Test DICOM Server)

A lightweight open-source PACS server for testing DICOM C-STORE, C-FIND, and DICOMweb operations.

**Ports:**
- `4242` - DICOM services (C-STORE, C-FIND, C-MOVE, etc.)
- `8042` - Web interface & REST API

**Credentials:**
- Username: `orthanc`
- Password: `orthanc`

**AE Title:** `ORTHANC`

**Web Interface:** http://localhost:8042

### RustFS (S3-Compatible Storage)

A lightweight S3-compatible object storage server for testing S3 operations.

**Ports:**
- `7070` - S3 API
- `7071` - Management Console

**Credentials:**
- Access Key: `user`
- Secret Key: `password`

**Endpoint:** http://localhost:7070

## Quick Start

### Start all services:
```bash
cd docker
docker-compose up -d
```

### Start only Orthanc:
```bash
docker-compose up -d orthanc
```

### Start only RustFS:
```bash
docker-compose up -d s3-rustfs
```

### Stop services:
```bash
docker-compose down
```

### View logs:
```bash
docker-compose logs -f orthanc
docker-compose logs -f s3-rustfs
```

## Testing DICOM with Orthanc

### Send DICOM files:
```bash
cd playground
node storescu-demo.mjs
```

This will send randomized DICOM files to Orthanc at `localhost:4242`.

### View stored studies:
Open http://localhost:8042 in your browser and login with `orthanc` / `orthanc`.

### REST API Examples:

```bash
# List all studies
curl -u orthanc:orthanc http://localhost:8042/studies

# Get study details
curl -u orthanc:orthanc http://localhost:8042/studies/{study-id}

# Download DICOM file
curl -u orthanc:orthanc http://localhost:8042/instances/{instance-id}/file -o image.dcm
```

## Testing S3 Storage

Configure your application with:
```javascript
{
  bucket: 'dicom-test',
  accessKey: 'user',
  secretKey: 'password',
  endpoint: 'http://localhost:7070',
  region: 'us-east-1'
}
```

## Data Persistence

Data is persisted in Docker volumes:
- `orthanc_data` - Orthanc database and DICOM files
- `rustfs_data` - RustFS object storage

To completely reset:
```bash
docker-compose down -v
```

## Troubleshooting

### Orthanc not accepting connections:
```bash
# Check if service is running
docker-compose ps

# View logs
docker-compose logs orthanc

# Restart
docker-compose restart orthanc
```

### Port conflicts:
If ports are already in use, edit `docker-compose.yml` and change the host port:
```yaml
ports:
  - "14242:4242"  # Use port 14242 on host instead of 4242
```

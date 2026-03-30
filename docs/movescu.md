# MoveScu - DICOM C-MOVE SCU Documentation

## Overview

The `MoveScu` class provides a C-MOVE Service Class User (SCU) implementation for retrieving DICOM studies, series, or instances from a PACS (Picture Archiving and Communication System). C-MOVE is a DICOM DIMSE service that requests a remote DICOM server to transfer objects to a specified destination AE (Application Entity).

**Key Concept**: Unlike C-GET (which sends data back to the requester), C-MOVE tells the source PACS to send DICOM instances to a *different* AE title that you specify. This destination must be configured in the source PACS's list of known modalities.

## Basic Usage

```javascript
import { MoveScu } from 'node-dicom-rs';

// Create MoveScu instance
const moveScu = new MoveScu({
  addr: '127.0.0.1:4242',           // PACS address
  callingAeTitle: 'MY_SCU',          // Your AE title
  calledAeTitle: 'ORTHANC',          // PACS AE title
  maxPduLength: 16384,               // Optional: Max PDU size
  verbose: true                       // Optional: Enable logging
});

// Move a study to a destination AE
const result = await moveScu.moveStudy(
  {
    StudyInstanceUID: '1.2.840.113619.2.55.3.4.1762893313.19303.1234567890.123',
    QueryRetrieveLevel: 'STUDY'
  },
  'DESTINATION_AE'  // Must be configured in source PACS
);

console.log(`Moved ${result.completed} of ${result.total} instances`);
```

## Configuration Options

### MoveScu Constructor

| Option | Type | Required | Default | Description |
|--------|------|----------|---------|-------------|
| `addr` | string | Yes | - | PACS address in format `host:port` |
| `callingAeTitle` | string | Yes | - | Your application's AE title |
| `calledAeTitle` | string | Yes | - | Remote PACS AE title |
| `maxPduLength` | number | No | `16384` | Maximum PDU (Protocol Data Unit) size in bytes |
| `verbose` | boolean | No | `false` | Enable detailed logging |

```javascript
const moveScu = new MoveScu({
  addr: 'pacs.hospital.org:11112',
  callingAeTitle: 'VIEWER_STATION',
  calledAeTitle: 'MAIN_PACS',
  maxPduLength: 32768,  // Larger PDU for better performance
  verbose: true
});
```

## Move Operation

### moveStudy() Method

```typescript
moveStudy(
  query: Record<string, string>,
  moveDestination: string,
  queryModel?: 'StudyRoot' | 'PatientRoot',
  onSubOperation?: (err: Error | null, event: MoveSubOperationEvent) => void,
  onCompleted?: (err: Error | null, event: MoveCompletedEvent) => void
): Promise<MoveResult>
```

**Parameters:**

- **query**: Key-value pairs defining what to move
  - `QueryRetrieveLevel`: Required - 'PATIENT', 'STUDY', 'SERIES', or 'IMAGE'
  - Matching keys (e.g., `StudyInstanceUID`, `SeriesInstanceUID`)
  
- **moveDestination**: AE title where instances should be sent (must be configured in source PACS)

- **queryModel**: Query model to use
  - `'StudyRoot'` (default): Study Root Query/Retrieve Information Model
  - `'PatientRoot'`: Patient Root Query/Retrieve Information Model

- **onSubOperation**: Optional callback for progress updates during the move

- **onCompleted**: Optional callback when move operation completes

**Returns**: Promise resolving to `MoveResult` with counts of transferred instances

## Query Models

### Study Root (Default)

Study-centric model where series and images are children of studies:

```javascript
// Move entire study
await moveScu.moveStudy({
  StudyInstanceUID: '1.2.3.4.5',
  QueryRetrieveLevel: 'STUDY'
}, 'DESTINATION_AE');

// Move specific series
await moveScu.moveStudy({
  StudyInstanceUID: '1.2.3.4.5',
  SeriesInstanceUID: '1.2.3.4.5.6',
  QueryRetrieveLevel: 'SERIES'
}, 'DESTINATION_AE');
```

### Patient Root

Patient-centric model where studies are children of patients:

```javascript
// Move all studies for a patient
await moveScu.moveStudy(
  {
    PatientID: 'PAT12345',
    QueryRetrieveLevel: 'PATIENT'
  },
  'DESTINATION_AE',
  'PatientRoot'
);
```

## Event Callbacks

### onSubOperation - Progress Tracking

Called for each progress update during the move operation:

```javascript
await moveScu.moveStudy(
  { StudyInstanceUID: '1.2.3.4.5', QueryRetrieveLevel: 'STUDY' },
  'DESTINATION_AE',
  'StudyRoot',
  (err, event) => {
    if (err) {
      console.error('Progress error:', err);
      return;
    }
    
    const progress = (event.completed / event.total * 100).toFixed(1);
    console.log(`Moving: ${event.completed}/${event.total} (${progress}%)`);
    
    if (event.failed > 0) {
      console.warn(`Failed: ${event.failed}`);
    }
    if (event.warning > 0) {
      console.warn(`Warnings: ${event.warning}`);
    }
  }
);
```

**MoveSubOperationEvent Properties:**
- `total`: Total number of instances to move
- `completed`: Number of instances successfully moved
- `failed`: Number of failed moves
- `warning`: Number of moves completed with warnings
- `remaining`: Number of instances remaining

### onCompleted - Completion Notification

Called when the entire move operation finishes:

```javascript
await moveScu.moveStudy(
  { StudyInstanceUID: '1.2.3.4.5', QueryRetrieveLevel: 'STUDY' },
  'DESTINATION_AE',
  'StudyRoot',
  null,  // No progress callback
  (err, event) => {
    if (err) {
      console.error('Move failed:', err);
      return;
    }
    
    console.log('Move completed in', event.durationMs, 'ms');
    console.log('Total:', event.total);
    console.log('Completed:', event.completed);
    console.log('Failed:', event.failed);
    console.log('Warnings:', event.warning);
    
    if (event.completed === event.total && event.failed === 0) {
      console.log('✓ All instances moved successfully');
    } else {
      console.warn('⚠ Some instances failed');
    }
  }
);
```

**MoveCompletedEvent Properties:**
- `total`: Total number of instances
- `completed`: Successfully moved count
- `failed`: Failed count
- `warning`: Warning count
- `durationMs`: Total operation time in milliseconds

## Return Value

The `moveStudy()` method returns a Promise that resolves to a `MoveResult` object:

```typescript
interface MoveResult {
  total: number;      // Total instances to move
  completed: number;  // Successfully moved
  failed: number;     // Failed moves
  warning: number;    // Moves with warnings
}
```

Example:
```javascript
const result = await moveScu.moveStudy(query, 'DESTINATION_AE');

if (result.failed > 0) {
  throw new Error(`Move incomplete: ${result.failed} failures`);
}

console.log(`Successfully moved ${result.completed} instances`);
```

## Common Move Patterns

### Move Entire Study

```javascript
await moveScu.moveStudy(
  {
    StudyInstanceUID: '1.2.840.113619.2.55.3.4.1762893313.19303.1234567890.123',
    QueryRetrieveLevel: 'STUDY'
  },
  'WORKSTATION_AE'
);
```

### Move Specific Series

```javascript
await moveScu.moveStudy(
  {
    StudyInstanceUID: '1.2.3.4.5',
    SeriesInstanceUID: '1.2.3.4.5.6',
    QueryRetrieveLevel: 'SERIES'
  },
  'WORKSTATION_AE'
);
```

### Move All Patient Studies

```javascript
await moveScu.moveStudy(
  {
    PatientID: 'PAT12345',
    QueryRetrieveLevel: 'PATIENT'
  },
  'ARCHIVE_AE',
  'PatientRoot'
);
```

### Move Multiple Studies with Progress

```javascript
const studyUIDs = [
  '1.2.3.4.5.1',
  '1.2.3.4.5.2',
  '1.2.3.4.5.3'
];

for (const uid of studyUIDs) {
  console.log(`\nMoving study ${uid}...`);
  
  await moveScu.moveStudy(
    {
      StudyInstanceUID: uid,
      QueryRetrieveLevel: 'STUDY'
    },
    'DESTINATION_AE',
    'StudyRoot',
    (err, event) => {
      const progress = (event.completed / event.total * 100).toFixed(0);
      process.stdout.write(`\rProgress: ${progress}%`);
    },
    (err, event) => {
      console.log(`\n✓ Completed in ${event.durationMs}ms`);
    }
  );
}
```

### Combine with FindScu

Use FindScu to discover studies, then move them:

```javascript
import { FindScu, MoveScu } from 'node-dicom-rs';

const findScu = new FindScu({ /* config */ });
const moveScu = new MoveScu({ /* config */ });

// Find studies from date range
const studies = [];
await findScu.findStudies(
  { StudyDate: '20240101-20240131' },
  (err, result) => {
    if (result) studies.push(result);
  }
);

// Move all found studies
for (const study of studies) {
  await moveScu.moveStudy(
    {
      StudyInstanceUID: study.StudyInstanceUID,
      QueryRetrieveLevel: 'STUDY'
    },
    'DESTINATION_AE'
  );
}
```

## Advanced Usage

### Move with Detailed Progress Tracking

```javascript
let lastUpdate = Date.now();

await moveScu.moveStudy(
  { StudyInstanceUID: '1.2.3.4.5', QueryRetrieveLevel: 'STUDY' },
  'DESTINATION_AE',
  'StudyRoot',
  (err, event) => {
    if (err) return;
    
    // Update every 500ms to avoid console spam
    const now = Date.now();
    if (now - lastUpdate > 500) {
      const percent = (event.completed / event.total * 100).toFixed(1);
      const rate = event.completed / ((now - startTime) / 1000);
      const eta = (event.remaining / rate).toFixed(0);
      
      console.log(
        `Progress: ${event.completed}/${event.total} (${percent}%) ` +
        `Rate: ${rate.toFixed(1)} img/s ETA: ${eta}s`
      );
      
      lastUpdate = now;
    }
  }
);
```

### Move with Retry Logic

```javascript
async function moveStudyWithRetry(moveScu, query, destination, maxRetries = 3) {
  for (let attempt = 1; attempt <= maxRetries; attempt++) {
    try {
      const result = await moveScu.moveStudy(query, destination);
      
      if (result.failed === 0) {
        return result;  // Success
      }
      
      if (attempt < maxRetries) {
        console.warn(`Attempt ${attempt} had ${result.failed} failures, retrying...`);
        await new Promise(resolve => setTimeout(resolve, 5000));
      } else {
        throw new Error(`Failed after ${maxRetries} attempts: ${result.failed} failures`);
      }
    } catch (error) {
      if (attempt >= maxRetries) throw error;
      console.warn(`Attempt ${attempt} failed:`, error.message);
      await new Promise(resolve => setTimeout(resolve, 5000));
    }
  }
}

await moveStudyWithRetry(moveScu, query, 'DESTINATION_AE');
```

### Batch Move with Rate Limiting

```javascript
async function batchMove(moveScu, studies, destination, concurrency = 2) {
  const results = [];
  
  for (let i = 0; i < studies.length; i += concurrency) {
    const batch = studies.slice(i, i + concurrency);
    
    const batchResults = await Promise.all(
      batch.map(study => 
        moveScu.moveStudy(
          {
            StudyInstanceUID: study.StudyInstanceUID,
            QueryRetrieveLevel: 'STUDY'
          },
          destination
        ).catch(err => ({ error: err.message, study }))
      )
    );
    
    results.push(...batchResults);
    
    // Progress
    console.log(`Completed ${Math.min(i + concurrency, studies.length)}/${studies.length} studies`);
  }
  
  return results;
}
```

## Error Handling

### Common Errors

```javascript
try {
  await moveScu.moveStudy(query, 'DESTINATION_AE');
} catch (error) {
  if (error.message.includes('Association')) {
    console.error('Failed to connect to PACS:', error);
    // Check network, PACS status, AE titles
  } else if (error.message.includes('Move destination unknown')) {
    console.error('Destination AE not configured in PACS');
    // Add destination to PACS modalities configuration
  } else if (error.message.includes('No matching')) {
    console.error('No instances found matching query');
    // Verify StudyInstanceUID exists
  } else {
    console.error('Move failed:', error);
  }
}
```

### Handling Partial Failures

```javascript
const result = await moveScu.moveStudy(query, 'DESTINATION_AE');

if (result.failed > 0) {
  console.error(
    `Partial failure: ${result.completed} succeeded, ${result.failed} failed`
  );
  
  // Could retry just the failed instances if you tracked UIDs
  // Or log error for manual review
}

if (result.warning > 0) {
  console.warn(
    `${result.warning} instances moved with warnings (check PACS logs)`
  );
}
```

## PACS Configuration Requirements

### Destination AE Configuration

C-MOVE requires the destination AE title to be configured in the source PACS. For Orthanc:

1. Edit `Configuration.json`:
```json
{
  "DicomModalities": {
    "DESTINATION_AE": {
      "AET": "DESTINATION_AE",
      "Host": "192.168.1.100",
      "Port": 4242
    }
  }
}
```

2. Or add via REST API:
```bash
curl -X PUT http://localhost:8042/modalities/DESTINATION_AE \
  -d '{"AET":"DESTINATION_AE", "Host":"192.168.1.100", "Port":4242}'
```

### Common Configuration Issues

| Issue | Cause | Solution |
|-------|-------|----------|
| "Move destination unknown" | Destination AE not configured | Add destination to PACS modalities |
| "Association rejected" | Incorrect AE titles | Verify calling/called AE titles match PACS config |
| No instances moved | Query doesn't match data | Use FindScu first to verify UIDs exist |
| Network timeout | Wrong host/port | Check PACS address and firewall rules |

## Performance Tips

1. **Use appropriate PDU size**: Larger PDU (32KB-64KB) can improve throughput
   ```javascript
   const moveScu = new MoveScu({ maxPduLength: 65536, /* ... */ });
   ```

2. **Move at study level**: More efficient than moving individual series
   ```javascript
   // Efficient
   await moveScu.moveStudy({ StudyInstanceUID: uid, QueryRetrieveLevel: 'STUDY' }, dest);
   
   // Less efficient (multiple operations)
   for (const seriesUID of seriesUIDs) {
     await moveScu.moveStudy({ SeriesInstanceUID: seriesUID, QueryRetrieveLevel: 'SERIES' }, dest);
   }
   ```

3. **Rate limiting**: Don't overwhelm PACS with concurrent moves
   ```javascript
   // Move studies with delay between batches
   for (const study of studies) {
     await moveScu.moveStudy(query, dest);
     await new Promise(resolve => setTimeout(resolve, 100));
   }
   ```

4. **Monitor progress**: Use callbacks to detect stalls or failures early

5. **Network optimization**: Ensure low-latency network between source and destination

## Comparison: C-MOVE vs C-GET

| Aspect | C-MOVE | C-GET |
|--------|--------|-------|
| Data flow | Source → Destination AE | Source → Requester |
| Configuration | Destination must be configured in source | No special configuration |
| Network | 2 associations (SCU→SCP, SCP→Dest) | 1 association |
| Use case | Forward to specific node | Retrieve to local system |
| Storage | Destination handles storage | Requester handles storage |

**When to use C-MOVE:**
- Routing studies to specific PACS
- Load balancing across multiple nodes
- Studies too large to transfer directly
- Forwarding between systems

**When to use C-GET:**
- Retrieving to local viewer
- Destination configuration not feasible
- Direct retrieval needed

## Troubleshooting

### Enable Verbose Logging

```javascript
const moveScu = new MoveScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'MY_SCU',
  calledAeTitle: 'ORTHANC',
  verbose: true  // Enable detailed logs
});
```

### Test with Orthanc

```bash
# Start Orthanc with modalities configured
docker run -p 4242:4242 -v orthanc-config:/etc/orthanc jodogne/orthanc

# Test move operation
node playground/movescu-demo.mjs
```

### Verify Configuration

```javascript
// Test connection first with FindScu
import { FindScu } from 'node-dicom-rs';

const findScu = new FindScu({ /* same config */ });
await findScu.echo();  // Verify connection works
```

## See Also

- [FindScu Documentation](./findscu.md) - Query DICOM archives
- [StoreScu Documentation](./storescu.md) - Send DICOM files
- [QueryBuilder Documentation](./querybuilder.md) - Type-safe query construction
- [DICOM Standard PS3.4](https://dicom.nema.org/medical/dicom/current/output/html/part04.html) - C-MOVE specification

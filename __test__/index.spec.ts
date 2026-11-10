import test from 'ava'
import { createServer as createHttpServer } from 'node:http'
import { createServer as createNetServer } from 'node:net'

import {
  DicomFile,
  WadoManifestAdapter,
  WadoServer,
  WadoStorageType
} from './../index'

function listen(server, host = '127.0.0.1') {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, host, resolve)
  })
}

function close(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => error ? reject(error) : resolve())
  })
}

async function getFreePort() {
  const server = createNetServer()
  await listen(server)
  const port = server.address().port
  await close(server)
  return port
}

test('read dicom file', async (t) => {
  const file = new DicomFile();

  await file.open('./__test__/fixtures/test.dcm');

  const data = file.extract(['PatientName']);

  t.is(data.PatientName, 'CompressedSamples^CT1');
})

test('WADO manifest accepts registered opaque object references', async (t) => {
  const server = new WadoServer(0, {
    storageType: WadoStorageType.S3,
    manifest: {
      adapter: WadoManifestAdapter.Memory,
      sqlitePath: ''
    },
    s3Storages: [{
      storageId: 'private-archive',
      config: {
        bucket: 'dicom-test',
        accessKey: 'test-access-key',
        secretKey: 'test-secret-key',
        endpoint: 'http://127.0.0.1:9000'
      }
    }],
    bindAddress: '127.0.0.1'
  })

  t.deepEqual(await server.getManifestMetrics(), {
    studyCount: 0,
    seriesCount: 0,
    instanceCount: 0,
    ingestBatchCount: 0,
    manifestLoaded: false,
    started: false
  })

  await t.notThrowsAsync(() => server.ingestManifest([{
    studyInstanceUid: '1.2.3',
    seriesInstanceUid: '1.2.3.4',
    sopInstanceUid: '1.2.3.4.5',
    object: {
      storageId: 'private-archive',
      objectKey: 'tenant/opaque-object-id',
      versionId: 'version-7'
    }
  }]))

  await server.ingestManifest([{
    studyInstanceUid: '1.2.9',
    seriesInstanceUid: '1.2.9.1',
    sopInstanceUid: '1.2.9.1.1',
    object: {
      storageId: 'private-archive',
      objectKey: 'tenant/another-opaque-object'
    }
  }])
  t.deepEqual(await server.getManifestMetrics(), {
    studyCount: 2,
    seriesCount: 2,
    instanceCount: 2,
    ingestBatchCount: 2,
    manifestLoaded: true,
    started: false
  })
})

test('WADO manifest rejects unregistered storage and duplicate UIDs', async (t) => {
  const server = new WadoServer(0, {
    storageType: WadoStorageType.S3,
    s3Storages: [{
      storageId: 'private-archive',
      config: {
        bucket: 'dicom-test',
        accessKey: 'test-access-key',
        secretKey: 'test-secret-key',
        endpoint: 'http://127.0.0.1:9000'
      }
    }],
    bindAddress: '127.0.0.1'
  })

  const entry = {
    studyInstanceUid: '1.2.3',
    seriesInstanceUid: '1.2.3.4',
    sopInstanceUid: '1.2.3.4.5',
    object: {
      storageId: 'private-archive',
      objectKey: 'tenant/opaque-object-id',
      versionId: 'version-7'
    }
  }

  await t.throwsAsync(() => server.ingestManifest([{
    ...entry,
    object: { ...entry.object, storageId: 'unregistered' }
  }]))
  await t.throwsAsync(() => server.ingestManifest([entry, entry]))
})

test('Memory manifest allows only incremental ingestion after start', async (t) => {
  const server = new WadoServer(await getFreePort(), {
    storageType: WadoStorageType.S3,
    manifest: { adapter: WadoManifestAdapter.Memory },
    s3Storages: [{
      storageId: 'private-archive',
      config: {
        bucket: 'dicom-test',
        accessKey: 'test-access-key',
        secretKey: 'test-secret-key',
        endpoint: 'http://127.0.0.1:9000'
      }
    }],
    bindAddress: '127.0.0.1'
  })

  await server.ingestManifest([])
  server.start()
  try {
    const arrivingEntry = {
      studyInstanceUid: '1.2.9',
      seriesInstanceUid: '1.2.9.1',
      sopInstanceUid: '1.2.9.1.1',
      object: { storageId: 'private-archive', objectKey: 'opaque-runtime-study' }
    }
    await t.throwsAsync(() => server.ingestManifest([arrivingEntry], { replaceAll: true }))
    await t.notThrowsAsync(() => server.ingestManifest([arrivingEntry]))
  } finally {
    server.stop()
  }
})

test('PostgreSQL manifest requires a connection URL', (t) => {
  t.throws(() => new WadoServer(0, {
    storageType: WadoStorageType.S3,
    manifest: { adapter: WadoManifestAdapter.Postgres },
    s3Storages: [{
      storageId: 'private-archive',
      config: {
        bucket: 'dicom-test',
        accessKey: 'test-access-key',
        secretKey: 'test-secret-key',
        endpoint: 'http://127.0.0.1:9000'
      }
    }],
    bindAddress: '127.0.0.1'
  }), { message: /manifest\.postgresUrl is required/ })
})

test('SQLite manifest replaces and removes entries before serving the full catalog', async (t) => {
  const s3Requests = []
  const storageServer = createHttpServer((request, response) => {
    s3Requests.push(request.url)
    response.writeHead(200, { 'Content-Length': '4' })
    response.end('test')
  })
  await listen(storageServer)
  const storagePort = storageServer.address().port
  const wadoPort = await getFreePort()
  const server = new WadoServer(wadoPort, {
    storageType: WadoStorageType.S3,
    manifest: {
      adapter: WadoManifestAdapter.Sqlite,
      sqlitePath: ':memory:'
    },
    s3Storages: [{
      storageId: 'private-archive',
      config: {
        bucket: 'dicom-test',
        accessKey: 'test-access-key',
        secretKey: 'test-secret-key',
        endpoint: `http://127.0.0.1:${storagePort}`
      }
    }],
    bindAddress: '127.0.0.1'
  })

  try {
    t.throws(() => server.start())
    await server.ingestManifest([])
    t.deepEqual(await server.getManifestMetrics(), {
      studyCount: 0,
      seriesCount: 0,
      instanceCount: 0,
      ingestBatchCount: 1,
      manifestLoaded: true,
      started: false
    })
    const oldEntry = {
      studyInstanceUid: '1.2.3',
      seriesInstanceUid: '1.2.3.4',
      sopInstanceUid: '1.2.3.4.5',
      object: { storageId: 'private-archive', objectKey: 'opaque-old' }
    }
    const removedEntry = {
      ...oldEntry,
      sopInstanceUid: '1.2.3.4.6',
      object: { storageId: 'private-archive', objectKey: 'opaque-removed' }
    }
    const currentEntry = {
      ...oldEntry,
      object: {
        storageId: 'private-archive',
        objectKey: 'opaque-current',
        versionId: 'version-2'
      }
    }

    await server.ingestManifest([oldEntry, removedEntry])
    t.deepEqual(await server.getManifestMetrics(), {
      studyCount: 1,
      seriesCount: 1,
      instanceCount: 2,
      ingestBatchCount: 2,
      manifestLoaded: true,
      started: false
    })
    await server.ingestManifest([currentEntry], { replaceAll: true })
    await t.throwsAsync(() => server.ingestManifest([currentEntry, currentEntry]))
    t.deepEqual(await server.getManifestMetrics(), {
      studyCount: 1,
      seriesCount: 1,
      instanceCount: 1,
      ingestBatchCount: 3,
      manifestLoaded: true,
      started: false
    })
    server.start()
    t.true((await server.getManifestMetrics()).started)

    const currentUrl = `http://127.0.0.1:${wadoPort}/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.5`
    let response
    for (let attempt = 0; attempt < 50; attempt += 1) {
      try {
        response = await fetch(currentUrl, { headers: { Accept: 'application/dicom' } })
        break
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 20))
      }
    }

    t.truthy(response)
    t.is(response.status, 200)
    t.is(await response.text(), 'test')

    const arrivingStudy = {
      studyInstanceUid: '1.2.9',
      seriesInstanceUid: '1.2.9.1',
      sopInstanceUid: '1.2.9.1.1',
      object: { storageId: 'private-archive', objectKey: 'opaque-runtime-study' }
    }
    await t.throwsAsync(() => server.ingestManifest([arrivingStudy], { replaceAll: true }))
    await server.ingestManifest([arrivingStudy])
    t.deepEqual(await server.getManifestMetrics(), {
      studyCount: 2,
      seriesCount: 2,
      instanceCount: 2,
      ingestBatchCount: 4,
      manifestLoaded: true,
      started: true
    })
    const arrivingResponse = await fetch(
      `http://127.0.0.1:${wadoPort}/studies/1.2.9/series/1.2.9.1/instances/1.2.9.1.1`
    )
    t.is(arrivingResponse.status, 200)
    await arrivingResponse.arrayBuffer()

    const removedResponse = await fetch(
      `http://127.0.0.1:${wadoPort}/studies/1.2.3/series/1.2.3.4/instances/1.2.3.4.6`
    )
    t.is(removedResponse.status, 404)
    t.is(s3Requests.length, 2)
    const requestedObject = new URL(s3Requests[0], 'http://localhost')
    t.true(requestedObject.pathname.endsWith('/opaque-current'))
    t.is(requestedObject.searchParams.get('versionId'), 'version-2')
    const runtimeObject = new URL(s3Requests[1], 'http://localhost')
    t.true(runtimeObject.pathname.endsWith('/opaque-runtime-study'))
  } finally {
    server.stop()
    await close(storageServer)
  }
})

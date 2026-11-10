#!/usr/bin/env node

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { WadoServer } from '../index.js';

const configPath = process.argv[2];
if (!configPath) {
  console.error('Usage: node playground/wado-s3-manifest-benchmark.mjs <benchmark-config.json>');
  process.exit(2);
}

const benchmarkConfig = JSON.parse(await readFile(resolve(configPath), 'utf8'));
const { s3Config, entries } = benchmarkConfig;
const iterations = benchmarkConfig.iterations ?? 10;
const basePort = benchmarkConfig.port ?? 18043;

if (!s3Config || !Array.isArray(entries) || entries.length === 0) {
  throw new Error('Config must contain s3Config and a non-empty entries array');
}
if (!Number.isInteger(iterations) || iterations < 1) {
  throw new Error('iterations must be a positive integer');
}
if (entries.some(({ versionId }) => versionId !== undefined && versionId !== null)) {
  throw new Error('Legacy comparison cannot pin S3 versions; omit versionId and keep test objects unchanged');
}
if (entries.some(({ studyInstanceUid, seriesInstanceUid, sopInstanceUid, objectKey }) =>
  objectKey !== `${studyInstanceUid}/${seriesInstanceUid}/${sopInstanceUid}.dcm`)) {
  throw new Error('Legacy comparison requires the UID-derived object key for every entry');
}

const studyUids = new Set(entries.map((entry) => entry.studyInstanceUid));
if (studyUids.size !== 1) {
  throw new Error('All benchmark entries must belong to one study');
}

const firstEntry = entries[0];
const routes = {
  instance: `/studies/${firstEntry.studyInstanceUid}/series/${firstEntry.seriesInstanceUid}/instances/${firstEntry.sopInstanceUid}`,
  instanceMetadata: `/studies/${firstEntry.studyInstanceUid}/series/${firstEntry.seriesInstanceUid}/instances/${firstEntry.sopInstanceUid}/metadata`,
  series: `/studies/${firstEntry.studyInstanceUid}/series/${firstEntry.seriesInstanceUid}`,
  study: `/studies/${firstEntry.studyInstanceUid}`,
};

const commonConfig = {
  bindAddress: '127.0.0.1',
  verbose: true,
};
const legacyServer = new WadoServer(basePort, {
  ...commonConfig,
  storageType: 'S3',
  s3Config,
});
const manifestServer = new WadoServer(basePort + 1, {
  ...commonConfig,
  storageType: 'S3',
  s3Storages: [{ storageId: 'benchmark-store', config: s3Config }],
});

await manifestServer.ingestManifest(entries.map((entry) => ({
  studyInstanceUid: entry.studyInstanceUid,
  seriesInstanceUid: entry.seriesInstanceUid,
  sopInstanceUid: entry.sopInstanceUid,
  object: {
    storageId: 'benchmark-store',
    objectKey: entry.objectKey,
    versionId: entry.versionId,
  },
})));

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

async function requestAndConsume(url) {
  const started = performance.now();
  const response = await fetch(url, { headers: { Accept: 'application/dicom' } });
  const body = await response.arrayBuffer();
  if (!response.ok) {
    throw new Error(`WADO request failed with HTTP ${response.status}`);
  }
  return { elapsedMs: performance.now() - started, bytes: body.byteLength };
}

async function waitUntilReady(url) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      await requestAndConsume(url);
      return;
    } catch {
      await delay(100);
    }
  }
  throw new Error(`WADO server did not become ready at ${url}`);
}

async function measure(url, count) {
  for (let warmup = 0; warmup < 2; warmup += 1) {
    await requestAndConsume(url);
  }

  const samples = [];
  let bytes = 0;
  for (let index = 0; index < count; index += 1) {
    const sample = await requestAndConsume(url);
    samples.push(sample.elapsedMs);
    bytes = sample.bytes;
  }
  samples.sort((left, right) => left - right);
  const middle = Math.floor(samples.length / 2);
  const median = samples.length % 2 === 0
    ? (samples[middle - 1] + samples[middle]) / 2
    : samples[middle];

  return {
    medianMs: Number(median.toFixed(2)),
    p95Ms: Number(samples[Math.min(samples.length - 1, Math.ceil(samples.length * 0.95) - 1)].toFixed(2)),
    bytes,
  };
}

try {
  legacyServer.start();
  manifestServer.start();

  await Promise.all([
    waitUntilReady(`http://127.0.0.1:${basePort}${routes.instance}`),
    waitUntilReady(`http://127.0.0.1:${basePort + 1}${routes.instance}`),
  ]);

  const results = {};
  for (const [operation, route] of Object.entries(routes)) {
    results[operation] = {
      legacy: await measure(`http://127.0.0.1:${basePort}${route}`, iterations),
      manifest: await measure(`http://127.0.0.1:${basePort + 1}${route}`, iterations),
    };
  }

  console.log(JSON.stringify({ iterations, results }, null, 2));
  console.log('Server-side stage timings are emitted as WADO_METRIC lines (lookup, S3 GET, DICOM parse).');
} finally {
  legacyServer.stop();
  manifestServer.stop();
}
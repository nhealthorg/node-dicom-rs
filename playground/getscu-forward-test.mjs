#!/usr/bin/env node

import { FindScu, GetScu } from '../index.mjs';

const SOURCE_ADDR = process.env.GET_SOURCE_ADDR ?? '127.0.0.1:4242';
const SOURCE_CALLED_AE = process.env.GET_SOURCE_CALLED_AE ?? 'ORTHANC';
const SOURCE_CALLING_AE = process.env.GET_SOURCE_CALLING_AE ?? 'GET-FWD-TEST';

const FORWARD_ADDR = process.env.GET_FORWARD_ADDR ?? '127.0.0.1:4446';
const FORWARD_CALLED_AE = process.env.GET_FORWARD_CALLED_AE ?? 'DEMO-SCP';
const FORWARD_CALLING_AE = process.env.GET_FORWARD_CALLING_AE ?? 'FORWARD-SCU';

console.log('GetScu forward test');
console.log(`Source PACS: ${SOURCE_ADDR} (${SOURCE_CALLED_AE})`);
console.log(`Forward target: ${FORWARD_ADDR} (${FORWARD_CALLED_AE})`);

async function findFirstStudy() {
  const studies = [];

  const finder = new FindScu({
    addr: SOURCE_ADDR,
    callingAeTitle: SOURCE_CALLING_AE,
    calledAeTitle: SOURCE_CALLED_AE,
    verbose: false
  });

  await finder.find({
    query: {
      StudyInstanceUID: '',
      PatientName: '',
      StudyDate: ''
    },
    queryModel: 'StudyRoot',
    onResult: (err, event) => {
      if (err || !event.data?.StudyInstanceUID) {
        return;
      }
      studies.push(event.data);
    }
  });

  return studies[0] ?? null;
}

const study = await findFirstStudy();

if (!study?.StudyInstanceUID) {
  console.error('No studies found in source PACS');
  process.exit(1);
}

console.log(`Using study ${study.StudyInstanceUID}`);

const getter = new GetScu({
  addr: SOURCE_ADDR,
  callingAeTitle: SOURCE_CALLING_AE,
  calledAeTitle: SOURCE_CALLED_AE,
  storageBackend: 'Forward',
  forwardTarget: {
    addr: FORWARD_ADDR,
    callingAeTitle: FORWARD_CALLING_AE,
    calledAeTitle: FORWARD_CALLED_AE
  },
  strictForward: true,
  verbose: true
});

const result = await getter.getStudy({
  query: {
    QueryRetrieveLevel: 'STUDY',
    StudyInstanceUID: study.StudyInstanceUID
  },
  queryModel: 'StudyRoot',
  onSubOperation: (err, event) => {
    if (err || !event.data) {
      return;
    }

    const total = event.data.completed + event.data.remaining;
    const suffix = event.data.forwardStatus
      ? ` | forward=${event.data.forwardStatus}${event.data.forwardedTo ? ` (${event.data.forwardedTo})` : ''}`
      : '';

    console.log(`Progress: ${event.data.completed}/${total}${suffix}`);

    if (event.data.forwardError) {
      console.error(`Forward error: ${event.data.forwardError}`);
    }
  },
  onCompleted: (err, event) => {
    if (err || !event.data) {
      return;
    }
    console.log(`Completed in ${event.data.durationSeconds.toFixed(2)}s`);
  }
});

console.log(`Forwarded ${result.completed} of ${result.total} instances`);

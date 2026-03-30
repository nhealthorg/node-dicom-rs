#!/usr/bin/env node

import { FindScu, GetScu } from '../index.mjs';

const OUTPUT_DIR = './playground/test-get-received';

async function findFirstStudy() {
    const studies = [];

    const finder = new FindScu({
        addr: '127.0.0.1:4242',
        callingAeTitle: 'GET-DEMO',
        calledAeTitle: 'ORTHANC',
        verbose: false
    });

    await finder.find({
        query: {
            StudyInstanceUID: '',
            PatientName: '',
            StudyDate: '',
            StudyDescription: ''
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

async function runFilesystemExample(studyInstanceUid) {
    console.log('\nFilesystem backend');

    const getScu = new GetScu({
        addr: '127.0.0.1:4242',
        callingAeTitle: 'GET-DEMO',
        calledAeTitle: 'ORTHANC',
        outDir: OUTPUT_DIR,
        storageBackend: 'Filesystem',
        verbose: true
    });

    const result = await getScu.getStudy({
        query: {
            QueryRetrieveLevel: 'STUDY',
            StudyInstanceUID: studyInstanceUid
        },
        queryModel: 'StudyRoot',
        onSubOperation: (err, event) => {
            if (err || !event.data) {
                return;
            }

            const total = event.data.completed + event.data.remaining;
            console.log(`  Progress: ${event.data.completed}/${total}`);
            if (event.data.file) {
                console.log(`  Stored: ${event.data.file}`);
            }
            if (event.data.forwardStatus) {
                console.log(`  Forward: ${event.data.forwardStatus}`);
                if (event.data.forwardError) {
                    console.log(`  Forward error: ${event.data.forwardError}`);
                }
            }
        },
        onCompleted: (err, event) => {
            if (err || !event.data) {
                return;
            }
            console.log(`  Completed in ${event.data.durationSeconds.toFixed(2)}s`);
        }
    });

    console.log(`Retrieved ${result.completed} of ${result.total} instances`);
    console.log(`Files written under ${OUTPUT_DIR}`);
}

function printS3Example(studyInstanceUid) {
    console.log('\nS3 backend example');
    console.log(`
const getScu = new GetScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'GET-DEMO',
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
    QueryRetrieveLevel: 'STUDY',
    StudyInstanceUID: '${studyInstanceUid}'
  }
});
`);
}

function printForwardExample(studyInstanceUid) {
        console.log('\nForward backend example');
        console.log(`
const getScu = new GetScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'GET-DEMO',
    calledAeTitle: 'ORTHANC',
    storageBackend: 'Forward',
    forwardTarget: {
        addr: '127.0.0.1:11112',
        callingAeTitle: 'FORWARD-SCU',
        calledAeTitle: 'DEST-SCP'
    },
    strictForward: true
});

await getScu.getStudy({
    query: {
        QueryRetrieveLevel: 'STUDY',
        StudyInstanceUID: '${studyInstanceUid}'
    }
});
`);
}

const study = await findFirstStudy();

if (!study?.StudyInstanceUID) {
    console.error('No studies found in Orthanc. Upload test data first.');
    process.exit(1);
}

console.log(`Using study ${study.StudyInstanceUID}`);
await runFilesystemExample(study.StudyInstanceUID);
printS3Example(study.StudyInstanceUID);
printForwardExample(study.StudyInstanceUID);

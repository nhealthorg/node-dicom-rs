#!/usr/bin/env node

import { FindScu, GetScu } from '../index.mjs';

const OUTPUT_DIR = './playground/test-get-received';

console.log('GetScu simple test');

const studies = [];
const finder = new FindScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'GET-TEST',
    calledAeTitle: 'ORTHANC',
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

if (studies.length === 0) {
    console.error('No studies found in Orthanc');
    process.exit(1);
}

const study = studies[0];
console.log(`Testing C-GET for study ${study.StudyInstanceUID}`);

const getScu = new GetScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'GET-TEST',
    calledAeTitle: 'ORTHANC',
    outDir: OUTPUT_DIR,
    storageBackend: 'Filesystem',
    verbose: true
});

const result = await getScu.getStudy({
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
        console.log(`Progress: ${event.data.completed}/${total}`);
    }
});

console.log(`Retrieved ${result.completed} of ${result.total} instances`);
console.log(`Output directory: ${OUTPUT_DIR}`);

#!/usr/bin/env node
/**
 * Simple FindScu Test - Basic C-FIND query
 */

import { FindScu } from '../index.js';

console.log('🔍 Simple FindSCU Test\n');

const finder = new FindScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'FINDSCU',
  calledAeTitle: 'ORTHANC',
  verbose: true
});

console.log('Querying for all studies...\n');

try {
  const results = await finder.find({
    query: {
      StudyInstanceUID: '',
      PatientName: '',
      PatientID: ''
    },
    queryModel: 'StudyRoot',
    onResult: (err, result) => {
      if (err) {
        console.error('❌ Error:', err);
      } else {
        console.log('📋 Match:', result.message);
        if (result.data) {
          console.log('   Data:', JSON.stringify(result.data, null, 2));
        }
      }
    },
    onCompleted: (err, completed) => {
      if (err) {
        console.error('❌ Completion Error:', err);
      } else {
        console.log('✅', completed.message);
      }
    }
  });

  console.log('\n📊 Total results:', results.length);
  process.exit(0);
} catch (error) {
  console.error('\n❌ Query failed:', error.message);
  console.error(error.stack);
  process.exit(1);
}

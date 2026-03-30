#!/usr/bin/env node
/**
 * FindScu Demo - Query DICOM PACS via C-FIND
 * 
 * Run: node playground/findscu-demo.mjs
 * Prerequisites: 
 * 1. Start Orthanc PACS: cd docker && docker-compose up -d orthanc
 * 2. Send test data: node playground/storescu-demo.mjs
 */

import { FindScu } from '../index.js';

console.log('🔍 FindSCU Demo - DICOM C-FIND Query Client\n');

// Configure connection to Orthanc PACS
const finderOptions = {
  addr: '127.0.0.1:4242',
  callingAeTitle: 'FINDSCU',
  calledAeTitle: 'ORTHANC',
  maxPduLength: 16384,
  verbose: true
};

console.log('Configuration:');
console.log(`  PACS Address: ${finderOptions.addr}`);
console.log(`  Calling AE:   ${finderOptions.callingAeTitle}`);
console.log(`  Called AE:    ${finderOptions.calledAeTitle}`);
console.log();

const finder = new FindScu(finderOptions);

/**
 * Demo 1: Find all studies (wildcard query)
 */
async function findAllStudies() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 1: Find All Studies (Wildcard Query)');
  console.log('═══════════════════════════════════════════════════════════\n');

  try {
    const results = await finder.find({
      query: {
        // Query parameters - use empty strings to retrieve all
        StudyInstanceUID: '',
        PatientName: '',
        PatientID: '',
        StudyDate: '',
        StudyDescription: '',
        AccessionNumber: '',
        Modality: ''
      },
      queryModel: 'StudyRoot',
      onResult: (err, result) => {
          if (err) {
            console.error('❌ Error:', err);
          } else {
            console.log('📋 Result:', result.message);
            if (result.data) {
              console.log('   Patient:', result.data.PatientName || 'N/A');
              console.log('   Study UID:', result.data.StudyInstanceUID || 'N/A');
              console.log('   Date:', result.data.StudyDate || 'N/A');
              console.log('   Description:', result.data.StudyDescription || 'N/A');
              console.log();
            }
          }
        },
        onCompleted: (err, completed) => {
          if (err) {
            console.error('❌ Completion Error:', err);
          } else {
            console.log('✅ ' + completed.message);
            console.log();
          }
        }
      }
    );

    console.log(`📊 Total results returned: ${results.length}\n`);
    return results;
  } catch (error) {
    console.error('❌ Query failed:', error.message);
    console.error(error);
  }
}

/**
 * Demo 2: Find studies by patient name (pattern matching)
 */
async function findByPatientName(patientName) {
  console.log('═══════════════════════════════════════════════════════════');
  console.log(`Demo 2: Find Studies by Patient Name: "${patientName}"`);
  console.log('═══════════════════════════════════════════════════════════\n');

  try {
    const results = await finder.find({
      query: {
        PatientName: patientName, // DICOM wildcards: * and ?
        StudyInstanceUID: '',
        StudyDate: '',
        StudyDescription: '',
        Modality: ''
      },
      queryModel: 'StudyRoot',
      onResult: (err, result) => {
          if (!err && result.data) {
            console.log(`  📌 ${result.data.PatientName} - ${result.data.StudyDescription || 'N/A'}`);
          }
        },
        onCompleted: (err, completed) => {
          if (!err) {
            console.log('\n✅ ' + completed.message);
            console.log();
          }
        }
      }
    );

    return results;
  } catch (error) {
    console.error('❌ Query failed:', error.message);
  }
}

/**
 * Demo 3: Find studies by date range
 */
async function findByDateRange(startDate, endDate) {
  console.log('═══════════════════════════════════════════════════════════');
  console.log(`Demo 3: Find Studies by Date Range: ${startDate} - ${endDate}`);
  console.log('═══════════════════════════════════════════════════════════\n');

  try {
    const results = await finder.find({
      query: {
        StudyDate: `${startDate}-${endDate}`, // DICOM date range format
        PatientName: '',
        StudyInstanceUID: '',
        StudyDescription: '',
        Modality: '',
        AccessionNumber: ''
      },
      queryModel: 'StudyRoot',
      onResult: (err, result) => {
          if (!err && result.data) {
            console.log(`  📅 ${result.data.StudyDate} - ${result.data.PatientName} - ${result.data.StudyDescription || 'N/A'}`);
          }
        },
        onCompleted: (err, completed) => {
          if (!err) {
            console.log('\n✅ ' + completed.message);
            console.log();
          }
        }
    });

    return results;
  } catch (error) {
    console.error('❌ Query failed:', error.message);
  }
}

/**
 * Demo 4: Find studies by modality
 */
async function findByModality(modality) {
  console.log('═══════════════════════════════════════════════════════════');
  console.log(`Demo 4: Find Studies by Modality: ${modality}`);
  console.log('═══════════════════════════════════════════════════════════\n');

  try {
    const results = await finder.find({
      query: {
        Modality: modality,
        PatientName: '',
        StudyInstanceUID: '',
        StudyDate: '',
        StudyDescription: '',
        AccessionNumber: ''
      },
      queryModel: 'StudyRoot',
      onResult: (err, result) => {
          if (!err && result.data) {
            console.log(`  🔬 ${result.data.Modality} - ${result.data.PatientName} - ${result.data.StudyDescription || 'N/A'}`);
          }
        },
        onCompleted: (err, completed) => {
          if (!err) {
            console.log('\n✅ ' + completed.message);
            console.log();
          }
        }
    });

    return results;
  } catch (error) {
    console.error('❌ Query failed:', error.message);
  }
}

/**
 * Demo 5: Patient Root Query
 */
async function findPatients() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 5: Find Patients (Patient Root Query)');
  console.log('═══════════════════════════════════════════════════════════\n');

  try {
    const results = await finder.find({
      query: {
        PatientName: '',
        PatientID: '',
        PatientBirthDate: '',
        PatientSex: ''
      },
      queryModel: 'PatientRoot', // Using Patient Root instead of Study Root
      onResult: (err, result) => {
          if (!err && result.data) {
            console.log(`  👤 ${result.data.PatientName} (ID: ${result.data.PatientID || 'N/A'}) - DOB: ${result.data.PatientBirthDate || 'N/A'}`);
          }
        },
        onCompleted: (err, completed) => {
          if (!err) {
            console.log('\n✅ ' + completed.message);
            console.log();
          }
        }
    });

    return results;
  } catch (error) {
    console.error('❌ Query failed:', error.message);
  }
}

/**
 * Demo 6: Advanced query with hex tag codes
 */
async function findWithHexTags() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 6: Query using Hex Tag Codes');
  console.log('═══════════════════════════════════════════════════════════\n');

  try {
    const results = await finder.find({
      query: {
        '00100010': '', // PatientName
        '0020000D': '', // StudyInstanceUID
        '00080020': '', // StudyDate
        '00081030': ''  // StudyDescription
      },
      queryModel: 'StudyRoot',
      onResult: (err, result) => {
          if (!err && result.data) {
            console.log('  📋 Study found');
            for (const [tag, value] of Object.entries(result.data)) {
              console.log(`     ${tag}: ${value}`);
            }
            console.log();
          }
        },
        onCompleted: (err, completed) => {
          if (!err) {
            console.log('✅ ' + completed.message);
            console.log();
          }
        }
    });

    return results;
  } catch (error) {
    console.error('❌ Query failed:', error.message);
  }
}

// Main execution
(async () => {
  try {
    // Run all demos
    await findAllStudies();
    
    // Get the first study's patient name for targeted query
    const allStudies = await finder.find({
      query: { PatientName: '', StudyInstanceUID: '' }
    });
    
    if (allStudies.length > 0 && allStudies[0].attributes?.PatientName) {
      const firstPatientName = allStudies[0].attributes.PatientName;
      await findByPatientName(firstPatientName);
    } else {
      console.log('ℹ️  No studies found in PACS. Skipping patient name query.\n');
    }

    // Query by date range (today and yesterday)
    const today = new Date();
    const yesterday = new Date(today);
    yesterday.setDate(yesterday.getDate() - 7); // Last 7 days
    
    const formatDate = (date) => {
      return date.toISOString().slice(0, 10).replace(/-/g, '');
    };
    
    await findByDateRange(formatDate(yesterday), formatDate(today));

    // Query by modality (CT is common in test data)
    await findByModality('CT');

    // Patient Root query
    await findPatients();

    // Query with hex tags
    await findWithHexTags();

    console.log('═══════════════════════════════════════════════════════════');
    console.log('✅ All demos completed successfully!');
    console.log('═══════════════════════════════════════════════════════════');

  } catch (error) {
    console.error('\n❌ Demo failed:', error);
    process.exit(1);
  }
})();

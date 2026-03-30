#!/usr/bin/env node
/**
 * QueryBuilder Demo - Type-safe DICOM query construction
 * 
 * Demonstrates the intuitive QueryBuilder API for constructing
 * DICOM C-FIND queries without needing to know exact tag names.
 */

import { FindScu, QueryBuilder } from '../index.js';

console.log('🔨 QueryBuilder Demo - Type-Safe DICOM Query Construction\n');

const finder = new FindScu({
  addr: '127.0.0.1:4242',
  callingAeTitle: 'FINDSCU',
  calledAeTitle: 'ORTHANC',
  verbose: false
});

/**
 * Demo 1: Simple study query with builder
 */
async function demo1_simpleStudyQuery() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 1: Simple Study Query');
  console.log('═══════════════════════════════════════════════════════════\n');

  const query = QueryBuilder.study()
    .patientName('*')
    .includeAllReturnAttributes();

  console.log('Query Model:', query.queryModel);
  console.log('Query Params:', query.params);
  console.log();

  const results = await finder.findWithQuery(query);
  
  console.log(`✅ Found ${results.length} studies\n`);
  
  if (results.length > 0) {
    console.log('First result:');
    console.log('  Patient:', results[0].attributes.PatientName);
    console.log('  Study Date:', results[0].attributes.StudyDate);
    console.log('  Description:', results[0].attributes.StudyDescription);
    console.log();
  }
}

/**
 * Demo 2: Study query with specific patient
 */
async function demo2_specificPatient() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 2: Query Specific Patient');
  console.log('═══════════════════════════════════════════════════════════\n');

  const query = QueryBuilder.study()
    .patientName('Fischer^*')  // Any patient with last name Fischer
    .includeAllReturnAttributes();

  const results = await finder.findWithQuery(
    query,
    (err, result) => {
      if (!err && result.data) {
        console.log(`  📋 ${result.data.PatientName} - ${result.data.StudyDescription || 'N/A'}`);
      }
    }
  );
  
  console.log(`\n✅ Found ${results.length} studies\n`);
}

/**
 * Demo 3: Date range query
 */
async function demo3_dateRange() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 3: Study Query with Date Range');
  console.log('═══════════════════════════════════════════════════════════\n');

  const query = QueryBuilder.study()
    .studyDateRange('19990101', '19991231')  // All studies in 1999
    .includeAllReturnAttributes();

  console.log('Query params:', query.params);
  console.log();

  const results = await finder.findWithQuery(
    query,
    (err, result) => {
      if (!err && result.data) {
        console.log(`  📅 ${result.data.StudyDate} - ${result.data.PatientName}`);
      }
    }
  );
  
  console.log(`\n✅ Found ${results.length} studies in 1999\n`);
}

/**
 * Demo 4: Modality-specific query
 */
async function demo4_modalityQuery() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 4: Query by Modality');
  console.log('═══════════════════════════════════════════════════════════\n');

  const query = QueryBuilder.study()
    .modality('CT')
    .studyDescription('*Thorax*')
    .includeAllReturnAttributes();

  const results = await finder.findWithQuery(
    query,
    (err, result) => {
      if (!err && result.data) {
        console.log(`  🔬 ${result.data.Modality} - ${result.data.StudyDescription}`);
      }
    }
  );
  
  console.log(`\n✅ Found ${results.length} CT Thorax studies\n`);
}

/**
 * Demo 5: Complex multi-criteria query
 */
async function demo5_complexQuery() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 5: Complex Multi-Criteria Query');
  console.log('═══════════════════════════════════════════════════════════\n');

  const query = QueryBuilder.study()
    .patientName('*')
    .studyDateFrom('19990101')  // From 1999 onwards
    .modality('CT')
    .includeAllReturnAttributes();

  console.log('Building query with chained methods:');
  console.log('  - Patient name: * (all)');
  console.log('  - Study date: from 19990101');
  console.log('  - Modality: CT');
  console.log();

  const results = await finder.findWithQuery(query);
  
  console.log(`✅ Found ${results.length} matching studies\n`);
  
  // Show summary by patient
  const byPatient = {};
  results.forEach(r => {
    const name = r.attributes.PatientName || 'Unknown';
    byPatient[name] = (byPatient[name] || 0) + 1;
  });
  
  console.log('Results by patient:');
  Object.entries(byPatient).forEach(([name, count]) => {
    console.log(`  ${name}: ${count} study(ies)`);
  });
  console.log();
}

/**
 * Demo 6: Patient Root query
 */
async function demo6_patientQuery() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 6: Patient Root Query');
  console.log('═══════════════════════════════════════════════════════════\n');

  const query = QueryBuilder.patient()
    .patientName('*')
    .includeAllReturnAttributes();

  console.log('Query Model:', query.queryModel);
  console.log();

  const results = await finder.findWithQuery(
    query,
    (err, result) => {
      if (!err && result.data) {
        console.log(`  👤 ${result.data.PatientName} (${result.data.PatientID}) - DOB: ${result.data.PatientBirthDate || 'N/A'}`);
      }
    }
  );
  
  console.log(`\n✅ Found ${results.length} patients\n`);
}

/**
 * Demo 7: Comparison with old API
 */
async function demo7_comparison() {
  console.log('═══════════════════════════════════════════════════════════');
  console.log('Demo 7: API Comparison (Old vs New)');
  console.log('═══════════════════════════════════════════════════════════\n');

  console.log('OLD API (manual tag names):');
  console.log('```javascript');
  console.log('const results = await finder.find({');
  console.log('  PatientName: "DOE^JOHN",');
  console.log('  StudyDate: "20240101-20240131",');
  console.log('  Modality: "CT"');
  console.log('}, "StudyRoot");');
  console.log('```\n');

  console.log('NEW API (type-safe builder):');
  console.log('```javascript');
  console.log('const query = QueryBuilder.study()');
  console.log('  .patientName("DOE^JOHN")');
  console.log('  .studyDateRange("20240101", "20240131")');
  console.log('  .modality("CT")');
  console.log('  .includeAllReturnAttributes();');
  console.log('');
  console.log('const results = await finder.findWithQuery(query);');
  console.log('```\n');

  console.log('✨ Benefits:');
  console.log('  • Autocomplete for all methods');
  console.log('  • Type-safe parameters');
  console.log('  • No need to remember DICOM tag names');
  console.log('  • Fluent, readable API');
  console.log('  • Helper methods like studyDateRange()');
  console.log();
}

// Run all demos
(async () => {
  try {
    await demo1_simpleStudyQuery();
    await demo2_specificPatient();
    await demo3_dateRange();
    await demo4_modalityQuery();
    await demo5_complexQuery();
    await demo6_patientQuery();
    await demo7_comparison();

    console.log('═══════════════════════════════════════════════════════════');
    console.log('✅ All QueryBuilder demos completed!');
    console.log('═══════════════════════════════════════════════════════════');

  } catch (error) {
    console.error('\n❌ Demo failed:', error);
    console.error(error.stack);
    process.exit(1);
  }
})();

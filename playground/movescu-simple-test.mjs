#!/usr/bin/env node
/**
 * Simple MoveScu Test - Minimal setup to test C-MOVE
 * 
 * Prerequisites:
 * 1. StoreSCP running: node playground/storescp-demo.mjs
 * 2. Orthanc running with data
 * 3. DEMO-SCP configured in Orthanc
 */

import { FindScu, MoveScu } from '../index.mjs';

console.log('🔍 MoveScu Diagnostic Test\n');

// Step 1: Check Orthanc connectivity (already verified by shell script)
console.log('Step 1: Orthanc connection...');
console.log('✅ Orthanc is available (verified by setup check)\n');

// Step 2: Find available studies
console.log('Step 2: Finding studies in Orthanc...');
const studies = [];

const findScu = new FindScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'TEST-SCU',
    calledAeTitle: 'ORTHANC',
    verbose: false
});

try {
    await findScu.find({
        query: {
            StudyInstanceUID: '',
            PatientName: '',
            PatientID: '',
            StudyDate: '',
            StudyDescription: '',
            Modality: ''
        },
        queryModel: 'StudyRoot',
        onResult: (err, result) => {
            if (err) {
                console.error('Find error:', err);
                return;
            }
            if (result && result.data && result.data.StudyInstanceUID) {
                studies.push(result.data);
            }
        }
    });
    
    if (studies.length === 0) {
        console.error('❌ No studies found in Orthanc');
        console.error('   Upload some DICOM files first: node playground/storescu-demo.mjs\n');
        process.exit(1);
    }
    
    console.log(`✅ Found ${studies.length} studies\n`);
    
    // Show first few studies
    console.log('Available studies:');
    studies.slice(0, 5).forEach((study, i) => {
        console.log(`  ${i + 1}. ${study.StudyInstanceUID || 'N/A'}`);
        console.log(`     Patient: ${study.PatientName || 'N/A'}`);
        console.log(`     Date: ${study.StudyDate || 'N/A'}`);
    });
    if (studies.length > 5) {
        console.log(`  ... and ${studies.length - 5} more`);
    }
    console.log();
    
} catch (error) {
    console.error('❌ Error finding studies:', error.message);
    process.exit(1);
}

// Step 3: Check if DEMO-SCP is configured in Orthanc
console.log('Step 3: Checking if DEMO-SCP is configured in Orthanc...');
try {
    const response = await fetch('http://localhost:8042/modalities/DEMO-SCP/configuration');
    
    if (response.ok) {
        const config = await response.json();
        console.log('✅ DEMO-SCP is configured:');
        console.log(`   AET: ${config.AET}`);
        console.log(`   Host: ${config.Host}`);
        console.log(`   Port: ${config.Port}\n`);
    } else {
        console.error('❌ DEMO-SCP is not configured in Orthanc');
        console.error('   Run: ./playground/configure-orthanc-demo-scp.sh');
        console.error('   Or manually:');
        console.error('   curl -X PUT http://localhost:8042/modalities/DEMO-SCP \\');
        console.error('     -d \'{"AET":"DEMO-SCP", "Host":"127.0.0.1", "Port":4446}\'\n');
        process.exit(1);
    }
} catch (error) {
    console.error('❌ Cannot check Orthanc configuration:', error.message);
    console.error('   Is Orthanc web interface accessible at http://localhost:8042?\n');
    process.exit(1);
}

// Step 4: Test C-MOVE with first available study
console.log('Step 4: Testing C-MOVE operation...');
const testStudy = studies[0];
console.log(`Will attempt to move: ${testStudy.StudyInstanceUID}`);
console.log(`Patient: ${testStudy.PatientName || 'N/A'}\n`);

const moveScu = new MoveScu({
    addr: '127.0.0.1:4242',
    callingAeTitle: 'MOVE-SCU',
    calledAeTitle: 'ORTHANC',
    verbose: true
});

try {
    console.log('Starting C-MOVE operation...\n');
    
    const result = await moveScu.moveStudy({
        query: {
            QueryRetrieveLevel: 'STUDY',
            StudyInstanceUID: testStudy.StudyInstanceUID
        },
        moveDestination: 'DEMO-SCP',
        queryModel: 'StudyRoot',
        onSubOperation: (err, event) => {
            if (err) {
                console.error('  ❌ Sub-operation error:', err.message);
                return;
            }
            if (event.data) {
                const progress = event.data.total > 0 
                    ? Math.round((event.data.completed / event.data.total) * 100)
                    : 0;
                console.log(`  📊 Progress: ${event.data.completed}/${event.data.total} (${progress}%) - Failed: ${event.data.failed}`);
            }
        },
        onCompleted: (err, event) => {
            if (err) {
                console.error('  ❌ Completion error:', err.message);
                return;
            }
            console.log(`\n  ✅ ${event.message}`);
        }
    });
    
    console.log('\n' + '='.repeat(80));
    console.log('✅ C-MOVE SUCCESSFUL!');
    console.log('='.repeat(80));
    console.log(`Total instances: ${result.total}`);
    console.log(`Successfully moved: ${result.completed}`);
    console.log(`Failed: ${result.failed}`);
    console.log(`Warnings: ${result.warning}`);
    console.log('\nCheck your StoreSCP terminal - files should have arrived!');
    console.log('Files saved to: ./playground/test-received/');
    
} catch (error) {
    console.error('\n' + '='.repeat(80));
    console.error('❌ C-MOVE FAILED');
    console.error('='.repeat(80));
    console.error('Error:', error.message);
    console.error('\nCommon issues:');
    console.error('  1. StoreSCP not running → Start it: node playground/storescp-demo.mjs');
    console.error('  2. StoreSCP on wrong port → Check it\'s listening on 4446');
    console.error('  3. DEMO-SCP config wrong → Verify Host/Port in Orthanc config');
    console.error('  4. Firewall blocking → Check if port 4446 is accessible');
    
    // Try to check if port 4446 is listening
    console.error('\nDebugging steps:');
    console.error('  1. Check if StoreSCP is running:');
    console.error('     lsof -i :4446');
    console.error('  2. Verify DEMO-SCP config:');
    console.error('     curl http://localhost:8042/modalities/DEMO-SCP');
    console.error('  3. Check Orthanc logs:');
    console.error('     docker logs orthanc');
    
    process.exit(1);
}

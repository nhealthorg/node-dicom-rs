#!/usr/bin/env node
/**
 * StoreScu Demo - Send DICOM files via C-STORE with randomized patient data
 * 
 * Run: node playground/storescu-demo.mjs
 * Prerequisites: 
 * 1. Run ./playground/downloadTestData.sh first
 * 2. Start receiver: node playground/storescp-demo.mjs
 */

import { StoreScu, DicomFile } from '../index.js';
import { readdir } from 'fs/promises';
import { join } from 'path';
import { mkdtemp, rm } from 'fs/promises';
import { tmpdir } from 'os';

console.log('📤 StoreSCU Demo - DICOM C-STORE Sender mit randomisierten Patientendaten\n');

// Realistische deutsche Namen für Randomisierung
const FIRST_NAMES = [
  'Anna', 'Ben', 'Clara', 'David', 'Emma', 'Felix', 'Hannah', 'Jonas',
  'Laura', 'Lukas', 'Marie', 'Max', 'Nina', 'Paul', 'Sarah', 'Tim',
  'Sophie', 'Leon', 'Lena', 'Noah', 'Mia', 'Finn', 'Lea', 'Elias'
];

const LAST_NAMES = [
  'Müller', 'Schmidt', 'Schneider', 'Fischer', 'Weber', 'Meyer', 'Wagner',
  'Becker', 'Schulz', 'Hoffmann', 'Koch', 'Bauer', 'Richter', 'Klein',
  'Wolf', 'Schröder', 'Neumann', 'Schwarz', 'Zimmermann', 'Braun', 'Hofmann'
];

const STUDY_DESCRIPTIONS = [
  'CT Abdomen', 'MRT Kopf', 'Röntgen Thorax', 'CT Thorax',
  'MRT Wirbelsäule', 'Ultraschall Abdomen', 'Angiographie'
];

// Hilfsfunktion: Zufälliges Element aus Array
function randomElement(array) {
  return array[Math.floor(Math.random() * array.length)];
}

// Hilfsfunktion: Zufällige Patienten-ID generieren
function generatePatientId() {
  return `PAT${String(Math.floor(Math.random() * 900000) + 100000)}`;
}

// Hilfsfunktion: Zufälliges Geburtsdatum (Alter zwischen 20-80)
function generateBirthDate() {
  const age = Math.floor(Math.random() * 60) + 20;
  const year = new Date().getFullYear() - age;
  const month = String(Math.floor(Math.random() * 12) + 1).padStart(2, '0');
  const day = String(Math.floor(Math.random() * 28) + 1).padStart(2, '0');
  return `${year}${month}${day}`;
}

// Hilfsfunktion: Zufälliges Geschlecht
function generateSex() {
  return Math.random() > 0.5 ? 'M' : 'F';
}

// Hilfsfunktion: Dateien aus Ordner lesen
async function getFilesFromFolder(folderPath) {
  const files = await readdir(folderPath);
  return files
    .filter(f => f.endsWith('.dcm'))
    .map(f => join(folderPath, f));
}

// Hauptfunktion: DICOM-Dateien mit randomisierten Patientendaten vorbereiten
async function prepareRandomizedFiles(inputFolder) {
  console.log('🔄 Bereite Dateien mit randomisierten Patientendaten vor...\n');
  
  // Temporären Ordner erstellen
  const tempDir = await mkdtemp(join(tmpdir(), 'dicom-randomized-'));
  console.log(`📁 Temporärer Ordner: ${tempDir}\n`);
  
  // Alle DICOM-Dateien finden
  const files = await getFilesFromFolder(inputFolder);
  console.log(`📋 Gefunden: ${files.length} DICOM-Dateien\n`);
  
  // Zufällige Patientendaten generieren
  const patientName = `${randomElement(LAST_NAMES)}^${randomElement(FIRST_NAMES)}`;
  const patientId = generatePatientId();
  const birthDate = generateBirthDate();
  const sex = generateSex();
  const studyDesc = randomElement(STUDY_DESCRIPTIONS);
  
  console.log('👤 Generierte Patientendaten:');
  console.log(`   Name: ${patientName}`);
  console.log(`   ID: ${patientId}`);
  console.log(`   Geburtsdatum: ${birthDate}`);
  console.log(`   Geschlecht: ${sex}`);
  console.log(`   Studie: ${studyDesc}\n`);
  
  // Dateien verarbeiten
  for (const filePath of files) {
    const fileName = filePath.split('/').pop();
    const outputPath = join(tempDir, fileName);
    
    const dicomFile = new DicomFile();
    await dicomFile.open(filePath);
    
    // Patient- und Studien-Tags aktualisieren
    dicomFile.updateTags({
      PatientName: patientName,
      PatientID: patientId,
      PatientBirthDate: birthDate,
      PatientSex: sex,
      StudyDescription: studyDesc,
      // Optional: Weitere Tags anonymisieren
      InstitutionName: 'Demo Klinik',
      ReferringPhysicianName: 'DR^DEMO',
    });
    
    await dicomFile.saveAsDicom(outputPath);
    dicomFile.close();
  }
  
  console.log(`✅ ${files.length} Dateien vorbereitet\n`);
  return tempDir;
}

// DICOM-Dateien vorbereiten und senden
const inputFolder = './playground/testdata/1.3.6.1.4.1.9328.50.2.160730';
const tempDir = await prepareRandomizedFiles(inputFolder);

const sender = new StoreScu({
  callingAeTitle: 'DEMO-SCU',
  calledAeTitle: 'ORTHANC',  // Orthanc AE Title
  addr: '127.0.0.1:4242',     // Orthanc DICOM Port
  verbose: false,
  throttleDelayMs: 10,
});

// Temporäre Dateien hinzufügen
sender.addFolder(tempDir);

console.log('🚀 Starte Transfer...\n');

// Send with progress tracking
await sender.send({
  onTransferStarted: (err, event) => {
    if (err) {
      console.error('❌ Transfer error:', err);
      return;
    }
    console.log('✅', event.message);
    console.log('   Dateien gesamt:', event.data?.totalFiles);
    console.log('');
  },
  
  onFileSent: (err, event) => {
    if (err) {
      console.error('❌ File error:', err);
      return;
    }
    const data = event.data;
    if (!data) return;
    
    console.log('✓ Gesendet:', data.file);
    console.log('  SOP Instance:', data.sopInstanceUid);
    console.log('  Dauer:', data.durationSeconds.toFixed(2), 'Sekunden');
  },
  
  onFileError: (err, event) => {
    const data = event.data;
    console.error('✗ Fehlgeschlagen:', data?.file);
    console.error('  Fehler:', data?.error);
  },
  
  onTransferCompleted: async (err, event) => {
    if (err) {
      console.error('❌ Completion error:', err);
      return;
    }
    const data = event.data;
    if (!data) return;
    
    console.log('\n🎉 Transfer abgeschlossen!');
    console.log('   Erfolgreich:', data.successful, '/', data.totalFiles);
    console.log('   Fehlgeschlagen:', data.failed);
    console.log('   Dauer:', data.durationSeconds.toFixed(2), 'Sekunden');
    
    // Temporären Ordner aufräumen
    console.log('\n🧹 Räume temporäre Dateien auf...');
    await rm(tempDir, { recursive: true, force: true });
    console.log('✅ Aufgeräumt!');
  }
});

console.log('\n✅ Demo abgeschlossen!');

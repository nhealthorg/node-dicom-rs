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
import { mkdir, mkdtemp, readFile, readdir, rm } from 'fs/promises';
import { basename, join } from 'path';
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

async function getDicomFiles(folderPath) {
  const entries = await readdir(folderPath, { withFileTypes: true });
  const nestedFiles = await Promise.all(entries.map(async entry => {
    const entryPath = join(folderPath, entry.name);
    if (entry.isDirectory()) return getDicomFiles(entryPath);
    return entry.isFile() && entry.name.toLowerCase().endsWith('.dcm') ? [entryPath] : [];
  }));
  return nestedFiles.flat().sort();
}

async function readPatientIds(filePath) {
  const contents = await readFile(filePath, 'utf8');
  return contents.split(/\r?\n/).slice(1)
    .map(line => line.split('\t')[0]?.trim())
    .filter(Boolean);
}

async function prepareRandomizedFiles(inputFolder, patientIds) {
  console.log('🔄 Bereite DICOM-Kopien mit Test-Patientendaten vor...\n');

  const tempDir = await mkdtemp(join(tmpdir(), 'dicom-randomized-'));
  console.log(`📁 Temporärer Ordner: ${tempDir}\n`);

  try {
    const studyDirs = (await readdir(inputFolder, { withFileTypes: true }))
      .filter(entry => entry.isDirectory() && /^\d+(\.\d+)+$/.test(entry.name))
      .sort((left, right) => left.name.localeCompare(right.name));

    if (studyDirs.length === 0) throw new Error(`Keine Studienordner in ${inputFolder} gefunden`);
    if (patientIds.length < studyDirs.length) {
      throw new Error(`Es gibt ${studyDirs.length} Studien, aber nur ${patientIds.length} Patienten-IDs`);
    }

    let fileCount = 0;
    for (const [index, studyDir] of studyDirs.entries()) {
      const sourceDir = join(inputFolder, studyDir.name);
      const files = await getDicomFiles(sourceDir);
      if (files.length === 0) continue;

      const patientName = `${randomElement(LAST_NAMES)}^${randomElement(FIRST_NAMES)}`;
      const patientId = patientIds[index];
      const outputDir = join(tempDir, studyDir.name);
      await mkdir(outputDir, { recursive: true });

      console.log(`👤 Studie ${studyDir.name}: ${patientName} (ID ${patientId}), ${files.length} Dateien`);

      for (const filePath of files) {
        const dicomFile = new DicomFile();
        try {
          await dicomFile.open(filePath);
          dicomFile.updateTags({
            PatientName: patientName,
            PatientID: patientId,
            PatientBirthDate: generateBirthDate(),
            PatientSex: generateSex(),
            InstitutionName: 'Demo Klinik',
            ReferringPhysicianName: 'DR^DEMO',
          });
          await dicomFile.saveAsDicom(join(outputDir, basename(filePath)));
          fileCount++;
        } finally {
          dicomFile.close();
        }
      }
    }

    console.log(`\n✅ ${fileCount} DICOM-Dateien für ${studyDirs.length} Studien vorbereitet\n`);
    return { tempDir, fileCount };
  } catch (error) {
    await rm(tempDir, { recursive: true, force: true });
    throw error;
  }
}

const inputFolder = './playground/testdata/1.3.6.1.4.1.9328.50.2.160730';
const patientListPath = './playground/testdata/stammdaten.txt';
const patientIds = await readPatientIds(patientListPath);
const { tempDir, fileCount } = await prepareRandomizedFiles('./playground/testdata', patientIds);

const sender = new StoreScu({
  callingAeTitle: 'DEMO-SCU',
  calledAeTitle: 'ORTHANC',  // Orthanc AE Title
  addr: '127.0.0.1:4242',     // Orthanc DICOM Port
  verbose: true,
  throttleDelayMs: 10,
});

sender.addFolder(tempDir);

console.log('🚀 Starte Transfer...\n');

let sentFiles = 0;
let failedFiles = 0;
try {
  await sender.send({
    onTransferStarted: (err, event) => {
      if (err) return console.error('❌ Transfer error:', err);
      console.log('✅', event.message);
      console.log('   Dateien gesamt:', event.data?.totalFiles ?? fileCount, '\n');
    },
    onFileSent: (err, event) => {
      if (err) return console.error('❌ File error:', err);
      sentFiles++;
      if (sentFiles % 100 === 0 || sentFiles === fileCount) {
        console.log(`✓ Fortschritt: ${sentFiles}/${fileCount} Dateien gesendet`);
      }
    },
    onFileError: (_err, event) => {
      failedFiles++;
      console.error('✗ Fehlgeschlagen:', event.data?.file, event.data?.error);
    },
    onTransferCompleted: (err, event) => {
      if (err) return console.error('❌ Completion error:', err);
      const data = event.data;
      if (!data) return;
      console.log('\n🎉 Transfer abgeschlossen!');
      console.log('   Erfolgreich:', data.successful, '/', data.totalFiles);
      console.log('   Fehlgeschlagen:', data.failed);
      console.log('   Dauer:', data.durationSeconds.toFixed(2), 'Sekunden');
    },
  });
  console.log(`\n✅ Demo abgeschlossen: ${sentFiles} gesendet, ${failedFiles} fehlgeschlagen`);
} finally {
  console.log('\n🧹 Räume temporäre Dateien auf...');
  await rm(tempDir, { recursive: true, force: true });
  console.log('✅ Aufgeräumt!');
}

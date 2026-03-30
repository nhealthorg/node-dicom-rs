use napi::bindgen_prelude::*;
use std::collections::HashMap;

use super::QueryModel;

/**
 * Fluent query builder for DICOM C-FIND operations.
 * 
 * Provides a type-safe, intuitive interface for building DICOM queries
 * without needing to know exact DICOM tag names.
 * 
 * @example
 * ```typescript
 * // Study query with patient name and date range
 * const query = QueryBuilder.study()
 *   .patientName("DOE^JOHN")
 *   .studyDateRange("20240101", "20240131")
 *   .modality("CT")
 *   .studyDescription("*Abdomen*");
 * 
 * const results = await finder.findWithQuery(query);
 * ```
 * 
 * @example
 * ```typescript
 * // Patient query
 * const query = QueryBuilder.patient()
 *   .patientId("PAT12345")
 *   .patientBirthDate("19900101")
 *   .patientSex("M");
 * ```
 * 
 * @example
 * ```typescript
 * // Modality worklist query
 * const query = QueryBuilder.modalityWorklist()
 *   .scheduledStationAeTitle("CT1")
 *   .scheduledProcedureStepStartDate("20240315");
 * ```
 */
#[napi]
pub struct QueryBuilder {
    query_model: QueryModel,
    params: HashMap<String, String>,
}

#[napi]
impl QueryBuilder {
    /**
     * Create a new Study Root query builder.
     * 
     * Use this for searching studies in the PACS.
     * 
     * @returns New query builder configured for Study Root queries
     */
    #[napi(factory)]
    pub fn study() -> Self {
        Self {
            query_model: QueryModel::StudyRoot,
            params: HashMap::new(),
        }
    }

    /**
     * Create a new Patient Root query builder.
     * 
     * Use this for searching patients in the PACS.
     * 
     * @returns New query builder configured for Patient Root queries
     */
    #[napi(factory)]
    pub fn patient() -> Self {
        Self {
            query_model: QueryModel::PatientRoot,
            params: HashMap::new(),
        }
    }

    /**
     * Create a new Modality Worklist query builder.
     * 
     * Use this for searching scheduled procedures.
     * 
     * @returns New query builder configured for Modality Worklist queries
     */
    #[napi(factory)]
    pub fn modality_worklist() -> Self {
        Self {
            query_model: QueryModel::ModalityWorklist,
            params: HashMap::new(),
        }
    }

    // ==================== Patient-Level Attributes ====================

    /**
     * Filter by patient name.
     * 
     * Supports DICOM wildcards: * (any characters) and ? (single character)
     * Use ^ to separate name components: "LastName^FirstName^MiddleName"
     * 
     * @param name - Patient name or pattern
     * @example query.patientName("DOE^JOHN")
     * @example query.patientName("DOE^*") // All patients with last name DOE
     */
    #[napi]
    pub fn patient_name(&mut self, name: String) -> &Self {
        self.params.insert("PatientName".to_string(), name);
        self
    }

    /**
     * Filter by patient ID.
     * 
     * @param id - Patient identifier
     * @example query.patientId("PAT12345")
     */
    #[napi]
    pub fn patient_id(&mut self, id: String) -> &Self {
        self.params.insert("PatientID".to_string(), id);
        self
    }

    /**
     * Filter by patient birth date.
     * 
     * Format: YYYYMMDD
     * 
     * @param date - Birth date in DICOM format
     * @example query.patientBirthDate("19900115")
     */
    #[napi]
    pub fn patient_birth_date(&mut self, date: String) -> &Self {
        self.params.insert("PatientBirthDate".to_string(), date);
        self
    }

    /**
     * Filter by patient sex.
     * 
     * @param sex - M (male), F (female), O (other), or empty
     * @example query.patientSex("M")
     */
    #[napi]
    pub fn patient_sex(&mut self, sex: String) -> &Self {
        self.params.insert("PatientSex".to_string(), sex);
        self
    }

    // ==================== Study-Level Attributes ====================

    /**
     * Filter by study instance UID.
     * 
     * @param uid - Study Instance UID
     * @example query.studyInstanceUid("1.2.840.113619.2.55.3.1")
     */
    #[napi]
    pub fn study_instance_uid(&mut self, uid: String) -> &Self {
        self.params.insert("StudyInstanceUID".to_string(), uid);
        self
    }

    /**
     * Filter by study date.
     * 
     * Format: YYYYMMDD
     * 
     * @param date - Study date in DICOM format
     * @example query.studyDate("20240115")
     */
    #[napi]
    pub fn study_date(&mut self, date: String) -> &Self {
        self.params.insert("StudyDate".to_string(), date);
        self
    }

    /**
     * Filter by study date range.
     * 
     * Format: YYYYMMDD-YYYYMMDD
     * 
     * @param start - Start date (YYYYMMDD)
     * @param end - End date (YYYYMMDD)
     * @example query.studyDateRange("20240101", "20240131")
     */
    #[napi]
    pub fn study_date_range(&mut self, start: String, end: String) -> &Self {
        self.params.insert("StudyDate".to_string(), format!("{}-{}", start, end));
        self
    }

    /**
     * Filter by study date from a specific date onwards.
     * 
     * @param start - Start date (YYYYMMDD)
     * @example query.studyDateFrom("20240101") // All studies from Jan 1, 2024
     */
    #[napi]
    pub fn study_date_from(&mut self, start: String) -> &Self {
        self.params.insert("StudyDate".to_string(), format!("{}-", start));
        self
    }

    /**
     * Filter by study date up to a specific date.
     * 
     * @param end - End date (YYYYMMDD)
     * @example query.studyDateTo("20240131") // All studies until Jan 31, 2024
     */
    #[napi]
    pub fn study_date_to(&mut self, end: String) -> &Self {
        self.params.insert("StudyDate".to_string(), format!("-{}", end));
        self
    }

    /**
     * Filter by study time.
     * 
     * Format: HHMMSS or HHMMSS.FFFFFF
     * 
     * @param time - Study time in DICOM format
     * @example query.studyTime("143000")
     */
    #[napi]
    pub fn study_time(&mut self, time: String) -> &Self {
        self.params.insert("StudyTime".to_string(), time);
        self
    }

    /**
     * Filter by accession number.
     * 
     * @param number - Accession number
     * @example query.accessionNumber("ACC123456")
     */
    #[napi]
    pub fn accession_number(&mut self, number: String) -> &Self {
        self.params.insert("AccessionNumber".to_string(), number);
        self
    }

    /**
     * Filter by study description.
     * 
     * Supports wildcards: * and ?
     * 
     * @param description - Study description or pattern
     * @example query.studyDescription("*CT*Abdomen*")
     */
    #[napi]
    pub fn study_description(&mut self, description: String) -> &Self {
        self.params.insert("StudyDescription".to_string(), description);
        self
    }

    /**
     * Filter by study ID.
     * 
     * @param id - Study ID
     * @example query.studyId("STUDY001")
     */
    #[napi]
    pub fn study_id(&mut self, id: String) -> &Self {
        self.params.insert("StudyID".to_string(), id);
        self
    }

    /**
     * Filter by modality.
     * 
     * Common values: CT, MR, US, XA, DX, CR, MG, PT, NM, etc.
     * 
     * @param modality - Modality code
     * @example query.modality("CT")
     */
    #[napi]
    pub fn modality(&mut self, modality: String) -> &Self {
        self.params.insert("Modality".to_string(), modality);
        self
    }

    /**
     * Filter by referring physician name.
     * 
     * Use ^ to separate name components: "LastName^FirstName"
     * 
     * @param name - Referring physician name
     * @example query.referringPhysicianName("SMITH^JOHN")
     */
    #[napi]
    pub fn referring_physician_name(&mut self, name: String) -> &Self {
        self.params.insert("ReferringPhysicianName".to_string(), name);
        self
    }

    // ==================== Series-Level Attributes ====================

    /**
     * Filter by series instance UID.
     * 
     * @param uid - Series Instance UID
     * @example query.seriesInstanceUid("1.2.840.113619.2.55.3.1.1")
     */
    #[napi]
    pub fn series_instance_uid(&mut self, uid: String) -> &Self {
        self.params.insert("SeriesInstanceUID".to_string(), uid);
        self
    }

    /**
     * Filter by series number.
     * 
     * @param number - Series number
     * @example query.seriesNumber("1")
     */
    #[napi]
    pub fn series_number(&mut self, number: String) -> &Self {
        self.params.insert("SeriesNumber".to_string(), number);
        self
    }

    /**
     * Filter by series description.
     * 
     * @param description - Series description
     * @example query.seriesDescription("Axial 5mm")
     */
    #[napi]
    pub fn series_description(&mut self, description: String) -> &Self {
        self.params.insert("SeriesDescription".to_string(), description);
        self
    }

    // ==================== Modality Worklist Attributes ====================

    /**
     * Filter by scheduled station AE title.
     * 
     * @param ae - Station AE title where procedure is scheduled
     * @example query.scheduledStationAeTitle("CT1")
     */
    #[napi]
    pub fn scheduled_station_ae_title(&mut self, ae: String) -> &Self {
        self.params.insert("ScheduledStationAETitle".to_string(), ae);
        self
    }

    /**
     * Filter by scheduled procedure step start date.
     * 
     * Format: YYYYMMDD
     * 
     * @param date - Scheduled start date
     * @example query.scheduledProcedureStepStartDate("20240315")
     */
    #[napi]
    pub fn scheduled_procedure_step_start_date(&mut self, date: String) -> &Self {
        self.params.insert("ScheduledProcedureStepStartDate".to_string(), date);
        self
    }

    /**
     * Filter by scheduled procedure step start time.
     * 
     * Format: HHMMSS
     * 
     * @param time - Scheduled start time
     * @example query.scheduledProcedureStepStartTime("143000")
     */
    #[napi]
    pub fn scheduled_procedure_step_start_time(&mut self, time: String) -> &Self {
        self.params.insert("ScheduledProcedureStepStartTime".to_string(), time);
        self
    }

    /**
     * Filter by scheduled performing physician name.
     * 
     * @param name - Physician name
     * @example query.scheduledPerformingPhysicianName("SMITH^JOHN")
     */
    #[napi]
    pub fn scheduled_performing_physician_name(&mut self, name: String) -> &Self {
        self.params.insert("ScheduledPerformingPhysicianName".to_string(), name);
        self
    }

    // ==================== Query Retrieval ====================

    /**
     * Include all standard return attributes for the query level.
     * 
     * This adds empty values for common DICOM attributes to ensure
     * they are returned in the C-FIND response.
     */
    #[napi]
    pub fn include_all_return_attributes(&mut self) -> &Self {
        match self.query_model {
            QueryModel::StudyRoot => {
                // Include common study-level return attributes
                self.params.entry("QueryRetrieveLevel".to_string()).or_insert_with(|| "STUDY".to_string());
                self.params.entry("StudyInstanceUID".to_string()).or_insert_with(String::new);
                self.params.entry("PatientName".to_string()).or_insert_with(String::new);
                self.params.entry("PatientID".to_string()).or_insert_with(String::new);
                self.params.entry("StudyDate".to_string()).or_insert_with(String::new);
                self.params.entry("StudyTime".to_string()).or_insert_with(String::new);
                self.params.entry("StudyDescription".to_string()).or_insert_with(String::new);
                self.params.entry("AccessionNumber".to_string()).or_insert_with(String::new);
                self.params.entry("Modality".to_string()).or_insert_with(String::new);
                self.params.entry("NumberOfStudyRelatedSeries".to_string()).or_insert_with(String::new);
                self.params.entry("NumberOfStudyRelatedInstances".to_string()).or_insert_with(String::new);
            }
            QueryModel::PatientRoot => {
                self.params.entry("QueryRetrieveLevel".to_string()).or_insert_with(|| "PATIENT".to_string());
                self.params.entry("PatientName".to_string()).or_insert_with(String::new);
                self.params.entry("PatientID".to_string()).or_insert_with(String::new);
                self.params.entry("PatientBirthDate".to_string()).or_insert_with(String::new);
                self.params.entry("PatientSex".to_string()).or_insert_with(String::new);
            }
            QueryModel::ModalityWorklist => {
                self.params.entry("ScheduledStationAETitle".to_string()).or_insert_with(String::new);
                self.params.entry("ScheduledProcedureStepStartDate".to_string()).or_insert_with(String::new);
                self.params.entry("ScheduledProcedureStepStartTime".to_string()).or_insert_with(String::new);
                self.params.entry("Modality".to_string()).or_insert_with(String::new);
                self.params.entry("ScheduledPerformingPhysicianName".to_string()).or_insert_with(String::new);
                self.params.entry("PatientName".to_string()).or_insert_with(String::new);
                self.params.entry("PatientID".to_string()).or_insert_with(String::new);
            }
        }
        self
    }

    /**
     * Get the query model for this builder.
     * 
     * @returns The query/retrieve information model
     */
    #[napi(getter)]
    pub fn query_model(&self) -> QueryModel {
        self.query_model.clone()
    }

    /**
     * Get the query parameters as a JavaScript object.
     * 
     * @returns Query parameters
     */
    #[napi(getter)]
    pub fn params(&self) -> HashMap<String, String> {
        self.params.clone()
    }
}

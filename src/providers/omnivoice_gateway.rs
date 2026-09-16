use super::omnivoice::{
    GenerateProjectOptions, OmniVoiceClient, OmniVoiceConnection, OmniVoiceError,
    OmniVoiceImportResult, OmniVoiceJobSubmission, OmniVoiceRemoteJob,
};

pub trait OmniVoiceProvider {
    fn base_url(&self) -> &str;

    fn test_connection(&self) -> Result<OmniVoiceConnection, OmniVoiceError>;

    fn import_project(
        &self,
        project_id: &str,
        script: &str,
        speak_section_titles: bool,
        max_chunk_words: u32,
        max_chunk_chars: u32,
    ) -> Result<OmniVoiceImportResult, OmniVoiceError>;

    fn generate_project(
        &self,
        project_id: &str,
        options: &GenerateProjectOptions,
        idempotency_key: &str,
    ) -> Result<OmniVoiceJobSubmission, OmniVoiceError>;

    fn get_job(&self, job_id: &str) -> Result<OmniVoiceRemoteJob, OmniVoiceError>;
}

impl OmniVoiceProvider for OmniVoiceClient {
    fn base_url(&self) -> &str {
        OmniVoiceClient::base_url(self)
    }

    fn test_connection(&self) -> Result<OmniVoiceConnection, OmniVoiceError> {
        OmniVoiceClient::test_connection(self)
    }

    fn import_project(
        &self,
        project_id: &str,
        script: &str,
        speak_section_titles: bool,
        max_chunk_words: u32,
        max_chunk_chars: u32,
    ) -> Result<OmniVoiceImportResult, OmniVoiceError> {
        OmniVoiceClient::import_project(
            self,
            project_id,
            script,
            speak_section_titles,
            max_chunk_words,
            max_chunk_chars,
        )
    }

    fn generate_project(
        &self,
        project_id: &str,
        options: &GenerateProjectOptions,
        idempotency_key: &str,
    ) -> Result<OmniVoiceJobSubmission, OmniVoiceError> {
        OmniVoiceClient::generate_project(self, project_id, options, idempotency_key)
    }

    fn get_job(&self, job_id: &str) -> Result<OmniVoiceRemoteJob, OmniVoiceError> {
        OmniVoiceClient::get_job(self, job_id)
    }
}

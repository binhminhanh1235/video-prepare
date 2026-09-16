use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{
    GenerateProjectOptions, OmniVoiceError, OmniVoiceProvider, ProjectError, ProjectStore,
    StoredProject, TaskState,
};

pub const AUDIO_STATUS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioAttemptStatus {
    pub attempt_id: String,
    pub server_base_url: String,
    pub remote_project_id: String,
    pub remote_source_hash: Option<String>,
    pub job_id: Option<String>,
    pub idempotency_key: String,
    pub state: TaskState,
    pub submitted_unix_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFlowStatus {
    pub schema_version: u32,
    pub project_id: String,
    pub state: TaskState,
    pub artifact_content_download: bool,
    pub artifact_content_endpoint: Option<String>,
    #[serde(default)]
    pub attempts: Vec<AudioAttemptStatus>,
}

impl AudioFlowStatus {
    fn initial(project_id: &str) -> Self {
        Self {
            schema_version: AUDIO_STATUS_SCHEMA_VERSION,
            project_id: project_id.to_owned(),
            state: TaskState::Pending,
            artifact_content_download: false,
            artifact_content_endpoint: None,
            attempts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioGenerationSettings {
    pub speak_section_titles: bool,
    pub max_chunk_words: u32,
    pub max_chunk_chars: u32,
    pub generate: GenerateProjectOptions,
}

impl Default for AudioGenerationSettings {
    fn default() -> Self {
        Self {
            speak_section_titles: false,
            max_chunk_words: 24,
            max_chunk_chars: 220,
            generate: GenerateProjectOptions::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRunSummary {
    pub attempt_id: String,
    pub state: TaskState,
    pub remote_project_id: String,
    pub job_id: Option<String>,
    pub artifact_content_download: bool,
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error(transparent)]
    Provider(#[from] OmniVoiceError),

    #[error(transparent)]
    Project(#[from] ProjectError),

    #[error("audio state JSON error for {path}: {message}")]
    Json { path: PathBuf, message: String },

    #[error("audio state I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("remote OmniVoice identity mismatch: {0}")]
    RemoteIdentityMismatch(String),

    #[error(
        "remote generation submission has unknown outcome; durable attempt state was preserved"
    )]
    RemoteSubmitUnknown,

    #[error("no UNKNOWN_REMOTE submission is available for retry")]
    NoUnknownSubmit,

    #[error("latest remote attempt belongs to `{attempt_server}`, not current server `{current_server}`")]
    ServerMismatch {
        attempt_server: String,
        current_server: String,
    },

    #[error("audio state is inconsistent: {0}")]
    StateMismatch(String),
}

pub struct AudioExecutor<'a, P: OmniVoiceProvider> {
    provider: &'a P,
}

impl<'a, P: OmniVoiceProvider> AudioExecutor<'a, P> {
    pub fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    pub fn submit_generation(
        &self,
        project: &mut StoredProject,
        settings: &AudioGenerationSettings,
    ) -> Result<AudioRunSummary, AudioError> {
        let connection = self.provider.test_connection()?;
        let mut status = load_audio_status(&project.root, &project.metadata.project_id)?;

        for attempt in &mut status.attempts {
            if attempt.state == TaskState::Running
                && attempt.server_base_url != self.provider.base_url()
            {
                attempt.state = TaskState::UnknownRemote;
                attempt.last_error = Some(
                    "runtime OmniVoice URL changed before the prior remote job was reconciled"
                        .to_owned(),
                );
            }
        }

        status.artifact_content_download = connection.artifact_content_download;
        status.artifact_content_endpoint = connection.artifact_content_endpoint.clone();

        let source_hash = omnivoice_source_hash(&project.prepared_script.omnivoice.raw_markdown);
        let remote_project_id =
            deterministic_remote_project_id(&project.metadata.project_id, &source_hash);
        let attempt_number = status.attempts.len() + 1;
        let attempt_id = format!("A{attempt_number:04}");
        let idempotency_key = stable_idempotency_key(
            &project.metadata.project_id,
            &project.metadata.input_sha256,
            attempt_number,
        );

        status.attempts.push(AudioAttemptStatus {
            attempt_id: attempt_id.clone(),
            server_base_url: self.provider.base_url().to_owned(),
            remote_project_id: remote_project_id.clone(),
            remote_source_hash: None,
            job_id: None,
            idempotency_key: idempotency_key.clone(),
            state: TaskState::Running,
            submitted_unix_ms: None,
            last_error: None,
        });
        status.state = TaskState::Running;
        persist_audio_and_coarse(project, &status, TaskState::Running)?;

        let imported = match self.provider.import_project(
            &remote_project_id,
            &project.prepared_script.omnivoice.raw_markdown,
            settings.speak_section_titles,
            settings.max_chunk_words,
            settings.max_chunk_chars,
        ) {
            Ok(result) => result,
            Err(error) => {
                let state = if error.is_ambiguous_remote_submit() {
                    TaskState::Interrupted
                } else {
                    TaskState::Failed
                };
                set_latest_error(&mut status, state, error.to_string());
                status.state = state;
                persist_audio_and_coarse(project, &status, state)?;
                return Err(AudioError::Provider(error));
            }
        };

        if imported.project_id != remote_project_id {
            let error = AudioError::RemoteIdentityMismatch(format!(
                "expected project id `{remote_project_id}`, received `{}`",
                imported.project_id
            ));
            set_latest_error(&mut status, TaskState::Failed, error.to_string());
            status.state = TaskState::Failed;
            persist_audio_and_coarse(project, &status, TaskState::Failed)?;
            return Err(error);
        }
        if imported.source_hash != source_hash {
            let error = AudioError::RemoteIdentityMismatch(format!(
                "expected source hash `{source_hash}`, received `{}`",
                imported.source_hash
            ));
            set_latest_error(&mut status, TaskState::Failed, error.to_string());
            status.state = TaskState::Failed;
            persist_audio_and_coarse(project, &status, TaskState::Failed)?;
            return Err(error);
        }

        if let Some(latest) = status.attempts.last_mut() {
            latest.remote_source_hash = Some(imported.source_hash);
            latest.last_error = None;
        }
        save_audio_status(&project.root, &status)?;

        match self.provider.generate_project(
            &remote_project_id,
            &settings.generate,
            &idempotency_key,
        ) {
            Ok(submission) => {
                if submission.job_id.trim().is_empty() {
                    let error = AudioError::RemoteIdentityMismatch(
                        "generate response returned an empty job id".to_owned(),
                    );
                    set_latest_error(&mut status, TaskState::Failed, error.to_string());
                    status.state = TaskState::Failed;
                    persist_audio_and_coarse(project, &status, TaskState::Failed)?;
                    return Err(error);
                }
                if let Some(latest) = status.attempts.last_mut() {
                    latest.job_id = Some(submission.job_id);
                    latest.submitted_unix_ms = Some(now_millis());
                    latest.state = TaskState::Running;
                    latest.last_error = None;
                }
                status.state = TaskState::Running;
                persist_audio_and_coarse(project, &status, TaskState::Running)?;
                Ok(summary(&status))
            }
            Err(error) if error.is_ambiguous_remote_submit() => {
                set_latest_error(&mut status, TaskState::UnknownRemote, error.to_string());
                status.state = TaskState::UnknownRemote;
                persist_audio_and_coarse(project, &status, TaskState::UnknownRemote)?;
                Err(AudioError::RemoteSubmitUnknown)
            }
            Err(error) => {
                set_latest_error(&mut status, TaskState::Failed, error.to_string());
                status.state = TaskState::Failed;
                persist_audio_and_coarse(project, &status, TaskState::Failed)?;
                Err(AudioError::Provider(error))
            }
        }
    }

    pub fn retry_unknown_submit(
        &self,
        project: &mut StoredProject,
        settings: &AudioGenerationSettings,
    ) -> Result<AudioRunSummary, AudioError> {
        self.provider.test_connection()?;
        let mut status = load_audio_status(&project.root, &project.metadata.project_id)?;
        let latest = status.attempts.last().ok_or(AudioError::NoUnknownSubmit)?;
        if latest.state != TaskState::UnknownRemote || latest.job_id.is_some() {
            return Err(AudioError::NoUnknownSubmit);
        }
        ensure_same_server(latest, self.provider.base_url())?;

        let remote_project_id = latest.remote_project_id.clone();
        let idempotency_key = latest.idempotency_key.clone();
        match self.provider.generate_project(
            &remote_project_id,
            &settings.generate,
            &idempotency_key,
        ) {
            Ok(submission) => {
                if submission.job_id.trim().is_empty() {
                    return Err(AudioError::RemoteIdentityMismatch(
                        "generate response returned an empty job id".to_owned(),
                    ));
                }
                let latest = status.attempts.last_mut().expect("checked above");
                latest.job_id = Some(submission.job_id);
                latest.submitted_unix_ms = Some(now_millis());
                latest.state = TaskState::Running;
                latest.last_error = None;
                status.state = TaskState::Running;
                persist_audio_and_coarse(project, &status, TaskState::Running)?;
                Ok(summary(&status))
            }
            Err(error) if error.is_ambiguous_remote_submit() => {
                set_latest_error(&mut status, TaskState::UnknownRemote, error.to_string());
                status.state = TaskState::UnknownRemote;
                persist_audio_and_coarse(project, &status, TaskState::UnknownRemote)?;
                Err(AudioError::RemoteSubmitUnknown)
            }
            Err(error) => {
                set_latest_error(&mut status, TaskState::Failed, error.to_string());
                status.state = TaskState::Failed;
                persist_audio_and_coarse(project, &status, TaskState::Failed)?;
                Err(AudioError::Provider(error))
            }
        }
    }

    pub fn reconnect_latest(
        &self,
        project: &mut StoredProject,
    ) -> Result<AudioRunSummary, AudioError> {
        let mut status = load_audio_status(&project.root, &project.metadata.project_id)?;
        let latest = status.attempts.last().ok_or_else(|| {
            AudioError::StateMismatch("audio attempt history is empty".to_owned())
        })?;
        ensure_same_server(latest, self.provider.base_url())?;
        let job_id = latest
            .job_id
            .clone()
            .ok_or_else(|| AudioError::StateMismatch("latest attempt has no job id".to_owned()))?;

        match self.provider.get_job(&job_id) {
            Ok(remote) => {
                if remote.job_id != job_id {
                    return Err(AudioError::RemoteIdentityMismatch(format!(
                        "expected job id `{job_id}`, received `{}`",
                        remote.job_id
                    )));
                }
                let (attempt_state, flow_state) = map_remote_job_state(&remote.status);
                let latest = status.attempts.last_mut().expect("checked above");
                latest.state = attempt_state;
                latest.last_error = None;
                status.state = flow_state;
                persist_audio_and_coarse(project, &status, flow_state)?;
                Ok(summary(&status))
            }
            Err(error) if error.is_ambiguous_remote_submit() => {
                set_latest_error(&mut status, TaskState::UnknownRemote, error.to_string());
                status.state = TaskState::UnknownRemote;
                persist_audio_and_coarse(project, &status, TaskState::UnknownRemote)?;
                Err(AudioError::Provider(error))
            }
            Err(error) => Err(AudioError::Provider(error)),
        }
    }
}

pub fn audio_status_path(project_root: &Path) -> PathBuf {
    project_root.join("audio-status.json")
}

pub fn load_audio_status(
    project_root: &Path,
    project_id: &str,
) -> Result<AudioFlowStatus, AudioError> {
    let path = audio_status_path(project_root);
    if !path.exists() {
        return Ok(AudioFlowStatus::initial(project_id));
    }
    let bytes = fs::read(&path).map_err(|source| AudioError::Io {
        path: path.clone(),
        source,
    })?;
    let status: AudioFlowStatus =
        serde_json::from_slice(&bytes).map_err(|error| AudioError::Json {
            path: path.clone(),
            message: error.to_string(),
        })?;
    if status.schema_version != AUDIO_STATUS_SCHEMA_VERSION {
        return Err(AudioError::StateMismatch(format!(
            "unsupported audio schema_version {}",
            status.schema_version
        )));
    }
    if status.project_id != project_id {
        return Err(AudioError::StateMismatch(
            "audio project_id does not match local project".to_owned(),
        ));
    }
    Ok(status)
}

pub fn deterministic_remote_project_id(project_id: &str, source_hash: &str) -> String {
    let prefix: String = source_hash.chars().take(12).collect();
    format!("vp-{project_id}-{prefix}")
}

pub fn omnivoice_source_hash(script: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(script.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn stable_idempotency_key(project_id: &str, input_hash: &str, attempt_number: usize) -> String {
    let prefix: String = input_hash.chars().take(16).collect();
    format!("video-prepare:{project_id}:{prefix}:audio:{attempt_number}")
}

pub(crate) fn save_audio_status(
    project_root: &Path,
    status: &AudioFlowStatus,
) -> Result<(), AudioError> {
    let path = audio_status_path(project_root);
    let parent = path
        .parent()
        .ok_or_else(|| AudioError::StateMismatch("audio status path has no parent".to_owned()))?;
    fs::create_dir_all(parent).map_err(|source| AudioError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|source| AudioError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    serde_json::to_writer_pretty(temp.as_file_mut(), status).map_err(|error| AudioError::Json {
        path: path.clone(),
        message: error.to_string(),
    })?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| AudioError::Io {
            path: path.clone(),
            source,
        })?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|source| AudioError::Io {
            path: path.clone(),
            source,
        })?;
    temp.persist(&path).map_err(|error| AudioError::Io {
        path,
        source: error.error,
    })?;
    Ok(())
}

pub(crate) fn persist_audio_and_coarse(
    project: &mut StoredProject,
    status: &AudioFlowStatus,
    coarse_state: TaskState,
) -> Result<(), AudioError> {
    save_audio_status(&project.root, status)?;
    project.status.audio_flow = coarse_state;
    for scene in &mut project.status.scenes {
        scene.audio = coarse_state;
    }
    project.status.overall = derive_overall(project.status.visual_flow, project.status.audio_flow);
    ProjectStore::save_status(&project.root, &project.status)?;
    Ok(())
}

fn derive_overall(visual: TaskState, audio: TaskState) -> TaskState {
    let states = [visual, audio];
    if states.contains(&TaskState::Running) {
        return TaskState::Running;
    }
    if states
        .iter()
        .all(|state| matches!(state, TaskState::Completed | TaskState::Skipped))
    {
        return TaskState::Completed;
    }
    if states.contains(&TaskState::Partial) {
        return TaskState::Partial;
    }
    if states.contains(&TaskState::UnknownRemote) {
        return if states.contains(&TaskState::Completed) {
            TaskState::Partial
        } else {
            TaskState::UnknownRemote
        };
    }
    if states.contains(&TaskState::Interrupted) {
        return if states.contains(&TaskState::Completed) {
            TaskState::Partial
        } else {
            TaskState::Interrupted
        };
    }
    if states.contains(&TaskState::Failed) {
        return if states
            .iter()
            .any(|state| matches!(state, TaskState::Completed | TaskState::Partial))
        {
            TaskState::Partial
        } else {
            TaskState::Failed
        };
    }
    TaskState::Pending
}

fn map_remote_job_state(remote_status: &str) -> (TaskState, TaskState) {
    match remote_status.trim().to_ascii_lowercase().as_str() {
        "completed" => (TaskState::Completed, TaskState::Partial),
        "failed" | "cancelled" => (TaskState::Failed, TaskState::Failed),
        "queued" | "pending" | "running" => (TaskState::Running, TaskState::Running),
        _ => (TaskState::UnknownRemote, TaskState::UnknownRemote),
    }
}

fn ensure_same_server(
    attempt: &AudioAttemptStatus,
    current_server: &str,
) -> Result<(), AudioError> {
    if attempt.server_base_url == current_server {
        Ok(())
    } else {
        Err(AudioError::ServerMismatch {
            attempt_server: attempt.server_base_url.clone(),
            current_server: current_server.to_owned(),
        })
    }
}

fn set_latest_error(status: &mut AudioFlowStatus, state: TaskState, message: String) {
    if let Some(latest) = status.attempts.last_mut() {
        latest.state = state;
        latest.last_error = Some(message);
    }
}

fn summary(status: &AudioFlowStatus) -> AudioRunSummary {
    let latest = status
        .attempts
        .last()
        .expect("audio summary requires at least one attempt");
    AudioRunSummary {
        attempt_id: latest.attempt_id.clone(),
        state: status.state,
        remote_project_id: latest.remote_project_id.clone(),
        job_id: latest.job_id.clone(),
        artifact_content_download: status.artifact_content_download,
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        parse_script, OmniVoiceConnection, OmniVoiceImportResult, OmniVoiceJobSubmission,
        OmniVoiceRemoteJob,
    };
    use std::{collections::VecDeque, sync::Mutex};

    struct MockOmniVoice {
        base_url: String,
        generates: Mutex<VecDeque<Result<OmniVoiceJobSubmission, OmniVoiceError>>>,
        captured_scripts: Mutex<Vec<String>>,
        captured_keys: Mutex<Vec<String>>,
        remote_job: Mutex<OmniVoiceRemoteJob>,
    }

    impl MockOmniVoice {
        fn new(base_url: &str) -> Self {
            Self {
                base_url: base_url.to_owned(),
                generates: Mutex::new(VecDeque::from([Ok(OmniVoiceJobSubmission {
                    job_id: "job-1".to_owned(),
                    status: "queued".to_owned(),
                    location: Some("/api/v1/jobs/job-1".to_owned()),
                })])),
                captured_scripts: Mutex::new(Vec::new()),
                captured_keys: Mutex::new(Vec::new()),
                remote_job: Mutex::new(OmniVoiceRemoteJob {
                    job_id: "job-1".to_owned(),
                    status: "running".to_owned(),
                    kind: Some("generate_project".to_owned()),
                }),
            }
        }

        fn with_generate_results(
            base_url: &str,
            results: Vec<Result<OmniVoiceJobSubmission, OmniVoiceError>>,
        ) -> Self {
            let mut provider = Self::new(base_url);
            provider.generates = Mutex::new(results.into());
            provider
        }
    }

    impl OmniVoiceProvider for MockOmniVoice {
        fn base_url(&self) -> &str {
            &self.base_url
        }

        fn test_connection(&self) -> Result<OmniVoiceConnection, OmniVoiceError> {
            Ok(OmniVoiceConnection {
                base_url: self.base_url.clone(),
                service: Some("omnivoice-studio".to_owned()),
                project_import_endpoint: "/api/v1/projects/import".to_owned(),
                generate_project_endpoint: "/api/v1/projects/{project_id}/generate".to_owned(),
                jobs_endpoint: Some("/api/v1/jobs".to_owned()),
                artifact_content_endpoint: Some(
                    "/api/v1/artifacts/{artifact_id}/content".to_owned(),
                ),
                artifact_content_download: true,
            })
        }

        fn import_project(
            &self,
            project_id: &str,
            script: &str,
            _speak_section_titles: bool,
            _max_chunk_words: u32,
            _max_chunk_chars: u32,
        ) -> Result<OmniVoiceImportResult, OmniVoiceError> {
            self.captured_scripts
                .lock()
                .unwrap()
                .push(script.to_owned());
            Ok(OmniVoiceImportResult {
                project_id: project_id.to_owned(),
                source_hash: omnivoice_source_hash(script),
                created: true,
            })
        }

        fn generate_project(
            &self,
            _project_id: &str,
            _options: &GenerateProjectOptions,
            idempotency_key: &str,
        ) -> Result<OmniVoiceJobSubmission, OmniVoiceError> {
            self.captured_keys
                .lock()
                .unwrap()
                .push(idempotency_key.to_owned());
            self.generates
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    Ok(OmniVoiceJobSubmission {
                        job_id: "job-default".to_owned(),
                        status: "queued".to_owned(),
                        location: None,
                    })
                })
        }

        fn get_job(&self, _job_id: &str) -> Result<OmniVoiceRemoteJob, OmniVoiceError> {
            Ok(self.remote_job.lock().unwrap().clone())
        }
    }

    #[test]
    fn submit_persists_exact_script_remote_identity_and_job_for_restart() {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let store = ProjectStore::new(temp.path());
        let mut project = store.create("demo-audio", raw, &prepared).unwrap();
        let provider = MockOmniVoice::new("https://studio-a.example");

        let summary = AudioExecutor::new(&provider)
            .submit_generation(&mut project, &AudioGenerationSettings::default())
            .unwrap();
        assert_eq!(summary.state, TaskState::Running);
        assert_eq!(summary.job_id.as_deref(), Some("job-1"));
        assert!(summary.artifact_content_download);
        assert_eq!(
            provider.captured_scripts.lock().unwrap().as_slice(),
            &[prepared.omnivoice.raw_markdown.clone()]
        );

        let status = load_audio_status(&project.root, "demo-audio").unwrap();
        let attempt = status.attempts.last().unwrap();
        assert_eq!(attempt.server_base_url, "https://studio-a.example");
        assert_eq!(attempt.job_id.as_deref(), Some("job-1"));
        assert_eq!(
            attempt.remote_source_hash.as_deref(),
            Some(omnivoice_source_hash(&prepared.omnivoice.raw_markdown).as_str())
        );

        let reopened = ProjectStore::open(&project.root).unwrap();
        assert_eq!(reopened.status.audio_flow, TaskState::Running);
        let reopened_audio = load_audio_status(&reopened.root, "demo-audio").unwrap();
        assert_eq!(reopened_audio.attempts[0].job_id.as_deref(), Some("job-1"));
    }

    #[test]
    fn ambiguous_submit_is_unknown_and_retry_reuses_same_idempotency_key() {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let store = ProjectStore::new(temp.path());
        let mut project = store.create("demo-unknown", raw, &prepared).unwrap();
        let provider = MockOmniVoice::with_generate_results(
            "https://studio.example",
            vec![
                Err(OmniVoiceError::Timeout),
                Ok(OmniVoiceJobSubmission {
                    job_id: "job-recovered".to_owned(),
                    status: "queued".to_owned(),
                    location: None,
                }),
            ],
        );
        let executor = AudioExecutor::new(&provider);

        assert!(matches!(
            executor.submit_generation(&mut project, &AudioGenerationSettings::default()),
            Err(AudioError::RemoteSubmitUnknown)
        ));
        let unknown = load_audio_status(&project.root, "demo-unknown").unwrap();
        assert_eq!(unknown.state, TaskState::UnknownRemote);
        assert!(unknown.attempts[0].job_id.is_none());

        let recovered = executor
            .retry_unknown_submit(&mut project, &AudioGenerationSettings::default())
            .unwrap();
        assert_eq!(recovered.job_id.as_deref(), Some("job-recovered"));
        let keys = provider.captured_keys.lock().unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], keys[1]);
    }

    #[test]
    fn changing_server_preserves_prior_attempt_as_unknown_remote() {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let store = ProjectStore::new(temp.path());
        let mut project = store.create("demo-switch", raw, &prepared).unwrap();
        let provider_a = MockOmniVoice::new("https://studio-a.example");
        AudioExecutor::new(&provider_a)
            .submit_generation(&mut project, &AudioGenerationSettings::default())
            .unwrap();

        let provider_b = MockOmniVoice::new("https://studio-b.example");
        AudioExecutor::new(&provider_b)
            .submit_generation(&mut project, &AudioGenerationSettings::default())
            .unwrap();

        let status = load_audio_status(&project.root, "demo-switch").unwrap();
        assert_eq!(status.attempts.len(), 2);
        assert_eq!(status.attempts[0].state, TaskState::UnknownRemote);
        assert_eq!(
            status.attempts[0].server_base_url,
            "https://studio-a.example"
        );
        assert_eq!(status.attempts[1].state, TaskState::Running);
        assert_eq!(
            status.attempts[1].server_base_url,
            "https://studio-b.example"
        );
    }

    #[test]
    fn completed_remote_job_is_partial_until_local_artifact_import_exists() {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let store = ProjectStore::new(temp.path());
        let mut project = store.create("demo-reconnect", raw, &prepared).unwrap();
        let provider = MockOmniVoice::new("https://studio.example");
        AudioExecutor::new(&provider)
            .submit_generation(&mut project, &AudioGenerationSettings::default())
            .unwrap();
        provider.remote_job.lock().unwrap().status = "completed".to_owned();

        let summary = AudioExecutor::new(&provider)
            .reconnect_latest(&mut project)
            .unwrap();
        assert_eq!(summary.state, TaskState::Partial);
        let status = load_audio_status(&project.root, "demo-reconnect").unwrap();
        assert_eq!(status.attempts[0].state, TaskState::Completed);
        assert_eq!(status.state, TaskState::Partial);
        assert_eq!(project.status.audio_flow, TaskState::Partial);
    }
}

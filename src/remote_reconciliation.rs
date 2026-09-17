use thiserror::Error;

use crate::{
    audio::{load_audio_status, persist_audio_and_coarse},
    sync_latest_audio_artifact, AudioArtifactError, OmniVoiceArtifactProvider, OmniVoiceProvider,
    ProjectError, StoredProject, TaskState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAudioDisposition {
    NoAttempt,
    AlreadyComplete,
    Reconciled,
    ChangedServer,
    ExplicitRetryRequired,
    RemoteUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAudioReconciliationReport {
    pub project_id: String,
    pub attempt_id: Option<String>,
    pub attempt_server: Option<String>,
    pub current_server: String,
    pub previous_state: TaskState,
    pub state: TaskState,
    pub disposition: RemoteAudioDisposition,
    pub queried_remote: bool,
    pub artifact_synced: bool,
    pub message: Option<String>,
}

#[derive(Debug, Error)]
pub enum RemoteAudioReconciliationError {
    #[error("audio reconciliation state error: {0}")]
    Audio(String),

    #[error(transparent)]
    Project(#[from] ProjectError),
}

pub fn reconcile_remote_audio<P>(
    provider: &P,
    project: &mut StoredProject,
) -> Result<RemoteAudioReconciliationReport, RemoteAudioReconciliationError>
where
    P: OmniVoiceProvider + OmniVoiceArtifactProvider,
{
    let mut status = load_audio_status(&project.root, &project.metadata.project_id)
        .map_err(|error| RemoteAudioReconciliationError::Audio(error.to_string()))?;
    let previous_state = status.state;
    let current_server = provider.base_url().to_owned();

    let Some(latest_index) = status.attempts.len().checked_sub(1) else {
        return Ok(report(
            project,
            None,
            None,
            current_server,
            previous_state,
            status.state,
            RemoteAudioDisposition::NoAttempt,
            false,
            false,
            Some("no remote audio attempt exists".to_owned()),
        ));
    };
    let latest = status.attempts[latest_index].clone();

    if latest.server_base_url != provider.base_url() {
        if matches!(latest.state, TaskState::Running | TaskState::UnknownRemote)
            || status.state == TaskState::Running
        {
            status.attempts[latest_index].state = TaskState::UnknownRemote;
            status.attempts[latest_index].last_error = Some(format!(
                "remote attempt belongs to `{}` but current runtime OmniVoice URL is `{}`; old job was not queried on the new server",
                latest.server_base_url,
                provider.base_url()
            ));
            status.state = TaskState::UnknownRemote;
            persist_audio_and_coarse(project, &status, TaskState::UnknownRemote)
                .map_err(|error| RemoteAudioReconciliationError::Audio(error.to_string()))?;
        }
        return Ok(report(
            project,
            Some(latest.attempt_id),
            Some(latest.server_base_url),
            current_server,
            previous_state,
            status.state,
            RemoteAudioDisposition::ChangedServer,
            false,
            false,
            Some("current OmniVoice URL does not own the persisted remote attempt".to_owned()),
        ));
    }

    if latest.job_id.is_none() {
        if matches!(latest.state, TaskState::Running | TaskState::UnknownRemote)
            || matches!(status.state, TaskState::Running | TaskState::UnknownRemote)
        {
            status.attempts[latest_index].state = TaskState::UnknownRemote;
            status.attempts[latest_index].last_error = Some(
                "remote submission outcome is unknown and no durable job_id is available; explicit idempotent retry is required"
                    .to_owned(),
            );
            status.state = TaskState::UnknownRemote;
            persist_audio_and_coarse(project, &status, TaskState::UnknownRemote)
                .map_err(|error| RemoteAudioReconciliationError::Audio(error.to_string()))?;
        }
        return Ok(report(
            project,
            Some(latest.attempt_id),
            Some(latest.server_base_url),
            current_server,
            previous_state,
            status.state,
            RemoteAudioDisposition::ExplicitRetryRequired,
            false,
            false,
            Some("no job_id is available, so remote state cannot be queried safely".to_owned()),
        ));
    }

    match sync_latest_audio_artifact(provider, project) {
        Ok(summary) => {
            let disposition = if summary.state == TaskState::Completed && !summary.downloaded {
                RemoteAudioDisposition::AlreadyComplete
            } else {
                RemoteAudioDisposition::Reconciled
            };
            Ok(report(
                project,
                Some(latest.attempt_id),
                Some(latest.server_base_url),
                current_server,
                previous_state,
                summary.state,
                disposition,
                summary.state != TaskState::Completed || summary.downloaded,
                summary.state == TaskState::Completed,
                None,
            ))
        }
        Err(AudioArtifactError::Provider(error)) => {
            let message = format!("remote state could not be proven: {error}");
            mark_unknown(project, status, latest_index, &message)?;
            Ok(report(
                project,
                Some(latest.attempt_id),
                Some(latest.server_base_url),
                current_server,
                previous_state,
                TaskState::UnknownRemote,
                RemoteAudioDisposition::RemoteUnknown,
                true,
                false,
                Some(message),
            ))
        }
        Err(error) => {
            let refreshed = load_audio_status(&project.root, &project.metadata.project_id)
                .map_err(|load_error| {
                    RemoteAudioReconciliationError::Audio(load_error.to_string())
                })?;
            Ok(report(
                project,
                Some(latest.attempt_id),
                Some(latest.server_base_url),
                current_server,
                previous_state,
                refreshed.state,
                RemoteAudioDisposition::Reconciled,
                true,
                false,
                Some(error.to_string()),
            ))
        }
    }
}

fn mark_unknown(
    project: &mut StoredProject,
    mut status: crate::AudioFlowStatus,
    latest_index: usize,
    message: &str,
) -> Result<(), RemoteAudioReconciliationError> {
    status.attempts[latest_index].state = TaskState::UnknownRemote;
    status.attempts[latest_index].last_error = Some(message.to_owned());
    status.state = TaskState::UnknownRemote;
    persist_audio_and_coarse(project, &status, TaskState::UnknownRemote)
        .map_err(|error| RemoteAudioReconciliationError::Audio(error.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn report(
    project: &StoredProject,
    attempt_id: Option<String>,
    attempt_server: Option<String>,
    current_server: String,
    previous_state: TaskState,
    state: TaskState,
    disposition: RemoteAudioDisposition,
    queried_remote: bool,
    artifact_synced: bool,
    message: Option<String>,
) -> RemoteAudioReconciliationReport {
    RemoteAudioReconciliationReport {
        project_id: project.metadata.project_id.clone(),
        attempt_id,
        attempt_server,
        current_server,
        previous_state,
        state,
        disposition,
        queried_remote,
        artifact_synced,
        message,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        sync::{Arc, Mutex},
    };

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::{
        audio::{save_audio_status, AudioAttemptStatus, AudioFlowStatus, AUDIO_STATUS_SCHEMA_VERSION},
        parse_script, GenerateProjectOptions, OmniVoiceArtifact, OmniVoiceArtifactDownload,
        OmniVoiceArtifactTransport, OmniVoiceConnection, OmniVoiceError, OmniVoiceImportResult,
        OmniVoiceJobSubmission, OmniVoiceRemoteJob, ProjectStore,
    };

    #[derive(Clone)]
    struct MockOmniVoice {
        base_url: String,
        job_result: Arc<Mutex<Result<OmniVoiceRemoteJob, OmniVoiceError>>>,
        get_job_calls: Arc<Mutex<u32>>,
        artifact_bytes: Arc<Vec<u8>>,
        expose_artifact: bool,
    }

    impl MockOmniVoice {
        fn new(base_url: &str, job_result: Result<OmniVoiceRemoteJob, OmniVoiceError>) -> Self {
            Self {
                base_url: base_url.to_owned(),
                job_result: Arc::new(Mutex::new(job_result)),
                get_job_calls: Arc::new(Mutex::new(0)),
                artifact_bytes: Arc::new(b"RIFF-remote-audio".to_vec()),
                expose_artifact: false,
            }
        }

        fn with_artifact(mut self) -> Self {
            self.expose_artifact = true;
            self
        }

        fn calls(&self) -> u32 {
            *self.get_job_calls.lock().unwrap()
        }
    }

    impl OmniVoiceProvider for MockOmniVoice {
        fn base_url(&self) -> &str {
            &self.base_url
        }

        fn test_connection(&self) -> Result<OmniVoiceConnection, OmniVoiceError> {
            unreachable!("remote reconciliation does not call test_connection")
        }

        fn import_project(
            &self,
            _project_id: &str,
            _script: &str,
            _speak_section_titles: bool,
            _max_chunk_words: u32,
            _max_chunk_chars: u32,
        ) -> Result<OmniVoiceImportResult, OmniVoiceError> {
            unreachable!("remote reconciliation never imports projects")
        }

        fn generate_project(
            &self,
            _project_id: &str,
            _options: &GenerateProjectOptions,
            _idempotency_key: &str,
        ) -> Result<OmniVoiceJobSubmission, OmniVoiceError> {
            unreachable!("remote reconciliation never submits generation")
        }

        fn get_job(&self, _job_id: &str) -> Result<OmniVoiceRemoteJob, OmniVoiceError> {
            *self.get_job_calls.lock().unwrap() += 1;
            self.job_result.lock().unwrap().clone()
        }
    }

    impl OmniVoiceArtifactProvider for MockOmniVoice {
        fn discover_artifact_transport(
            &self,
        ) -> Result<OmniVoiceArtifactTransport, OmniVoiceError> {
            if !self.expose_artifact {
                return Err(OmniVoiceError::MissingCapability(
                    "artifact_content_download".to_owned(),
                ));
            }
            Ok(OmniVoiceArtifactTransport {
                artifacts_endpoint: "/api/v1/artifacts".to_owned(),
                artifact_content_endpoint: "/api/v1/artifacts/{artifact_id}/content".to_owned(),
            })
        }

        fn list_artifacts(
            &self,
            _transport: &OmniVoiceArtifactTransport,
            project_id: &str,
        ) -> Result<Vec<OmniVoiceArtifact>, OmniVoiceError> {
            if !self.expose_artifact {
                return Ok(Vec::new());
            }
            Ok(vec![OmniVoiceArtifact {
                id: "art_0123456789abcdef".to_owned(),
                kind: "project_audio".to_owned(),
                project_id: Some(project_id.to_owned()),
                section_id: None,
                chunk_id: None,
                filename: "full.wav".to_owned(),
                relative_path: format!("projects/{project_id}/output/full.wav"),
                format: Some("wav".to_owned()),
                size_bytes: self.artifact_bytes.len() as u64,
                duration_seconds: 1.0,
                sample_rate: 24_000,
                channels: 1,
            }])
        }

        fn download_artifact_atomic(
            &self,
            _transport: &OmniVoiceArtifactTransport,
            _artifact_id: &str,
            final_path: &Path,
        ) -> Result<OmniVoiceArtifactDownload, OmniVoiceError> {
            let parent = final_path.parent().unwrap();
            fs::create_dir_all(parent).unwrap();
            fs::write(final_path, self.artifact_bytes.as_slice()).unwrap();
            Ok(OmniVoiceArtifactDownload {
                bytes: self.artifact_bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(self.artifact_bytes.as_slice())),
            })
        }
    }

    fn create_demo() -> (tempfile::TempDir, StoredProject) {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let project = ProjectStore::new(temp.path())
            .create("demo", raw, &prepared)
            .unwrap();
        (temp, project)
    }

    fn persist_attempt(
        project: &mut StoredProject,
        server: &str,
        flow_state: TaskState,
        attempt_state: TaskState,
        job_id: Option<&str>,
    ) {
        let status = AudioFlowStatus {
            schema_version: AUDIO_STATUS_SCHEMA_VERSION,
            project_id: project.metadata.project_id.clone(),
            state: flow_state,
            artifact_content_download: true,
            artifact_content_endpoint: Some("/api/v1/artifacts/{artifact_id}/content".to_owned()),
            attempts: vec![AudioAttemptStatus {
                attempt_id: "A0001".to_owned(),
                server_base_url: server.to_owned(),
                remote_project_id: "remote-demo".to_owned(),
                remote_source_hash: Some("source-hash".to_owned()),
                job_id: job_id.map(str::to_owned),
                idempotency_key: "idempotent".to_owned(),
                state: attempt_state,
                submitted_unix_ms: None,
                last_error: None,
            }],
        };
        save_audio_status(&project.root, &status).unwrap();
        persist_audio_and_coarse(project, &status, flow_state).unwrap();
    }

    #[test]
    fn changed_server_never_queries_old_job_on_new_server() {
        let (_temp, mut project) = create_demo();
        persist_attempt(
            &mut project,
            "https://old.example",
            TaskState::Running,
            TaskState::Running,
            Some("job-1"),
        );
        let provider = MockOmniVoice::new(
            "https://new.example",
            Ok(OmniVoiceRemoteJob {
                job_id: "job-1".to_owned(),
                status: "completed".to_owned(),
                kind: None,
            }),
        );

        let report = reconcile_remote_audio(&provider, &mut project).unwrap();
        assert_eq!(provider.calls(), 0);
        assert_eq!(report.disposition, RemoteAudioDisposition::ChangedServer);
        assert_eq!(report.state, TaskState::UnknownRemote);
        let stored = load_audio_status(&project.root, &project.metadata.project_id).unwrap();
        assert_eq!(stored.state, TaskState::UnknownRemote);
        assert_eq!(stored.attempts[0].state, TaskState::UnknownRemote);
    }

    #[test]
    fn same_server_running_queries_once_and_stays_running() {
        let (_temp, mut project) = create_demo();
        persist_attempt(
            &mut project,
            "https://voice.example",
            TaskState::Running,
            TaskState::Running,
            Some("job-1"),
        );
        let provider = MockOmniVoice::new(
            "https://voice.example",
            Ok(OmniVoiceRemoteJob {
                job_id: "job-1".to_owned(),
                status: "running".to_owned(),
                kind: None,
            }),
        );

        let report = reconcile_remote_audio(&provider, &mut project).unwrap();
        assert_eq!(provider.calls(), 1);
        assert_eq!(report.state, TaskState::Running);
        assert!(report.queried_remote);
    }

    #[test]
    fn timeout_becomes_unknown_remote_not_failed() {
        let (_temp, mut project) = create_demo();
        persist_attempt(
            &mut project,
            "https://voice.example",
            TaskState::Running,
            TaskState::Running,
            Some("job-1"),
        );
        let provider = MockOmniVoice::new("https://voice.example", Err(OmniVoiceError::Timeout));

        let report = reconcile_remote_audio(&provider, &mut project).unwrap();
        assert_eq!(provider.calls(), 1);
        assert_eq!(report.state, TaskState::UnknownRemote);
        let stored = load_audio_status(&project.root, &project.metadata.project_id).unwrap();
        assert_eq!(stored.state, TaskState::UnknownRemote);
        assert_ne!(stored.state, TaskState::Failed);
    }

    #[test]
    fn unknown_without_job_id_requires_explicit_retry_without_network() {
        let (_temp, mut project) = create_demo();
        persist_attempt(
            &mut project,
            "https://voice.example",
            TaskState::UnknownRemote,
            TaskState::UnknownRemote,
            None,
        );
        let provider = MockOmniVoice::new(
            "https://voice.example",
            Ok(OmniVoiceRemoteJob {
                job_id: "unused".to_owned(),
                status: "completed".to_owned(),
                kind: None,
            }),
        );

        let report = reconcile_remote_audio(&provider, &mut project).unwrap();
        assert_eq!(provider.calls(), 0);
        assert_eq!(
            report.disposition,
            RemoteAudioDisposition::ExplicitRetryRequired
        );
        assert_eq!(report.state, TaskState::UnknownRemote);
    }

    #[test]
    fn observed_remote_failure_is_persisted_as_failed() {
        let (_temp, mut project) = create_demo();
        persist_attempt(
            &mut project,
            "https://voice.example",
            TaskState::Running,
            TaskState::Running,
            Some("job-1"),
        );
        let provider = MockOmniVoice::new(
            "https://voice.example",
            Ok(OmniVoiceRemoteJob {
                job_id: "job-1".to_owned(),
                status: "failed".to_owned(),
                kind: None,
            }),
        );

        let report = reconcile_remote_audio(&provider, &mut project).unwrap();
        assert_eq!(provider.calls(), 1);
        assert_eq!(report.state, TaskState::Failed);
        let stored = load_audio_status(&project.root, &project.metadata.project_id).unwrap();
        assert_eq!(stored.state, TaskState::Failed);
    }

    #[test]
    fn completed_remote_job_syncs_canonical_artifact_and_completes_locally() {
        let (_temp, mut project) = create_demo();
        persist_attempt(
            &mut project,
            "https://voice.example",
            TaskState::Running,
            TaskState::Running,
            Some("job-1"),
        );
        let provider = MockOmniVoice::new(
            "https://voice.example",
            Ok(OmniVoiceRemoteJob {
                job_id: "job-1".to_owned(),
                status: "completed".to_owned(),
                kind: None,
            }),
        )
        .with_artifact();

        let report = reconcile_remote_audio(&provider, &mut project).unwrap();
        assert_eq!(provider.calls(), 1);
        assert_eq!(report.state, TaskState::Completed);
        assert!(report.artifact_synced);
        assert!(project.root.join("audio/artifacts/full.wav").is_file());
    }

    #[test]
    fn report_debug_surface_has_no_secret_fields() {
        let (_temp, mut project) = create_demo();
        persist_attempt(
            &mut project,
            "https://voice.example",
            TaskState::Running,
            TaskState::Running,
            Some("job-1"),
        );
        let provider = MockOmniVoice::new(
            "https://voice.example",
            Ok(OmniVoiceRemoteJob {
                job_id: "job-1".to_owned(),
                status: "running".to_owned(),
                kind: None,
            }),
        );
        let report = reconcile_remote_audio(&provider, &mut project).unwrap();
        let rendered = format!("{report:?}").to_ascii_lowercase();
        assert!(!rendered.contains("token"));
        assert!(!rendered.contains("api_key"));
        assert!(!rendered.contains("authorization"));
    }
}

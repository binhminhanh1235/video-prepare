use std::{path::PathBuf, sync::Arc};

use crate::{
    load_audio_status, load_visual_status, sync_latest_audio_artifact, AudioExecutor,
    AudioFlowStatus, AudioGenerationSettings, GenerateProjectOptions, HttpAssetDownloader,
    OmniVoiceClient, PexelsProvider, ProjectError, ProjectStore, QualityPreset,
    RuntimeSettingsSnapshot, StoredProject, TaskState, VisualExecutor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    Run,
    Resume,
    RetryFailed,
}

impl RunAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Run => "Run",
            Self::Resume => "Resume",
            Self::RetryFailed => "Retry Failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowRunDisposition {
    Executed,
    SkippedByPolicy,
    SkippedByAction,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRunReport {
    pub disposition: FlowRunDisposition,
    pub state: Option<TaskState>,
    pub message: Option<String>,
}

impl FlowRunReport {
    fn skipped_by_policy() -> Self {
        Self {
            disposition: FlowRunDisposition::SkippedByPolicy,
            state: None,
            message: Some("SKIPPED_BY_POLICY".to_owned()),
        }
    }

    fn skipped_by_action(state: Option<TaskState>, message: impl Into<String>) -> Self {
        Self {
            disposition: FlowRunDisposition::SkippedByAction,
            state,
            message: Some(message.into()),
        }
    }

    fn executed(state: Option<TaskState>, message: Option<String>) -> Self {
        Self {
            disposition: FlowRunDisposition::Executed,
            state,
            message,
        }
    }

    fn blocked(state: Option<TaskState>, message: impl Into<String>) -> Self {
        Self {
            disposition: FlowRunDisposition::Blocked,
            state,
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRunReport {
    pub project_id: String,
    pub data_root: PathBuf,
    pub settings_revision: u64,
    pub action: RunAction,
    pub visual: FlowRunReport,
    pub audio: FlowRunReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioNextStep {
    SubmitNew,
    RetryUnknownSubmit,
    SyncExisting,
    NoopCompleted,
    SkipByAction,
    BlockedInconsistent,
}

pub fn execute_project_run(
    snapshot: Arc<RuntimeSettingsSnapshot>,
    project_id: &str,
    action: RunAction,
) -> Result<ProjectRunReport, ProjectError> {
    let mut project = ProjectStore::new(&snapshot.safe.data_root).load(project_id)?;

    let visual = execute_visual_flow(&snapshot, &mut project, action);
    let audio = execute_audio_flow(&snapshot, &mut project, action);

    Ok(ProjectRunReport {
        project_id: project.metadata.project_id.clone(),
        data_root: snapshot.safe.data_root.clone(),
        settings_revision: snapshot.revision,
        action,
        visual,
        audio,
    })
}

fn execute_visual_flow(
    snapshot: &RuntimeSettingsSnapshot,
    project: &mut StoredProject,
    action: RunAction,
) -> FlowRunReport {
    if !snapshot.safe.visual_flow_enabled {
        return FlowRunReport::skipped_by_policy();
    }

    let current = load_visual_status(project);
    if action == RunAction::RetryFailed {
        if let Ok(status) = &current {
            if !retryable_state(status.state) {
                return FlowRunReport::skipped_by_action(
                    Some(status.state),
                    format!("visual flow is {:?}, not retryable", status.state),
                );
            }
        }
    }

    let Some(api_key) = snapshot.secrets.pexels_api_key.as_deref() else {
        return FlowRunReport::blocked(
            current.as_ref().ok().map(|status| status.state),
            "Pexels API key is required while Visual Flow is enabled",
        );
    };
    let provider = match PexelsProvider::new(api_key) {
        Ok(provider) => provider,
        Err(error) => {
            return FlowRunReport::blocked(
                current.as_ref().ok().map(|status| status.state),
                format!("Pexels client initialization failed: {error}"),
            );
        }
    };
    let downloader = match HttpAssetDownloader::new() {
        Ok(downloader) => downloader,
        Err(error) => {
            return FlowRunReport::blocked(
                current.as_ref().ok().map(|status| status.state),
                format!("asset downloader initialization failed: {error}"),
            );
        }
    };

    let result = VisualExecutor::new(&provider, &downloader).run(project);
    let persisted = load_visual_status(project).ok().map(|status| status.state);
    match result {
        Ok(summary) => FlowRunReport::executed(Some(summary.state), None),
        Err(error) => FlowRunReport::executed(persisted, Some(error.to_string())),
    }
}

fn execute_audio_flow(
    snapshot: &RuntimeSettingsSnapshot,
    project: &mut StoredProject,
    action: RunAction,
) -> FlowRunReport {
    if !snapshot.safe.audio_flow_enabled {
        return FlowRunReport::skipped_by_policy();
    }

    let status = match load_audio_status(&project.root, &project.metadata.project_id) {
        Ok(status) => status,
        Err(error) => {
            return FlowRunReport::blocked(
                Some(project.status.audio_flow),
                format!("audio state cannot be loaded: {error}"),
            );
        }
    };
    let step = plan_audio_next_step(action, &status, &snapshot.safe.omnivoice_url);
    match step {
        AudioNextStep::NoopCompleted => {
            return FlowRunReport::skipped_by_action(
                Some(TaskState::Completed),
                "audio flow is already completed",
            );
        }
        AudioNextStep::SkipByAction => {
            return FlowRunReport::skipped_by_action(
                Some(status.state),
                format!(
                    "audio flow is {:?}, not selected by {}",
                    status.state,
                    action.label()
                ),
            );
        }
        AudioNextStep::BlockedInconsistent => {
            return FlowRunReport::blocked(
                Some(status.state),
                "audio attempt is RUNNING/PARTIAL without a job id; automatic resubmission is blocked to avoid duplicate remote work",
            );
        }
        AudioNextStep::SubmitNew
        | AudioNextStep::RetryUnknownSubmit
        | AudioNextStep::SyncExisting => {}
    }

    let client = match OmniVoiceClient::new(
        &snapshot.safe.omnivoice_url,
        snapshot.secrets.omnivoice_token.clone(),
    ) {
        Ok(client) => client,
        Err(error) => {
            return FlowRunReport::blocked(
                Some(status.state),
                format!("OmniVoice client initialization failed: {error}"),
            );
        }
    };
    let generation = audio_generation_settings(snapshot);
    let operation = match step {
        AudioNextStep::SubmitNew => AudioExecutor::new(&client)
            .submit_generation(project, &generation)
            .map(|_| ()),
        AudioNextStep::RetryUnknownSubmit => AudioExecutor::new(&client)
            .retry_unknown_submit(project, &generation)
            .map(|_| ()),
        AudioNextStep::SyncExisting => Ok(()),
        _ => unreachable!("handled above"),
    };

    if let Err(error) = operation {
        let persisted = load_audio_status(&project.root, &project.metadata.project_id)
            .ok()
            .map(|value| value.state);
        return FlowRunReport::executed(persisted, Some(error.to_string()));
    }

    match sync_latest_audio_artifact(&client, project) {
        Ok(summary) => FlowRunReport::executed(Some(summary.state), None),
        Err(error) => {
            let persisted = load_audio_status(&project.root, &project.metadata.project_id)
                .ok()
                .map(|value| value.state);
            FlowRunReport::executed(persisted, Some(error.to_string()))
        }
    }
}

pub fn plan_audio_next_step(
    action: RunAction,
    status: &AudioFlowStatus,
    current_server: &str,
) -> AudioNextStep {
    if status.state == TaskState::Completed {
        return AudioNextStep::NoopCompleted;
    }
    if action == RunAction::RetryFailed && !retryable_state(status.state) {
        return AudioNextStep::SkipByAction;
    }

    let Some(latest) = status.attempts.last() else {
        return AudioNextStep::SubmitNew;
    };
    if latest.server_base_url != current_server {
        return AudioNextStep::SubmitNew;
    }

    match status.state {
        TaskState::UnknownRemote => {
            if latest.job_id.is_some() {
                AudioNextStep::SyncExisting
            } else {
                AudioNextStep::RetryUnknownSubmit
            }
        }
        TaskState::Running => {
            if latest.job_id.is_some() {
                AudioNextStep::SyncExisting
            } else {
                AudioNextStep::BlockedInconsistent
            }
        }
        TaskState::Partial => {
            if latest.job_id.is_some() {
                AudioNextStep::SyncExisting
            } else {
                AudioNextStep::BlockedInconsistent
            }
        }
        TaskState::Failed | TaskState::Interrupted | TaskState::Pending => AudioNextStep::SubmitNew,
        TaskState::Completed => AudioNextStep::NoopCompleted,
        TaskState::Skipped => {
            if action == RunAction::RetryFailed {
                AudioNextStep::SkipByAction
            } else {
                AudioNextStep::SubmitNew
            }
        }
    }
}

fn retryable_state(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Failed | TaskState::Partial | TaskState::Interrupted | TaskState::UnknownRemote
    )
}

fn audio_generation_settings(snapshot: &RuntimeSettingsSnapshot) -> AudioGenerationSettings {
    AudioGenerationSettings {
        speak_section_titles: snapshot.safe.read_section_titles,
        max_chunk_words: 24,
        max_chunk_chars: 220,
        generate: GenerateProjectOptions {
            voice_name: Some(snapshot.safe.voice_name.clone()),
            voice_variant: Some(snapshot.safe.voice_variant.clone()),
            language: Some(snapshot.safe.language.clone()),
            sections: None,
            resume: true,
            strict: false,
            quality_preset: Some(quality_name(snapshot.safe.quality_preset).to_owned()),
        },
    }
}

fn quality_name(value: QualityPreset) -> &'static str {
    value.as_str()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        parse_script, AudioAttemptStatus, RuntimeSecrets, SafePreferences,
        AUDIO_STATUS_SCHEMA_VERSION,
    };

    fn audio_status(state: TaskState, server: &str, job_id: Option<&str>) -> AudioFlowStatus {
        AudioFlowStatus {
            schema_version: AUDIO_STATUS_SCHEMA_VERSION,
            project_id: "demo".to_owned(),
            state,
            artifact_content_download: true,
            artifact_content_endpoint: Some("/api/v1/artifacts/{artifact_id}/content".to_owned()),
            attempts: vec![AudioAttemptStatus {
                attempt_id: "A0001".to_owned(),
                server_base_url: server.to_owned(),
                remote_project_id: "vp-demo-hash".to_owned(),
                remote_source_hash: Some("hash".to_owned()),
                job_id: job_id.map(str::to_owned),
                idempotency_key: "secret-ish-internal-key".to_owned(),
                state,
                submitted_unix_ms: None,
                last_error: None,
            }],
        }
    }

    #[test]
    fn audio_plan_prevents_duplicate_same_server_submissions() {
        assert_eq!(
            plan_audio_next_step(
                RunAction::Run,
                &audio_status(TaskState::Running, "https://voice.example", Some("job-1")),
                "https://voice.example",
            ),
            AudioNextStep::SyncExisting
        );
        assert_eq!(
            plan_audio_next_step(
                RunAction::Resume,
                &audio_status(TaskState::UnknownRemote, "https://voice.example", None),
                "https://voice.example",
            ),
            AudioNextStep::RetryUnknownSubmit
        );
        assert_eq!(
            plan_audio_next_step(
                RunAction::Run,
                &audio_status(TaskState::Running, "https://voice.example", None),
                "https://voice.example",
            ),
            AudioNextStep::BlockedInconsistent
        );
    }

    #[test]
    fn changed_server_creates_new_attempt_instead_of_reusing_old_remote_identity() {
        assert_eq!(
            plan_audio_next_step(
                RunAction::Resume,
                &audio_status(TaskState::UnknownRemote, "https://old.example", None),
                "https://new.example",
            ),
            AudioNextStep::SubmitNew
        );
    }

    #[test]
    fn retry_failed_only_selects_retryable_states() {
        for state in [
            TaskState::Pending,
            TaskState::Running,
            TaskState::Completed,
            TaskState::Skipped,
        ] {
            assert_eq!(
                plan_audio_next_step(
                    RunAction::RetryFailed,
                    &audio_status(state, "https://voice.example", Some("job-1")),
                    "https://voice.example",
                ),
                if state == TaskState::Completed {
                    AudioNextStep::NoopCompleted
                } else {
                    AudioNextStep::SkipByAction
                }
            );
        }
        for state in [
            TaskState::Failed,
            TaskState::Interrupted,
            TaskState::Partial,
            TaskState::UnknownRemote,
        ] {
            assert_ne!(
                plan_audio_next_step(
                    RunAction::RetryFailed,
                    &audio_status(state, "https://voice.example", Some("job-1")),
                    "https://voice.example",
                ),
                AudioNextStep::SkipByAction
            );
        }
    }

    #[test]
    fn completed_audio_is_never_resubmitted() {
        assert_eq!(
            plan_audio_next_step(
                RunAction::Run,
                &audio_status(TaskState::Completed, "https://voice.example", Some("job-1")),
                "https://voice.example",
            ),
            AudioNextStep::NoopCompleted
        );
    }

    #[test]
    fn both_flows_off_requires_no_network_and_mutates_no_project_state() {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let project = ProjectStore::new(temp.path())
            .create("demo", raw, &prepared)
            .unwrap();
        let before = fs::read(project.root.join("status.json")).unwrap();
        let snapshot = Arc::new(RuntimeSettingsSnapshot {
            revision: 7,
            safe: SafePreferences {
                data_root: temp.path().to_path_buf(),
                ..SafePreferences::default()
            },
            secrets: RuntimeSecrets::default(),
        });

        let report = execute_project_run(snapshot, "demo", RunAction::Run).unwrap();
        assert_eq!(report.settings_revision, 7);
        assert_eq!(
            report.visual.disposition,
            FlowRunDisposition::SkippedByPolicy
        );
        assert_eq!(
            report.audio.disposition,
            FlowRunDisposition::SkippedByPolicy
        );
        assert_eq!(before, fs::read(project.root.join("status.json")).unwrap());
        let debug = format!("{report:?}");
        assert!(!debug.contains("token"));
        assert!(!debug.contains("api_key"));
    }

    #[test]
    fn missing_visual_key_is_local_block_and_does_not_require_network() {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        ProjectStore::new(temp.path())
            .create("demo", raw, &prepared)
            .unwrap();
        let snapshot = Arc::new(RuntimeSettingsSnapshot {
            revision: 2,
            safe: SafePreferences {
                data_root: temp.path().to_path_buf(),
                visual_flow_enabled: true,
                audio_flow_enabled: false,
                ..SafePreferences::default()
            },
            secrets: RuntimeSecrets::default(),
        });

        let report = execute_project_run(snapshot, "demo", RunAction::Run).unwrap();
        assert_eq!(report.visual.disposition, FlowRunDisposition::Blocked);
        assert_eq!(report.visual.state, Some(TaskState::Pending));
        assert!(report
            .visual
            .message
            .as_deref()
            .unwrap()
            .contains("Pexels API key"));
        assert_eq!(
            report.audio.disposition,
            FlowRunDisposition::SkippedByPolicy
        );
    }
}

use std::{path::Path, sync::Arc};

use crate::{
    append_run_log_event, Diagnostic, DiagnosticCategory, FlowRetryReport, FlowRunDisposition,
    FlowRunReport, FlowTarget, ManualVisualError, ManualVisualImportSummary,
    OmniVoiceArtifactProvider, OmniVoiceProvider, ProjectError, ProjectRunReport,
    RemoteAudioDisposition, RemoteAudioReconciliationError, RemoteAudioReconciliationReport,
    RunAction, RunLogEvent, RuntimeSettingsSnapshot, StoredProject, TaskState,
};

pub fn execute_project_run(
    snapshot: Arc<RuntimeSettingsSnapshot>,
    project_id: &str,
    action: RunAction,
) -> Result<ProjectRunReport, ProjectError> {
    let result = crate::orchestration::execute_project_run(snapshot.clone(), project_id, action);
    if let Ok(report) = &result {
        let project_root = report.data_root.join("projects").join(&report.project_id);
        let secrets = secret_refs(&snapshot);
        let action_name = run_action_name(action);

        best_effort_append(
            &project_root,
            RunLogEvent::new(
                &report.project_id,
                "project_run",
                "completed",
                aggregate_state(&report.visual, &report.audio),
            )
            .with_metadata("action", action_name)
            .with_metadata("settings_revision", report.settings_revision.to_string()),
            &secrets,
        );
        log_flow_report(
            &project_root,
            &report.project_id,
            "visual",
            action_name,
            &report.visual,
            &secrets,
        );
        log_flow_report(
            &project_root,
            &report.project_id,
            "audio",
            action_name,
            &report.audio,
            &secrets,
        );
    }
    result
}

pub fn execute_flow_retry(
    snapshot: Arc<RuntimeSettingsSnapshot>,
    project_id: &str,
    target: FlowTarget,
) -> Result<FlowRetryReport, ProjectError> {
    let result = crate::orchestration::execute_flow_retry(snapshot.clone(), project_id, target);
    if let Ok(report) = &result {
        let project_root = report.data_root.join("projects").join(&report.project_id);
        let secrets = secret_refs(&snapshot);
        let flow = flow_target_name(report.target);
        let mut event = RunLogEvent::new(
            &report.project_id,
            "flow_retry",
            disposition_name(report.flow.disposition),
            report.flow.state,
        )
        .with_metadata("flow", flow)
        .with_metadata("settings_revision", report.settings_revision.to_string());
        if let Some(diagnostic) = flow_diagnostic(flow, &report.flow) {
            event = event.with_diagnostic(diagnostic);
        }
        best_effort_append(&project_root, event, &secrets);
    }
    result
}

pub fn reconcile_remote_audio<P>(
    provider: &P,
    project: &mut StoredProject,
) -> Result<RemoteAudioReconciliationReport, RemoteAudioReconciliationError>
where
    P: OmniVoiceProvider + OmniVoiceArtifactProvider,
{
    let result = crate::remote_reconciliation::reconcile_remote_audio(provider, project);
    let event = match &result {
        Ok(report) => {
            let mut event = RunLogEvent::new(
                &project.metadata.project_id,
                "remote_audio_reconcile",
                remote_disposition_name(report.disposition),
                Some(report.state),
            )
            .with_metadata("queried_remote", report.queried_remote.to_string())
            .with_metadata("artifact_synced", report.artifact_synced.to_string());
            if let Some(attempt_id) = &report.attempt_id {
                event = event.with_metadata("attempt_id", attempt_id);
            }
            if let Some(diagnostic) = remote_diagnostic(report) {
                event = event.with_diagnostic(diagnostic);
            }
            event
        }
        Err(error) => RunLogEvent::new(
            &project.metadata.project_id,
            "remote_audio_reconcile",
            "error",
            Some(project.status.audio_flow),
        )
        .with_diagnostic(Diagnostic::new(
            "AUDIO_RECONCILIATION_ERROR",
            DiagnosticCategory::Internal,
            true,
            error.to_string(),
        )),
    };
    best_effort_append(&project.root, event, &[]);
    result
}

pub fn import_manual_visual_asset(
    project: &mut StoredProject,
    scene_id: &str,
    visual_id: &str,
    source_path: impl AsRef<Path>,
) -> Result<ManualVisualImportSummary, ManualVisualError> {
    let result = crate::manual_visual::import_manual_visual_asset(
        project,
        scene_id,
        visual_id,
        source_path,
    );
    let event = match &result {
        Ok(summary) => RunLogEvent::new(
            &summary.project_id,
            "manual_visual_import",
            if summary.already_present {
                "already_present"
            } else {
                "imported"
            },
            Some(summary.flow_state),
        )
        .with_metadata("scene_id", &summary.scene_id)
        .with_metadata("visual_id", &summary.visual_id)
        .with_metadata("slot", summary.slot.to_string())
        .with_metadata("relative_path", &summary.relative_path),
        Err(error) => RunLogEvent::new(
            &project.metadata.project_id,
            "manual_visual_import",
            "error",
            Some(project.status.visual_flow),
        )
        .with_metadata("scene_id", scene_id)
        .with_metadata("visual_id", visual_id)
        .with_diagnostic(manual_visual_diagnostic(error)),
    };
    best_effort_append(&project.root, event, &[]);
    result
}

fn log_flow_report(
    project_root: &Path,
    project_id: &str,
    flow: &'static str,
    action: &'static str,
    report: &FlowRunReport,
    secrets: &[&str],
) {
    let mut event = RunLogEvent::new(
        project_id,
        "flow_run",
        disposition_name(report.disposition),
        report.state,
    )
    .with_metadata("flow", flow)
    .with_metadata("action", action);
    if let Some(diagnostic) = flow_diagnostic(flow, report) {
        event = event.with_diagnostic(diagnostic);
    }
    best_effort_append(project_root, event, secrets);
}

fn flow_diagnostic(flow: &str, report: &FlowRunReport) -> Option<Diagnostic> {
    let message = report.message.as_deref()?;
    if report.state == Some(TaskState::UnknownRemote) {
        return Some(Diagnostic::remote_unknown(message));
    }
    match report.disposition {
        FlowRunDisposition::Blocked => Some(Diagnostic::new(
            if flow == "audio" {
                "AUDIO_FLOW_BLOCKED"
            } else {
                "VISUAL_FLOW_BLOCKED"
            },
            DiagnosticCategory::Configuration,
            false,
            message,
        )),
        FlowRunDisposition::Executed if report.state == Some(TaskState::Failed) => {
            Some(Diagnostic::new(
                if flow == "audio" {
                    "AUDIO_FLOW_FAILED"
                } else {
                    "VISUAL_FLOW_FAILED"
                },
                DiagnosticCategory::Internal,
                true,
                message,
            ))
        }
        FlowRunDisposition::Executed if report.state == Some(TaskState::Partial) => {
            Some(Diagnostic::new(
                if flow == "audio" {
                    "AUDIO_FLOW_PARTIAL"
                } else {
                    "VISUAL_FLOW_PARTIAL"
                },
                DiagnosticCategory::Integrity,
                true,
                message,
            ))
        }
        _ => None,
    }
}

fn remote_diagnostic(report: &RemoteAudioReconciliationReport) -> Option<Diagnostic> {
    let message = report.message.as_deref()?;
    match report.disposition {
        RemoteAudioDisposition::RemoteUnknown | RemoteAudioDisposition::ExplicitRetryRequired => {
            Some(Diagnostic::remote_unknown(message))
        }
        RemoteAudioDisposition::ChangedServer => Some(Diagnostic::new(
            "AUDIO_SERVER_CHANGED",
            DiagnosticCategory::Configuration,
            false,
            message,
        )),
        _ if report.state == TaskState::Failed => Some(Diagnostic::remote_failure(message)),
        _ if report.state == TaskState::Partial => Some(Diagnostic::new(
            "AUDIO_ARTIFACT_INCOMPLETE",
            DiagnosticCategory::Integrity,
            true,
            message,
        )),
        _ => None,
    }
}

fn manual_visual_diagnostic(error: &ManualVisualError) -> Diagnostic {
    match error {
        ManualVisualError::SourceMissing(_)
        | ManualVisualError::SourceNotFile(_)
        | ManualVisualError::EmptySource(_)
        | ManualVisualError::UnsupportedExtension(_)
        | ManualVisualError::SceneNotFound(_)
        | ManualVisualError::VisualNotFound { .. }
        | ManualVisualError::MediaMismatch { .. }
        | ManualVisualError::RequestAlreadyComplete { .. } => Diagnostic::new(
            "MANUAL_VISUAL_INVALID_INPUT",
            DiagnosticCategory::Validation,
            false,
            "manual visual input or target is invalid",
        ),
        ManualVisualError::DestinationOccupied(_) => Diagnostic::new(
            "MANUAL_VISUAL_DESTINATION_OCCUPIED",
            DiagnosticCategory::Integrity,
            false,
            "manual visual destination is occupied by different content",
        ),
        ManualVisualError::Io { .. } => Diagnostic::storage(
            "MANUAL_VISUAL_IO",
            "manual visual import encountered a local I/O error",
        ),
        ManualVisualError::Json { .. }
        | ManualVisualError::Visual(_)
        | ManualVisualError::Project(_)
        | ManualVisualError::Reconciliation(_)
        | ManualVisualError::InvalidState(_) => Diagnostic::new(
            "MANUAL_VISUAL_STATE_ERROR",
            DiagnosticCategory::Internal,
            true,
            "manual visual import could not update durable project state",
        ),
    }
}

fn aggregate_state(visual: &FlowRunReport, audio: &FlowRunReport) -> Option<TaskState> {
    let states = [visual.state, audio.state];
    if states.iter().flatten().any(|state| *state == TaskState::Failed) {
        Some(TaskState::Failed)
    } else if states
        .iter()
        .flatten()
        .any(|state| *state == TaskState::UnknownRemote)
    {
        Some(TaskState::UnknownRemote)
    } else if states.iter().flatten().any(|state| *state == TaskState::Partial) {
        Some(TaskState::Partial)
    } else if states
        .iter()
        .flatten()
        .any(|state| *state == TaskState::Running)
    {
        Some(TaskState::Running)
    } else if states
        .iter()
        .flatten()
        .all(|state| *state == TaskState::Completed)
        && states.iter().any(Option::is_some)
    {
        Some(TaskState::Completed)
    } else {
        states.into_iter().flatten().next()
    }
}

fn secret_refs(snapshot: &RuntimeSettingsSnapshot) -> Vec<&str> {
    let mut values = Vec::with_capacity(2);
    if let Some(value) = snapshot.secrets.pexels_api_key.as_deref() {
        values.push(value);
    }
    if let Some(value) = snapshot.secrets.omnivoice_token.as_deref() {
        values.push(value);
    }
    values
}

fn best_effort_append(project_root: &Path, event: RunLogEvent, secrets: &[&str]) {
    let _ = append_run_log_event(project_root, event, secrets);
}

fn run_action_name(action: RunAction) -> &'static str {
    match action {
        RunAction::Run => "run",
        RunAction::Resume => "resume",
        RunAction::RetryFailed => "retry_failed",
    }
}

fn flow_target_name(target: FlowTarget) -> &'static str {
    match target {
        FlowTarget::Visual => "visual",
        FlowTarget::Audio => "audio",
    }
}

fn disposition_name(disposition: FlowRunDisposition) -> &'static str {
    match disposition {
        FlowRunDisposition::Executed => "executed",
        FlowRunDisposition::SkippedByPolicy => "skipped_by_policy",
        FlowRunDisposition::SkippedByAction => "skipped_by_action",
        FlowRunDisposition::Blocked => "blocked",
    }
}

fn remote_disposition_name(disposition: RemoteAudioDisposition) -> &'static str {
    match disposition {
        RemoteAudioDisposition::NoAttempt => "no_attempt",
        RemoteAudioDisposition::AlreadyComplete => "already_complete",
        RemoteAudioDisposition::Reconciled => "reconciled",
        RemoteAudioDisposition::ChangedServer => "changed_server",
        RemoteAudioDisposition::ExplicitRetryRequired => "explicit_retry_required",
        RemoteAudioDisposition::RemoteUnknown => "remote_unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_script, RuntimeSecrets, SafePreferences};
    use std::fs;

    const SCRIPT: &str = r#"--- SCENES ---
format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: image
        queries: ["calm window"]
        count: 1
--- OMNIVOICE ---
# Demo

## S01 - 0:00 - 0:05

[WARM] Hello.
"#;

    #[test]
    fn project_run_appends_parseable_redacted_flow_events() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = parse_script(SCRIPT).unwrap();
        crate::ProjectStore::new(temp.path())
            .create("demo", SCRIPT, &prepared)
            .unwrap();
        let secret = "pexels-do-not-log";
        let snapshot = Arc::new(RuntimeSettingsSnapshot {
            revision: 7,
            safe: SafePreferences {
                data_root: temp.path().to_path_buf(),
                visual_flow_enabled: false,
                audio_flow_enabled: false,
                ..SafePreferences::default()
            },
            secrets: RuntimeSecrets {
                pexels_api_key: Some(secret.to_owned()),
                omnivoice_token: Some("voice-do-not-log".to_owned()),
            },
        });

        let report = execute_project_run(snapshot, "demo", RunAction::Run).unwrap();
        assert_eq!(report.project_id, "demo");
        let raw = fs::read_to_string(
            temp.path().join("projects/demo/logs/run.ndjson"),
        )
        .unwrap();
        assert_eq!(raw.lines().count(), 3);
        assert!(!raw.contains(secret));
        assert!(!raw.contains("voice-do-not-log"));
        for line in raw.lines() {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }
    }
}

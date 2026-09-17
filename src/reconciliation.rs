use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{
    audio::{load_audio_status, persist_audio_and_coarse},
    audio_artifacts::reconcile_local_audio_artifact,
    load_visual_status, visual_status_path, ProjectError, ProjectStore, StoredProject, TaskState,
    VisualError, VisualFlowStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalReconciliationReport {
    pub project_id: String,
    pub visual_state: TaskState,
    pub audio_state: TaskState,
    pub overall: TaskState,
    pub invalid_visual_assets: u32,
    pub preserved_visual_assets: u32,
    pub notes: Vec<String>,
}

#[derive(Debug, Error)]
pub enum LocalReconciliationError {
    #[error(transparent)]
    Visual(#[from] VisualError),

    #[error("audio state reconciliation failed: {0}")]
    Audio(String),

    #[error(transparent)]
    Project(#[from] ProjectError),

    #[error("local reconciliation I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("local reconciliation JSON error for {path}: {message}")]
    Json { path: PathBuf, message: String },
}

pub fn reconcile_local_project(
    project: &mut StoredProject,
) -> Result<LocalReconciliationReport, LocalReconciliationError> {
    let (visual, invalid_visual_assets, preserved_visual_assets) = reconcile_visual(project)?;
    let mut notes = Vec::new();
    let audio = reconcile_audio(project, &mut notes)?;

    project.status.visual_flow = visual.state;
    project.status.audio_flow = audio.state;
    for scene in &mut project.status.scenes {
        if let Some(visual_scene) = visual
            .scenes
            .iter()
            .find(|candidate| candidate.id == scene.id)
        {
            scene.visual = visual_scene.state;
        }
        scene.audio = audio.state;
    }
    project.status.overall = derive_overall(project.status.visual_flow, project.status.audio_flow);
    ProjectStore::save_status(&project.root, &project.status)?;

    Ok(LocalReconciliationReport {
        project_id: project.metadata.project_id.clone(),
        visual_state: visual.state,
        audio_state: audio.state,
        overall: project.status.overall,
        invalid_visual_assets,
        preserved_visual_assets,
        notes,
    })
}

fn reconcile_visual(
    project: &mut StoredProject,
) -> Result<(VisualFlowStatus, u32, u32), LocalReconciliationError> {
    let path = visual_status_path(&project.root);
    let existed = path.is_file();
    let mut status = load_visual_status(project)?;
    let before = status.clone();
    let mut invalid_assets = 0_u32;
    let mut preserved_assets = 0_u32;

    for (scene_index, scene_status) in status.scenes.iter_mut().enumerate() {
        for (request_index, request_status) in scene_status.requests.iter_mut().enumerate() {
            let target = project.prepared_script.scenes[scene_index].visuals[request_index].count;
            let previous_state = request_status.state;
            let mut request_invalid = 0_u32;

            request_status.assets.retain(|asset| {
                let local_path = project.root.join(&asset.relative_path);
                match file_proof(&local_path) {
                    Ok((bytes, hash)) if bytes == asset.bytes && hash == asset.sha256 => {
                        preserved_assets = preserved_assets.saturating_add(1);
                        true
                    }
                    _ => {
                        invalid_assets = invalid_assets.saturating_add(1);
                        request_invalid = request_invalid.saturating_add(1);
                        false
                    }
                }
            });

            if request_status.assets.len() >= target as usize {
                request_status.state = TaskState::Completed;
                request_status.last_error = None;
            } else if previous_state == TaskState::Running {
                request_status.state = TaskState::Interrupted;
                request_status.last_error = Some(format!(
                    "previous process ended while this visual request was RUNNING; {}/{} verified asset(s) preserved",
                    request_status.assets.len(),
                    target
                ));
            } else if !request_status.assets.is_empty() {
                request_status.state = TaskState::Partial;
                if request_invalid > 0 || previous_state == TaskState::Completed {
                    request_status.last_error = Some(format!(
                        "local visual proof is incomplete; {}/{} verified asset(s) remain",
                        request_status.assets.len(),
                        target
                    ));
                }
            } else if matches!(previous_state, TaskState::Completed | TaskState::Partial)
                || request_invalid > 0
            {
                request_status.state = TaskState::Interrupted;
                request_status.last_error = Some(
                    "previous visual state had no remaining verified local assets; retry required"
                        .to_owned(),
                );
            }
        }
    }

    recompute_visual_states(&mut status);
    let coarse_mismatch = project.status.visual_flow != status.state
        || project.status.scenes.iter().any(|scene| {
            status
                .scenes
                .iter()
                .find(|candidate| candidate.id == scene.id)
                .is_some_and(|candidate| candidate.state != scene.visual)
        });

    if existed || status != before || coarse_mismatch {
        write_json_atomic(&path, &status)?;
        project.status.visual_flow = status.state;
        for scene in &mut project.status.scenes {
            if let Some(visual_scene) = status
                .scenes
                .iter()
                .find(|candidate| candidate.id == scene.id)
            {
                scene.visual = visual_scene.state;
            }
        }
    }

    Ok((status, invalid_assets, preserved_assets))
}

fn reconcile_audio(
    project: &mut StoredProject,
    notes: &mut Vec<String>,
) -> Result<crate::AudioFlowStatus, LocalReconciliationError> {
    let before = load_audio_status(&project.root, &project.metadata.project_id)
        .map_err(|error| LocalReconciliationError::Audio(error.to_string()))?;

    match reconcile_local_audio_artifact(project) {
        Ok(Some(_)) | Ok(None) => {}
        Err(error) => notes.push(format!(
            "local audio artifact proof requires repair: {error}"
        )),
    }

    let mut audio = load_audio_status(&project.root, &project.metadata.project_id)
        .map_err(|error| LocalReconciliationError::Audio(error.to_string()))?;

    if audio.state == TaskState::Completed {
        let verified = matches!(
            reconcile_local_audio_artifact(project),
            Ok(Some(summary)) if summary.state == TaskState::Completed
        );
        if !verified {
            audio = load_audio_status(&project.root, &project.metadata.project_id)
                .map_err(|error| LocalReconciliationError::Audio(error.to_string()))?;
            audio.state = TaskState::Partial;
            if let Some(latest) = audio.attempts.last_mut() {
                latest.last_error = Some(
                    "local audio completion proof is missing, invalid, or no longer matches the canonical artifact; repair required"
                        .to_owned(),
                );
            }
            persist_audio_and_coarse(project, &audio, TaskState::Partial)
                .map_err(|error| LocalReconciliationError::Audio(error.to_string()))?;
            notes.push(
                "audio COMPLETED was downgraded to PARTIAL because durable local proof was not verified"
                    .to_owned(),
            );
        }
    } else if before.state == TaskState::Completed && audio.state != TaskState::Completed {
        notes.push(
            "audio COMPLETED was downgraded because the canonical local artifact did not verify"
                .to_owned(),
        );
    }

    load_audio_status(&project.root, &project.metadata.project_id)
        .map_err(|error| LocalReconciliationError::Audio(error.to_string()))
}

fn recompute_visual_states(status: &mut VisualFlowStatus) {
    for scene in &mut status.scenes {
        let states: Vec<TaskState> = scene.requests.iter().map(|request| request.state).collect();
        scene.state = aggregate_states(&states);
    }
    let states: Vec<TaskState> = status.scenes.iter().map(|scene| scene.state).collect();
    status.state = aggregate_states(&states);
}

fn aggregate_states(states: &[TaskState]) -> TaskState {
    if states.is_empty() || states.iter().all(|state| *state == TaskState::Pending) {
        return TaskState::Pending;
    }
    if states.iter().any(|state| *state == TaskState::Running) {
        return TaskState::Running;
    }
    if states.iter().all(|state| *state == TaskState::Completed) {
        return TaskState::Completed;
    }
    if states
        .iter()
        .any(|state| matches!(state, TaskState::Completed | TaskState::Partial))
    {
        return TaskState::Partial;
    }
    if states.iter().any(|state| *state == TaskState::Interrupted) {
        return TaskState::Interrupted;
    }
    if states.iter().any(|state| *state == TaskState::Failed) {
        return TaskState::Failed;
    }
    TaskState::Partial
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

fn file_proof(path: &Path) -> Result<(u64, String), LocalReconciliationError> {
    let mut file = File::open(path).map_err(|source| LocalReconciliationError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| LocalReconciliationError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), LocalReconciliationError> {
    let parent = path
        .parent()
        .ok_or_else(|| LocalReconciliationError::Json {
            path: path.to_path_buf(),
            message: "JSON target has no parent".to_owned(),
        })?;
    fs::create_dir_all(parent).map_err(|source| LocalReconciliationError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temp =
        NamedTempFile::new_in(parent).map_err(|source| LocalReconciliationError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    serde_json::to_writer_pretty(temp.as_file_mut(), value).map_err(|error| {
        LocalReconciliationError::Json {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| LocalReconciliationError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|source| LocalReconciliationError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.persist(path)
        .map_err(|error| LocalReconciliationError::Io {
            path: path.to_path_buf(),
            source: error.error,
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        audio::{
            save_audio_status, AudioAttemptStatus, AudioFlowStatus, AUDIO_STATUS_SCHEMA_VERSION,
        },
        parse_script, PersistedAssetKind, ProjectStore, VisualAssetStatus, VisualRequestStatus,
        VisualSceneStatus, VISUAL_STATUS_SCHEMA_VERSION,
    };

    fn create_demo() -> (tempfile::TempDir, StoredProject) {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let project = ProjectStore::new(temp.path())
            .create("demo", raw, &prepared)
            .unwrap();
        (temp, project)
    }

    fn write_visual_status(
        project: &StoredProject,
        state: TaskState,
        asset: Option<VisualAssetStatus>,
    ) {
        let mut scenes = Vec::new();
        for scene in &project.prepared_script.scenes {
            let mut requests = Vec::new();
            for request in &scene.visuals {
                requests.push(VisualRequestStatus {
                    id: request.id.clone(),
                    state: if scene.id == "S01" && request.id == "V01" {
                        state
                    } else {
                        TaskState::Pending
                    },
                    attempted_queries: Vec::new(),
                    successful_query: None,
                    assets: if scene.id == "S01" && request.id == "V01" {
                        asset.clone().into_iter().collect()
                    } else {
                        Vec::new()
                    },
                    last_error: None,
                });
            }
            scenes.push(VisualSceneStatus {
                id: scene.id.clone(),
                state: if scene.id == "S01" {
                    state
                } else {
                    TaskState::Pending
                },
                requests,
            });
        }
        let visual = VisualFlowStatus {
            schema_version: VISUAL_STATUS_SCHEMA_VERSION,
            project_id: project.metadata.project_id.clone(),
            state,
            scenes,
        };
        write_json_atomic(&visual_status_path(&project.root), &visual).unwrap();
    }

    fn asset(project: &StoredProject, bytes: &[u8]) -> VisualAssetStatus {
        let relative_path = "scenes/S01/videos/V01/asset-001.mp4".to_owned();
        let path = project.root.join(&relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        VisualAssetStatus {
            slot: 1,
            kind: PersistedAssetKind::Video,
            provider: "pexels".to_owned(),
            provider_asset_id: "asset-1".to_owned(),
            relative_path,
            provenance_relative_path: "scenes/S01/videos/V01/asset-001.provenance.json".to_owned(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            bytes: bytes.len() as u64,
        }
    }

    #[test]
    fn fresh_project_remains_pending() {
        let (_temp, mut project) = create_demo();
        let report = reconcile_local_project(&mut project).unwrap();
        assert_eq!(report.visual_state, TaskState::Pending);
        assert_eq!(report.audio_state, TaskState::Pending);
        assert_eq!(report.overall, TaskState::Pending);
    }

    #[test]
    fn stale_visual_running_becomes_interrupted_and_preserves_valid_asset() {
        let (_temp, mut project) = create_demo();
        let valid = asset(&project, b"valid-asset");
        write_visual_status(&project, TaskState::Running, Some(valid.clone()));

        let report = reconcile_local_project(&mut project).unwrap();
        assert_eq!(report.visual_state, TaskState::Interrupted);
        assert_eq!(report.preserved_visual_assets, 1);
        let visual = load_visual_status(&project).unwrap();
        let request = &visual.scenes[0].requests[0];
        assert_eq!(request.state, TaskState::Interrupted);
        assert_eq!(request.assets, vec![valid]);
    }

    #[test]
    fn missing_completed_visual_asset_is_removed_and_downgraded() {
        let (_temp, mut project) = create_demo();
        let missing = VisualAssetStatus {
            slot: 1,
            kind: PersistedAssetKind::Video,
            provider: "pexels".to_owned(),
            provider_asset_id: "gone".to_owned(),
            relative_path: "scenes/S01/videos/V01/gone.mp4".to_owned(),
            provenance_relative_path: "scenes/S01/videos/V01/gone.provenance.json".to_owned(),
            sha256: "deadbeef".to_owned(),
            bytes: 123,
        };
        write_visual_status(&project, TaskState::Completed, Some(missing));

        let report = reconcile_local_project(&mut project).unwrap();
        assert_eq!(report.invalid_visual_assets, 1);
        let visual = load_visual_status(&project).unwrap();
        assert_eq!(visual.scenes[0].requests[0].state, TaskState::Interrupted);
        assert!(visual.scenes[0].requests[0].assets.is_empty());
        assert_eq!(visual.scenes[0].requests[1].state, TaskState::Pending);
    }

    #[test]
    fn completed_audio_without_durable_local_proof_becomes_partial() {
        let (_temp, mut project) = create_demo();
        let audio = AudioFlowStatus {
            schema_version: AUDIO_STATUS_SCHEMA_VERSION,
            project_id: project.metadata.project_id.clone(),
            state: TaskState::Completed,
            artifact_content_download: true,
            artifact_content_endpoint: Some("/api/v1/artifacts/{artifact_id}/content".to_owned()),
            attempts: vec![AudioAttemptStatus {
                attempt_id: "A0001".to_owned(),
                server_base_url: "https://voice.example".to_owned(),
                remote_project_id: "remote".to_owned(),
                remote_source_hash: Some("hash".to_owned()),
                job_id: Some("job-1".to_owned()),
                idempotency_key: "idempotent".to_owned(),
                state: TaskState::Completed,
                submitted_unix_ms: None,
                last_error: None,
            }],
        };
        save_audio_status(&project.root, &audio).unwrap();
        project.status.audio_flow = TaskState::Completed;
        for scene in &mut project.status.scenes {
            scene.audio = TaskState::Completed;
        }
        ProjectStore::save_status(&project.root, &project.status).unwrap();

        let report = reconcile_local_project(&mut project).unwrap();
        assert_eq!(report.audio_state, TaskState::Partial);
        let reloaded = load_audio_status(&project.root, &project.metadata.project_id).unwrap();
        assert_eq!(reloaded.state, TaskState::Partial);
    }

    #[test]
    fn remote_running_audio_is_not_rewritten_by_local_reconciliation() {
        let (_temp, mut project) = create_demo();
        let audio = AudioFlowStatus {
            schema_version: AUDIO_STATUS_SCHEMA_VERSION,
            project_id: project.metadata.project_id.clone(),
            state: TaskState::Running,
            artifact_content_download: true,
            artifact_content_endpoint: Some("/api/v1/artifacts/{artifact_id}/content".to_owned()),
            attempts: vec![AudioAttemptStatus {
                attempt_id: "A0001".to_owned(),
                server_base_url: "https://voice.example".to_owned(),
                remote_project_id: "remote".to_owned(),
                remote_source_hash: Some("hash".to_owned()),
                job_id: Some("job-1".to_owned()),
                idempotency_key: "idempotent".to_owned(),
                state: TaskState::Running,
                submitted_unix_ms: None,
                last_error: None,
            }],
        };
        save_audio_status(&project.root, &audio).unwrap();

        let report = reconcile_local_project(&mut project).unwrap();
        assert_eq!(report.audio_state, TaskState::Running);
        let reloaded = load_audio_status(&project.root, &project.metadata.project_id).unwrap();
        assert_eq!(reloaded.state, TaskState::Running);
        assert_eq!(reloaded.attempts[0].state, TaskState::Running);
    }

    #[test]
    fn report_surface_contains_no_runtime_secret_fields() {
        let (_temp, mut project) = create_demo();
        let report = reconcile_local_project(&mut project).unwrap();
        let debug = format!("{report:?}");
        assert!(!debug.to_ascii_lowercase().contains("api_key"));
        assert!(!debug.to_ascii_lowercase().contains("token"));
    }
}

use std::fs;

use sha2::{Digest, Sha256};
use video_prepare::{
    audio_artifact_proof_path, audio_status_path, parse_script, reconcile_local_project,
    visual_status_path, AudioArtifactProof, AudioAttemptStatus, AudioFlowStatus,
    PersistedAssetKind, ProjectStore, TaskState, VisualAssetStatus, VisualFlowStatus,
    VisualRequestStatus, VisualSceneStatus, AUDIO_ARTIFACT_SCHEMA_VERSION,
    AUDIO_STATUS_SCHEMA_VERSION, VISUAL_STATUS_SCHEMA_VERSION,
};

fn create_demo() -> (tempfile::TempDir, video_prepare::StoredProject) {
    let temp = tempfile::tempdir().unwrap();
    let raw = include_str!("../examples/demo.vprep");
    let prepared = parse_script(raw).unwrap();
    let project = ProjectStore::new(temp.path())
        .create("demo", raw, &prepared)
        .unwrap();
    (temp, project)
}

fn write_json(path: &std::path::Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[test]
fn fully_verified_visual_completion_survives_reconciliation() {
    let (_temp, mut project) = create_demo();
    let mut scenes = Vec::new();

    for scene in &project.prepared_script.scenes {
        let mut requests = Vec::new();
        for request in &scene.visuals {
            let mut assets = Vec::new();
            for slot in 1..=request.count {
                let relative_path = format!(
                    "scenes/{}/videos/{}/asset-{slot:03}.mp4",
                    scene.id, request.id
                );
                let bytes = format!("{}:{}:{slot}", scene.id, request.id).into_bytes();
                let absolute = project.root.join(&relative_path);
                fs::create_dir_all(absolute.parent().unwrap()).unwrap();
                fs::write(&absolute, &bytes).unwrap();
                assets.push(VisualAssetStatus {
                    slot,
                    kind: PersistedAssetKind::Video,
                    provider: "pexels".to_owned(),
                    provider_asset_id: format!("{}-{}-{slot}", scene.id, request.id),
                    relative_path,
                    provenance_relative_path: format!(
                        "scenes/{}/videos/{}/asset-{slot:03}.provenance.json",
                        scene.id, request.id
                    ),
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    bytes: bytes.len() as u64,
                });
            }
            requests.push(VisualRequestStatus {
                id: request.id.clone(),
                state: TaskState::Completed,
                attempted_queries: Vec::new(),
                successful_query: None,
                assets,
                last_error: None,
            });
        }
        scenes.push(VisualSceneStatus {
            id: scene.id.clone(),
            state: TaskState::Completed,
            requests,
        });
    }

    let status = VisualFlowStatus {
        schema_version: VISUAL_STATUS_SCHEMA_VERSION,
        project_id: project.metadata.project_id.clone(),
        state: TaskState::Completed,
        scenes,
    };
    write_json(&visual_status_path(&project.root), &status);

    let report = reconcile_local_project(&mut project).unwrap();
    assert_eq!(report.visual_state, TaskState::Completed);
    assert_eq!(report.invalid_visual_assets, 0);
    assert!(report.preserved_visual_assets > 0);

    let stored: VisualFlowStatus =
        serde_json::from_slice(&fs::read(visual_status_path(&project.root)).unwrap()).unwrap();
    assert_eq!(stored.state, TaskState::Completed);
    assert!(stored.scenes.iter().all(|scene| scene.state == TaskState::Completed));
}

#[test]
fn verified_audio_completion_survives_reconciliation() {
    let (_temp, mut project) = create_demo();
    let wav = b"RIFF-local-proof";
    let local_relative_path = "audio/artifacts/full.wav".to_owned();
    let local_path = project.root.join(&local_relative_path);
    fs::create_dir_all(local_path.parent().unwrap()).unwrap();
    fs::write(&local_path, wav).unwrap();
    let sha256 = format!("{:x}", Sha256::digest(wav));

    let audio = AudioFlowStatus {
        schema_version: AUDIO_STATUS_SCHEMA_VERSION,
        project_id: project.metadata.project_id.clone(),
        state: TaskState::Completed,
        artifact_content_download: true,
        artifact_content_endpoint: Some("/api/v1/artifacts/{artifact_id}/content".to_owned()),
        attempts: vec![AudioAttemptStatus {
            attempt_id: "A0001".to_owned(),
            server_base_url: "https://voice.example".to_owned(),
            remote_project_id: "remote-demo".to_owned(),
            remote_source_hash: Some("source-hash".to_owned()),
            job_id: Some("job-1".to_owned()),
            idempotency_key: "idempotent".to_owned(),
            state: TaskState::Completed,
            submitted_unix_ms: None,
            last_error: None,
        }],
    };
    write_json(&audio_status_path(&project.root), &audio);

    let proof = AudioArtifactProof {
        schema_version: AUDIO_ARTIFACT_SCHEMA_VERSION,
        attempt_id: "A0001".to_owned(),
        server_base_url: "https://voice.example".to_owned(),
        remote_project_id: "remote-demo".to_owned(),
        remote_source_hash: "source-hash".to_owned(),
        job_id: "job-1".to_owned(),
        artifact_id: "art-full".to_owned(),
        artifact_kind: "project_audio".to_owned(),
        remote_relative_path: "projects/remote-demo/output/full.wav".to_owned(),
        remote_filename: "full.wav".to_owned(),
        remote_format: Some("wav".to_owned()),
        remote_size_bytes: wav.len() as u64,
        duration_seconds: 1.0,
        sample_rate: 24_000,
        channels: 1,
        local_relative_path,
        local_size_bytes: wav.len() as u64,
        sha256,
    };
    write_json(&audio_artifact_proof_path(&project.root), &proof);

    let report = reconcile_local_project(&mut project).unwrap();
    assert_eq!(report.audio_state, TaskState::Completed);

    let stored: AudioFlowStatus =
        serde_json::from_slice(&fs::read(audio_status_path(&project.root)).unwrap()).unwrap();
    assert_eq!(stored.state, TaskState::Completed);
    assert_eq!(stored.attempts[0].state, TaskState::Completed);
}

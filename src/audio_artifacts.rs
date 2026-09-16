use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{
    audio::{load_audio_status, persist_audio_and_coarse, save_audio_status},
    OmniVoiceArtifact, OmniVoiceArtifactProvider, OmniVoiceArtifactTransport, OmniVoiceError,
    OmniVoiceProvider, ProjectError, StoredProject, TaskState,
};

pub const AUDIO_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioArtifactProof {
    pub schema_version: u32,
    pub attempt_id: String,
    pub server_base_url: String,
    pub remote_project_id: String,
    pub remote_source_hash: String,
    pub job_id: String,
    pub artifact_id: String,
    pub artifact_kind: String,
    pub remote_relative_path: String,
    pub remote_filename: String,
    pub remote_format: Option<String>,
    pub remote_size_bytes: u64,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u32,
    pub local_relative_path: String,
    pub local_size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioArtifactSyncSummary {
    pub state: TaskState,
    pub artifact_id: Option<String>,
    pub local_relative_path: Option<String>,
    pub downloaded: bool,
}

#[derive(Debug, Error)]
pub enum AudioArtifactError {
    #[error(transparent)]
    Provider(#[from] OmniVoiceError),

    #[error(transparent)]
    Project(#[from] ProjectError),

    #[error("audio artifact state I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("audio artifact state JSON error for {path}: {message}")]
    Json { path: PathBuf, message: String },

    #[error("latest audio attempt is not ready for artifact import: {0}")]
    AttemptNotReady(String),

    #[error("remote audio job is completed but canonical output/full.wav is not available yet")]
    ArtifactNotReady,

    #[error("remote artifact catalog contains multiple canonical output/full.wav candidates")]
    AmbiguousCanonicalArtifact,

    #[error("remote artifact metadata does not match downloaded bytes: expected {expected}, got {actual}")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("local audio artifact proof is invalid: {0}")]
    InvalidLocalProof(String),

    #[error("latest remote attempt belongs to `{attempt_server}`, not current server `{current_server}`")]
    ServerMismatch {
        attempt_server: String,
        current_server: String,
    },
}

pub fn audio_artifact_proof_path(project_root: &Path) -> PathBuf {
    project_root.join("audio").join("artifact-status.json")
}

pub fn load_audio_artifact_proof(
    project_root: &Path,
) -> Result<Option<AudioArtifactProof>, AudioArtifactError> {
    let path = audio_artifact_proof_path(project_root);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|source| AudioArtifactError::Io {
        path: path.clone(),
        source,
    })?;
    let proof: AudioArtifactProof =
        serde_json::from_slice(&bytes).map_err(|error| AudioArtifactError::Json {
            path: path.clone(),
            message: error.to_string(),
        })?;
    if proof.schema_version != AUDIO_ARTIFACT_SCHEMA_VERSION {
        return Err(AudioArtifactError::InvalidLocalProof(format!(
            "unsupported schema_version {}",
            proof.schema_version
        )));
    }
    Ok(Some(proof))
}

pub fn reconcile_local_audio_artifact(
    project: &mut StoredProject,
) -> Result<Option<AudioArtifactSyncSummary>, AudioArtifactError> {
    let Some(proof) = load_audio_artifact_proof(&project.root)? else {
        return Ok(None);
    };
    let local_path = project.root.join(&proof.local_relative_path);
    match sha256_file(&local_path) {
        Ok((bytes, hash)) if bytes == proof.local_size_bytes && hash == proof.sha256 => {
            let mut audio = load_audio_status(&project.root, &project.metadata.project_id)
                .map_err(map_audio_error)?;
            audio.state = TaskState::Completed;
            if let Some(attempt) = audio
                .attempts
                .iter_mut()
                .find(|attempt| attempt.attempt_id == proof.attempt_id)
            {
                attempt.state = TaskState::Completed;
                attempt.last_error = None;
            }
            persist_audio_and_coarse(project, &audio, TaskState::Completed)
                .map_err(map_audio_error)?;
            Ok(Some(AudioArtifactSyncSummary {
                state: TaskState::Completed,
                artifact_id: Some(proof.artifact_id),
                local_relative_path: Some(proof.local_relative_path),
                downloaded: false,
            }))
        }
        _ => {
            let mut audio = load_audio_status(&project.root, &project.metadata.project_id)
                .map_err(map_audio_error)?;
            audio.state = TaskState::Partial;
            if let Some(attempt) = audio
                .attempts
                .iter_mut()
                .find(|attempt| attempt.attempt_id == proof.attempt_id)
            {
                if attempt.state == TaskState::Completed {
                    attempt.last_error = Some(
                        "local audio artifact is missing or checksum-mismatched; repair required"
                            .to_owned(),
                    );
                }
            }
            persist_audio_and_coarse(project, &audio, TaskState::Partial)
                .map_err(map_audio_error)?;
            Ok(None)
        }
    }
}

pub fn sync_latest_audio_artifact<P>(
    provider: &P,
    project: &mut StoredProject,
) -> Result<AudioArtifactSyncSummary, AudioArtifactError>
where
    P: OmniVoiceProvider + OmniVoiceArtifactProvider,
{
    if let Some(summary) = reconcile_local_audio_artifact(project)? {
        return Ok(summary);
    }

    let mut audio =
        load_audio_status(&project.root, &project.metadata.project_id).map_err(map_audio_error)?;
    let latest_index = audio.attempts.len().checked_sub(1).ok_or_else(|| {
        AudioArtifactError::AttemptNotReady("no remote attempt exists".to_owned())
    })?;
    let latest = audio.attempts[latest_index].clone();

    if latest.server_base_url != provider.base_url() {
        audio.attempts[latest_index].state = TaskState::UnknownRemote;
        audio.attempts[latest_index].last_error =
            Some("current OmniVoice URL differs from the server that owns this attempt".to_owned());
        audio.state = TaskState::UnknownRemote;
        persist_audio_and_coarse(project, &audio, TaskState::UnknownRemote)
            .map_err(map_audio_error)?;
        return Err(AudioArtifactError::ServerMismatch {
            attempt_server: latest.server_base_url,
            current_server: provider.base_url().to_owned(),
        });
    }

    let job_id = latest.job_id.clone().ok_or_else(|| {
        AudioArtifactError::AttemptNotReady("latest attempt has no job id".to_owned())
    })?;
    let remote = provider.get_job(&job_id)?;
    if remote.job_id != job_id {
        return Err(AudioArtifactError::AttemptNotReady(format!(
            "expected remote job `{job_id}`, received `{}`",
            remote.job_id
        )));
    }

    match remote.status.trim().to_ascii_lowercase().as_str() {
        "queued" | "pending" | "running" => {
            audio.attempts[latest_index].state = TaskState::Running;
            audio.attempts[latest_index].last_error = None;
            audio.state = TaskState::Running;
            persist_audio_and_coarse(project, &audio, TaskState::Running)
                .map_err(map_audio_error)?;
            return Ok(AudioArtifactSyncSummary {
                state: TaskState::Running,
                artifact_id: None,
                local_relative_path: None,
                downloaded: false,
            });
        }
        "failed" | "cancelled" => {
            audio.attempts[latest_index].state = TaskState::Failed;
            audio.attempts[latest_index].last_error = Some(format!(
                "remote generate_project job ended with status {}",
                remote.status
            ));
            audio.state = TaskState::Failed;
            persist_audio_and_coarse(project, &audio, TaskState::Failed)
                .map_err(map_audio_error)?;
            return Ok(AudioArtifactSyncSummary {
                state: TaskState::Failed,
                artifact_id: None,
                local_relative_path: None,
                downloaded: false,
            });
        }
        "completed" => {
            audio.attempts[latest_index].state = TaskState::Completed;
            audio.attempts[latest_index].last_error = None;
            audio.state = TaskState::Partial;
            persist_audio_and_coarse(project, &audio, TaskState::Partial)
                .map_err(map_audio_error)?;
        }
        other => {
            audio.attempts[latest_index].state = TaskState::UnknownRemote;
            audio.attempts[latest_index].last_error =
                Some(format!("unrecognized remote job status `{other}`"));
            audio.state = TaskState::UnknownRemote;
            persist_audio_and_coarse(project, &audio, TaskState::UnknownRemote)
                .map_err(map_audio_error)?;
            return Ok(AudioArtifactSyncSummary {
                state: TaskState::UnknownRemote,
                artifact_id: None,
                local_relative_path: None,
                downloaded: false,
            });
        }
    }

    let remote_source_hash = latest.remote_source_hash.clone().ok_or_else(|| {
        AudioArtifactError::AttemptNotReady("latest attempt has no remote source hash".to_owned())
    })?;
    let transport = provider.discover_artifact_transport()?;
    let artifacts = provider.list_artifacts(&transport, &latest.remote_project_id)?;
    let artifact = select_canonical_project_audio(&artifacts, &latest.remote_project_id)?;

    let filename = "full.wav";
    let local_relative_path = format!("audio/artifacts/{filename}");
    let final_path = project.root.join(&local_relative_path);
    let download = provider.download_artifact_atomic(&transport, &artifact.id, &final_path)?;
    if artifact.size_bytes > 0 && artifact.size_bytes != download.bytes {
        let _ = fs::remove_file(&final_path);
        return Err(AudioArtifactError::SizeMismatch {
            expected: artifact.size_bytes,
            actual: download.bytes,
        });
    }
    let (verified_bytes, verified_hash) = sha256_file(&final_path)?;
    if verified_bytes != download.bytes || verified_hash != download.sha256 {
        let _ = fs::remove_file(&final_path);
        return Err(AudioArtifactError::InvalidLocalProof(
            "download receipt does not match persisted file".to_owned(),
        ));
    }

    let proof = AudioArtifactProof {
        schema_version: AUDIO_ARTIFACT_SCHEMA_VERSION,
        attempt_id: latest.attempt_id.clone(),
        server_base_url: latest.server_base_url.clone(),
        remote_project_id: latest.remote_project_id.clone(),
        remote_source_hash,
        job_id,
        artifact_id: artifact.id.clone(),
        artifact_kind: artifact.kind.clone(),
        remote_relative_path: artifact.relative_path.clone(),
        remote_filename: artifact.filename.clone(),
        remote_format: artifact.format.clone(),
        remote_size_bytes: artifact.size_bytes,
        duration_seconds: artifact.duration_seconds,
        sample_rate: artifact.sample_rate,
        channels: artifact.channels,
        local_relative_path: local_relative_path.clone(),
        local_size_bytes: verified_bytes,
        sha256: verified_hash,
    };
    save_audio_artifact_proof(&project.root, &proof)?;

    audio.attempts[latest_index].state = TaskState::Completed;
    audio.attempts[latest_index].last_error = None;
    audio.state = TaskState::Completed;
    persist_audio_and_coarse(project, &audio, TaskState::Completed).map_err(map_audio_error)?;

    Ok(AudioArtifactSyncSummary {
        state: TaskState::Completed,
        artifact_id: Some(artifact.id),
        local_relative_path: Some(local_relative_path),
        downloaded: true,
    })
}

fn select_canonical_project_audio<'a>(
    artifacts: &'a [OmniVoiceArtifact],
    remote_project_id: &str,
) -> Result<&'a OmniVoiceArtifact, AudioArtifactError> {
    let mut candidates: Vec<&OmniVoiceArtifact> = artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind == "project_audio"
                && artifact.filename == "full.wav"
                && artifact.relative_path.ends_with("/output/full.wav")
                && artifact
                    .project_id
                    .as_deref()
                    .map(|value| value == remote_project_id)
                    .unwrap_or(true)
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.id.cmp(&right.id))
    });
    match candidates.len() {
        0 => Err(AudioArtifactError::ArtifactNotReady),
        1 => Ok(candidates[0]),
        _ => Err(AudioArtifactError::AmbiguousCanonicalArtifact),
    }
}

fn save_audio_artifact_proof(
    project_root: &Path,
    proof: &AudioArtifactProof,
) -> Result<(), AudioArtifactError> {
    let path = audio_artifact_proof_path(project_root);
    let parent = path.parent().ok_or_else(|| {
        AudioArtifactError::InvalidLocalProof("proof path has no parent".to_owned())
    })?;
    fs::create_dir_all(parent).map_err(|source| AudioArtifactError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|source| AudioArtifactError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    serde_json::to_writer_pretty(temp.as_file_mut(), proof).map_err(|error| {
        AudioArtifactError::Json {
            path: path.clone(),
            message: error.to_string(),
        }
    })?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| AudioArtifactError::Io {
            path: path.clone(),
            source,
        })?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|source| AudioArtifactError::Io {
            path: path.clone(),
            source,
        })?;
    temp.persist(&path)
        .map_err(|error| AudioArtifactError::Io {
            path,
            source: error.error,
        })?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<(u64, String), AudioArtifactError> {
    let mut file = fs::File::open(path).map_err(|source| AudioArtifactError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| AudioArtifactError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    if total == 0 {
        return Err(AudioArtifactError::InvalidLocalProof(
            "local artifact is empty".to_owned(),
        ));
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}

fn map_audio_error(error: crate::AudioError) -> AudioArtifactError {
    match error {
        crate::AudioError::Project(project) => AudioArtifactError::Project(project),
        other => AudioArtifactError::InvalidLocalProof(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        parse_script, AudioExecutor, AudioGenerationSettings, GenerateProjectOptions,
        OmniVoiceArtifactDownload, OmniVoiceConnection, OmniVoiceImportResult,
        OmniVoiceJobSubmission, OmniVoiceRemoteJob, ProjectStore,
    };
    use std::sync::Mutex;

    struct MockProvider {
        base_url: String,
        job_status: Mutex<String>,
        artifact: Mutex<Option<OmniVoiceArtifact>>,
        downloads: Mutex<u32>,
        payload: Vec<u8>,
    }

    impl MockProvider {
        fn completed(payload: &[u8]) -> Self {
            let hash_id = "art_0123456789abcdef".to_owned();
            Self {
                base_url: "https://studio.example".to_owned(),
                job_status: Mutex::new("completed".to_owned()),
                artifact: Mutex::new(Some(OmniVoiceArtifact {
                    id: hash_id,
                    kind: "project_audio".to_owned(),
                    project_id: None,
                    section_id: None,
                    chunk_id: None,
                    filename: "full.wav".to_owned(),
                    relative_path: "projects/demo/output/full.wav".to_owned(),
                    format: Some("wav".to_owned()),
                    size_bytes: payload.len() as u64,
                    duration_seconds: 1.0,
                    sample_rate: 24000,
                    channels: 1,
                })),
                downloads: Mutex::new(0),
                payload: payload.to_vec(),
            }
        }
    }

    impl OmniVoiceProvider for MockProvider {
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
            Ok(OmniVoiceImportResult {
                project_id: project_id.to_owned(),
                source_hash: crate::omnivoice_source_hash(script),
                created: true,
            })
        }

        fn generate_project(
            &self,
            _project_id: &str,
            _options: &GenerateProjectOptions,
            _idempotency_key: &str,
        ) -> Result<OmniVoiceJobSubmission, OmniVoiceError> {
            Ok(OmniVoiceJobSubmission {
                job_id: "job-1".to_owned(),
                status: "queued".to_owned(),
                location: None,
            })
        }

        fn get_job(&self, job_id: &str) -> Result<OmniVoiceRemoteJob, OmniVoiceError> {
            Ok(OmniVoiceRemoteJob {
                job_id: job_id.to_owned(),
                status: self.job_status.lock().unwrap().clone(),
                kind: Some("generate_project".to_owned()),
            })
        }
    }

    impl OmniVoiceArtifactProvider for MockProvider {
        fn discover_artifact_transport(
            &self,
        ) -> Result<OmniVoiceArtifactTransport, OmniVoiceError> {
            Ok(OmniVoiceArtifactTransport {
                artifacts_endpoint: "/api/v1/artifacts".to_owned(),
                artifact_content_endpoint: "/api/v1/artifacts/{artifact_id}/content".to_owned(),
            })
        }

        fn list_artifacts(
            &self,
            _transport: &OmniVoiceArtifactTransport,
            _project_id: &str,
        ) -> Result<Vec<OmniVoiceArtifact>, OmniVoiceError> {
            Ok(self.artifact.lock().unwrap().clone().into_iter().collect())
        }

        fn download_artifact_atomic(
            &self,
            _transport: &OmniVoiceArtifactTransport,
            _artifact_id: &str,
            final_path: &Path,
        ) -> Result<OmniVoiceArtifactDownload, OmniVoiceError> {
            *self.downloads.lock().unwrap() += 1;
            fs::create_dir_all(final_path.parent().unwrap()).unwrap();
            fs::write(final_path, &self.payload).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&self.payload);
            Ok(OmniVoiceArtifactDownload {
                bytes: self.payload.len() as u64,
                sha256: format!("{:x}", hasher.finalize()),
            })
        }
    }

    fn project_and_provider() -> (tempfile::TempDir, StoredProject, MockProvider) {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let store = ProjectStore::new(temp.path());
        let mut project = store.create("demo-artifact", raw, &prepared).unwrap();
        let provider = MockProvider::completed(b"RIFF-not-a-real-wav-but-stable-test-payload");
        AudioExecutor::new(&provider)
            .submit_generation(&mut project, &AudioGenerationSettings::default())
            .unwrap();
        (temp, project, provider)
    }

    #[test]
    fn completed_remote_job_downloads_and_proves_local_completion() {
        let (_temp, mut project, provider) = project_and_provider();
        let summary = sync_latest_audio_artifact(&provider, &mut project).unwrap();
        assert_eq!(summary.state, TaskState::Completed);
        assert!(summary.downloaded);
        assert_eq!(*provider.downloads.lock().unwrap(), 1);
        let proof = load_audio_artifact_proof(&project.root).unwrap().unwrap();
        assert_eq!(proof.local_relative_path, "audio/artifacts/full.wav");
        assert!(!proof.server_base_url.contains("token"));
        assert_eq!(project.status.audio_flow, TaskState::Completed);
        assert!(project
            .status
            .scenes
            .iter()
            .all(|scene| scene.audio == TaskState::Completed));
    }

    #[test]
    fn reopen_skips_verified_artifact_but_corruption_downgrades_and_repairs() {
        let (_temp, mut project, provider) = project_and_provider();
        sync_latest_audio_artifact(&provider, &mut project).unwrap();
        let reopened_root = project.root.clone();
        let mut reopened = ProjectStore::open(&reopened_root).unwrap();
        let skipped = sync_latest_audio_artifact(&provider, &mut reopened).unwrap();
        assert!(!skipped.downloaded);
        assert_eq!(*provider.downloads.lock().unwrap(), 1);

        fs::write(reopened.root.join("audio/artifacts/full.wav"), b"corrupt").unwrap();
        let repaired = sync_latest_audio_artifact(&provider, &mut reopened).unwrap();
        assert!(repaired.downloaded);
        assert_eq!(*provider.downloads.lock().unwrap(), 2);
        assert_eq!(reopened.status.audio_flow, TaskState::Completed);
    }

    #[test]
    fn remote_completed_without_canonical_artifact_stays_partial() {
        let (_temp, mut project, provider) = project_and_provider();
        *provider.artifact.lock().unwrap() = None;
        let error = sync_latest_audio_artifact(&provider, &mut project).unwrap_err();
        assert!(matches!(error, AudioArtifactError::ArtifactNotReady));
        assert_eq!(project.status.audio_flow, TaskState::Partial);
    }

    #[test]
    fn non_terminal_remote_job_does_not_discover_or_download() {
        let (_temp, mut project, provider) = project_and_provider();
        *provider.job_status.lock().unwrap() = "running".to_owned();
        let summary = sync_latest_audio_artifact(&provider, &mut project).unwrap();
        assert_eq!(summary.state, TaskState::Running);
        assert_eq!(*provider.downloads.lock().unwrap(), 0);
    }
}

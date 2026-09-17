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
    load_visual_status, reconcile_local_project, visual_status_path, LocalReconciliationError,
    MediaKind, PersistedAssetKind, ProjectError, ProjectStore, ProvenanceCreator,
    ProvenanceRendition, StoredProject, TaskState, VisualAssetStatus, VisualError,
    VisualFlowStatus,
};

pub const MANUAL_VISUAL_PROVENANCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualVisualImportSummary {
    pub project_id: String,
    pub scene_id: String,
    pub visual_id: String,
    pub slot: u32,
    pub kind: PersistedAssetKind,
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
    pub request_state: TaskState,
    pub scene_state: TaskState,
    pub flow_state: TaskState,
    pub overall: TaskState,
    pub already_present: bool,
}

#[derive(Debug, Error)]
pub enum ManualVisualError {
    #[error("manual visual source does not exist: {0}")]
    SourceMissing(PathBuf),

    #[error("manual visual source is not a regular file: {0}")]
    SourceNotFile(PathBuf),

    #[error("manual visual source is empty: {0}")]
    EmptySource(PathBuf),

    #[error("manual visual source extension is unsupported: {0}")]
    UnsupportedExtension(String),

    #[error("scene `{0}` does not exist in the prepared script")]
    SceneNotFound(String),

    #[error("visual request `{visual_id}` does not exist in scene `{scene_id}`")]
    VisualNotFound { scene_id: String, visual_id: String },

    #[error("manual {kind} file is incompatible with visual request media `{expected}`")]
    MediaMismatch { kind: String, expected: String },

    #[error("visual request `{visual_id}` in scene `{scene_id}` already has all required assets")]
    RequestAlreadyComplete { scene_id: String, visual_id: String },

    #[error("manual visual destination is occupied by different content: {0}")]
    DestinationOccupied(PathBuf),

    #[error("manual visual state is invalid: {0}")]
    InvalidState(String),

    #[error("manual visual I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("manual visual JSON error for {path}: {message}")]
    Json { path: PathBuf, message: String },

    #[error(transparent)]
    Visual(#[from] VisualError),

    #[error(transparent)]
    Project(#[from] ProjectError),

    #[error(transparent)]
    Reconciliation(#[from] LocalReconciliationError),
}

#[derive(Debug, Clone, Serialize)]
struct ManualVisualProvenance {
    schema_version: u32,
    source: &'static str,
    scene_id: String,
    visual_id: String,
    slot: u32,
    kind: PersistedAssetKind,
    provider: &'static str,
    provider_asset_id: String,
    source_url: &'static str,
    creator: ProvenanceCreator,
    query: &'static str,
    rendition: ProvenanceRendition,
    relative_path: String,
    sha256: String,
    bytes: u64,
}

pub fn import_manual_visual_asset(
    project: &mut StoredProject,
    scene_id: &str,
    visual_id: &str,
    source_path: impl AsRef<Path>,
) -> Result<ManualVisualImportSummary, ManualVisualError> {
    let source_path = source_path.as_ref();
    let metadata = fs::metadata(source_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ManualVisualError::SourceMissing(source_path.to_path_buf())
        } else {
            ManualVisualError::Io {
                path: source_path.to_path_buf(),
                source,
            }
        }
    })?;
    if !metadata.is_file() {
        return Err(ManualVisualError::SourceNotFile(
            source_path.to_path_buf(),
        ));
    }
    if metadata.len() == 0 {
        return Err(ManualVisualError::EmptySource(source_path.to_path_buf()));
    }

    let extension = normalized_extension(source_path)?;
    let (kind, mime_type) = classify_extension(&extension)?;

    let scene_index = project
        .prepared_script
        .scenes
        .iter()
        .position(|scene| scene.id == scene_id)
        .ok_or_else(|| ManualVisualError::SceneNotFound(scene_id.to_owned()))?;
    let request_index = project.prepared_script.scenes[scene_index]
        .visuals
        .iter()
        .position(|request| request.id == visual_id)
        .ok_or_else(|| ManualVisualError::VisualNotFound {
            scene_id: scene_id.to_owned(),
            visual_id: visual_id.to_owned(),
        })?;
    let request = &project.prepared_script.scenes[scene_index].visuals[request_index];
    ensure_media_compatible(request.media, kind)?;

    reconcile_local_project(project)?;
    let mut status = load_visual_status(project)?;
    let source_proof = file_proof(source_path)?;

    let request_status = status
        .scenes
        .get(scene_index)
        .and_then(|scene| scene.requests.get(request_index))
        .ok_or_else(|| {
            ManualVisualError::InvalidState(
                "visual-status shape no longer matches the prepared script".to_owned(),
            )
        })?;

    if let Some(existing) = request_status.assets.iter().find(|asset| {
        asset.sha256 == source_proof.sha256
            && file_proof(&project.root.join(&asset.relative_path))
                .is_ok_and(|proof| proof.sha256 == asset.sha256 && proof.bytes == asset.bytes)
    }) {
        return Ok(build_summary(
            project,
            &status,
            scene_index,
            request_index,
            existing.clone(),
            true,
        ));
    }

    if request_status.assets.len() >= request.count as usize {
        return Err(ManualVisualError::RequestAlreadyComplete {
            scene_id: scene_id.to_owned(),
            visual_id: visual_id.to_owned(),
        });
    }

    let slot = next_missing_slot(&request_status.assets, request.count);
    let hash_prefix = source_proof
        .sha256
        .get(..8)
        .ok_or_else(|| ManualVisualError::InvalidState("invalid SHA-256 proof".to_owned()))?;
    let folder = kind_folder(kind);
    let relative_path = format!(
        "scenes/{scene_id}/{folder}/{visual_id}/manual-{slot:03}-{hash_prefix}.{extension}"
    );
    let provenance_relative_path = format!(
        "scenes/{scene_id}/{folder}/{visual_id}/manual-{slot:03}-{hash_prefix}.provenance.json"
    );
    let destination = project.root.join(&relative_path);

    copy_verified_atomic(source_path, &destination, &source_proof)?;

    let provider_asset_id = format!("manual-{}", &source_proof.sha256[..16]);
    let provenance = ManualVisualProvenance {
        schema_version: MANUAL_VISUAL_PROVENANCE_SCHEMA_VERSION,
        source: "manual",
        scene_id: scene_id.to_owned(),
        visual_id: visual_id.to_owned(),
        slot,
        kind,
        provider: "manual",
        provider_asset_id: provider_asset_id.clone(),
        source_url: "manual://imported",
        creator: ProvenanceCreator {
            provider_creator_id: None,
            name: "Manual Import".to_owned(),
            profile_url: None,
        },
        query: "manual import",
        rendition: ProvenanceRendition {
            label: "manual".to_owned(),
            url: "manual://imported".to_owned(),
            mime_type: Some(mime_type.to_owned()),
            width: None,
            height: None,
            fps: None,
        },
        relative_path: relative_path.clone(),
        sha256: source_proof.sha256.clone(),
        bytes: source_proof.bytes,
    };
    write_json_atomic(&project.root.join(&provenance_relative_path), &provenance)?;

    let asset = VisualAssetStatus {
        slot,
        kind,
        provider: "manual".to_owned(),
        provider_asset_id,
        relative_path,
        provenance_relative_path,
        sha256: source_proof.sha256,
        bytes: source_proof.bytes,
    };

    let request_status = status
        .scenes
        .get_mut(scene_index)
        .and_then(|scene| scene.requests.get_mut(request_index))
        .ok_or_else(|| {
            ManualVisualError::InvalidState(
                "visual-status shape no longer matches the prepared script".to_owned(),
            )
        })?;
    request_status.assets.push(asset.clone());
    request_status
        .assets
        .sort_by_key(|candidate| candidate.slot);
    request_status.successful_query = None;
    request_status.last_error = None;
    request_status.state = if request_status.assets.len() >= request.count as usize {
        TaskState::Completed
    } else {
        TaskState::Partial
    };

    recompute_states(&mut status);
    persist_visual_state(project, &status)?;
    reconcile_local_project(project)?;
    let refreshed = load_visual_status(project)?;

    Ok(build_summary(
        project,
        &refreshed,
        scene_index,
        request_index,
        asset,
        false,
    ))
}

fn build_summary(
    project: &StoredProject,
    status: &VisualFlowStatus,
    scene_index: usize,
    request_index: usize,
    asset: VisualAssetStatus,
    already_present: bool,
) -> ManualVisualImportSummary {
    ManualVisualImportSummary {
        project_id: project.metadata.project_id.clone(),
        scene_id: status.scenes[scene_index].id.clone(),
        visual_id: status.scenes[scene_index].requests[request_index]
            .id
            .clone(),
        slot: asset.slot,
        kind: asset.kind,
        relative_path: asset.relative_path,
        sha256: asset.sha256,
        bytes: asset.bytes,
        request_state: status.scenes[scene_index].requests[request_index].state,
        scene_state: status.scenes[scene_index].state,
        flow_state: status.state,
        overall: project.status.overall,
        already_present,
    }
}

fn persist_visual_state(
    project: &mut StoredProject,
    status: &VisualFlowStatus,
) -> Result<(), ManualVisualError> {
    write_json_atomic(&visual_status_path(&project.root), status)?;
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
    ProjectStore::save_status(&project.root, &project.status)?;
    Ok(())
}

fn recompute_states(status: &mut VisualFlowStatus) {
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

fn ensure_media_compatible(
    expected: MediaKind,
    actual: PersistedAssetKind,
) -> Result<(), ManualVisualError> {
    let compatible = matches!(
        (expected, actual),
        (MediaKind::Image, PersistedAssetKind::Image)
            | (MediaKind::Video, PersistedAssetKind::Video)
            | (MediaKind::Either, _)
    );
    if compatible {
        return Ok(());
    }
    Err(ManualVisualError::MediaMismatch {
        kind: kind_text(actual).to_owned(),
        expected: match expected {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
            MediaKind::Either => "either",
        }
        .to_owned(),
    })
}

fn normalized_extension(path: &Path) -> Result<String, ManualVisualError> {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ManualVisualError::UnsupportedExtension("<none>".to_owned()))
}

fn classify_extension(
    extension: &str,
) -> Result<(PersistedAssetKind, &'static str), ManualVisualError> {
    match extension {
        "jpg" | "jpeg" => Ok((PersistedAssetKind::Image, "image/jpeg")),
        "png" => Ok((PersistedAssetKind::Image, "image/png")),
        "webp" => Ok((PersistedAssetKind::Image, "image/webp")),
        "mp4" | "m4v" => Ok((PersistedAssetKind::Video, "video/mp4")),
        "webm" => Ok((PersistedAssetKind::Video, "video/webm")),
        "mov" => Ok((PersistedAssetKind::Video, "video/quicktime")),
        "mkv" => Ok((PersistedAssetKind::Video, "video/x-matroska")),
        other => Err(ManualVisualError::UnsupportedExtension(other.to_owned())),
    }
}

fn kind_folder(kind: PersistedAssetKind) -> &'static str {
    match kind {
        PersistedAssetKind::Image => "images",
        PersistedAssetKind::Video => "videos",
    }
}

fn kind_text(kind: PersistedAssetKind) -> &'static str {
    match kind {
        PersistedAssetKind::Image => "image",
        PersistedAssetKind::Video => "video",
    }
}

fn next_missing_slot(assets: &[VisualAssetStatus], target: u32) -> u32 {
    for slot in 1..=target {
        if !assets.iter().any(|asset| asset.slot == slot) {
            return slot;
        }
    }
    target.saturating_add(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileProof {
    bytes: u64,
    sha256: String,
}

fn file_proof(path: &Path) -> Result<FileProof, ManualVisualError> {
    let mut file = File::open(path).map_err(|source| ManualVisualError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| ManualVisualError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes = bytes.saturating_add(read as u64);
    }
    if bytes == 0 {
        return Err(ManualVisualError::EmptySource(path.to_path_buf()));
    }
    Ok(FileProof {
        bytes,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn copy_verified_atomic(
    source_path: &Path,
    destination: &Path,
    expected: &FileProof,
) -> Result<(), ManualVisualError> {
    if destination.is_file() {
        let existing = file_proof(destination)?;
        if existing == *expected {
            return Ok(());
        }
        return Err(ManualVisualError::DestinationOccupied(
            destination.to_path_buf(),
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        ManualVisualError::InvalidState("manual visual destination has no parent".to_owned())
    })?;
    fs::create_dir_all(parent).map_err(|source| ManualVisualError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut source_file = File::open(source_path).map_err(|source| ManualVisualError::Io {
        path: source_path.to_path_buf(),
        source,
    })?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|source| ManualVisualError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let copied = std::io::copy(&mut source_file, temp.as_file_mut()).map_err(|source| {
        ManualVisualError::Io {
            path: destination.to_path_buf(),
            source,
        }
    })?;
    if copied != expected.bytes {
        return Err(ManualVisualError::InvalidState(format!(
            "manual visual copy length changed during import: expected {}, copied {copied}",
            expected.bytes
        )));
    }
    temp.as_file_mut()
        .sync_all()
        .map_err(|source| ManualVisualError::Io {
            path: destination.to_path_buf(),
            source,
        })?;
    temp.persist_noclobber(destination)
        .map_err(|error| ManualVisualError::Io {
            path: destination.to_path_buf(),
            source: error.error,
        })?;
    let copied_proof = file_proof(destination)?;
    if copied_proof != *expected {
        let _ = fs::remove_file(destination);
        return Err(ManualVisualError::InvalidState(
            "manual visual checksum changed during atomic import".to_owned(),
        ));
    }
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), ManualVisualError> {
    let parent = path
        .parent()
        .ok_or_else(|| ManualVisualError::InvalidState("JSON target has no parent".to_owned()))?;
    fs::create_dir_all(parent).map_err(|source| ManualVisualError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|source| ManualVisualError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    serde_json::to_writer_pretty(temp.as_file_mut(), value).map_err(|error| {
        ManualVisualError::Json {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| ManualVisualError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|source| ManualVisualError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.persist(path).map_err(|error| ManualVisualError::Io {
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::{
        AssetDownloadError, AssetDownloader, CreatorAttribution, DownloadReceipt,
        RateLimitMetadata, StockAssetCandidate, StockProvider, StockProviderError, StockRendition,
        StockSearchPage, StockSearchRequest, VisualExecutor,
    };

    const SCRIPT: &str = r#"--- SCENES ---

format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: image
        queries:
          - "quiet room"
        count: 2

--- OMNIVOICE ---

# Manual Demo

## S01 - 0:00-0:10

Manual narration.
"#;

    fn create_project() -> (tempfile::TempDir, StoredProject) {
        let temp = tempfile::tempdir().unwrap();
        let prepared = crate::parse_script(SCRIPT).unwrap();
        let project = ProjectStore::new(temp.path())
            .create("manual-demo", SCRIPT, &prepared)
            .unwrap();
        (temp, project)
    }

    #[test]
    fn import_copies_asset_with_manual_provenance_without_source_path_leak() {
        let (temp, mut project) = create_project();
        let source = temp.path().join("outside-source.jpg");
        fs::write(&source, b"manual-image-bytes").unwrap();

        let summary = import_manual_visual_asset(&mut project, "S01", "V01", &source).unwrap();
        assert_eq!(summary.slot, 1);
        assert_eq!(summary.kind, PersistedAssetKind::Image);
        assert_eq!(summary.request_state, TaskState::Partial);
        assert!(!summary.already_present);
        assert!(project.root.join(&summary.relative_path).is_file());

        let status = load_visual_status(&project).unwrap();
        let asset = &status.scenes[0].requests[0].assets[0];
        assert_eq!(asset.provider, "manual");
        let provenance =
            fs::read_to_string(project.root.join(&asset.provenance_relative_path)).unwrap();
        assert!(provenance.contains("\"source\": \"manual\""));
        assert!(provenance.contains("manual://imported"));
        assert!(!provenance.contains(source.to_string_lossy().as_ref()));
    }

    #[test]
    fn repeated_same_file_is_idempotent() {
        let (temp, mut project) = create_project();
        let source = temp.path().join("same.jpg");
        fs::write(&source, b"same-manual-image").unwrap();

        let first = import_manual_visual_asset(&mut project, "S01", "V01", &source).unwrap();
        let second = import_manual_visual_asset(&mut project, "S01", "V01", &source).unwrap();
        assert_eq!(first.slot, second.slot);
        assert!(second.already_present);
        let status = load_visual_status(&project).unwrap();
        assert_eq!(status.scenes[0].requests[0].assets.len(), 1);
    }

    #[test]
    fn incompatible_media_is_rejected_without_mutating_status() {
        let (temp, mut project) = create_project();
        let source = temp.path().join("clip.mp4");
        fs::write(&source, b"video-bytes").unwrap();

        let error = import_manual_visual_asset(&mut project, "S01", "V01", &source).unwrap_err();
        assert!(matches!(error, ManualVisualError::MediaMismatch { .. }));
        let status = load_visual_status(&project).unwrap();
        assert!(status.scenes[0].requests[0].assets.is_empty());
    }

    #[test]
    fn filling_required_slots_completes_request_and_visual_flow() {
        let (temp, mut project) = create_project();
        let source_a = temp.path().join("a.jpg");
        let source_b = temp.path().join("b.png");
        fs::write(&source_a, b"image-a").unwrap();
        fs::write(&source_b, b"image-b").unwrap();

        let first = import_manual_visual_asset(&mut project, "S01", "V01", &source_a).unwrap();
        assert_eq!(first.request_state, TaskState::Partial);
        let second = import_manual_visual_asset(&mut project, "S01", "V01", &source_b).unwrap();
        assert_eq!(second.request_state, TaskState::Completed);
        assert_eq!(second.scene_state, TaskState::Completed);
        assert_eq!(second.flow_state, TaskState::Completed);
    }

    #[derive(Clone, Default)]
    struct MockProvider {
        calls: Arc<Mutex<u32>>,
    }

    impl StockProvider for MockProvider {
        fn provider_name(&self) -> &'static str {
            "mock"
        }

        fn search_images(
            &self,
            request: &StockSearchRequest,
        ) -> Result<StockSearchPage, StockProviderError> {
            *self.calls.lock().unwrap() += 1;
            Ok(StockSearchPage {
                page: request.page,
                per_page: request.per_page as u32,
                total_results: 1,
                next_page: None,
                items: vec![StockAssetCandidate {
                    provider: "mock",
                    provider_asset_id: "provider-asset-1".to_owned(),
                    kind: crate::AssetKind::Image,
                    source_url: "https://example.invalid/asset.jpg".to_owned(),
                    creator: CreatorAttribution {
                        provider_creator_id: Some("creator-1".to_owned()),
                        name: "Mock Creator".to_owned(),
                        profile_url: None,
                    },
                    width: Some(1280),
                    height: Some(720),
                    duration_seconds: None,
                    preview_url: None,
                    alt_text: None,
                    renditions: vec![StockRendition {
                        label: "original".to_owned(),
                        url: "mock://asset".to_owned(),
                        mime_type: Some("image/jpeg".to_owned()),
                        width: Some(1280),
                        height: Some(720),
                        fps: None,
                    }],
                }],
                rate_limit: RateLimitMetadata::default(),
            })
        }

        fn search_videos(
            &self,
            _request: &StockSearchRequest,
        ) -> Result<StockSearchPage, StockProviderError> {
            unreachable!("image-only test")
        }
    }

    #[derive(Default)]
    struct MockDownloader;

    impl AssetDownloader for MockDownloader {
        fn download_atomic(
            &self,
            _url: &str,
            final_path: &Path,
        ) -> Result<DownloadReceipt, AssetDownloadError> {
            let bytes = b"provider-image";
            fs::create_dir_all(final_path.parent().unwrap()).unwrap();
            fs::write(final_path, bytes).unwrap();
            Ok(DownloadReceipt {
                bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            })
        }
    }

    #[test]
    fn visual_resume_preserves_manual_slot_and_fetches_only_missing_slot() {
        let (temp, mut project) = create_project();
        let source = temp.path().join("manual.jpg");
        fs::write(&source, b"manual-slot-one").unwrap();
        import_manual_visual_asset(&mut project, "S01", "V01", &source).unwrap();

        let provider = MockProvider::default();
        let downloader = MockDownloader;
        let summary = VisualExecutor::new(&provider, &downloader)
            .run(&mut project)
            .unwrap();
        assert_eq!(summary.state, TaskState::Completed);
        assert_eq!(*provider.calls.lock().unwrap(), 1);

        let status = load_visual_status(&project).unwrap();
        let assets = &status.scenes[0].requests[0].assets;
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].slot, 1);
        assert_eq!(assets[0].provider, "manual");
        assert_eq!(assets[1].slot, 2);
        assert_eq!(assets[1].provider, "mock");
    }
}

use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{
    AssetKind, CreatorAttribution, MediaKind, ProjectError, ProjectStore, StockAssetCandidate,
    StockProvider, StockProviderError, StockRendition, StockSearchRequest, StoredProject,
    TaskState, VisualRequest,
};

pub const VISUAL_STATUS_SCHEMA_VERSION: u32 = 1;
pub const PROVENANCE_SCHEMA_VERSION: u32 = 1;
const SEARCH_PER_PAGE: u8 = 20;
const MAX_VIDEO_WIDTH: u32 = 1920;
const MAX_VIDEO_HEIGHT: u32 = 1080;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PersistedAssetKind {
    Image,
    Video,
}

impl From<AssetKind> for PersistedAssetKind {
    fn from(value: AssetKind) -> Self {
        match value {
            AssetKind::Image => Self::Image,
            AssetKind::Video => Self::Video,
        }
    }
}

impl PersistedAssetKind {
    fn folder(self) -> &'static str {
        match self {
            Self::Image => "images",
            Self::Video => "videos",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualAssetStatus {
    pub slot: u32,
    pub kind: PersistedAssetKind,
    pub provider: String,
    pub provider_asset_id: String,
    pub relative_path: String,
    pub provenance_relative_path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualRequestStatus {
    pub id: String,
    pub state: TaskState,
    #[serde(default)]
    pub attempted_queries: Vec<String>,
    pub successful_query: Option<String>,
    #[serde(default)]
    pub assets: Vec<VisualAssetStatus>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualSceneStatus {
    pub id: String,
    pub state: TaskState,
    pub requests: Vec<VisualRequestStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualFlowStatus {
    pub schema_version: u32,
    pub project_id: String,
    pub state: TaskState,
    pub scenes: Vec<VisualSceneStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualAssetProvenance {
    pub schema_version: u32,
    pub scene_id: String,
    pub visual_id: String,
    pub slot: u32,
    pub provider: String,
    pub provider_asset_id: String,
    pub source_url: String,
    pub creator: ProvenanceCreator,
    pub query: String,
    pub rendition: ProvenanceRendition,
    pub relative_path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceCreator {
    pub provider_creator_id: Option<String>,
    pub name: String,
    pub profile_url: Option<String>,
}

impl From<&CreatorAttribution> for ProvenanceCreator {
    fn from(value: &CreatorAttribution) -> Self {
        Self {
            provider_creator_id: value.provider_creator_id.clone(),
            name: value.name.clone(),
            profile_url: value.profile_url.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceRendition {
    pub label: String,
    pub url: String,
    pub mime_type: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
}

impl From<&StockRendition> for ProvenanceRendition {
    fn from(value: &StockRendition) -> Self {
        Self {
            label: value.label.clone(),
            url: value.url.clone(),
            mime_type: value.mime_type.clone(),
            width: value.width,
            height: value.height,
            fps: value.fps,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadReceipt {
    pub bytes: u64,
    pub sha256: String,
}

pub trait AssetDownloader {
    fn download_atomic(
        &self,
        url: &str,
        final_path: &Path,
    ) -> Result<DownloadReceipt, AssetDownloadError>;
}

#[derive(Debug, Error)]
pub enum AssetDownloadError {
    #[error("asset URL is invalid")]
    InvalidUrl,

    #[error("asset server returned HTTP {status}")]
    HttpStatus { status: u16 },

    #[error("asset transport error: {message}")]
    Transport { message: String },

    #[error("asset payload is empty")]
    EmptyPayload,

    #[error("asset content length mismatch: expected {expected}, received {actual}")]
    ContentLengthMismatch { expected: u64, actual: u64 },

    #[error("asset I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct HttpAssetDownloader {
    client: Client,
}

impl HttpAssetDownloader {
    pub fn new() -> Result<Self, AssetDownloadError> {
        let client = Client::builder()
            .user_agent(concat!("video-prepare/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| AssetDownloadError::Transport {
                message: error.without_url().to_string(),
            })?;
        Ok(Self { client })
    }
}

impl AssetDownloader for HttpAssetDownloader {
    fn download_atomic(
        &self,
        url: &str,
        final_path: &Path,
    ) -> Result<DownloadReceipt, AssetDownloadError> {
        let parsed = Url::parse(url).map_err(|_| AssetDownloadError::InvalidUrl)?;
        let mut response =
            self.client
                .get(parsed)
                .send()
                .map_err(|error| AssetDownloadError::Transport {
                    message: error.without_url().to_string(),
                })?;
        if !response.status().is_success() {
            return Err(AssetDownloadError::HttpStatus {
                status: response.status().as_u16(),
            });
        }
        let expected_length = response.content_length();
        let parent = final_path.parent().ok_or(AssetDownloadError::InvalidUrl)?;
        fs::create_dir_all(parent).map_err(|source| AssetDownloadError::Io {
            path: parent.to_path_buf(),
            source,
        })?;

        let mut temp = NamedTempFile::new_in(parent).map_err(|source| AssetDownloadError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];

        loop {
            let read =
                response
                    .read(&mut buffer)
                    .map_err(|error| AssetDownloadError::Transport {
                        message: error.to_string(),
                    })?;
            if read == 0 {
                break;
            }
            temp.as_file_mut()
                .write_all(&buffer[..read])
                .map_err(|source| AssetDownloadError::Io {
                    path: final_path.to_path_buf(),
                    source,
                })?;
            hasher.update(&buffer[..read]);
            total = total.saturating_add(read as u64);
        }

        if total == 0 {
            return Err(AssetDownloadError::EmptyPayload);
        }
        if let Some(expected) = expected_length {
            if expected != total {
                return Err(AssetDownloadError::ContentLengthMismatch {
                    expected,
                    actual: total,
                });
            }
        }

        temp.as_file_mut()
            .sync_all()
            .map_err(|source| AssetDownloadError::Io {
                path: final_path.to_path_buf(),
                source,
            })?;
        temp.persist(final_path)
            .map_err(|error| AssetDownloadError::Io {
                path: final_path.to_path_buf(),
                source: error.error,
            })?;

        Ok(DownloadReceipt {
            bytes: total,
            sha256: format!("{:x}", hasher.finalize()),
        })
    }
}

#[derive(Debug, Error)]
pub enum VisualError {
    #[error(transparent)]
    Project(#[from] ProjectError),

    #[error("visual state is invalid: {0}")]
    InvalidState(String),

    #[error("visual JSON error for {path}: {message}")]
    Json { path: PathBuf, message: String },

    #[error("visual I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRunSummary {
    pub state: TaskState,
    pub completed_assets: u32,
    pub missing_assets: u32,
}

pub struct VisualExecutor<'a, P, D> {
    provider: &'a P,
    downloader: &'a D,
}

impl<'a, P, D> VisualExecutor<'a, P, D>
where
    P: StockProvider,
    D: AssetDownloader,
{
    pub fn new(provider: &'a P, downloader: &'a D) -> Self {
        Self {
            provider,
            downloader,
        }
    }

    pub fn run(&self, project: &mut StoredProject) -> Result<VisualRunSummary, VisualError> {
        let mut status = load_or_initialize_visual_status(project)?;
        reconcile_assets(project, &mut status)?;
        persist_visual_state(project, &status)?;

        for scene_index in 0..project.prepared_script.scenes.len() {
            let request_count = project.prepared_script.scenes[scene_index].visuals.len();
            for request_index in 0..request_count {
                let request =
                    project.prepared_script.scenes[scene_index].visuals[request_index].clone();
                let current = &status.scenes[scene_index].requests[request_index];
                if current.state == TaskState::Completed
                    && current.assets.len() >= request.count as usize
                {
                    continue;
                }

                status.scenes[scene_index].requests[request_index].state = TaskState::Running;
                status.scenes[scene_index].requests[request_index].last_error = None;
                recompute_states(&mut status);
                persist_visual_state(project, &status)?;

                self.execute_request(project, &mut status, scene_index, request_index, &request)?;
                recompute_states(&mut status);
                persist_visual_state(project, &status)?;
            }
        }

        recompute_states(&mut status);
        persist_visual_state(project, &status)?;
        Ok(build_summary(project, &status))
    }

    fn execute_request(
        &self,
        project: &mut StoredProject,
        status: &mut VisualFlowStatus,
        scene_index: usize,
        request_index: usize,
        request: &VisualRequest,
    ) -> Result<(), VisualError> {
        let scene_id = project.prepared_script.scenes[scene_index].id.as_str();
        let query_plan = build_query_plan(&request.queries);
        let mut used_assets: HashSet<(String, String)> = status.scenes[scene_index].requests
            [request_index]
            .assets
            .iter()
            .map(|asset| (asset.provider.clone(), asset.provider_asset_id.clone()))
            .collect();

        for query in query_plan {
            if status.scenes[scene_index].requests[request_index]
                .assets
                .len()
                >= request.count as usize
            {
                break;
            }
            status.scenes[scene_index].requests[request_index]
                .attempted_queries
                .push(query.clone());

            let candidates = match self.search(request.media, &query) {
                Ok(candidates) => candidates,
                Err(error) => {
                    let request_status = &mut status.scenes[scene_index].requests[request_index];
                    request_status.state = state_for_asset_count(
                        request_status.assets.len(),
                        request.count as usize,
                        true,
                    );
                    request_status.last_error = Some(safe_provider_error(&error));
                    return Ok(());
                }
            };

            for candidate in candidates {
                if status.scenes[scene_index].requests[request_index]
                    .assets
                    .len()
                    >= request.count as usize
                {
                    break;
                }
                let asset_key = (
                    candidate.provider.to_owned(),
                    candidate.provider_asset_id.clone(),
                );
                if used_assets.contains(&asset_key) {
                    continue;
                }
                let Some(rendition) = select_rendition(&candidate) else {
                    continue;
                };
                let slot = next_missing_slot(
                    &status.scenes[scene_index].requests[request_index].assets,
                    request.count,
                );
                let kind = PersistedAssetKind::from(candidate.kind);
                let extension = extension_for_rendition(&rendition);
                let relative_path = format!(
                    "scenes/{scene_id}/{}/{}/asset-{slot:03}.{extension}",
                    kind.folder(),
                    request.id
                );
                let final_path = project.root.join(&relative_path);

                match self.downloader.download_atomic(&rendition.url, &final_path) {
                    Ok(receipt) => {
                        let provenance_relative_path = format!(
                            "scenes/{scene_id}/{}/{}/asset-{slot:03}.provenance.json",
                            kind.folder(),
                            request.id
                        );
                        let provenance = VisualAssetProvenance {
                            schema_version: PROVENANCE_SCHEMA_VERSION,
                            scene_id: scene_id.to_owned(),
                            visual_id: request.id.clone(),
                            slot,
                            provider: candidate.provider.to_owned(),
                            provider_asset_id: candidate.provider_asset_id.clone(),
                            source_url: candidate.source_url.clone(),
                            creator: ProvenanceCreator::from(&candidate.creator),
                            query: query.clone(),
                            rendition: ProvenanceRendition::from(&rendition),
                            relative_path: relative_path.clone(),
                            sha256: receipt.sha256.clone(),
                            bytes: receipt.bytes,
                        };
                        write_json_atomic(
                            &project.root.join(&provenance_relative_path),
                            &provenance,
                        )?;

                        let request_status =
                            &mut status.scenes[scene_index].requests[request_index];
                        request_status.assets.push(VisualAssetStatus {
                            slot,
                            kind,
                            provider: candidate.provider.to_owned(),
                            provider_asset_id: candidate.provider_asset_id.clone(),
                            relative_path,
                            provenance_relative_path,
                            sha256: receipt.sha256,
                            bytes: receipt.bytes,
                        });
                        request_status.assets.sort_by_key(|asset| asset.slot);
                        request_status.successful_query = Some(query.clone());
                        request_status.last_error = None;
                        request_status.state = state_for_asset_count(
                            request_status.assets.len(),
                            request.count as usize,
                            false,
                        );
                        used_assets.insert(asset_key);
                        recompute_states(status);
                        persist_visual_state(project, status)?;
                    }
                    Err(error) => {
                        status.scenes[scene_index].requests[request_index].last_error =
                            Some(error.to_string());
                    }
                }
            }
        }

        let request_status = &mut status.scenes[scene_index].requests[request_index];
        if request_status.assets.len() >= request.count as usize {
            request_status.state = TaskState::Completed;
            request_status.last_error = None;
        } else {
            request_status.state =
                state_for_asset_count(request_status.assets.len(), request.count as usize, true);
            if request_status.last_error.is_none() {
                request_status.last_error =
                    Some("no downloadable stock candidate found".to_owned());
            }
        }
        Ok(())
    }

    fn search(
        &self,
        media: MediaKind,
        query: &str,
    ) -> Result<Vec<StockAssetCandidate>, StockProviderError> {
        let request = StockSearchRequest::new(query, 1, SEARCH_PER_PAGE)?;
        match media {
            MediaKind::Image => Ok(self.provider.search_images(&request)?.items),
            MediaKind::Video => Ok(self.provider.search_videos(&request)?.items),
            MediaKind::Either => {
                let mut items = self.provider.search_videos(&request)?.items;
                items.extend(self.provider.search_images(&request)?.items);
                Ok(items)
            }
        }
    }
}

pub fn visual_status_path(project_root: &Path) -> PathBuf {
    project_root.join("visual-status.json")
}

pub fn load_visual_status(project: &StoredProject) -> Result<VisualFlowStatus, VisualError> {
    load_or_initialize_visual_status(project)
}

fn load_or_initialize_visual_status(
    project: &StoredProject,
) -> Result<VisualFlowStatus, VisualError> {
    let path = visual_status_path(&project.root);
    if !path.is_file() {
        return Ok(initial_visual_status(project));
    }
    let bytes = fs::read(&path).map_err(|source| VisualError::Io {
        path: path.clone(),
        source,
    })?;
    let status: VisualFlowStatus =
        serde_json::from_slice(&bytes).map_err(|error| VisualError::Json {
            path: path.clone(),
            message: error.to_string(),
        })?;
    validate_visual_status(project, &status)?;
    Ok(status)
}

fn initial_visual_status(project: &StoredProject) -> VisualFlowStatus {
    VisualFlowStatus {
        schema_version: VISUAL_STATUS_SCHEMA_VERSION,
        project_id: project.metadata.project_id.clone(),
        state: TaskState::Pending,
        scenes: project
            .prepared_script
            .scenes
            .iter()
            .map(|scene| VisualSceneStatus {
                id: scene.id.clone(),
                state: TaskState::Pending,
                requests: scene
                    .visuals
                    .iter()
                    .map(|request| VisualRequestStatus {
                        id: request.id.clone(),
                        state: TaskState::Pending,
                        attempted_queries: Vec::new(),
                        successful_query: None,
                        assets: Vec::new(),
                        last_error: None,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn validate_visual_status(
    project: &StoredProject,
    status: &VisualFlowStatus,
) -> Result<(), VisualError> {
    if status.schema_version != VISUAL_STATUS_SCHEMA_VERSION {
        return Err(VisualError::InvalidState(format!(
            "unsupported visual schema_version {}",
            status.schema_version
        )));
    }
    if status.project_id != project.metadata.project_id {
        return Err(VisualError::InvalidState(
            "visual project_id does not match project metadata".to_owned(),
        ));
    }
    if status.scenes.len() != project.prepared_script.scenes.len() {
        return Err(VisualError::InvalidState(
            "visual scene count does not match script".to_owned(),
        ));
    }
    for (stored_scene, script_scene) in status.scenes.iter().zip(&project.prepared_script.scenes) {
        if stored_scene.id != script_scene.id {
            return Err(VisualError::InvalidState(
                "visual scene ids do not match script".to_owned(),
            ));
        }
        if stored_scene.requests.len() != script_scene.visuals.len() {
            return Err(VisualError::InvalidState(format!(
                "visual request count does not match scene {}",
                script_scene.id
            )));
        }
        for (stored_request, script_request) in
            stored_scene.requests.iter().zip(&script_scene.visuals)
        {
            if stored_request.id != script_request.id {
                return Err(VisualError::InvalidState(format!(
                    "visual request ids do not match scene {}",
                    script_scene.id
                )));
            }
        }
    }
    Ok(())
}

fn reconcile_assets(
    project: &StoredProject,
    status: &mut VisualFlowStatus,
) -> Result<(), VisualError> {
    for (scene_index, scene) in status.scenes.iter_mut().enumerate() {
        for (request_index, request_status) in scene.requests.iter_mut().enumerate() {
            let target_count =
                project.prepared_script.scenes[scene_index].visuals[request_index].count;
            let was_running = request_status.state == TaskState::Running;
            let mut invalid = false;
            request_status.assets.retain(|asset| {
                let path = project.root.join(&asset.relative_path);
                match sha256_file(&path) {
                    Ok(hash) if hash == asset.sha256 => true,
                    _ => {
                        invalid = true;
                        false
                    }
                }
            });
            if request_status.assets.len() >= target_count as usize {
                request_status.state = TaskState::Completed;
                request_status.last_error = None;
            } else if !request_status.assets.is_empty() {
                request_status.state = TaskState::Partial;
                if invalid {
                    request_status.last_error =
                        Some("completed asset is missing or hash-mismatched".to_owned());
                }
            } else if was_running || invalid || request_status.state == TaskState::Completed {
                request_status.state = TaskState::Interrupted;
                request_status.last_error =
                    Some("completed/running asset state requires recovery".to_owned());
            }
        }
    }
    recompute_states(status);
    Ok(())
}

fn persist_visual_state(
    project: &mut StoredProject,
    status: &VisualFlowStatus,
) -> Result<(), VisualError> {
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
    let scene_states: Vec<TaskState> = status.scenes.iter().map(|scene| scene.state).collect();
    status.state = aggregate_states(&scene_states);
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
    let has_success = states
        .iter()
        .any(|state| matches!(state, TaskState::Completed | TaskState::Partial));
    if has_success {
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

fn state_for_asset_count(current: usize, target: usize, exhausted: bool) -> TaskState {
    if current >= target {
        TaskState::Completed
    } else if current > 0 {
        TaskState::Partial
    } else if exhausted {
        TaskState::Failed
    } else {
        TaskState::Running
    }
}

fn build_summary(project: &StoredProject, status: &VisualFlowStatus) -> VisualRunSummary {
    let completed_assets = status
        .scenes
        .iter()
        .flat_map(|scene| &scene.requests)
        .map(|request| request.assets.len() as u32)
        .sum();
    let requested_assets: u32 = project
        .prepared_script
        .scenes
        .iter()
        .flat_map(|scene| &scene.visuals)
        .map(|request| request.count)
        .sum();
    VisualRunSummary {
        state: status.state,
        completed_assets,
        missing_assets: requested_assets.saturating_sub(completed_assets),
    }
}

pub fn build_query_plan(queries: &[String]) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "a", "an", "and", "at", "for", "in", "of", "on", "the", "to", "with",
    ];

    let mut seen = HashSet::new();
    let mut plan = Vec::new();
    for raw in queries {
        let exact = collapse_whitespace(raw.trim());
        push_unique(&mut plan, &mut seen, exact.clone());

        let sanitized = collapse_whitespace(
            &exact
                .chars()
                .map(|character| {
                    if character.is_alphanumeric() || character.is_whitespace() {
                        character
                    } else {
                        ' '
                    }
                })
                .collect::<String>(),
        );
        push_unique(&mut plan, &mut seen, sanitized.clone());

        let reduced = sanitized
            .split_whitespace()
            .filter(|token| !STOPWORDS.contains(&token.to_ascii_lowercase().as_str()))
            .collect::<Vec<_>>();
        if reduced.len() >= 2 {
            push_unique(&mut plan, &mut seen, reduced.join(" "));
        }
    }
    plan
}

fn push_unique(plan: &mut Vec<String>, seen: &mut HashSet<String>, query: String) {
    if !query.is_empty() && seen.insert(query.clone()) {
        plan.push(query);
    }
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn select_rendition(candidate: &StockAssetCandidate) -> Option<StockRendition> {
    let mut renditions: Vec<StockRendition> = candidate
        .renditions
        .iter()
        .filter(|rendition| !rendition.url.trim().is_empty())
        .cloned()
        .collect();
    if renditions.is_empty() {
        return None;
    }

    match candidate.kind {
        AssetKind::Image => {
            if let Some(original) = renditions
                .iter()
                .find(|rendition| rendition.label.eq_ignore_ascii_case("original"))
            {
                return Some(original.clone());
            }
            renditions.sort_by(|left, right| {
                rendition_area(right)
                    .cmp(&rendition_area(left))
                    .then_with(|| left.label.cmp(&right.label))
            });
            renditions.into_iter().next()
        }
        AssetKind::Video => {
            let mp4: Vec<StockRendition> = renditions
                .iter()
                .filter(|rendition| {
                    rendition
                        .mime_type
                        .as_deref()
                        .is_some_and(|mime| mime.eq_ignore_ascii_case("video/mp4"))
                })
                .cloned()
                .collect();
            if !mp4.is_empty() {
                renditions = mp4;
            }

            let mut under_cap: Vec<StockRendition> = renditions
                .iter()
                .filter(|rendition| {
                    matches!(
                        (rendition.width, rendition.height),
                        (Some(width), Some(height))
                            if width <= MAX_VIDEO_WIDTH && height <= MAX_VIDEO_HEIGHT
                    )
                })
                .cloned()
                .collect();
            if !under_cap.is_empty() {
                under_cap.sort_by(|left, right| {
                    rendition_area(right)
                        .cmp(&rendition_area(left))
                        .then_with(|| left.label.cmp(&right.label))
                });
                return under_cap.into_iter().next();
            }

            let mut known: Vec<StockRendition> = renditions
                .iter()
                .filter(|rendition| rendition.width.is_some() && rendition.height.is_some())
                .cloned()
                .collect();
            if !known.is_empty() {
                known.sort_by(|left, right| {
                    rendition_area(left)
                        .cmp(&rendition_area(right))
                        .then_with(|| left.label.cmp(&right.label))
                });
                return known.into_iter().next();
            }

            renditions.sort_by(|left, right| left.label.cmp(&right.label));
            renditions.into_iter().next()
        }
    }
}

fn rendition_area(rendition: &StockRendition) -> u64 {
    u64::from(rendition.width.unwrap_or(0)) * u64::from(rendition.height.unwrap_or(0))
}

fn next_missing_slot(assets: &[VisualAssetStatus], target: u32) -> u32 {
    for slot in 1..=target {
        if !assets.iter().any(|asset| asset.slot == slot) {
            return slot;
        }
    }
    target.saturating_add(1)
}

fn extension_for_rendition(rendition: &StockRendition) -> String {
    if let Some(mime) = rendition.mime_type.as_deref() {
        match mime.to_ascii_lowercase().as_str() {
            "video/mp4" => return "mp4".to_owned(),
            "video/webm" => return "webm".to_owned(),
            "image/jpeg" | "image/jpg" => return "jpg".to_owned(),
            "image/png" => return "png".to_owned(),
            "image/webp" => return "webp".to_owned(),
            _ => {}
        }
    }
    if let Ok(url) = Url::parse(&rendition.url) {
        if let Some(extension) = Path::new(url.path())
            .extension()
            .and_then(|value| value.to_str())
        {
            let normalized = extension.to_ascii_lowercase();
            if !normalized.is_empty()
                && normalized.len() <= 8
                && normalized
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
            {
                return normalized;
            }
        }
    }
    "bin".to_owned()
}

fn safe_provider_error(error: &StockProviderError) -> String {
    error.to_string()
}

fn sha256_file(path: &Path) -> Result<String, VisualError> {
    let mut file = File::open(path).map_err(|source| VisualError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| VisualError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), VisualError> {
    let parent = path
        .parent()
        .ok_or_else(|| VisualError::InvalidState("JSON target has no parent".to_owned()))?;
    fs::create_dir_all(parent).map_err(|source| VisualError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temp = NamedTempFile::new_in(parent).map_err(|source| VisualError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    serde_json::to_writer_pretty(temp.as_file_mut(), value).map_err(|error| VisualError::Json {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| VisualError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|source| VisualError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.persist(path).map_err(|error| VisualError::Io {
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    use super::*;
    use crate::{parse_script, ProjectStore, StockSearchPage};

    #[derive(Default)]
    struct MockProvider {
        pages: Mutex<HashMap<(AssetKind, String), Vec<StockAssetCandidate>>>,
        calls: Mutex<Vec<(AssetKind, String)>>,
    }

    impl MockProvider {
        fn set(&self, kind: AssetKind, query: &str, items: Vec<StockAssetCandidate>) {
            self.pages
                .lock()
                .unwrap()
                .insert((kind, query.to_owned()), items);
        }

        fn calls(&self) -> Vec<(AssetKind, String)> {
            self.calls.lock().unwrap().clone()
        }

        fn search_kind(
            &self,
            kind: AssetKind,
            request: &StockSearchRequest,
        ) -> Result<StockSearchPage, StockProviderError> {
            self.calls
                .lock()
                .unwrap()
                .push((kind, request.query.clone()));
            let items = self
                .pages
                .lock()
                .unwrap()
                .get(&(kind, request.query.clone()))
                .cloned()
                .unwrap_or_default();
            Ok(StockSearchPage {
                page: 1,
                per_page: SEARCH_PER_PAGE as u32,
                total_results: items.len() as u64,
                next_page: None,
                items,
                rate_limit: Default::default(),
            })
        }
    }

    impl StockProvider for MockProvider {
        fn provider_name(&self) -> &'static str {
            "mock"
        }

        fn search_images(
            &self,
            request: &StockSearchRequest,
        ) -> Result<StockSearchPage, StockProviderError> {
            self.search_kind(AssetKind::Image, request)
        }

        fn search_videos(
            &self,
            request: &StockSearchRequest,
        ) -> Result<StockSearchPage, StockProviderError> {
            self.search_kind(AssetKind::Video, request)
        }
    }

    #[derive(Default)]
    struct MockDownloader {
        bodies: Mutex<HashMap<String, Vec<u8>>>,
        calls: Mutex<Vec<String>>,
    }

    impl MockDownloader {
        fn set(&self, url: &str, bytes: &[u8]) {
            self.bodies
                .lock()
                .unwrap()
                .insert(url.to_owned(), bytes.to_vec());
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl AssetDownloader for MockDownloader {
        fn download_atomic(
            &self,
            url: &str,
            final_path: &Path,
        ) -> Result<DownloadReceipt, AssetDownloadError> {
            self.calls.lock().unwrap().push(url.to_owned());
            let bytes = self
                .bodies
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or_default();
            if bytes.is_empty() {
                return Err(AssetDownloadError::EmptyPayload);
            }
            let parent = final_path.parent().unwrap();
            fs::create_dir_all(parent).unwrap();
            let mut temp = NamedTempFile::new_in(parent).unwrap();
            temp.write_all(&bytes).unwrap();
            temp.as_file_mut().sync_all().unwrap();
            temp.persist(final_path).unwrap();
            Ok(DownloadReceipt {
                bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            })
        }
    }

    fn candidate(id: &str, kind: AssetKind, rendition_url: &str) -> StockAssetCandidate {
        StockAssetCandidate {
            provider: "mock",
            provider_asset_id: id.to_owned(),
            kind,
            source_url: format!("https://source.test/{id}"),
            creator: CreatorAttribution {
                provider_creator_id: Some("creator-1".to_owned()),
                name: "Creator".to_owned(),
                profile_url: Some("https://source.test/creator".to_owned()),
            },
            width: Some(1920),
            height: Some(1080),
            duration_seconds: if kind == AssetKind::Video {
                Some(8)
            } else {
                None
            },
            preview_url: None,
            alt_text: None,
            renditions: vec![StockRendition {
                label: if kind == AssetKind::Image {
                    "original".to_owned()
                } else {
                    "hd".to_owned()
                },
                url: rendition_url.to_owned(),
                mime_type: Some(if kind == AssetKind::Image {
                    "image/jpeg".to_owned()
                } else {
                    "video/mp4".to_owned()
                }),
                width: Some(1920),
                height: Some(1080),
                fps: if kind == AssetKind::Video {
                    Some(30.0)
                } else {
                    None
                },
            }],
        }
    }

    fn script_two_visuals() -> String {
        r#"--- SCENES ---
format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: image
        queries: ["calm man & window"]
        count: 1
      - id: V02
        media: video
        queries: ["busy street"]
        count: 1
--- OMNIVOICE ---
# Demo
## S01 — 0:00–0:10
[WARM] Demo narration.
"#
        .to_owned()
    }

    fn create_project(raw: &str) -> (tempfile::TempDir, StoredProject) {
        let temp = tempfile::tempdir().unwrap();
        let prepared = parse_script(raw).unwrap();
        let project = ProjectStore::new(temp.path())
            .create("demo", raw, &prepared)
            .unwrap();
        (temp, project)
    }

    #[test]
    fn query_plan_is_deterministic_and_deduplicated() {
        let plan = build_query_plan(&[
            "  calm man & window  ".to_owned(),
            "calm man window".to_owned(),
        ]);
        assert_eq!(plan, vec!["calm man & window", "calm man window"]);
    }

    #[test]
    fn exact_success_stops_fallback_and_persists_provenance() {
        let raw = script_two_visuals();
        let (_temp, mut project) = create_project(&raw);
        let provider = MockProvider::default();
        let downloader = MockDownloader::default();
        provider.set(
            AssetKind::Image,
            "calm man & window",
            vec![candidate(
                "img-1",
                AssetKind::Image,
                "https://asset.test/img-1.jpg",
            )],
        );
        provider.set(
            AssetKind::Video,
            "busy street",
            vec![candidate(
                "vid-1",
                AssetKind::Video,
                "https://asset.test/vid-1.mp4",
            )],
        );
        downloader.set("https://asset.test/img-1.jpg", b"image-bytes");
        downloader.set("https://asset.test/vid-1.mp4", b"video-bytes");

        let summary = VisualExecutor::new(&provider, &downloader)
            .run(&mut project)
            .unwrap();
        assert_eq!(summary.state, TaskState::Completed);
        assert_eq!(summary.completed_assets, 2);
        assert_eq!(summary.missing_assets, 0);

        let calls = provider.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, "calm man & window");
        assert_eq!(calls[1].1, "busy street");

        let status = load_visual_status(&project).unwrap();
        let image = &status.scenes[0].requests[0].assets[0];
        assert!(project.root.join(&image.relative_path).is_file());
        let provenance: VisualAssetProvenance = serde_json::from_slice(
            &fs::read(project.root.join(&image.provenance_relative_path)).unwrap(),
        )
        .unwrap();
        assert_eq!(provenance.provider_asset_id, "img-1");
        assert_eq!(provenance.query, "calm man & window");
        assert_eq!(provenance.sha256, image.sha256);
    }

    #[test]
    fn fallback_runs_after_empty_exact_query() {
        let raw = script_two_visuals();
        let (_temp, mut project) = create_project(&raw);
        let provider = MockProvider::default();
        let downloader = MockDownloader::default();
        provider.set(
            AssetKind::Image,
            "calm man window",
            vec![candidate(
                "img-2",
                AssetKind::Image,
                "https://asset.test/img-2.jpg",
            )],
        );
        provider.set(
            AssetKind::Video,
            "busy street",
            vec![candidate(
                "vid-2",
                AssetKind::Video,
                "https://asset.test/vid-2.mp4",
            )],
        );
        downloader.set("https://asset.test/img-2.jpg", b"image-2");
        downloader.set("https://asset.test/vid-2.mp4", b"video-2");

        VisualExecutor::new(&provider, &downloader)
            .run(&mut project)
            .unwrap();
        let calls = provider.calls();
        assert_eq!(calls[0].1, "calm man & window");
        assert_eq!(calls[1].1, "calm man window");
        let status = load_visual_status(&project).unwrap();
        assert_eq!(
            status.scenes[0].requests[0].successful_query.as_deref(),
            Some("calm man window")
        );
    }

    #[test]
    fn partial_retry_skips_verified_completed_request() {
        let raw = script_two_visuals();
        let (_temp, mut project) = create_project(&raw);
        let provider = MockProvider::default();
        let downloader = MockDownloader::default();
        provider.set(
            AssetKind::Image,
            "calm man & window",
            vec![candidate(
                "img-3",
                AssetKind::Image,
                "https://asset.test/img-3.jpg",
            )],
        );
        downloader.set("https://asset.test/img-3.jpg", b"image-3");

        let first = VisualExecutor::new(&provider, &downloader)
            .run(&mut project)
            .unwrap();
        assert_eq!(first.state, TaskState::Partial);
        let first_calls = provider.calls();
        assert!(first_calls
            .iter()
            .any(|(kind, _)| *kind == AssetKind::Image));
        assert!(first_calls
            .iter()
            .any(|(kind, _)| *kind == AssetKind::Video));

        provider.set(
            AssetKind::Video,
            "busy street",
            vec![candidate(
                "vid-3",
                AssetKind::Video,
                "https://asset.test/vid-3.mp4",
            )],
        );
        downloader.set("https://asset.test/vid-3.mp4", b"video-3");
        provider.calls.lock().unwrap().clear();
        downloader.calls.lock().unwrap().clear();

        let reopened = ProjectStore::open(&project.root).unwrap();
        project = reopened;
        let second = VisualExecutor::new(&provider, &downloader)
            .run(&mut project)
            .unwrap();
        assert_eq!(second.state, TaskState::Completed);
        assert!(provider
            .calls()
            .iter()
            .all(|(kind, _)| *kind == AssetKind::Video));
        assert_eq!(downloader.calls(), vec!["https://asset.test/vid-3.mp4"]);
    }

    #[test]
    fn missing_completed_asset_is_retried_after_reopen() {
        let raw = script_two_visuals();
        let (_temp, mut project) = create_project(&raw);
        let provider = MockProvider::default();
        let downloader = MockDownloader::default();
        provider.set(
            AssetKind::Image,
            "calm man & window",
            vec![candidate(
                "img-4",
                AssetKind::Image,
                "https://asset.test/img-4.jpg",
            )],
        );
        provider.set(
            AssetKind::Video,
            "busy street",
            vec![candidate(
                "vid-4",
                AssetKind::Video,
                "https://asset.test/vid-4.mp4",
            )],
        );
        downloader.set("https://asset.test/img-4.jpg", b"image-4");
        downloader.set("https://asset.test/vid-4.mp4", b"video-4");
        VisualExecutor::new(&provider, &downloader)
            .run(&mut project)
            .unwrap();

        let status = load_visual_status(&project).unwrap();
        fs::remove_file(
            project
                .root
                .join(&status.scenes[0].requests[0].assets[0].relative_path),
        )
        .unwrap();
        provider.calls.lock().unwrap().clear();
        downloader.calls.lock().unwrap().clear();

        project = ProjectStore::open(&project.root).unwrap();
        let summary = VisualExecutor::new(&provider, &downloader)
            .run(&mut project)
            .unwrap();
        assert_eq!(summary.state, TaskState::Completed);
        assert!(provider
            .calls()
            .iter()
            .all(|(kind, _)| *kind == AssetKind::Image));
    }

    #[test]
    fn http_downloader_rejects_zero_bytes_and_truncated_payload_without_final_file() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = Arc::new(listener);
        let server_clone = Arc::clone(&server);
        thread::spawn(move || {
            for body in [b"".as_slice(), b"short".as_slice()] {
                let (mut stream, _) = server_clone.accept().unwrap();
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer);
                let declared = if body.is_empty() { 0 } else { body.len() + 10 };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
                stream.flush().unwrap();
            }
        });

        let downloader = HttpAssetDownloader::new().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let zero_path = temp.path().join("zero.bin");
        let zero = downloader.download_atomic(&format!("http://{address}/zero"), &zero_path);
        assert!(matches!(zero, Err(AssetDownloadError::EmptyPayload)));
        assert!(!zero_path.exists());

        let short_path = temp.path().join("short.bin");
        let short = downloader.download_atomic(&format!("http://{address}/short"), &short_path);
        assert!(short.is_err());
        assert!(!short_path.exists());
    }
}

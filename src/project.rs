use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{parse_script, PreparedScript, ScriptError};

pub const PROJECT_SCHEMA_VERSION: u32 = 1;
pub const STATUS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskState {
    Pending,
    Running,
    Partial,
    Completed,
    Failed,
    Skipped,
    Interrupted,
    UnknownRemote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub schema_version: u32,
    pub project_id: String,
    pub title: String,
    pub input_sha256: String,
    pub created_unix_ms: u64,
    pub scene_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRuntimeStatus {
    pub id: String,
    pub visual: TaskState,
    pub audio: TaskState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectStatus {
    pub schema_version: u32,
    pub project_id: String,
    pub overall: TaskState,
    pub visual_flow: TaskState,
    pub audio_flow: TaskState,
    pub scenes: Vec<SceneRuntimeStatus>,
}

impl ProjectStatus {
    fn initial(project_id: &str, prepared: &PreparedScript) -> Self {
        let scenes = prepared
            .scenes
            .iter()
            .map(|scene| SceneRuntimeStatus {
                id: scene.id.clone(),
                visual: TaskState::Pending,
                audio: TaskState::Pending,
            })
            .collect();

        Self {
            schema_version: STATUS_SCHEMA_VERSION,
            project_id: project_id.to_owned(),
            overall: TaskState::Pending,
            visual_flow: TaskState::Pending,
            audio_flow: TaskState::Pending,
            scenes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProject {
    pub root: PathBuf,
    pub metadata: ProjectMetadata,
    pub status: ProjectStatus,
    pub prepared_script: PreparedScript,
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("invalid project id `{0}`")]
    InvalidProjectId(String),

    #[error("project already exists: {0}")]
    ProjectExists(PathBuf),

    #[error("project does not exist or is incomplete: {0}")]
    ProjectMissing(PathBuf),

    #[error("source script and PreparedScript do not describe the same input")]
    PreparedScriptMismatch,

    #[error("project input snapshot hash mismatch: expected {expected}, found {actual}")]
    InputHashMismatch { expected: String, actual: String },

    #[error("project metadata is inconsistent: {0}")]
    MetadataMismatch(String),

    #[error("project status is inconsistent: {0}")]
    StatusMismatch(String),

    #[error("script validation failed: {0}")]
    Script(#[from] ScriptError),

    #[error("JSON error for {path}: {message}")]
    Json { path: PathBuf, message: String },

    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct ProjectStore {
    data_root: PathBuf,
}

impl ProjectStore {
    pub fn new(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn projects_root(&self) -> PathBuf {
        self.data_root.join("projects")
    }

    pub fn create(
        &self,
        project_id: &str,
        raw_script: &str,
        prepared: &PreparedScript,
    ) -> Result<StoredProject, ProjectError> {
        validate_project_id(project_id)?;

        if parse_script(raw_script)? != *prepared {
            return Err(ProjectError::PreparedScriptMismatch);
        }

        let projects_root = self.projects_root();
        create_dir_all(&projects_root)?;

        let project_root = projects_root.join(project_id);
        if project_root.exists() {
            return Err(ProjectError::ProjectExists(project_root));
        }

        let staging = projects_root.join(format!(
            ".{project_id}.create-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        create_dir(&staging)?;

        if let Err(error) = create_project_files(project_id, raw_script, prepared, &staging) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        fs::rename(&staging, &project_root).map_err(|source| ProjectError::Io {
            path: project_root.clone(),
            source,
        })?;

        Self::open(&project_root)
    }

    pub fn load(&self, project_id: &str) -> Result<StoredProject, ProjectError> {
        validate_project_id(project_id)?;
        Self::open(self.projects_root().join(project_id))
    }

    pub fn delete(&self, project_id: &str) -> Result<PathBuf, ProjectError> {
        validate_project_id(project_id)?;
        let project_root = self.projects_root().join(project_id);
        if !project_root.exists() {
            return Err(ProjectError::ProjectMissing(project_root));
        }
        fs::remove_dir_all(&project_root).map_err(|source| ProjectError::Io {
            path: project_root.clone(),
            source,
        })?;
        Ok(project_root)
    }

    pub fn open(project_root: impl AsRef<Path>) -> Result<StoredProject, ProjectError> {
        let project_root = project_root.as_ref().to_path_buf();
        let project_json = project_root.join("project.json");
        let status_json = project_root.join("status.json");
        let snapshot = project_root.join("input/script.vprep");

        if !project_json.is_file() || !status_json.is_file() || !snapshot.is_file() {
            return Err(ProjectError::ProjectMissing(project_root));
        }

        let metadata: ProjectMetadata = read_json(&project_json)?;
        validate_metadata(&project_root, &metadata)?;

        let status: ProjectStatus = read_json(&status_json)?;
        validate_status(&metadata, &status)?;

        let raw_script = fs::read_to_string(&snapshot).map_err(|source| ProjectError::Io {
            path: snapshot,
            source,
        })?;
        let prepared_script = parse_script(&raw_script)?;
        validate_snapshot(&metadata, &prepared_script)?;

        Ok(StoredProject {
            root: project_root,
            metadata,
            status,
            prepared_script,
        })
    }

    pub fn save_status(
        project_root: impl AsRef<Path>,
        status: &ProjectStatus,
    ) -> Result<(), ProjectError> {
        let project_root = project_root.as_ref();
        let metadata: ProjectMetadata = read_json(&project_root.join("project.json"))?;
        validate_status(&metadata, status)?;
        atomic_write_json(&project_root.join("status.json"), status)
    }
}

fn create_project_files(
    project_id: &str,
    raw_script: &str,
    prepared: &PreparedScript,
    staging: &Path,
) -> Result<(), ProjectError> {
    create_dir_all(&staging.join("input"))?;
    create_dir_all(&staging.join("audio/artifacts"))?;
    create_dir_all(&staging.join("logs"))?;

    for scene in &prepared.scenes {
        create_dir_all(&staging.join("scenes").join(&scene.id).join("images"))?;
        create_dir_all(&staging.join("scenes").join(&scene.id).join("videos"))?;
    }

    write_new_bytes(&staging.join("input/script.vprep"), raw_script.as_bytes())?;

    let metadata = ProjectMetadata {
        schema_version: PROJECT_SCHEMA_VERSION,
        project_id: project_id.to_owned(),
        title: prepared.omnivoice.title.clone(),
        input_sha256: prepared.input_sha256.clone(),
        created_unix_ms: now_millis(),
        scene_ids: prepared
            .scenes
            .iter()
            .map(|scene| scene.id.clone())
            .collect(),
    };
    let status = ProjectStatus::initial(project_id, prepared);

    atomic_write_json(&staging.join("project.json"), &metadata)?;
    atomic_write_json(&staging.join("status.json"), &status)
}

fn validate_metadata(root: &Path, metadata: &ProjectMetadata) -> Result<(), ProjectError> {
    validate_project_id(&metadata.project_id)?;
    if metadata.schema_version != PROJECT_SCHEMA_VERSION {
        return Err(ProjectError::MetadataMismatch(format!(
            "unsupported schema_version {}",
            metadata.schema_version
        )));
    }
    if root.file_name().and_then(|value| value.to_str()) != Some(metadata.project_id.as_str()) {
        return Err(ProjectError::MetadataMismatch(
            "folder name does not match project_id".to_owned(),
        ));
    }
    Ok(())
}

fn validate_status(metadata: &ProjectMetadata, status: &ProjectStatus) -> Result<(), ProjectError> {
    if status.schema_version != STATUS_SCHEMA_VERSION {
        return Err(ProjectError::StatusMismatch(format!(
            "unsupported schema_version {}",
            status.schema_version
        )));
    }
    if status.project_id != metadata.project_id {
        return Err(ProjectError::StatusMismatch(
            "status project_id does not match metadata".to_owned(),
        ));
    }

    let status_ids: Vec<&str> = status
        .scenes
        .iter()
        .map(|scene| scene.id.as_str())
        .collect();
    let metadata_ids: Vec<&str> = metadata.scene_ids.iter().map(String::as_str).collect();
    if status_ids != metadata_ids {
        return Err(ProjectError::StatusMismatch(
            "status scenes do not match metadata scene_ids".to_owned(),
        ));
    }
    Ok(())
}

fn validate_snapshot(
    metadata: &ProjectMetadata,
    prepared: &PreparedScript,
) -> Result<(), ProjectError> {
    if prepared.input_sha256 != metadata.input_sha256 {
        return Err(ProjectError::InputHashMismatch {
            expected: metadata.input_sha256.clone(),
            actual: prepared.input_sha256.clone(),
        });
    }
    if prepared.omnivoice.title != metadata.title {
        return Err(ProjectError::MetadataMismatch(
            "title does not match input snapshot".to_owned(),
        ));
    }

    let scene_ids: Vec<&str> = prepared
        .scenes
        .iter()
        .map(|scene| scene.id.as_str())
        .collect();
    let metadata_ids: Vec<&str> = metadata.scene_ids.iter().map(String::as_str).collect();
    if scene_ids != metadata_ids {
        return Err(ProjectError::MetadataMismatch(
            "scene_ids do not match input snapshot".to_owned(),
        ));
    }
    Ok(())
}

fn validate_project_id(project_id: &str) -> Result<(), ProjectError> {
    let valid = !project_id.is_empty()
        && project_id.len() <= 128
        && project_id == project_id.trim()
        && project_id != "."
        && project_id != ".."
        && !project_id.contains('/')
        && !project_id.contains('\\')
        && project_id
            .chars()
            .next()
            .is_some_and(|value| value.is_ascii_alphanumeric())
        && project_id
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.'));

    if valid {
        Ok(())
    } else {
        Err(ProjectError::InvalidProjectId(project_id.to_owned()))
    }
}

fn create_dir(path: &Path) -> Result<(), ProjectError> {
    fs::create_dir(path).map_err(|source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn create_dir_all(path: &Path) -> Result<(), ProjectError> {
    fs::create_dir_all(path).map_err(|source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ProjectError> {
    let bytes = fs::read(path).map_err(|source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|error| ProjectError::Json {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn write_new_bytes(path: &Path, bytes: &[u8]) -> Result<(), ProjectError> {
    let mut file = File::options()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(bytes).map_err(|source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    file.sync_all().map_err(|source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ProjectError> {
    atomic_write_json_with_hook(path, value, |_| Ok(()))
}

fn atomic_write_json_with_hook<T, F>(
    path: &Path,
    value: &T,
    before_persist: F,
) -> Result<(), ProjectError>
where
    T: Serialize,
    F: FnOnce(&Path) -> std::io::Result<()>,
{
    let parent = path.parent().ok_or_else(|| {
        ProjectError::MetadataMismatch("atomic write target has no parent directory".to_owned())
    })?;
    create_dir_all(parent)?;

    let mut temp = NamedTempFile::new_in(parent).map_err(|source| ProjectError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    serde_json::to_writer_pretty(temp.as_file_mut(), value).map_err(|error| {
        ProjectError::Json {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    before_persist(temp.path()).map_err(|source| ProjectError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    temp.persist(path).map_err(|error| ProjectError::Io {
        path: path.to_path_buf(),
        source: error.error,
    })?;
    Ok(())
}

#[doc(hidden)]
pub fn test_atomic_status_write_failure(
    project_root: &Path,
    status: &ProjectStatus,
) -> Result<(), ProjectError> {
    atomic_write_json_with_hook(&project_root.join("status.json"), status, |_| {
        Err(std::io::Error::other("injected pre-persist failure"))
    })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

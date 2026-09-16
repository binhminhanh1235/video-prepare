use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
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
    pub created_unix_ms: u128,
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
        Self {
            schema_version: STATUS_SCHEMA_VERSION,
            project_id: project_id.to_owned(),
            overall: TaskState::Pending,
            visual_flow: TaskState::Pending,
            audio_flow: TaskState::Pending,
            scenes: prepared
                .scenes
                .iter()
                .map(|scene| SceneRuntimeStatus {
                    id: scene.id.clone(),
                    visual: TaskState::Pending,
                    audio: TaskState::Pending,
                })
                .collect(),
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

        let parsed = parse_script(raw_script)?;
        if &parsed != prepared {
            return Err(ProjectError::PreparedScriptMismatch);
        }

        let projects_root = self.projects_root();
        fs::create_dir_all(&projects_root).map_err(|source| ProjectError::Io {
            path: projects_root.clone(),
            source,
        })?;

        let project_root = projects_root.join(project_id);
        if project_root.exists() {
            return Err(ProjectError::ProjectExists(project_root));
        }

        let staging = projects_root.join(format!(
            ".{project_id}.create-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        fs::create_dir(&staging).map_err(|source| ProjectError::Io {
            path: staging.clone(),
            source,
        })?;

        let result = self.create_in_staging(project_id, raw_script, prepared, &staging);
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }

        fs::rename(&staging, &project_root).map_err(|source| ProjectError::Io {
            path: project_root.clone(),
            source,
        })?;

        Self::open(&project_root)
    }

    fn create_in_staging(
        &self,
        project_id: &str,
        raw_script: &str,
        prepared: &PreparedScript,
        staging: &Path,
    ) -> Result<(), ProjectError> {
        let input_dir = staging.join("input");
        let audio_dir = staging.join("audio").join("artifacts");
        let logs_dir = staging.join("logs");
        fs::create_dir_all(&input_dir).map_err(|source| ProjectError::Io {
            path: input_dir.clone(),
            source,
        })?;
        fs::create_dir_all(&audio_dir).map_err(|source| ProjectError::Io {
            path: audio_dir.clone(),
            source,
        })?;
        fs::create_dir_all(&logs_dir).map_err(|source| ProjectError::Io {
            path: logs_dir.clone(),
            source,
        })?;

        for scene in &prepared.scenes {
            let scene_root = staging.join("scenes").join(&scene.id);
            for media_dir in ["images", "videos"] {
                let path = scene_root.join(media_dir);
                fs::create_dir_all(&path).map_err(|source| ProjectError::Io {
                    path: path.clone(),
                    source,
                })?;
            }
        }

        let snapshot = input_dir.join("script.vprep");
        write_new_bytes(&snapshot, raw_script.as_bytes())?;

        let metadata = ProjectMetadata {
            schema_version: PROJECT_SCHEMA_VERSION,
            project_id: project_id.to_owned(),
            title: prepared.omnivoice.title.clone(),
            input_sha256: prepared.input_sha256.clone(),
            created_unix_ms: now_millis(),
            scene_ids: prepared.scenes.iter().map(|scene| scene.id.clone()).collect(),
        };
        let status = ProjectStatus::initial(project_id, prepared);

        atomic_write_json(&staging.join("project.json"), &metadata)?;
        atomic_write_json(&staging.join("status.json"), &status)?;
        Ok(())
    }

    pub fn load(&self, project_id: &str) -> Result<StoredProject, ProjectError> {
        validate_project_id(project_id)?;
        Self::open(&self.projects_root().join(project_id))
    }

    pub fn open(project_root: impl AsRef<Path>) -> Result<StoredProject, ProjectError> {
        let project_root = project_root.as_ref().to_path_buf();
        let project_json = project_root.join("project.json");
        let status_json = project_root.join("status.json");
        let snapshot = project_root.join("input").join("script.vprep");

        if !project_json.is_file() || !status_json.is_file() || !snapshot.is_file() {
            return Err(ProjectError::ProjectMissing(project_root));
        }

        let metadata: ProjectMetadata = read_json(&project_json)?;
        validate_project_id(&metadata.project_id)?;
        if metadata.schema_version != PROJECT_SCHEMA_VERSION {
            return Err(ProjectError::MetadataMismatch(format!(
                "unsupported schema_version {}",
                metadata.schema_version
            )));
        }

        let folder_name = project_root.file_name().and_then(|name| name.to_str());
        if folder_name != Some(metadata.project_id.as_str()) {
            return Err(ProjectError::MetadataMismatch(
                "folder name does not match project_id".to_owned(),
            ));
        }

        let status: ProjectStatus = read_json(&status_json)?;
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

        let raw_script = fs::read_to_string(&snapshot).map_err(|source| ProjectError::Io {
            path: snapshot.clone(),
            source,
        })?;
        let prepared_script = parse_script(&raw_script)?;
        if prepared_script.input_sha256 != metadata.input_sha256 {
            return Err(ProjectError::InputHashMismatch {
                expected: metadata.input_sha256.clone(),
                actual: prepared_script.input_sha256.clone(),
            });
        }
        if prepared_script.omnivoice.title != metadata.title {
            return Err(ProjectError::MetadataMismatch(
                "title does not match input snapshot".to_owned(),
            ));
        }
        let scene_ids: Vec<String> = prepared_script
            .scenes
            .iter()
            .map(|scene| scene.id.clone())
            .collect();
        if scene_ids != metadata.scene_ids {
            return Err(ProjectError::MetadataMismatch(
                "scene_ids do not match input snapshot".to_owned(),
            ));
        }
        let status_scene_ids: Vec<&str> = status.scenes.iter().map(|scene| scene.id.as_str()).collect();
        let metadata_scene_ids: Vec<&str> = metadata.scene_ids.iter().map(String::as_str).collect();
        if status_scene_ids != metadata_scene_ids {
            return Err(ProjectError::StatusMismatch(
                "status scenes do not match metadata scene_ids".to_owned(),
            ));
        }

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
        if status.project_id != metadata.project_id {
            return Err(ProjectError::StatusMismatch(
                "status project_id does not match metadata".to_owned(),
            ));
        }
        let status_scene_ids: Vec<&str> = status.scenes.iter().map(|scene| scene.id.as_str()).collect();
        let metadata_scene_ids: Vec<&str> = metadata.scene_ids.iter().map(String::as_str).collect();
        if status_scene_ids != metadata_scene_ids {
            return Err(ProjectError::StatusMismatch(
                "status scenes do not match metadata scene_ids".to_owned(),
            ));
        }
        atomic_write_json(&project_root.join("status.json"), status)
    }
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
            .is_some_and(|ch| ch.is_ascii_alphanumeric())
        && project_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));

    if valid {
        Ok(())
    } else {
        Err(ProjectError::InvalidProjectId(project_id.to_owned()))
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ProjectError> {
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
    let parent = path.parent().ok_or_else(|| ProjectError::MetadataMismatch(
        "atomic write target has no parent directory".to_owned(),
    ))?;
    fs::create_dir_all(parent).map_err(|source| ProjectError::Io {
        path: parent.to_path_buf(),
        source,
    })?;

    let mut temp = NamedTempFile::new_in(parent).map_err(|source| ProjectError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    serde_json::to_writer_pretty(temp.as_file_mut(), value).map_err(|error| ProjectError::Json {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| ProjectError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    temp.as_file_mut().sync_all().map_err(|source| ProjectError::Io {
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

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = include_str!("../examples/demo.vprep");

    fn temp_data_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "video-prepare-test-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn create_persist_and_reopen_project() {
        let data_root = temp_data_root();
        let prepared = parse_script(DEMO).unwrap();
        let store = ProjectStore::new(&data_root);
        let created = store.create("demo-project", DEMO, &prepared).unwrap();

        assert_eq!(created.metadata.project_id, "demo-project");
        assert_eq!(created.metadata.input_sha256, prepared.input_sha256);
        assert_eq!(created.status.overall, TaskState::Pending);
        assert_eq!(
            fs::read_to_string(created.root.join("input/script.vprep")).unwrap(),
            DEMO
        );
        for scene in ["S01", "S02", "S03"] {
            assert!(created.root.join("scenes").join(scene).join("images").is_dir());
            assert!(created.root.join("scenes").join(scene).join("videos").is_dir());
        }

        drop(created);
        let reopened = store.load("demo-project").unwrap();
        assert_eq!(reopened.prepared_script, prepared);
        assert_eq!(reopened.metadata.scene_ids, vec!["S01", "S02", "S03"]);

        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn rejects_path_traversal_project_ids() {
        let data_root = temp_data_root();
        let prepared = parse_script(DEMO).unwrap();
        let store = ProjectStore::new(&data_root);

        for id in ["../foo", "foo/bar", "foo\\bar", ".", "..", " bad"] {
            assert!(matches!(
                store.create(id, DEMO, &prepared),
                Err(ProjectError::InvalidProjectId(_))
            ));
        }

        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn detects_modified_input_snapshot() {
        let data_root = temp_data_root();
        let prepared = parse_script(DEMO).unwrap();
        let store = ProjectStore::new(&data_root);
        let project = store.create("demo", DEMO, &prepared).unwrap();
        let snapshot = project.root.join("input/script.vprep");
        fs::write(&snapshot, DEMO.replace("Silence Is Powerful", "Silence Changed")).unwrap();

        assert!(matches!(
            ProjectStore::open(&project.root),
            Err(ProjectError::InputHashMismatch { .. })
        ));

        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn detects_missing_input_snapshot() {
        let data_root = temp_data_root();
        let prepared = parse_script(DEMO).unwrap();
        let store = ProjectStore::new(&data_root);
        let project = store.create("demo", DEMO, &prepared).unwrap();
        fs::remove_file(project.root.join("input/script.vprep")).unwrap();

        assert!(matches!(
            ProjectStore::open(&project.root),
            Err(ProjectError::ProjectMissing(_))
        ));

        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn atomic_write_keeps_old_status_if_failure_happens_before_persist() {
        let data_root = temp_data_root();
        let prepared = parse_script(DEMO).unwrap();
        let store = ProjectStore::new(&data_root);
        let project = store.create("demo", DEMO, &prepared).unwrap();
        let status_path = project.root.join("status.json");
        let before = fs::read(&status_path).unwrap();

        let mut changed = project.status.clone();
        changed.overall = TaskState::Running;
        let result = atomic_write_json_with_hook(&status_path, &changed, |_| {
            Err(std::io::Error::other("injected pre-persist failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&status_path).unwrap(), before);

        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn save_and_reopen_updated_status() {
        let data_root = temp_data_root();
        let prepared = parse_script(DEMO).unwrap();
        let store = ProjectStore::new(&data_root);
        let project = store.create("demo", DEMO, &prepared).unwrap();
        let mut status = project.status.clone();
        status.overall = TaskState::Running;
        status.visual_flow = TaskState::Running;
        status.scenes[0].visual = TaskState::Completed;

        ProjectStore::save_status(&project.root, &status).unwrap();
        let reopened = ProjectStore::open(&project.root).unwrap();
        assert_eq!(reopened.status, status);

        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn project_files_do_not_contain_runtime_secrets() {
        let data_root = temp_data_root();
        let prepared = parse_script(DEMO).unwrap();
        let store = ProjectStore::new(&data_root);
        let project = store.create("demo", DEMO, &prepared).unwrap();
        let project_json = fs::read_to_string(project.root.join("project.json")).unwrap();
        let status_json = fs::read_to_string(project.root.join("status.json")).unwrap();

        for forbidden in ["pexels_api_key", "omnivoice_token", "omnivoice_url"] {
            assert!(!project_json.contains(forbidden));
            assert!(!status_json.contains(forbidden));
        }

        fs::remove_dir_all(data_root).unwrap();
    }
}

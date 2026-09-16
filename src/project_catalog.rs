use std::{
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::{parse_script, ProjectError, ProjectStore, StoredProject, TaskState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    pub project_id: String,
    pub title: String,
    pub root: PathBuf,
    pub created_unix_ms: u64,
    pub scene_count: usize,
    pub overall: TaskState,
    pub visual_flow: TaskState,
    pub audio_flow: TaskState,
}

impl From<&StoredProject> for ProjectSummary {
    fn from(project: &StoredProject) -> Self {
        Self {
            project_id: project.metadata.project_id.clone(),
            title: project.metadata.title.clone(),
            root: project.root.clone(),
            created_unix_ms: project.metadata.created_unix_ms,
            scene_count: project.metadata.scene_ids.len(),
            overall: project.status.overall,
            visual_flow: project.status.visual_flow,
            audio_flow: project.status.audio_flow,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiscoveryError {
    pub root: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectCatalog {
    pub projects: Vec<ProjectSummary>,
    pub errors: Vec<ProjectDiscoveryError>,
}

#[derive(Debug, Error)]
pub enum ProjectCatalogError {
    #[error("cannot read projects directory {path}: {source}")]
    ReadProjectsDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot inspect entry under {path}: {source}")]
    ReadDirectoryEntry {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot inspect file type for {path}: {source}")]
    ReadFileType {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum ProjectActionError {
    #[error("cannot read script {path}: {source}")]
    ScriptRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    Project(#[from] ProjectError),
}

pub fn discover_projects(
    data_root: impl AsRef<Path>,
) -> Result<ProjectCatalog, ProjectCatalogError> {
    let projects_root = data_root.as_ref().join("projects");
    if !projects_root.exists() {
        return Ok(ProjectCatalog::default());
    }

    let entries = fs::read_dir(&projects_root).map_err(|source| {
        ProjectCatalogError::ReadProjectsDirectory {
            path: projects_root.clone(),
            source,
        }
    })?;

    let mut catalog = ProjectCatalog::default();
    for entry in entries {
        let entry = entry.map_err(|source| ProjectCatalogError::ReadDirectoryEntry {
            path: projects_root.clone(),
            source,
        })?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|source| ProjectCatalogError::ReadFileType {
                path: path.clone(),
                source,
            })?;
        if !file_type.is_dir() {
            continue;
        }

        match ProjectStore::open(&path) {
            Ok(project) => catalog.projects.push(ProjectSummary::from(&project)),
            Err(error) => catalog.errors.push(ProjectDiscoveryError {
                root: path,
                message: error.to_string(),
            }),
        }
    }

    catalog.projects.sort_by(|left, right| {
        right
            .created_unix_ms
            .cmp(&left.created_unix_ms)
            .then_with(|| left.project_id.cmp(&right.project_id))
    });
    catalog
        .errors
        .sort_by(|left, right| left.root.cmp(&right.root));
    Ok(catalog)
}

pub fn create_project_from_script_path(
    data_root: impl AsRef<Path>,
    project_id: &str,
    script_path: impl AsRef<Path>,
) -> Result<StoredProject, ProjectActionError> {
    let script_path = script_path.as_ref().to_path_buf();
    let raw =
        fs::read_to_string(&script_path).map_err(|source| ProjectActionError::ScriptRead {
            path: script_path,
            source,
        })?;
    let prepared = parse_script(&raw).map_err(ProjectError::from)?;
    ProjectStore::new(data_root.as_ref())
        .create(project_id, &raw, &prepared)
        .map_err(ProjectActionError::from)
}

pub fn open_project_from_data_root(
    data_root: impl AsRef<Path>,
    project_id: &str,
) -> Result<StoredProject, ProjectError> {
    ProjectStore::new(data_root.as_ref()).load(project_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_script, ProjectStore};

    #[test]
    fn missing_projects_directory_returns_empty_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let catalog = discover_projects(temp.path()).unwrap();
        assert!(catalog.projects.is_empty());
        assert!(catalog.errors.is_empty());
    }

    #[test]
    fn valid_projects_are_listed_and_corrupt_sibling_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let raw = include_str!("../examples/demo.vprep");
        let prepared = parse_script(raw).unwrap();
        let store = ProjectStore::new(temp.path());
        store.create("alpha", raw, &prepared).unwrap();
        store.create("beta", raw, &prepared).unwrap();

        let corrupt = store.projects_root().join("broken");
        fs::create_dir_all(&corrupt).unwrap();
        fs::write(corrupt.join("project.json"), b"{}").unwrap();

        let hidden = store.projects_root().join(".staging-leftover");
        fs::create_dir_all(&hidden).unwrap();

        let catalog = discover_projects(temp.path()).unwrap();
        assert_eq!(catalog.projects.len(), 2);
        let ids: Vec<&str> = catalog
            .projects
            .iter()
            .map(|project| project.project_id.as_str())
            .collect();
        assert!(ids.contains(&"alpha"));
        assert!(ids.contains(&"beta"));
        assert_eq!(catalog.errors.len(), 1);
        assert_eq!(catalog.errors[0].root.file_name().unwrap(), "broken");
        assert!(catalog
            .errors
            .iter()
            .all(|error| !error.root.ends_with(".staging-leftover")));
    }

    #[test]
    fn ordering_is_deterministic_for_same_timestamp_fallback() {
        let mut catalog = ProjectCatalog {
            projects: vec![
                ProjectSummary {
                    project_id: "b".to_owned(),
                    title: "B".to_owned(),
                    root: PathBuf::from("b"),
                    created_unix_ms: 100,
                    scene_count: 1,
                    overall: TaskState::Pending,
                    visual_flow: TaskState::Pending,
                    audio_flow: TaskState::Pending,
                },
                ProjectSummary {
                    project_id: "a".to_owned(),
                    title: "A".to_owned(),
                    root: PathBuf::from("a"),
                    created_unix_ms: 100,
                    scene_count: 1,
                    overall: TaskState::Pending,
                    visual_flow: TaskState::Pending,
                    audio_flow: TaskState::Pending,
                },
            ],
            errors: Vec::new(),
        };
        catalog.projects.sort_by(|left, right| {
            right
                .created_unix_ms
                .cmp(&left.created_unix_ms)
                .then_with(|| left.project_id.cmp(&right.project_id))
        });
        assert_eq!(catalog.projects[0].project_id, "a");
        assert_eq!(catalog.projects[1].project_id, "b");
    }

    #[test]
    fn create_list_and_open_round_trip_matches_desktop_actions() {
        let temp = tempfile::tempdir().unwrap();
        let script_path = temp.path().join("input.vprep");
        fs::write(&script_path, include_str!("../examples/demo.vprep")).unwrap();

        let created =
            create_project_from_script_path(temp.path(), "desktop-project", &script_path).unwrap();
        let catalog = discover_projects(temp.path()).unwrap();
        let opened = open_project_from_data_root(temp.path(), "desktop-project").unwrap();

        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(catalog.projects[0].project_id, "desktop-project");
        assert_eq!(opened.metadata, created.metadata);
        assert_eq!(opened.status, created.status);
    }

    #[test]
    fn invalid_script_create_leaves_no_project() {
        let temp = tempfile::tempdir().unwrap();
        let script_path = temp.path().join("invalid.vprep");
        fs::write(&script_path, "not a video prepare script").unwrap();

        assert!(create_project_from_script_path(temp.path(), "bad", &script_path).is_err());
        let catalog = discover_projects(temp.path()).unwrap();
        assert!(catalog.projects.is_empty());
        assert!(catalog.errors.is_empty());
    }

    #[test]
    fn switching_data_root_does_not_mutate_projects_in_previous_root() {
        let temp = tempfile::tempdir().unwrap();
        let old_root = temp.path().join("old");
        let new_root = temp.path().join("new");
        let script_path = temp.path().join("input.vprep");
        fs::write(&script_path, include_str!("../examples/demo.vprep")).unwrap();

        create_project_from_script_path(&old_root, "kept", &script_path).unwrap();
        let new_catalog = discover_projects(&new_root).unwrap();
        let old_catalog = discover_projects(&old_root).unwrap();

        assert!(new_catalog.projects.is_empty());
        assert_eq!(old_catalog.projects.len(), 1);
        assert_eq!(old_catalog.projects[0].project_id, "kept");
        assert!(old_root.join("projects/kept/project.json").is_file());
    }
}

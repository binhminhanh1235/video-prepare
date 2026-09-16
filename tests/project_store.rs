use std::{fs, path::PathBuf};

use video_prepare::project::test_atomic_status_write_failure;
use video_prepare::{parse_script, ProjectError, ProjectStore, TaskState};

const DEMO: &str = include_str!("../examples/demo.vprep");

fn temp_data_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "video-prepare-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
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
        assert!(created
            .root
            .join("scenes")
            .join(scene)
            .join("images")
            .is_dir());
        assert!(created
            .root
            .join("scenes")
            .join(scene)
            .join("videos")
            .is_dir());
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
    fs::write(
        &snapshot,
        DEMO.replace("Silence Is Powerful", "Silence Changed"),
    )
    .unwrap();

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
fn atomic_write_keeps_old_status_on_pre_persist_failure() {
    let data_root = temp_data_root();
    let prepared = parse_script(DEMO).unwrap();
    let store = ProjectStore::new(&data_root);
    let project = store.create("demo", DEMO, &prepared).unwrap();
    let status_path = project.root.join("status.json");
    let before = fs::read(&status_path).unwrap();

    let mut changed = project.status.clone();
    changed.overall = TaskState::Running;
    assert!(test_atomic_status_write_failure(&project.root, &changed).is_err());
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
fn project_state_does_not_persist_runtime_secrets() {
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

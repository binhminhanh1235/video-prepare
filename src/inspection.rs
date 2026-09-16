use crate::{
    load_audio_status, load_visual_status, AudioFlowStatus, StoredProject, TaskState,
    VisualFlowStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionProblem {
    pub area: String,
    pub scope: String,
    pub state: TaskState,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRequestInspection {
    pub id: String,
    pub state: TaskState,
    pub target_count: u32,
    pub completed_assets: usize,
    pub attempted_queries: Vec<String>,
    pub successful_query: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneInspection {
    pub id: String,
    pub start_time: String,
    pub end_time: String,
    pub visual_state: TaskState,
    pub audio_state: TaskState,
    pub visual_detail_available: bool,
    pub visual_requests: Vec<VisualRequestInspection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioAttemptInspection {
    pub attempt_id: String,
    pub server_base_url: String,
    pub remote_project_id: String,
    pub job_id: Option<String>,
    pub state: TaskState,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInspection {
    pub state: TaskState,
    pub detail_available: bool,
    pub artifact_content_download: bool,
    pub attempts: Vec<AudioAttemptInspection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInspection {
    pub project_id: String,
    pub title: String,
    pub overall: TaskState,
    pub visual_flow: TaskState,
    pub audio_flow: TaskState,
    pub scenes: Vec<SceneInspection>,
    pub audio: AudioInspection,
    pub problems: Vec<InspectionProblem>,
}

impl ProjectInspection {
    pub fn incomplete_count(&self) -> usize {
        self.problems.len()
    }

    pub fn scene(&self, scene_id: &str) -> Option<&SceneInspection> {
        self.scenes.iter().find(|scene| scene.id == scene_id)
    }
}

pub fn inspect_project(project: &StoredProject) -> ProjectInspection {
    let visual = load_visual_status(project);
    let audio = load_audio_status(&project.root, &project.metadata.project_id);

    let visual_status = visual.as_ref().ok();
    let audio_status = audio.as_ref().ok();
    let mut problems = Vec::new();

    if let Err(error) = &visual {
        problems.push(InspectionProblem {
            area: "visual".to_owned(),
            scope: "project".to_owned(),
            state: project.status.visual_flow,
            message: format!("Visual status unavailable: {error}"),
        });
    }
    if let Err(error) = &audio {
        problems.push(InspectionProblem {
            area: "audio".to_owned(),
            scope: "project".to_owned(),
            state: project.status.audio_flow,
            message: format!("Audio status unavailable: {error}"),
        });
    }

    let scenes = project
        .prepared_script
        .scenes
        .iter()
        .map(|scene| {
            let section = project
                .prepared_script
                .omnivoice
                .sections
                .iter()
                .find(|section| section.id == scene.id)
                .expect("validated scene/section mapping");
            let coarse = project
                .status
                .scenes
                .iter()
                .find(|status| status.id == scene.id)
                .expect("validated project scene status");
            let visual_scene = visual_status.and_then(|status| {
                status
                    .scenes
                    .iter()
                    .find(|candidate| candidate.id == scene.id)
            });

            let visual_requests = if let Some(visual_scene) = visual_scene {
                scene
                    .visuals
                    .iter()
                    .map(|request| {
                        let stored = visual_scene
                            .requests
                            .iter()
                            .find(|candidate| candidate.id == request.id)
                            .expect("validated visual request status");
                        if stored.state != TaskState::Completed {
                            problems.push(InspectionProblem {
                                area: "visual".to_owned(),
                                scope: format!("{}/{}", scene.id, request.id),
                                state: stored.state,
                                message: stored.last_error.clone().unwrap_or_else(|| {
                                    format!(
                                        "Visual request incomplete: {}/{} asset(s)",
                                        stored.assets.len(),
                                        request.count
                                    )
                                }),
                            });
                        }
                        VisualRequestInspection {
                            id: request.id.clone(),
                            state: stored.state,
                            target_count: request.count,
                            completed_assets: stored.assets.len(),
                            attempted_queries: stored.attempted_queries.clone(),
                            successful_query: stored.successful_query.clone(),
                            last_error: stored.last_error.clone(),
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

            SceneInspection {
                id: scene.id.clone(),
                start_time: section.start_time.clone(),
                end_time: section.end_time.clone(),
                visual_state: visual_scene
                    .map(|status| status.state)
                    .unwrap_or(coarse.visual),
                audio_state: coarse.audio,
                visual_detail_available: visual_scene.is_some(),
                visual_requests,
            }
        })
        .collect();

    let audio_inspection = match audio_status {
        Some(status) => {
            if status.state != TaskState::Completed {
                let message = status
                    .attempts
                    .last()
                    .and_then(|attempt| attempt.last_error.clone())
                    .unwrap_or_else(|| format!("Audio flow is {:?}", status.state));
                problems.push(InspectionProblem {
                    area: "audio".to_owned(),
                    scope: "flow".to_owned(),
                    state: status.state,
                    message,
                });
            }
            audio_inspection(status)
        }
        None => AudioInspection {
            state: project.status.audio_flow,
            detail_available: false,
            artifact_content_download: false,
            attempts: Vec::new(),
        },
    };

    ProjectInspection {
        project_id: project.metadata.project_id.clone(),
        title: project.metadata.title.clone(),
        overall: project.status.overall,
        visual_flow: visual_status
            .map(|status| status.state)
            .unwrap_or(project.status.visual_flow),
        audio_flow: audio_status
            .map(|status| status.state)
            .unwrap_or(project.status.audio_flow),
        scenes,
        audio: audio_inspection,
        problems,
    }
}

fn audio_inspection(status: &AudioFlowStatus) -> AudioInspection {
    AudioInspection {
        state: status.state,
        detail_available: true,
        artifact_content_download: status.artifact_content_download,
        attempts: status
            .attempts
            .iter()
            .map(|attempt| AudioAttemptInspection {
                attempt_id: attempt.attempt_id.clone(),
                server_base_url: attempt.server_base_url.clone(),
                remote_project_id: attempt.remote_project_id.clone(),
                job_id: attempt.job_id.clone(),
                state: attempt.state,
                last_error: attempt.last_error.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        audio_status_path, parse_script, visual_status_path, AudioAttemptStatus, ProjectStore,
        VisualAssetStatus, VisualRequestStatus, VisualSceneStatus, AUDIO_STATUS_SCHEMA_VERSION,
        VISUAL_STATUS_SCHEMA_VERSION,
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

    fn pending_request(id: &str) -> VisualRequestStatus {
        VisualRequestStatus {
            id: id.to_owned(),
            state: TaskState::Pending,
            attempted_queries: Vec::new(),
            successful_query: None,
            assets: Vec::new(),
            last_error: None,
        }
    }

    #[test]
    fn fresh_project_is_truthfully_pending_in_canonical_scene_order() {
        let (_temp, project) = create_demo();
        let inspection = inspect_project(&project);

        assert_eq!(inspection.visual_flow, TaskState::Pending);
        assert_eq!(inspection.audio_flow, TaskState::Pending);
        assert_eq!(
            inspection
                .scenes
                .iter()
                .map(|scene| scene.id.as_str())
                .collect::<Vec<_>>(),
            vec!["S01", "S02", "S03"]
        );
        assert_eq!(inspection.scenes[0].start_time, "0:00");
        assert_eq!(inspection.scenes[0].end_time, "0:20");
        assert_eq!(inspection.title, "Why Silence Is Powerful");
        assert!(inspection.scenes[0].visual_detail_available);
    }

    #[test]
    fn visual_partial_and_failed_request_errors_are_exposed() {
        let (_temp, project) = create_demo();
        let visual = VisualFlowStatus {
            schema_version: VISUAL_STATUS_SCHEMA_VERSION,
            project_id: project.metadata.project_id.clone(),
            state: TaskState::Partial,
            scenes: vec![
                VisualSceneStatus {
                    id: "S01".to_owned(),
                    state: TaskState::Partial,
                    requests: vec![
                        VisualRequestStatus {
                            id: "V01".to_owned(),
                            state: TaskState::Partial,
                            attempted_queries: vec!["query-a".to_owned()],
                            successful_query: Some("query-a".to_owned()),
                            assets: vec![VisualAssetStatus {
                                slot: 1,
                                kind: crate::PersistedAssetKind::Video,
                                provider: "pexels".to_owned(),
                                provider_asset_id: "42".to_owned(),
                                relative_path: "dummy.mp4".to_owned(),
                                provenance_relative_path: "dummy.json".to_owned(),
                                sha256: "abc".to_owned(),
                                bytes: 10,
                            }],
                            last_error: Some("one asset still missing".to_owned()),
                        },
                        VisualRequestStatus {
                            id: "V02".to_owned(),
                            state: TaskState::Failed,
                            attempted_queries: vec!["query-b".to_owned()],
                            successful_query: None,
                            assets: Vec::new(),
                            last_error: Some("rate limited".to_owned()),
                        },
                    ],
                },
                VisualSceneStatus {
                    id: "S02".to_owned(),
                    state: TaskState::Pending,
                    requests: vec![pending_request("V01")],
                },
                VisualSceneStatus {
                    id: "S03".to_owned(),
                    state: TaskState::Pending,
                    requests: vec![pending_request("V01")],
                },
            ],
        };
        fs::write(
            visual_status_path(&project.root),
            serde_json::to_vec_pretty(&visual).unwrap(),
        )
        .unwrap();

        let inspection = inspect_project(&project);
        let scene = inspection.scene("S01").unwrap();
        assert_eq!(scene.visual_state, TaskState::Partial);
        assert_eq!(scene.visual_requests[0].completed_assets, 1);
        assert_eq!(scene.visual_requests[0].target_count, 2);
        assert_eq!(scene.visual_requests[1].state, TaskState::Failed);
        assert!(inspection
            .problems
            .iter()
            .any(|problem| problem.scope == "S01/V01" && problem.message.contains("missing")));
        assert!(inspection
            .problems
            .iter()
            .any(|problem| problem.scope == "S01/V02" && problem.message == "rate limited"));
    }

    #[test]
    fn audio_unknown_remote_attempt_is_exposed_without_completion_guess() {
        let (_temp, project) = create_demo();
        let audio = AudioFlowStatus {
            schema_version: AUDIO_STATUS_SCHEMA_VERSION,
            project_id: project.metadata.project_id.clone(),
            state: TaskState::UnknownRemote,
            artifact_content_download: false,
            artifact_content_endpoint: None,
            attempts: vec![AudioAttemptStatus {
                attempt_id: "A0001".to_owned(),
                server_base_url: "https://voice.example".to_owned(),
                remote_project_id: "vp-demo-abc".to_owned(),
                remote_source_hash: Some("abc".to_owned()),
                job_id: None,
                idempotency_key: "redacted-test-key".to_owned(),
                state: TaskState::UnknownRemote,
                submitted_unix_ms: None,
                last_error: Some("submit timeout".to_owned()),
            }],
        };
        fs::write(
            audio_status_path(&project.root),
            serde_json::to_vec_pretty(&audio).unwrap(),
        )
        .unwrap();

        let inspection = inspect_project(&project);
        assert_eq!(inspection.audio.state, TaskState::UnknownRemote);
        assert_eq!(inspection.audio.attempts.len(), 1);
        assert_eq!(
            inspection.audio.attempts[0].server_base_url,
            "https://voice.example"
        );
        assert_eq!(inspection.audio.attempts[0].job_id, None);
        assert!(inspection
            .problems
            .iter()
            .any(|problem| problem.area == "audio" && problem.message == "submit timeout"));
    }

    #[test]
    fn corrupt_flow_state_is_reported_instead_of_fabricated_as_healthy() {
        let (_temp, project) = create_demo();
        fs::write(visual_status_path(&project.root), b"{not-json").unwrap();
        fs::write(audio_status_path(&project.root), b"{not-json").unwrap();

        let inspection = inspect_project(&project);
        assert!(!inspection.scenes[0].visual_detail_available);
        assert!(!inspection.audio.detail_available);
        assert!(inspection
            .problems
            .iter()
            .any(|problem| problem.message.starts_with("Visual status unavailable:")));
        assert!(inspection
            .problems
            .iter()
            .any(|problem| problem.message.starts_with("Audio status unavailable:")));
    }
}

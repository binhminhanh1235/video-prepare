from pathlib import Path

path = Path("src/desktop.rs")
text = path.read_text()

replacements = [
(
"""    create_project_from_script_path, discover_projects, execute_flow_retry, execute_project_run,
    inspect_project, load_audio_status, open_project_from_data_root, reconcile_remote_audio,
    test_omnivoice_connection, test_pexels_connection, ConnectionTestReport, ConnectionTestTarget,
    FlowRetryReport, FlowRunDisposition, FlowRunReport, FlowTarget, OmniVoiceClient,
    ProjectCatalog, ProjectInspection, ProjectRunReport, QualityPreset,
    RemoteAudioReconciliationReport, RunAction, RuntimeSettingsDraft, RuntimeSettingsStore,
    StoredProject,
""",
"""    create_project_from_script_path, discover_projects, execute_flow_retry, execute_project_run,
    import_manual_visual_asset, inspect_project, load_audio_status, open_project_from_data_root,
    reconcile_remote_audio, test_omnivoice_connection, test_pexels_connection,
    ConnectionTestReport, ConnectionTestTarget, FlowRetryReport, FlowRunDisposition, FlowRunReport,
    FlowTarget, ManualVisualImportSummary, OmniVoiceClient, ProjectCatalog, ProjectInspection,
    ProjectRunReport, QualityPreset, RemoteAudioReconciliationReport, RunAction,
    RuntimeSettingsDraft, RuntimeSettingsStore, StoredProject,
"""
),
(
"""struct ConnectionWorker {
    receiver: Receiver<Result<ConnectionTestReport, String>>,
    target: ConnectionTestTarget,
    applied_revision_at_start: u64,
}

pub struct VideoPrepareApp {
""",
"""struct ConnectionWorker {
    receiver: Receiver<Result<ConnectionTestReport, String>>,
    target: ConnectionTestTarget,
    applied_revision_at_start: u64,
}

struct ManualVisualWorker {
    receiver: Receiver<Result<ManualVisualImportSummary, String>>,
    project_id: String,
    data_root: PathBuf,
    scene_id: String,
    visual_id: String,
}

pub struct VideoPrepareApp {
"""
),
(
"""    connection_worker: Option<ConnectionWorker>,
    last_connection_report: Option<ConnectionTestReport>,
    connection_status: String,
    connection_status_is_error: bool,
}
""",
"""    connection_worker: Option<ConnectionWorker>,
    last_connection_report: Option<ConnectionTestReport>,
    connection_status: String,
    connection_status_is_error: bool,
    manual_visual_path: String,
    manual_visual_worker: Option<ManualVisualWorker>,
    last_manual_visual_import: Option<ManualVisualImportSummary>,
    manual_visual_status: String,
    manual_visual_status_is_error: bool,
}
"""
),
(
"""            connection_worker: None,
            last_connection_report: None,
            connection_status: String::new(),
            connection_status_is_error: false,
        };
""",
"""            connection_worker: None,
            last_connection_report: None,
            connection_status: String::new(),
            connection_status_is_error: false,
            manual_visual_path: String::new(),
            manual_visual_worker: None,
            last_manual_visual_import: None,
            manual_visual_status: String::new(),
            manual_visual_status_is_error: false,
        };
"""
),
(
"""        self.poll_remote_reconcile_worker(ctx);
        self.poll_connection_worker(ctx);
""",
"""        self.poll_remote_reconcile_worker(ctx);
        self.poll_connection_worker(ctx);
        self.poll_manual_visual_worker(ctx);
"""
),
(
"""            || self.remote_reconcile_worker.is_some()
            || self.connection_worker.is_some()
""",
"""            || self.remote_reconcile_worker.is_some()
            || self.connection_worker.is_some()
            || self.manual_visual_worker.is_some()
"""
),
(
"""        ui.add_space(10.0);
        ui.group(|ui| {
            ui.strong("Visual requests");
""",
"""        ui.add_space(10.0);
        ui.group(|ui| {
            ui.strong("Manual visual takeover");
            ui.small(
                "Choose a local image/video. Video Prepare copies it into the project, stores checksum/provenance, and does not persist the original absolute path.",
            );
            ui.horizontal(|ui| {
                ui.label("Local file");
                ui.text_edit_singleline(&mut self.manual_visual_path);
            });
            if let Some(worker) = &self.manual_visual_worker {
                ui.small(format!(
                    "Importing {} / {} for project `{}`...",
                    worker.scene_id, worker.visual_id, worker.project_id
                ));
            }
            if !self.manual_visual_status.is_empty() {
                if self.manual_visual_status_is_error {
                    ui.colored_label(ui.visuals().error_fg_color, &self.manual_visual_status);
                } else {
                    ui.label(&self.manual_visual_status);
                }
            }
            if let Some(summary) = &self.last_manual_visual_import {
                ui.small(format!(
                    "Last import: {}/{} slot {} -> {} ({:?}).",
                    summary.scene_id,
                    summary.visual_id,
                    summary.slot,
                    summary.relative_path,
                    summary.request_state
                ));
            }
        });

        ui.add_space(10.0);
        ui.group(|ui| {
            ui.strong("Visual requests");
"""
),
(
"""                    if let Some(error) = &request.last_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                }
""",
"""                    if let Some(error) = &request.last_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    if request.completed_assets < request.target_count {
                        let source_ready = !self.manual_visual_path.trim().is_empty();
                        if ui
                            .add_enabled(
                                !mutation_busy && source_ready,
                                egui::Button::new(format!("Import local file into {}", request.id)),
                            )
                            .clicked()
                        {
                            self.start_manual_visual_import(&scene.id, &request.id);
                        }
                    }
                }
"""
),
(
"""    fn mutation_worker_active(&self) -> bool {
        self.run_worker.is_some()
            || self.flow_retry_worker.is_some()
            || self.remote_reconcile_worker.is_some()
    }
""",
"""    fn mutation_worker_active(&self) -> bool {
        self.run_worker.is_some()
            || self.flow_retry_worker.is_some()
            || self.remote_reconcile_worker.is_some()
            || self.manual_visual_worker.is_some()
    }

    fn start_manual_visual_import(&mut self, scene_id: &str, visual_id: &str) {
        if self.mutation_worker_active() {
            return;
        }
        let source_path = self.manual_visual_path.trim().to_owned();
        if source_path.is_empty() {
            self.manual_visual_status = "Choose a local image/video file first.".to_owned();
            self.manual_visual_status_is_error = true;
            return;
        }
        let Some(project_id) = self
            .selected_project
            .as_ref()
            .map(|project| project.metadata.project_id.clone())
        else {
            self.manual_visual_status = "No selected project for manual import.".to_owned();
            self.manual_visual_status_is_error = true;
            return;
        };
        let data_root = self.settings.current().safe.data_root.clone();
        let worker_project_id = project_id.clone();
        let worker_data_root = data_root.clone();
        let worker_scene_id = scene_id.to_owned();
        let worker_visual_id = visual_id.to_owned();
        let thread_scene_id = worker_scene_id.clone();
        let thread_visual_id = worker_visual_id.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = (|| {
                let mut project =
                    open_project_from_data_root(&worker_data_root, &worker_project_id)
                        .map_err(|error| error.to_string())?;
                import_manual_visual_asset(
                    &mut project,
                    &thread_scene_id,
                    &thread_visual_id,
                    &source_path,
                )
                .map_err(|error| error.to_string())
            })();
            let _ = sender.send(result);
        });
        self.manual_visual_worker = Some(ManualVisualWorker {
            receiver,
            project_id: project_id.clone(),
            data_root,
            scene_id: worker_scene_id.clone(),
            visual_id: worker_visual_id.clone(),
        });
        self.last_manual_visual_import = None;
        self.manual_visual_status = format!(
            "Manual import started for {worker_scene_id}/{worker_visual_id} in `{project_id}`."
        );
        self.manual_visual_status_is_error = false;
    }

    fn poll_manual_visual_worker(&mut self, ctx: &egui::Context) {
        let outcome = match self.manual_visual_worker.as_ref() {
            None => return,
            Some(worker) => match worker.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(250));
                    None
                }
                Err(TryRecvError::Disconnected) => Some(Err(
                    "manual-visual worker disconnected before returning a result".to_owned(),
                )),
            },
        };
        let Some(outcome) = outcome else {
            return;
        };
        let worker = self.manual_visual_worker.take().expect("checked above");
        match outcome {
            Ok(summary) => {
                self.manual_visual_status = if summary.already_present {
                    format!(
                        "Manual asset already present for {}/{} slot {}.",
                        summary.scene_id, summary.visual_id, summary.slot
                    )
                } else {
                    format!(
                        "Imported manual asset for {}/{} slot {}.",
                        summary.scene_id, summary.visual_id, summary.slot
                    )
                };
                self.manual_visual_status_is_error = false;
                self.manual_visual_path.clear();
                let current_root = self.settings.current().safe.data_root.clone();
                let selected_matches = self
                    .selected_project
                    .as_ref()
                    .is_some_and(|project| project.metadata.project_id == summary.project_id);
                if current_root == worker.data_root && selected_matches {
                    self.reload_selected_project();
                    self.refresh_projects();
                } else {
                    self.manual_visual_status.push_str(
                        " Result persisted to its original Data Root; current selection/settings changed, so auto-refresh was skipped.",
                    );
                }
                self.last_manual_visual_import = Some(summary);
            }
            Err(error) => {
                self.manual_visual_status = format!(
                    "Manual import failed for {}/{} in `{}`: {error}",
                    worker.scene_id, worker.visual_id, worker.project_id
                );
                self.manual_visual_status_is_error = true;
                self.last_manual_visual_import = None;
            }
        }
    }
"""
),
]

for old, new in replacements:
    if old in text:
        text = text.replace(old, new, 1)
    elif new not in text:
        raise SystemExit("desktop patch anchor not found:\n" + old[:160])

path.write_text(text)

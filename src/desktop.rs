use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};

use eframe::egui;

use crate::{
    create_project_from_script_path, discover_projects, execute_flow_retry, execute_project_run,
    inspect_project, load_audio_status, open_project_from_data_root, reconcile_remote_audio,
    test_omnivoice_connection, test_pexels_connection, ConnectionTestReport, ConnectionTestTarget,
    FlowRetryReport, FlowRunDisposition, FlowRunReport, FlowTarget, OmniVoiceClient, ProjectCatalog,
    ProjectInspection, ProjectRunReport, QualityPreset, RemoteAudioReconciliationReport, RunAction,
    RuntimeSettingsDraft, RuntimeSettingsStore, StoredProject,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Projects,
    Dashboard,
    Scene,
    Settings,
}

struct RunWorker {
    receiver: Receiver<Result<ProjectRunReport, String>>,
    project_id: String,
    data_root: PathBuf,
    revision: u64,
    action: RunAction,
}

struct FlowRetryWorker {
    receiver: Receiver<Result<FlowRetryReport, String>>,
    project_id: String,
    data_root: PathBuf,
    revision: u64,
    target: FlowTarget,
}

struct RemoteReconcileWorker {
    receiver: Receiver<Result<RemoteAudioReconciliationReport, String>>,
    project_id: String,
    data_root: PathBuf,
    revision: u64,
}

struct ConnectionWorker {
    receiver: Receiver<Result<ConnectionTestReport, String>>,
    target: ConnectionTestTarget,
    applied_revision_at_start: u64,
}

pub struct VideoPrepareApp {
    settings: RuntimeSettingsStore,
    draft: RuntimeSettingsDraft,
    screen: Screen,
    status: String,
    status_is_error: bool,
    catalog: ProjectCatalog,
    catalog_data_root: PathBuf,
    selected_project: Option<StoredProject>,
    inspection: Option<ProjectInspection>,
    selected_scene_id: Option<String>,
    create_project_id: String,
    create_script_path: String,
    project_status: String,
    project_status_is_error: bool,
    run_worker: Option<RunWorker>,
    last_run_report: Option<ProjectRunReport>,
    run_status: String,
    run_status_is_error: bool,
    flow_retry_worker: Option<FlowRetryWorker>,
    last_flow_retry_report: Option<FlowRetryReport>,
    flow_retry_status: String,
    flow_retry_status_is_error: bool,
    remote_reconcile_worker: Option<RemoteReconcileWorker>,
    last_remote_reconcile_report: Option<RemoteAudioReconciliationReport>,
    remote_reconcile_status: String,
    remote_reconcile_status_is_error: bool,
    connection_worker: Option<ConnectionWorker>,
    last_connection_report: Option<ConnectionTestReport>,
    connection_status: String,
    connection_status_is_error: bool,
}

impl Default for VideoPrepareApp {
    fn default() -> Self {
        let settings = RuntimeSettingsStore::default();
        let draft = settings.draft();
        let catalog_data_root = settings.current().safe.data_root.clone();
        let mut app = Self {
            settings,
            draft,
            screen: Screen::Projects,
            status: "Runtime settings are memory-only until applied.".to_owned(),
            status_is_error: false,
            catalog: ProjectCatalog::default(),
            catalog_data_root,
            selected_project: None,
            inspection: None,
            selected_scene_id: None,
            create_project_id: String::new(),
            create_script_path: String::new(),
            project_status: String::new(),
            project_status_is_error: false,
            run_worker: None,
            last_run_report: None,
            run_status: String::new(),
            run_status_is_error: false,
            flow_retry_worker: None,
            last_flow_retry_report: None,
            flow_retry_status: String::new(),
            flow_retry_status_is_error: false,
            remote_reconcile_worker: None,
            last_remote_reconcile_report: None,
            remote_reconcile_status: String::new(),
            remote_reconcile_status_is_error: false,
            connection_worker: None,
            last_connection_report: None,
            connection_status: String::new(),
            connection_status_is_error: false,
        };
        app.refresh_projects();
        app
    }
}

impl eframe::App for VideoPrepareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_run_worker(ctx);
        self.poll_flow_retry_worker(ctx);
        self.poll_remote_reconcile_worker(ctx);
        self.poll_connection_worker(ctx);

        egui::TopBottomPanel::top("top-nav").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Video Prepare");
                ui.separator();
                nav_button(ui, &mut self.screen, Screen::Projects, "Projects");
                nav_button(ui, &mut self.screen, Screen::Dashboard, "Dashboard");
                nav_button(ui, &mut self.screen, Screen::Scene, "Scene");
                nav_button(ui, &mut self.screen, Screen::Settings, "Settings");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.screen {
            Screen::Projects => self.projects_ui(ui),
            Screen::Dashboard => self.dashboard_ui(ui),
            Screen::Scene => self.scene_detail_ui(ui),
            Screen::Settings => self.settings_ui(ui),
        });

        if self.run_worker.is_some()
            || self.flow_retry_worker.is_some()
            || self.remote_reconcile_worker.is_some()
            || self.connection_worker.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl VideoPrepareApp {
    fn projects_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Projects");
        ui.horizontal(|ui| {
            ui.label(format!("Data Root: {}", self.catalog_data_root.display()));
            if ui.button("Refresh").clicked() {
                self.refresh_projects();
            }
        });
        ui.add_space(8.0);

        ui.group(|ui| {
            ui.strong("Create from script");
            field(ui, "Project ID", &mut self.create_project_id);
            field(ui, "Script .vprep", &mut self.create_script_path);
            if ui.button("Create Project").clicked() {
                self.create_project_from_script();
            }
        });

        if !self.project_status.is_empty() {
            ui.add_space(6.0);
            if self.project_status_is_error {
                ui.colored_label(ui.visuals().error_fg_color, &self.project_status);
            } else {
                ui.label(&self.project_status);
            }
        }

        ui.add_space(10.0);
        if self.catalog.projects.is_empty() {
            ui.label("No valid projects found in this Data Root.");
        } else {
            ui.strong(format!("Projects ({})", self.catalog.projects.len()));
            let projects = self.catalog.projects.clone();
            for project in projects {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(&project.title);
                            ui.label(format!("id: {}", project.project_id));
                            ui.label(format!(
                                "overall: {:?} | visual: {:?} | audio: {:?}",
                                project.overall, project.visual_flow, project.audio_flow
                            ));
                            ui.small(format!("scenes: {}", project.scene_count));
                        });
                        if ui.button("Open").clicked() {
                            self.open_project(&project.project_id);
                        }
                    });
                });
                ui.add_space(4.0);
            }
        }

        if !self.catalog.errors.is_empty() {
            ui.add_space(10.0);
            ui.strong("Incomplete / unreadable project folders");
            for error in self.catalog.errors.clone() {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{}: {}", error.root.display(), error.message),
                );
            }
        }
    }

    fn dashboard_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Project dashboard");
        if self.selected_project.is_none() {
            ui.label("No project selected. Open a project from Projects first.");
            if ui.button("Go to Projects").clicked() {
                self.screen = Screen::Projects;
            }
            return;
        }

        let worker_active = self.mutation_worker_active();
        let remote_available = self.remote_reconciliation_available(true);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!worker_active, egui::Button::new("Run"))
                .clicked()
            {
                self.start_run(RunAction::Run);
            }
            if ui
                .add_enabled(!worker_active, egui::Button::new("Resume"))
                .clicked()
            {
                self.start_run(RunAction::Resume);
            }
            if ui
                .add_enabled(!worker_active, egui::Button::new("Retry Failed"))
                .clicked()
            {
                self.start_run(RunAction::RetryFailed);
            }
            if ui
                .add_enabled(
                    !worker_active && remote_available,
                    egui::Button::new("Reconcile Remote Audio"),
                )
                .clicked()
            {
                self.start_remote_audio_reconciliation(true);
            }
            if ui
                .add_enabled(!worker_active, egui::Button::new("Refresh from disk"))
                .clicked()
            {
                self.reload_selected_project();
            }
        });

        if let Some(worker) = &self.run_worker {
            ui.small(format!(
                "{} running for `{}` with settings revision {} and Data Root {}",
                worker.action.label(),
                worker.project_id,
                worker.revision,
                worker.data_root.display()
            ));
        }
        if let Some(worker) = &self.flow_retry_worker {
            ui.small(format!(
                "{} retry running for `{}` with settings revision {} and Data Root {}",
                worker.target.label(),
                worker.project_id,
                worker.revision,
                worker.data_root.display()
            ));
        }
        if let Some(worker) = &self.remote_reconcile_worker {
            ui.small(format!(
                "Remote Audio reconciliation running for `{}` with settings revision {} and Data Root {}",
                worker.project_id,
                worker.revision,
                worker.data_root.display()
            ));
        }
        if !self.run_status.is_empty() {
            if self.run_status_is_error {
                ui.colored_label(ui.visuals().error_fg_color, &self.run_status);
            } else {
                ui.label(&self.run_status);
            }
        }
        if let Some(report) = &self.last_run_report {
            ui.group(|ui| {
                ui.strong(format!(
                    "Last {} | settings revision {}",
                    report.action.label(),
                    report.settings_revision
                ));
                render_flow_report(ui, "Visual", &report.visual);
                render_flow_report(ui, "Audio", &report.audio);
            });
        }
        if !self.remote_reconcile_status.is_empty() {
            if self.remote_reconcile_status_is_error {
                ui.colored_label(ui.visuals().error_fg_color, &self.remote_reconcile_status);
            } else {
                ui.label(&self.remote_reconcile_status);
            }
        }
        if let Some(report) = &self.last_remote_reconcile_report {
            ui.group(|ui| render_remote_reconciliation_report(ui, report));
        }

        let Some(inspection) = self.inspection.clone() else {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Inspection is unavailable. Refresh the selected project.",
            );
            return;
        };

        ui.add_space(8.0);
        ui.strong(&inspection.title);
        ui.label(format!("Project ID: {}", inspection.project_id));
        if let Some(project) = &self.selected_project {
            ui.label(format!("Root: {}", project.root.display()));
        }
        ui.label(format!(
            "Overall: {:?} | Visual: {:?} | Audio: {:?}",
            inspection.overall, inspection.visual_flow, inspection.audio_flow
        ));
        ui.label(format!(
            "Incomplete / error items: {}",
            inspection.incomplete_count()
        ));

        if !inspection.problems.is_empty() {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.strong("Incomplete / errors");
                for problem in &inspection.problems {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        format!(
                            "{} [{} / {}]: {}",
                            problem.scope,
                            problem.area,
                            state_text(problem.state),
                            problem.message
                        ),
                    );
                }
            });
        }

        ui.add_space(10.0);
        ui.strong("Scenes");
        for scene in inspection.scenes {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.strong(format!(
                            "{}  {}-{}",
                            scene.id, scene.start_time, scene.end_time
                        ));
                        ui.label(format!(
                            "visual: {} | audio: {}",
                            state_text(scene.visual_state),
                            state_text(scene.audio_state)
                        ));
                        if scene.visual_detail_available {
                            let completed = scene
                                .visual_requests
                                .iter()
                                .filter(|request| request.state == crate::TaskState::Completed)
                                .count();
                            ui.small(format!(
                                "visual requests: {completed}/{} completed",
                                scene.visual_requests.len()
                            ));
                        } else {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                "visual request detail unavailable",
                            );
                        }
                    });
                    if ui.button("Inspect Scene").clicked() {
                        self.selected_scene_id = Some(scene.id.clone());
                        self.screen = Screen::Scene;
                    }
                });
            });
            ui.add_space(4.0);
        }
    }

    fn scene_detail_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scene detail");
        let Some(inspection) = self.inspection.clone() else {
            ui.label("No inspected project is selected.");
            return;
        };
        let Some(scene_id) = self.selected_scene_id.clone() else {
            ui.label("Choose Inspect Scene from the project dashboard.");
            if ui.button("Go to Dashboard").clicked() {
                self.screen = Screen::Dashboard;
            }
            return;
        };
        let Some(scene) = inspection.scene(&scene_id).cloned() else {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Scene `{scene_id}` no longer exists in the inspected project."),
            );
            return;
        };

        let mutation_busy = self.mutation_worker_active();
        ui.horizontal(|ui| {
            if ui.button("Back to Dashboard").clicked() {
                self.screen = Screen::Dashboard;
            }
            if ui
                .add_enabled(!mutation_busy, egui::Button::new("Refresh from disk"))
                .clicked()
            {
                self.reload_selected_project();
            }
        });
        ui.add_space(8.0);
        ui.strong(format!("{} - {}", inspection.title, scene.id));
        ui.label(format!(
            "Narration window: {} to {}",
            scene.start_time, scene.end_time
        ));
        ui.label(format!(
            "Visual: {} | Audio: {}",
            state_text(scene.visual_state),
            state_text(scene.audio_state)
        ));

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Flow retry");
            ui.small(
                "These are project-level flow retries triggered from Scene detail. They do not imply targeted per-scene execution.",
            );
            ui.horizontal(|ui| {
                let visual_retryable = retryable_flow_state(inspection.visual_flow);
                if ui
                    .add_enabled(
                        !mutation_busy && visual_retryable,
                        egui::Button::new("Retry Visual Flow"),
                    )
                    .clicked()
                {
                    self.start_flow_retry(FlowTarget::Visual);
                }
                let audio_retryable = retryable_flow_state(inspection.audio_flow);
                if ui
                    .add_enabled(
                        !mutation_busy && audio_retryable,
                        egui::Button::new("Retry Audio Flow"),
                    )
                    .clicked()
                {
                    self.start_flow_retry(FlowTarget::Audio);
                }
            });
            if !retryable_flow_state(inspection.visual_flow) {
                ui.small(format!(
                    "Visual Flow is {}, so Retry Visual Flow is not applicable.",
                    state_text(inspection.visual_flow)
                ));
            }
            if !retryable_flow_state(inspection.audio_flow) {
                ui.small(format!(
                    "Audio Flow is {}, so Retry Audio Flow is not applicable.",
                    state_text(inspection.audio_flow)
                ));
            }
            if let Some(worker) = &self.flow_retry_worker {
                ui.small(format!(
                    "{} retry running with settings revision {}.",
                    worker.target.label(),
                    worker.revision
                ));
            }
            if !self.flow_retry_status.is_empty() {
                if self.flow_retry_status_is_error {
                    ui.colored_label(ui.visuals().error_fg_color, &self.flow_retry_status);
                } else {
                    ui.label(&self.flow_retry_status);
                }
            }
            if let Some(report) = &self.last_flow_retry_report {
                render_flow_report(ui, report.target.label(), &report.flow);
            }
        });

        ui.add_space(10.0);
        ui.group(|ui| {
            ui.strong("Visual requests");
            if !scene.visual_detail_available {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "Visual status detail could not be loaded. See Dashboard errors.",
                );
            } else {
                for request in &scene.visual_requests {
                    ui.separator();
                    ui.label(format!(
                        "{}: {} | assets {}/{}",
                        request.id,
                        state_text(request.state),
                        request.completed_assets,
                        request.target_count
                    ));
                    if !request.attempted_queries.is_empty() {
                        ui.small(format!(
                            "attempted queries: {}",
                            request.attempted_queries.join(" | ")
                        ));
                    }
                    if let Some(query) = &request.successful_query {
                        ui.small(format!("successful query: {query}"));
                    }
                    if let Some(error) = &request.last_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                }
            }
        });

        ui.add_space(10.0);
        ui.group(|ui| {
            ui.strong("Audio flow / remote attempts");
            ui.label(format!("Flow: {}", state_text(inspection.audio.state)));
            if !inspection.audio.detail_available {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "Audio status detail could not be loaded. See Dashboard errors.",
                );
            } else if inspection.audio.attempts.is_empty() {
                ui.label("No remote audio attempt has been submitted yet.");
            } else {
                for attempt in &inspection.audio.attempts {
                    ui.separator();
                    ui.label(format!(
                        "{}: {}",
                        attempt.attempt_id,
                        state_text(attempt.state)
                    ));
                    ui.small(format!("server: {}", attempt.server_base_url));
                    ui.small(format!("remote project: {}", attempt.remote_project_id));
                    ui.small(format!(
                        "job: {}",
                        attempt.job_id.as_deref().unwrap_or("not assigned")
                    ));
                    if let Some(error) = &attempt.last_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                }
            }
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Runtime Settings");
        ui.label(format!(
            "Applied revision: {}",
            self.settings.current().revision
        ));
        ui.add_space(8.0);

        let connection_busy = self.connection_worker.is_some();

        ui.group(|ui| {
            ui.strong("Project");
            ui.horizontal(|ui| {
                ui.label("Data Root");
                ui.text_edit_singleline(&mut self.draft.data_root);
            });
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Flows");
            ui.checkbox(&mut self.draft.visual_flow_enabled, "Visual Flow");
            ui.checkbox(&mut self.draft.audio_flow_enabled, "Audio Flow");
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Visual");
            ui.horizontal(|ui| {
                ui.label("Pexels API Key");
                ui.add(egui::TextEdit::singleline(&mut self.draft.pexels_api_key).password(true));
            });
            ui.horizontal(|ui| {
                ui.label("Download concurrency");
                ui.add(egui::DragValue::new(&mut self.draft.download_concurrency).range(1..=32));
            });
            if ui
                .add_enabled(!connection_busy, egui::Button::new("Test Pexels"))
                .clicked()
            {
                self.start_connection_test(ConnectionTestTarget::Pexels);
            }
            ui.small("The test uses a copy of the current draft. Secrets remain memory-only.");
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.strong("Audio");
            field(ui, "OmniVoice URL", &mut self.draft.omnivoice_url);
            ui.horizontal(|ui| {
                ui.label("API Token");
                ui.add(egui::TextEdit::singleline(&mut self.draft.omnivoice_token).password(true));
            });
            field(ui, "Voice", &mut self.draft.voice_name);
            field(ui, "Variant", &mut self.draft.voice_variant);
            field(ui, "Language", &mut self.draft.language);
            ui.horizontal(|ui| {
                ui.label("Quality");
                egui::ComboBox::from_id_salt("quality-preset")
                    .selected_text(self.draft.quality_preset.as_str())
                    .show_ui(ui, |ui| {
                        for preset in QualityPreset::ALL {
                            ui.selectable_value(
                                &mut self.draft.quality_preset,
                                preset,
                                preset.as_str(),
                            );
                        }
                    });
            });
            ui.checkbox(&mut self.draft.read_section_titles, "Read section titles");
            if ui
                .add_enabled(!connection_busy, egui::Button::new("Test OmniVoice"))
                .clicked()
            {
                self.start_connection_test(ConnectionTestTarget::OmniVoice);
            }
        });

        if let Some(worker) = &self.connection_worker {
            ui.add_space(8.0);
            ui.small(format!(
                "Testing {} with a captured draft. Applied revision at start: {}.",
                worker.target.label(),
                worker.applied_revision_at_start
            ));
        }
        if !self.connection_status.is_empty() {
            ui.add_space(6.0);
            if self.connection_status_is_error {
                ui.colored_label(ui.visuals().error_fg_color, &self.connection_status);
            } else {
                ui.label(&self.connection_status);
            }
        }
        if let Some(report) = &self.last_connection_report {
            ui.add_space(6.0);
            ui.group(|ui| render_connection_report(ui, report));
        }

        ui.add_space(10.0);
        let mut refresh_catalog = false;
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                self.draft = self.settings.draft();
                self.status = "Draft restored from the last applied runtime snapshot.".to_owned();
                self.status_is_error = false;
            }
            if ui.button("Apply").clicked() {
                let previous_root = self.settings.current().safe.data_root.clone();
                match self.settings.apply(&self.draft) {
                    Ok(snapshot) => {
                        self.draft = RuntimeSettingsDraft::from_snapshot(&snapshot);
                        self.status =
                            format!("Applied runtime settings revision {}.", snapshot.revision);
                        self.status_is_error = false;
                        refresh_catalog = previous_root != snapshot.safe.data_root;
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.status_is_error = true;
                    }
                }
            }
        });
        if refresh_catalog {
            self.selected_project = None;
            self.inspection = None;
            self.selected_scene_id = None;
            self.last_run_report = None;
            self.last_flow_retry_report = None;
            self.last_remote_reconcile_report = None;
            self.remote_reconcile_status.clear();
            self.remote_reconcile_status_is_error = false;
            self.refresh_projects();
        }

        ui.add_space(8.0);
        if self.status_is_error {
            ui.colored_label(ui.visuals().error_fg_color, &self.status);
        } else {
            ui.label(&self.status);
        }
    }

    fn refresh_projects(&mut self) {
        let data_root = self.settings.current().safe.data_root.clone();
        self.catalog_data_root = data_root.clone();
        match discover_projects(&data_root) {
            Ok(catalog) => {
                self.catalog = catalog;
                self.project_status = format!(
                    "Loaded {} project(s), {} discovery error(s).",
                    self.catalog.projects.len(),
                    self.catalog.errors.len()
                );
                self.project_status_is_error = false;
            }
            Err(error) => {
                self.catalog = ProjectCatalog::default();
                self.project_status = error.to_string();
                self.project_status_is_error = true;
            }
        }
    }

    fn create_project_from_script(&mut self) {
        let project_id = self.create_project_id.trim().to_owned();
        let script_path = self.create_script_path.trim().to_owned();
        let data_root = self.settings.current().safe.data_root.clone();
        match create_project_from_script_path(&data_root, &project_id, &script_path) {
            Ok(project) => {
                self.project_status = format!("Created project `{}`.", project.metadata.project_id);
                self.project_status_is_error = false;
                self.create_project_id.clear();
                self.create_script_path.clear();
                self.select_project(project);
                self.refresh_projects();
                self.screen = Screen::Dashboard;
            }
            Err(error) => {
                self.project_status = format!("Project create failed: {error}");
                self.project_status_is_error = true;
            }
        }
    }

    fn open_project(&mut self, project_id: &str) {
        let data_root = self.settings.current().safe.data_root.clone();
        match open_project_from_data_root(&data_root, project_id) {
            Ok(project) => {
                self.project_status = format!("Opened project `{project_id}`.");
                self.project_status_is_error = false;
                self.select_project(project);
                self.screen = Screen::Dashboard;
                self.start_remote_audio_reconciliation(false);
            }
            Err(error) => {
                self.project_status = format!("Project open failed: {error}");
                self.project_status_is_error = true;
                self.refresh_projects();
            }
        }
    }

    fn select_project(&mut self, project: StoredProject) {
        self.inspection = Some(inspect_project(&project));
        self.selected_scene_id = None;
        self.last_run_report = None;
        self.run_status.clear();
        self.run_status_is_error = false;
        self.last_flow_retry_report = None;
        self.flow_retry_status.clear();
        self.flow_retry_status_is_error = false;
        self.last_remote_reconcile_report = None;
        self.remote_reconcile_status.clear();
        self.remote_reconcile_status_is_error = false;
        self.selected_project = Some(project);
    }

    fn reload_selected_project(&mut self) {
        let Some(project_id) = self
            .selected_project
            .as_ref()
            .map(|project| project.metadata.project_id.clone())
        else {
            self.project_status = "No selected project to refresh.".to_owned();
            self.project_status_is_error = true;
            return;
        };
        let data_root = self.settings.current().safe.data_root.clone();
        match open_project_from_data_root(&data_root, &project_id) {
            Ok(project) => {
                self.inspection = Some(inspect_project(&project));
                self.selected_project = Some(project);
                self.project_status = format!("Refreshed project `{project_id}` from disk.");
                self.project_status_is_error = false;
            }
            Err(error) => {
                self.project_status = format!("Project refresh failed: {error}");
                self.project_status_is_error = true;
                self.inspection = None;
            }
        }
    }

    fn mutation_worker_active(&self) -> bool {
        self.run_worker.is_some()
            || self.flow_retry_worker.is_some()
            || self.remote_reconcile_worker.is_some()
    }

    fn remote_reconciliation_available(&self, manual: bool) -> bool {
        if self.mutation_worker_active() {
            return false;
        }
        let snapshot = self.settings.current();
        if !snapshot.safe.audio_flow_enabled || snapshot.safe.omnivoice_url.trim().is_empty() {
            return false;
        }
        let Some(project) = &self.selected_project else {
            return false;
        };
        let Ok(status) = load_audio_status(&project.root, &project.metadata.project_id) else {
            return false;
        };
        if status.attempts.is_empty() {
            return false;
        }
        manual
            || matches!(
                status.state,
                crate::TaskState::Running
                    | crate::TaskState::UnknownRemote
                    | crate::TaskState::Partial
            )
    }

    fn start_remote_audio_reconciliation(&mut self, manual: bool) {
        if !self.remote_reconciliation_available(manual) {
            if manual {
                self.remote_reconcile_status =
                    "Remote Audio reconciliation is not available. Enable Audio Flow, apply a valid OmniVoice URL, and ensure a remote attempt exists."
                        .to_owned();
                self.remote_reconcile_status_is_error = true;
            }
            return;
        }
        let Some(project_id) = self
            .selected_project
            .as_ref()
            .map(|project| project.metadata.project_id.clone())
        else {
            return;
        };
        let snapshot = self.settings.current();
        let data_root = snapshot.safe.data_root.clone();
        let revision = snapshot.revision;
        let base_url = snapshot.safe.omnivoice_url.clone();
        let token = snapshot.secrets.omnivoice_token.clone();
        let worker_project_id = project_id.clone();
        let worker_data_root = data_root.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = (|| {
                let client = OmniVoiceClient::new(base_url, token).map_err(|error| error.to_string())?;
                let mut project = open_project_from_data_root(&worker_data_root, &worker_project_id)
                    .map_err(|error| error.to_string())?;
                reconcile_remote_audio(&client, &mut project).map_err(|error| error.to_string())
            })();
            let _ = sender.send(result);
        });

        self.remote_reconcile_worker = Some(RemoteReconcileWorker {
            receiver,
            project_id: project_id.clone(),
            data_root,
            revision,
        });
        self.last_remote_reconcile_report = None;
        self.remote_reconcile_status = format!(
            "Remote Audio reconciliation started for `{project_id}` with runtime settings revision {revision}."
        );
        self.remote_reconcile_status_is_error = false;
    }

    fn poll_remote_reconcile_worker(&mut self, ctx: &egui::Context) {
        let outcome = match self.remote_reconcile_worker.as_ref() {
            None => return,
            Some(worker) => match worker.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(250));
                    None
                }
                Err(TryRecvError::Disconnected) => Some(Err(
                    "remote-reconciliation worker disconnected before returning a result"
                        .to_owned(),
                )),
            },
        };
        let Some(outcome) = outcome else {
            return;
        };

        let worker = self.remote_reconcile_worker.take().expect("checked above");
        match outcome {
            Ok(report) => {
                self.remote_reconcile_status = format!(
                    "Remote Audio reconciliation finished for `{}`: {} -> {} ({:?}).",
                    report.project_id,
                    state_text(report.previous_state),
                    state_text(report.state),
                    report.disposition
                );
                self.remote_reconcile_status_is_error = false;

                let current_root = self.settings.current().safe.data_root.clone();
                let selected_matches = self
                    .selected_project
                    .as_ref()
                    .is_some_and(|project| project.metadata.project_id == report.project_id);
                if current_root == worker.data_root && selected_matches {
                    self.reload_selected_project();
                    self.refresh_projects();
                } else {
                    self.remote_reconcile_status.push_str(
                        " Result persisted to its original Data Root; current selection/settings changed, so auto-refresh was skipped.",
                    );
                }
                self.last_remote_reconcile_report = Some(report);
            }
            Err(error) => {
                self.remote_reconcile_status = format!(
                    "Remote Audio reconciliation failed for `{}` at settings revision {}: {error}",
                    worker.project_id, worker.revision
                );
                self.remote_reconcile_status_is_error = true;
                self.last_remote_reconcile_report = None;
            }
        }
    }

    fn start_run(&mut self, action: RunAction) {
        if self.mutation_worker_active() {
            return;
        }
        let Some(project_id) = self
            .selected_project
            .as_ref()
            .map(|project| project.metadata.project_id.clone())
        else {
            self.run_status = "No selected project to run.".to_owned();
            self.run_status_is_error = true;
            return;
        };

        let snapshot = self.settings.current();
        let data_root = snapshot.safe.data_root.clone();
        let revision = snapshot.revision;
        let worker_project_id = project_id.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = execute_project_run(snapshot, &worker_project_id, action)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });

        self.run_worker = Some(RunWorker {
            receiver,
            project_id: project_id.clone(),
            data_root,
            revision,
            action,
        });
        self.last_run_report = None;
        self.run_status = format!(
            "{} started for `{project_id}` with runtime settings revision {revision}.",
            action.label()
        );
        self.run_status_is_error = false;
    }

    fn poll_run_worker(&mut self, ctx: &egui::Context) {
        let outcome = match self.run_worker.as_ref() {
            None => return,
            Some(worker) => match worker.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(250));
                    None
                }
                Err(TryRecvError::Disconnected) => Some(Err(
                    "run worker disconnected before returning a result".to_owned(),
                )),
            },
        };
        let Some(outcome) = outcome else {
            return;
        };

        self.run_worker = None;
        match outcome {
            Ok(report) => {
                self.run_status = format!(
                    "{} finished for `{}` using settings revision {}.",
                    report.action.label(),
                    report.project_id,
                    report.settings_revision
                );
                self.run_status_is_error = false;

                let current_root = self.settings.current().safe.data_root.clone();
                let selected_matches = self
                    .selected_project
                    .as_ref()
                    .is_some_and(|project| project.metadata.project_id == report.project_id);
                if current_root == report.data_root && selected_matches {
                    self.reload_selected_project();
                    self.refresh_projects();
                } else {
                    self.run_status.push_str(
                        " Result persisted to its original Data Root; current selection/settings changed, so auto-refresh was skipped.",
                    );
                }
                self.last_run_report = Some(report);
            }
            Err(error) => {
                self.run_status = format!("Project run failed before flow execution: {error}");
                self.run_status_is_error = true;
            }
        }
    }

    fn start_flow_retry(&mut self, target: FlowTarget) {
        if self.mutation_worker_active() {
            return;
        }
        let Some(project_id) = self
            .selected_project
            .as_ref()
            .map(|project| project.metadata.project_id.clone())
        else {
            self.flow_retry_status = "No selected project to retry.".to_owned();
            self.flow_retry_status_is_error = true;
            return;
        };

        let snapshot = self.settings.current();
        let data_root = snapshot.safe.data_root.clone();
        let revision = snapshot.revision;
        let worker_project_id = project_id.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = execute_flow_retry(snapshot, &worker_project_id, target)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });

        self.flow_retry_worker = Some(FlowRetryWorker {
            receiver,
            project_id: project_id.clone(),
            data_root,
            revision,
            target,
        });
        self.last_flow_retry_report = None;
        self.flow_retry_status = format!(
            "{} retry started for `{project_id}` with runtime settings revision {revision}.",
            target.label()
        );
        self.flow_retry_status_is_error = false;
    }

    fn poll_flow_retry_worker(&mut self, ctx: &egui::Context) {
        let outcome = match self.flow_retry_worker.as_ref() {
            None => return,
            Some(worker) => match worker.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(250));
                    None
                }
                Err(TryRecvError::Disconnected) => Some(Err(
                    "flow-retry worker disconnected before returning a result".to_owned(),
                )),
            },
        };
        let Some(outcome) = outcome else {
            return;
        };

        self.flow_retry_worker = None;
        match outcome {
            Ok(report) => {
                self.flow_retry_status = format!(
                    "{} retry finished for `{}` using settings revision {}.",
                    report.target.label(),
                    report.project_id,
                    report.settings_revision
                );
                self.flow_retry_status_is_error = false;

                let current_root = self.settings.current().safe.data_root.clone();
                let selected_matches = self
                    .selected_project
                    .as_ref()
                    .is_some_and(|project| project.metadata.project_id == report.project_id);
                if current_root == report.data_root && selected_matches {
                    self.reload_selected_project();
                    self.refresh_projects();
                } else {
                    self.flow_retry_status.push_str(
                        " Result persisted to its original Data Root; current selection/settings changed, so auto-refresh was skipped.",
                    );
                }
                self.last_flow_retry_report = Some(report);
            }
            Err(error) => {
                self.flow_retry_status =
                    format!("Flow retry failed before flow execution: {error}");
                self.flow_retry_status_is_error = true;
            }
        }
    }

    fn start_connection_test(&mut self, target: ConnectionTestTarget) {
        if self.connection_worker.is_some() {
            return;
        }

        let applied_revision_at_start = self.settings.current().revision;
        let (sender, receiver) = mpsc::channel();
        match target {
            ConnectionTestTarget::Pexels => {
                let api_key = self.draft.pexels_api_key.clone();
                thread::spawn(move || {
                    let result =
                        test_pexels_connection(&api_key).map_err(|error| error.to_string());
                    let _ = sender.send(result);
                });
            }
            ConnectionTestTarget::OmniVoice => {
                let base_url = self.draft.omnivoice_url.clone();
                let token = Some(self.draft.omnivoice_token.clone());
                thread::spawn(move || {
                    let result = test_omnivoice_connection(&base_url, token)
                        .map_err(|error| error.to_string());
                    let _ = sender.send(result);
                });
            }
        }

        self.connection_worker = Some(ConnectionWorker {
            receiver,
            target,
            applied_revision_at_start,
        });
        self.last_connection_report = None;
        self.connection_status = format!(
            "{} connection test started using a captured copy of the current draft.",
            target.label()
        );
        self.connection_status_is_error = false;
    }

    fn poll_connection_worker(&mut self, ctx: &egui::Context) {
        let outcome = match self.connection_worker.as_ref() {
            None => return,
            Some(worker) => match worker.receiver.try_recv() {
                Ok(result) => Some((worker.target, worker.applied_revision_at_start, result)),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(250));
                    None
                }
                Err(TryRecvError::Disconnected) => Some((
                    worker.target,
                    worker.applied_revision_at_start,
                    Err("connection-test worker disconnected before returning a result".to_owned()),
                )),
            },
        };
        let Some((target, applied_revision_at_start, outcome)) = outcome else {
            return;
        };

        self.connection_worker = None;
        match outcome {
            Ok(report) => {
                self.connection_status = format!(
                    "{} connection test passed. It used the captured draft; applied revision at start was {}.",
                    target.label(),
                    applied_revision_at_start
                );
                self.connection_status_is_error = false;
                self.last_connection_report = Some(report);
            }
            Err(error) => {
                self.connection_status =
                    format!("{} connection test failed: {error}", target.label());
                self.connection_status_is_error = true;
                self.last_connection_report = None;
            }
        }
    }
}

pub fn run_desktop() -> eframe::Result<()> {
    eframe::run_native(
        "Video Prepare",
        eframe::NativeOptions::default(),
        Box::new(|_creation_context| Ok(Box::new(VideoPrepareApp::default()))),
    )
}

fn nav_button(ui: &mut egui::Ui, screen: &mut Screen, target: Screen, label: &str) {
    if ui.selectable_label(*screen == target, label).clicked() {
        *screen = target;
    }
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value);
    });
}

fn state_text(state: crate::TaskState) -> &'static str {
    match state {
        crate::TaskState::Pending => "PENDING",
        crate::TaskState::Running => "RUNNING",
        crate::TaskState::Partial => "PARTIAL",
        crate::TaskState::Completed => "COMPLETED",
        crate::TaskState::Failed => "FAILED",
        crate::TaskState::Skipped => "SKIPPED",
        crate::TaskState::Interrupted => "INTERRUPTED",
        crate::TaskState::UnknownRemote => "UNKNOWN_REMOTE",
    }
}

fn retryable_flow_state(state: crate::TaskState) -> bool {
    matches!(
        state,
        crate::TaskState::Failed
            | crate::TaskState::Partial
            | crate::TaskState::Interrupted
            | crate::TaskState::UnknownRemote
    )
}

fn render_flow_report(ui: &mut egui::Ui, label: &str, report: &FlowRunReport) {
    ui.label(format!(
        "{label}: {}{}",
        flow_disposition_text(report.disposition),
        report
            .state
            .map(|state| format!(" / {}", state_text(state)))
            .unwrap_or_default()
    ));
    if let Some(message) = &report.message {
        if matches!(report.disposition, FlowRunDisposition::Blocked) {
            ui.colored_label(ui.visuals().error_fg_color, message);
        } else {
            ui.small(message);
        }
    }
}

fn flow_disposition_text(disposition: FlowRunDisposition) -> &'static str {
    match disposition {
        FlowRunDisposition::Executed => "EXECUTED",
        FlowRunDisposition::SkippedByPolicy => "SKIPPED_BY_POLICY",
        FlowRunDisposition::SkippedByAction => "SKIPPED_BY_ACTION",
        FlowRunDisposition::Blocked => "BLOCKED",
    }
}

fn render_remote_reconciliation_report(
    ui: &mut egui::Ui,
    report: &RemoteAudioReconciliationReport,
) {
    ui.strong("Remote Audio reconciliation");
    ui.label(format!(
        "{} -> {} | {:?}",
        state_text(report.previous_state),
        state_text(report.state),
        report.disposition
    ));
    ui.small(format!("Current server: {}", report.current_server));
    if let Some(server) = &report.attempt_server {
        ui.small(format!("Attempt server: {server}"));
    }
    if let Some(attempt_id) = &report.attempt_id {
        ui.small(format!("Attempt: {attempt_id}"));
    }
    ui.small(format!(
        "Remote queried: {} | local artifact verified/synced: {}",
        report.queried_remote, report.artifact_synced
    ));
    if let Some(message) = &report.message {
        ui.small(message);
    }
}

fn render_connection_report(ui: &mut egui::Ui, report: &ConnectionTestReport) {
    match report {
        ConnectionTestReport::Pexels(report) => {
            ui.strong("Pexels connection");
            ui.label(format!("Provider: {}", report.provider));
            ui.small(format!(
                "Rate limit: limit={} remaining={} reset_unix={}",
                optional_u64(report.rate_limit.limit),
                optional_u64(report.rate_limit.remaining),
                optional_u64(report.rate_limit.reset_unix)
            ));
        }
        ConnectionTestReport::OmniVoice(report) => {
            ui.strong("OmniVoice connection");
            ui.label(format!("Base URL: {}", report.base_url));
            ui.small(format!(
                "Service: {}",
                report.service.as_deref().unwrap_or("not advertised")
            ));
            ui.small(format!(
                "Project import: {}",
                report.project_import_endpoint
            ));
            ui.small(format!(
                "Generate project: {}",
                report.generate_project_endpoint
            ));
            ui.small(format!(
                "Jobs: {}",
                report.jobs_endpoint.as_deref().unwrap_or("not advertised")
            ));
            ui.small(format!(
                "Artifact content download: {}",
                if report.artifact_content_download {
                    "supported"
                } else {
                    "not advertised"
                }
            ));
        }
    }
}

fn optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

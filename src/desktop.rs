use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};

use eframe::egui;

use crate::{
    create_project_from_script_path, create_project_from_script_text, discover_projects,
    execute_flow_retry, execute_project_run, extract_omnivoice_url, import_manual_visual_asset,
    inspect_project, load_audio_status, open_project_from_data_root, reconcile_remote_audio,
    test_omnivoice_connection, test_pexels_connection, ConnectionTestReport, ConnectionTestTarget,
    FlowRetryReport, FlowRunDisposition, FlowRunReport, FlowTarget, ManualVisualImportSummary,
    OmniVoiceClient, ProjectCatalog, ProjectInspection, ProjectRunReport, QualityPreset,
    RemoteAudioReconciliationReport, RunAction, RuntimeSettingsDraft, RuntimeSettingsStore,
    StoredProject,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Projects,
    Dashboard,
    Scene,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectFilter {
    All,
    InProgress,
    NeedsAttention,
    Ready,
}

impl ProjectFilter {
    const ALL: [Self; 4] = [
        Self::All,
        Self::InProgress,
        Self::NeedsAttention,
        Self::Ready,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::InProgress => "In progress",
            Self::NeedsAttention => "Needs attention",
            Self::Ready => "Ready",
        }
    }

    fn matches(self, project: &crate::ProjectSummary) -> bool {
        match self {
            Self::All => true,
            Self::InProgress => project.overall != crate::TaskState::Completed,
            Self::NeedsAttention => project.attention_count > 0,
            Self::Ready => project.overall == crate::TaskState::Completed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceAction {
    Start,
    Resume,
    RetryFailed,
    CheckAudio,
    Ready,
}

impl WorkspaceAction {
    fn label(self) -> &'static str {
        match self {
            Self::Start => "Start preparation",
            Self::Resume => "Continue preparation",
            Self::RetryFailed => "Retry failed items",
            Self::CheckAudio => "Check audio status",
            Self::Ready => "Project ready ✓",
        }
    }
}

fn recommended_workspace_action(
    overall: crate::TaskState,
    audio_flow: crate::TaskState,
    remote_available: bool,
) -> WorkspaceAction {
    if remote_available
        && matches!(
            audio_flow,
            crate::TaskState::Running | crate::TaskState::UnknownRemote
        )
    {
        WorkspaceAction::CheckAudio
    } else if overall == crate::TaskState::Completed {
        WorkspaceAction::Ready
    } else if overall == crate::TaskState::Failed {
        WorkspaceAction::RetryFailed
    } else if matches!(
        overall,
        crate::TaskState::Partial | crate::TaskState::Interrupted | crate::TaskState::Running
    ) {
        WorkspaceAction::Resume
    } else {
        WorkspaceAction::Start
    }
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
    captured_draft: RuntimeSettingsDraft,
}

struct ManualVisualWorker {
    receiver: Receiver<Result<ManualVisualImportSummary, String>>,
    project_id: String,
    data_root: PathBuf,
    scene_id: String,
    visual_id: String,
}

enum ThumbnailCacheEntry {
    Ready {
        texture: egui::TextureHandle,
        source_width: u32,
        source_height: u32,
    },
    Failed(String),
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
    create_script_text: String,
    create_script_path: String,
    show_create_project: bool,
    project_search: String,
    project_filter: ProjectFilter,
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
    show_omnivoice_quick_connect: bool,
    omnivoice_quick_input: String,
    omnivoice_quick_status: String,
    omnivoice_quick_status_is_error: bool,
    omnivoice_quick_apply_pending: bool,
    manual_visual_path: String,
    manual_visual_worker: Option<ManualVisualWorker>,
    last_manual_visual_import: Option<ManualVisualImportSummary>,
    manual_visual_status: String,
    manual_visual_status_is_error: bool,
    asset_action_status: String,
    asset_action_status_is_error: bool,
    workspace_action_status: String,
    workspace_action_status_is_error: bool,
    thumbnail_cache: HashMap<PathBuf, ThumbnailCacheEntry>,
}

impl Default for VideoPrepareApp {
    fn default() -> Self {
        let (settings, load_warning) = RuntimeSettingsStore::load_persistent_or_default();
        let draft = settings.draft();
        let catalog_data_root = settings.current().safe.data_root.clone();
        let (status, status_is_error) = match load_warning {
            Some(error) => (
                format!(
                    "Could not load saved preferences: {error}. Defaults are active; Apply settings will try to repair the saved file."
                ),
                true,
            ),
            None => match settings.persistence_path() {
                Some(path) if settings.loaded_from_disk() => {
                    let mut message = format!(
                        "Loaded saved settings revision {} from {}.",
                        settings.current().revision,
                        path.display()
                    );
                    if settings.secrets_restored() {
                        message.push_str(" Secure API credentials were restored from the OS credential store.");
                    }
                    if let Some(warning) = settings.secret_persistence_warning() {
                        message.push_str(&format!(" {warning}"));
                    }
                    (message, false)
                }
                Some(path) => (
                    format!(
                        "No saved settings file exists yet. Apply settings will create {}.",
                        path.display()
                    ),
                    false,
                ),
                None => (
                    "Settings persistence is unavailable in this environment.".to_owned(),
                    true,
                ),
            },
        };
        let mut app = Self {
            settings,
            draft,
            screen: Screen::Projects,
            status,
            status_is_error,
            catalog: ProjectCatalog::default(),
            catalog_data_root,
            selected_project: None,
            inspection: None,
            selected_scene_id: None,
            create_project_id: String::new(),
            create_script_text: String::new(),
            create_script_path: String::new(),
            show_create_project: false,
            project_search: String::new(),
            project_filter: ProjectFilter::All,
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
            show_omnivoice_quick_connect: false,
            omnivoice_quick_input: String::new(),
            omnivoice_quick_status: String::new(),
            omnivoice_quick_status_is_error: false,
            omnivoice_quick_apply_pending: false,
            manual_visual_path: String::new(),
            manual_visual_worker: None,
            last_manual_visual_import: None,
            manual_visual_status: String::new(),
            manual_visual_status_is_error: false,
            asset_action_status: String::new(),
            asset_action_status_is_error: false,
            workspace_action_status: String::new(),
            workspace_action_status_is_error: false,
            thumbnail_cache: HashMap::new(),
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
        self.poll_manual_visual_worker(ctx);
        self.handle_dropped_vprep(ctx);

        egui::TopBottomPanel::top("top-nav").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("Video Prepare").size(22.0));
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                nav_button(ui, &mut self.screen, Screen::Projects, "Projects");
                if self.selected_project.is_some() {
                    nav_button(ui, &mut self.screen, Screen::Dashboard, "Workspace");
                }
                nav_button(ui, &mut self.screen, Screen::Settings, "Settings");
            });
            ui.add_space(6.0);
        });

        if self.mutation_worker_active() {
            egui::TopBottomPanel::bottom("active-work").show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spinner();
                    ui.strong("Preparation in progress");
                    if let Some(worker) = &self.run_worker {
                        ui.label(format!("{} · {}", worker.project_id, worker.action.label()));
                    } else if let Some(worker) = &self.flow_retry_worker {
                        ui.label(format!(
                            "{} retry · {}",
                            worker.target.label(),
                            worker.project_id
                        ));
                    } else if let Some(worker) = &self.remote_reconcile_worker {
                        ui.label(format!("Checking audio status · {}", worker.project_id));
                    } else if let Some(worker) = &self.manual_visual_worker {
                        ui.label(format!(
                            "Importing {} / {}",
                            worker.scene_id, worker.visual_id
                        ));
                    }
                });
                ui.add_space(4.0);
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            match self.screen {
                Screen::Projects => self.projects_ui(ui),
                Screen::Dashboard => self.dashboard_ui(ui),
                Screen::Scene => self.scene_detail_ui(ui),
                Screen::Settings => self.settings_ui(ui),
            }
        });

        if self.run_worker.is_some()
            || self.flow_retry_worker.is_some()
            || self.remote_reconcile_worker.is_some()
            || self.connection_worker.is_some()
            || self.manual_visual_worker.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl VideoPrepareApp {
    fn projects_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(980.0);
                ui.horizontal(|ui| {
                    ui.heading("Projects");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if self.show_create_project { "Close" } else { "+ New project" };
                        if ui.button(label).clicked() {
                            self.show_create_project = !self.show_create_project;
                        }
                    });
                });
                ui.label(
                    egui::RichText::new(
                        "Continue an existing project or start a new one from a .vprep script.",
                    )
                    .weak(),
                );
                ui.add_space(10.0);

                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.strong("Data Root");
                        ui.monospace(self.catalog_data_root.display().to_string());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Refresh projects").clicked() {
                                self.refresh_projects();
                            }
                        });
                    });
                });

                if self.show_create_project || self.catalog.projects.is_empty() {
                    ui.add_space(12.0);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.heading(egui::RichText::new("New project").size(20.0));
                    ui.label(
                        egui::RichText::new(
                            "Paste is the preferred input. You can also choose or drag a .vprep file into the app.",
                        )
                        .weak(),
                    );
                    let hovering_vprep = ui.ctx().input(|input| {
                        input.raw.hovered_files.iter().any(|file| {
                            file.path.as_ref().is_some_and(|path| is_vprep_path(path))
                        })
                    });
                    egui::Frame::group(ui.style())
                        .fill(if hovering_vprep {
                            ui.visuals().selection.bg_fill
                        } else {
                            ui.visuals().faint_bg_color
                        })
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(if hovering_vprep {
                                    "Release to load .vprep"
                                } else {
                                    "Drop .vprep here"
                                });
                                ui.label(
                                    egui::RichText::new(
                                        "The file is loaded into the editor so you can review or edit it before creating the project.",
                                    )
                                    .weak(),
                                );
                            });
                        });
                    ui.add_space(10.0);

                    ui.label(egui::RichText::new("Project ID").strong());
                    let project_id_response = ui.add_sized(
                        [420.0, 34.0],
                        egui::TextEdit::singleline(&mut self.create_project_id)
                            .hint_text("e.g. hashmap-deep-dive"),
                    );
                    if project_id_response.changed() {
                        self.create_project_id = normalize_project_id_input(&self.create_project_id);
                    }
                    ui.small("Spaces are converted to hyphens automatically.");

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Paste script").strong());
                        ui.label(
                            egui::RichText::new("Preferred")
                                .color(egui::Color32::from_rgb(96, 165, 250))
                                .small(),
                        );
                    });
                    ui.label(
                        egui::RichText::new(
                            "Paste the complete .vprep content here. If both inputs are filled, this text wins.",
                        )
                        .weak(),
                    );
                    ui.add_sized(
                        [ui.available_width(), 240.0],
                        egui::TextEdit::multiline(&mut self.create_script_text)
                            .code_editor()
                            .desired_rows(12)
                            .hint_text("Paste your structured .vprep script here..."),
                    );

                    let parsed_create_script = if self.create_script_text.trim().is_empty() {
                        None
                    } else {
                        Some(crate::parse_script(self.create_script_text.trim()))
                    };
                    if let Some(result) = &parsed_create_script {
                        ui.add_space(8.0);
                        match result {
                            Ok(script) => {
                                if self.create_project_id.trim().is_empty() {
                                    self.create_project_id = slugify_project_id(&script.omnivoice.title);
                                }
                                let visual_requests: usize =
                                    script.scenes.iter().map(|scene| scene.visuals.len()).sum();
                                let requested_assets: u32 = script
                                    .scenes
                                    .iter()
                                    .flat_map(|scene| scene.visuals.iter())
                                    .map(|visual| visual.count)
                                    .sum();
                                let duration_seconds = script
                                    .omnivoice
                                    .sections
                                    .last()
                                    .map(|section| section.end_seconds)
                                    .unwrap_or_default();

                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new("✓ Script ready")
                                                .strong()
                                                .color(egui::Color32::from_rgb(134, 239, 172)),
                                        );
                                        ui.label(egui::RichText::new(&script.omnivoice.title).strong());
                                    });
                                    ui.add_space(4.0);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(format!("{} scenes", script.scenes.len()));
                                        ui.separator();
                                        ui.label(format!("{visual_requests} visual requests"));
                                        ui.separator();
                                        ui.label(format!("{requested_assets} requested assets"));
                                        ui.separator();
                                        ui.label(format!(
                                            "{} narration sections",
                                            script.omnivoice.sections.len()
                                        ));
                                        ui.separator();
                                        ui.label(format!(
                                            "{} timeline",
                                            format_duration(duration_seconds)
                                        ));
                                    });
                                    ui.collapsing("Scene IDs", |ui| {
                                        ui.label(
                                            egui::RichText::new(
                                                script
                                                    .scenes
                                                    .iter()
                                                    .map(|scene| scene.id.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join(" · "),
                                            )
                                            .monospace(),
                                        );
                                    });
                                });
                            }
                            Err(error) => {
                                egui::Frame::group(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        format!("Script needs attention: {} · {error}", error.code()),
                                    );
                                    ui.label(
                                        egui::RichText::new(
                                            "Fix the script in the editor above. Project creation stays disabled until validation passes.",
                                        )
                                        .weak(),
                                    );
                                });
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.collapsing("Import a .vprep file instead", |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Optional fallback for scripts already saved on disk.",
                            )
                            .weak(),
                        );
                        ui.horizontal(|ui| {
                            let browse_width = 150.0;
                            let field_width = (ui.available_width()
                                - browse_width
                                - ui.spacing().item_spacing.x)
                                .max(220.0);
                            ui.add_sized(
                                [field_width, 34.0],
                                egui::TextEdit::singleline(&mut self.create_script_path)
                                    .interactive(false)
                                    .hint_text("No .vprep file selected"),
                            );
                            if ui
                                .add_sized([browse_width, 34.0], egui::Button::new("Choose .vprep"))
                                .clicked()
                            {
                                match pick_vprep_file() {
                                    Ok(Some(path)) => {
                                        self.load_vprep_into_create_form(path);
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        self.project_status = error;
                                        self.project_status_is_error = true;
                                    }
                                }
                            }
                        });
                    });

                    let pasted_ready = !self.create_script_text.trim().is_empty();
                    let file_ready = !self.create_script_path.trim().is_empty();
                    let id_ready = !self.create_project_id.trim().is_empty();
                    let script_valid = parsed_create_script
                        .as_ref()
                        .is_some_and(|result| result.is_ok());
                    if pasted_ready && file_ready {
                        ui.label(
                            egui::RichText::new(
                                "Pasted script is ready. The file path will be ignored for this create action.",
                            )
                            .color(egui::Color32::from_rgb(96, 165, 250)),
                        );
                    }

                    ui.add_space(10.0);
                    if ui
                        .add_enabled(
                            id_ready && script_valid && (pasted_ready || file_ready),
                            egui::Button::new(egui::RichText::new("Create project").strong()),
                        )
                        .clicked()
                    {
                        self.create_project_from_script();
                    }
                    });
                }

                if !self.project_status.is_empty() {
                    ui.add_space(10.0);
                    if self.project_status_is_error {
                        ui.colored_label(ui.visuals().error_fg_color, &self.project_status);
                    } else {
                        ui.label(
                            egui::RichText::new(&self.project_status)
                                .color(egui::Color32::from_rgb(134, 239, 172)),
                        );
                    }
                }

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.heading(egui::RichText::new("Your projects").size(20.0));
                    ui.label(
                        egui::RichText::new(format!("{} total", self.catalog.projects.len())).weak(),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_sized(
                        [320.0, 32.0],
                        egui::TextEdit::singleline(&mut self.project_search)
                            .hint_text("Search projects..."),
                    );
                    for filter in ProjectFilter::ALL {
                        if ui
                            .selectable_label(self.project_filter == filter, filter.label())
                            .clicked()
                        {
                            self.project_filter = filter;
                        }
                    }
                });
                ui.add_space(8.0);
                let query = self.project_search.trim().to_ascii_lowercase();
                let filtered_projects: Vec<_> = self
                    .catalog
                    .projects
                    .iter()
                    .filter(|project| {
                        let search_matches = query.is_empty()
                            || project.title.to_ascii_lowercase().contains(&query)
                            || project.project_id.to_ascii_lowercase().contains(&query);
                        search_matches && self.project_filter.matches(project)
                    })
                    .cloned()
                    .collect();

                if self.catalog.projects.is_empty() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(egui::RichText::new("No projects yet").strong());
                        ui.label(
                            egui::RichText::new(
                                "Paste a script above to create the first project in this Data Root.",
                            )
                            .weak(),
                        );
                    });
                } else if filtered_projects.is_empty() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.strong("No matching projects");
                        ui.label(
                            egui::RichText::new(
                                "Try a different search or select another project filter.",
                            )
                            .weak(),
                        );
                    });
                } else {
                    for project in filtered_projects {
                        let project_id = project.project_id.clone();
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(&project.title).strong().size(17.0),
                                    );
                                    ui.monospace(format!("id: {}", project.project_id));
                                    ui.horizontal_wrapped(|ui| {
                                        status_badge(ui, "Overall", project.overall);
                                        status_badge(ui, "Visual", project.visual_flow);
                                        status_badge(ui, "Audio", project.audio_flow);
                                    });
                                    if project.visual_assets_total > 0 {
                                        let ready = project
                                            .visual_assets_ready
                                            .min(project.visual_assets_total);
                                        ui.add(
                                            egui::ProgressBar::new(
                                                ready as f32 / project.visual_assets_total as f32,
                                            )
                                            .desired_width(300.0)
                                            .text(format!(
                                                "{ready}/{} visual assets ready",
                                                project.visual_assets_total
                                            )),
                                        );
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{} scene{}",
                                                project.scene_count,
                                                if project.scene_count == 1 { "" } else { "s" }
                                            ))
                                            .weak(),
                                        );
                                        if project.attention_count > 0 {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{} need attention",
                                                    project.attention_count
                                                ))
                                                .color(egui::Color32::from_rgb(250, 204, 21)),
                                            );
                                        }
                                    });
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("Continue").clicked() {
                                            self.open_project(&project_id);
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(6.0);
                    }
                }

                if !self.catalog.errors.is_empty() {
                    ui.add_space(14.0);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.strong("Needs attention");
                        ui.label(
                            egui::RichText::new(
                                "These project folders could not be loaded cleanly.",
                            )
                            .weak(),
                        );
                        for error in self.catalog.errors.clone() {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                format!("{}: {}", error.root.display(), error.message),
                            );
                        }
                    });
                }
            });
    }

    fn dashboard_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
  .auto_shrink([false, false])
  .show(ui, |ui| {
      ui.set_max_width(1040.0);
      ui.heading(egui::RichText::new("Project workspace").size(24.0));
      ui.label(
          egui::RichText::new(
              "Run the project, inspect flow health, and jump into scenes that need attention.",
          )
          .weak(),
      );
      ui.add_space(12.0);

      if self.selected_project.is_none() {
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label(egui::RichText::new("No project selected").strong().size(18.0));
              ui.label(
                  egui::RichText::new(
                      "Open a project from Projects to see execution status and scene progress.",
                  )
                  .weak(),
              );
              ui.add_space(8.0);
              if ui.button("Go to Projects").clicked() {
                  self.screen = Screen::Projects;
              }
          });
          return;
      }

      let Some(inspection) = self.inspection.clone() else {
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.colored_label(
                  ui.visuals().error_fg_color,
                  "Inspection is unavailable. Refresh the selected project from disk.",
              );
              if ui.button("Refresh from disk").clicked() {
                  self.reload_selected_project();
              }
          });
          return;
      };

      let project_root = self
          .selected_project
          .as_ref()
          .map(|project| project.root.clone());
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal(|ui| {
              ui.vertical(|ui| {
                  ui.label(egui::RichText::new(&inspection.title).strong().size(20.0));
                  ui.monospace(format!("project: {}", inspection.project_id));
                  if let Some(project) = &self.selected_project {
                      ui.label(
                          egui::RichText::new(format!("Root: {}", project.root.display()))
                              .weak(),
                      );
                  }
              });
              ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                  if let Some(root) = project_root.clone() {
                      if ui.button("Open project folder").clicked() {
                          match open_folder(&root) {
                              Ok(()) => {
                                  self.workspace_action_status =
                                      format!("Opened project folder: {}", root.display());
                                  self.workspace_action_status_is_error = false;
                              }
                              Err(error) => {
                                  self.workspace_action_status = error;
                                  self.workspace_action_status_is_error = true;
                              }
                          }
                      }
                  }
                  let incomplete = inspection.incomplete_count();
                  ui.label(
                      egui::RichText::new(if incomplete == 0 {
                          "No incomplete items".to_owned()
                      } else {
                          format!("{incomplete} item(s) need attention")
                      })
                      .color(if incomplete == 0 {
                          egui::Color32::from_rgb(134, 239, 172)
                      } else {
                          egui::Color32::from_rgb(250, 204, 21)
                      }),
                  );
              });
          });
          ui.add_space(8.0);
          ui.horizontal_wrapped(|ui| {
              status_badge(ui, "Overall", inspection.overall);
              status_badge(ui, "Visual", inspection.visual_flow);
              status_badge(ui, "Audio", inspection.audio_flow);
          });
      });
      if !self.workspace_action_status.is_empty() {
          if self.workspace_action_status_is_error {
              ui.colored_label(
                  ui.visuals().error_fg_color,
                  &self.workspace_action_status,
              );
          } else {
              ui.label(egui::RichText::new(&self.workspace_action_status).weak());
          }
      }

      ui.add_space(12.0);
      let applied_snapshot = self.settings.current();
      let applied_omnivoice = applied_snapshot.safe.omnivoice_url.clone();
      let audio_enabled = applied_snapshot.safe.audio_flow_enabled;
      let omnivoice_configured = !applied_omnivoice.is_empty();
      let omnivoice_test_busy = self
          .connection_worker
          .as_ref()
          .is_some_and(|worker| worker.target == ConnectionTestTarget::OmniVoice);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal(|ui| {
              ui.vertical(|ui| {
                  ui.label(egui::RichText::new("OmniVoice").strong().size(18.0));
                  if omnivoice_configured {
                      ui.label(
                          egui::RichText::new(format!("● Connected endpoint · {applied_omnivoice}"))
                              .color(egui::Color32::from_rgb(134, 239, 172)),
                      );
                  } else {
                      ui.label(
                          egui::RichText::new("○ Not configured")
                              .color(egui::Color32::from_rgb(250, 204, 21)),
                      );
                  }
                  if !audio_enabled {
                      ui.label(
                          egui::RichText::new("Audio Flow is currently disabled in Settings.").weak(),
                      );
                  }
              });
              ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                  let label = if self.show_omnivoice_quick_connect {
                      "Close"
                  } else {
                      "Change connection"
                  };
                  if ui.button(label).clicked() {
                      self.show_omnivoice_quick_connect = !self.show_omnivoice_quick_connect;
                      if self.show_omnivoice_quick_connect
                          && self.omnivoice_quick_input.trim().is_empty()
                          && omnivoice_configured
                      {
                          self.omnivoice_quick_input = applied_omnivoice.clone();
                      }
                  }
              });
          });

          if self.show_omnivoice_quick_connect {
              ui.separator();
              ui.label(
                  egui::RichText::new(
                      "Paste the REST URL or the complete OmniVoiceStudio startup output. Video Prepare will detect `Public REST API:` automatically.",
                  )
                  .weak(),
              );
              ui.add_sized(
                  [ui.available_width(), 82.0],
                  egui::TextEdit::multiline(&mut self.omnivoice_quick_input)
                      .desired_rows(3)
                      .hint_text("Public REST API: https://.../api/v1"),
              );

              let detected = extract_omnivoice_url(&self.omnivoice_quick_input).ok();
              if let Some(endpoint) = &detected {
                  ui.label(
                      egui::RichText::new(format!("Detected service root: {endpoint}"))
                          .color(egui::Color32::from_rgb(134, 239, 172)),
                  );
              } else if !self.omnivoice_quick_input.trim().is_empty() {
                  ui.label(
                      egui::RichText::new("No valid REST endpoint detected yet.")
                          .color(ui.visuals().warn_fg_color),
                  );
              }

              ui.horizontal_wrapped(|ui| {
                  if ui
                      .add_enabled(
                          detected.is_some() && !omnivoice_test_busy,
                          egui::Button::new(egui::RichText::new("Test & use").strong()),
                      )
                      .clicked()
                  {
                      self.start_quick_omnivoice_connection();
                  }
                  if omnivoice_test_busy {
                      ui.spinner();
                      ui.label(egui::RichText::new("Testing OmniVoice...").weak());
                  }
                  if ui.button("Open Settings").clicked() {
                      self.screen = Screen::Settings;
                  }
              });

              if !self.omnivoice_quick_status.is_empty() {
                  if self.omnivoice_quick_status_is_error {
                      ui.colored_label(
                          ui.visuals().error_fg_color,
                          &self.omnivoice_quick_status,
                      );
                  } else {
                      ui.label(
                          egui::RichText::new(&self.omnivoice_quick_status)
                              .color(egui::Color32::from_rgb(134, 239, 172)),
                      );
                  }
              }
          }
      });

      ui.add_space(12.0);
      let worker_active = self.mutation_worker_active();
      let remote_available = self.remote_reconciliation_available(true);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(egui::RichText::new("Next step").strong().size(18.0));
          ui.label(
              egui::RichText::new(
                  "Video Prepare picks the safest next action from the persisted project state.",
              )
              .weak(),
          );
          ui.add_space(8.0);
          let smart_action = recommended_workspace_action(
              inspection.overall,
              inspection.audio_flow,
              remote_available,
          );
          if ui
              .add_enabled(
                  !worker_active && smart_action != WorkspaceAction::Ready,
                  egui::Button::new(egui::RichText::new(smart_action.label()).strong()),
              )
              .clicked()
          {
              match smart_action {
                  WorkspaceAction::Resume => self.start_run(RunAction::Resume),
                  WorkspaceAction::RetryFailed => self.start_run(RunAction::RetryFailed),
                  WorkspaceAction::CheckAudio => self.start_remote_audio_reconciliation(true),
                  WorkspaceAction::Start => self.start_run(RunAction::Run),
                  WorkspaceAction::Ready => {}
              }
          }
          if worker_active {
              ui.horizontal(|ui| {
                  ui.spinner();
                  ui.label(egui::RichText::new("Working in the background").weak());
              });
          }
          ui.add_space(6.0);
          ui.collapsing("Advanced actions", |ui| {
              ui.horizontal_wrapped(|ui| {
                  if ui.add_enabled(!worker_active, egui::Button::new("Run all")).clicked() {
                      self.start_run(RunAction::Run);
                  }
                  if ui.add_enabled(!worker_active, egui::Button::new("Resume")).clicked() {
                      self.start_run(RunAction::Resume);
                  }
                  if ui.add_enabled(!worker_active, egui::Button::new("Retry failed")).clicked() {
                      self.start_run(RunAction::RetryFailed);
                  }
                  if ui
                      .add_enabled(!worker_active && remote_available, egui::Button::new("Check remote audio"))
                      .clicked()
                  {
                      self.start_remote_audio_reconciliation(true);
                  }
                  if ui.add_enabled(!worker_active, egui::Button::new("Refresh from disk")).clicked() {
                      self.reload_selected_project();
                  }
              });
          });
      });

      let has_activity = self.run_worker.is_some()
          || self.flow_retry_worker.is_some()
          || self.remote_reconcile_worker.is_some()
          || !self.run_status.is_empty()
          || !self.remote_reconcile_status.is_empty()
          || self.last_run_report.is_some()
          || self.last_remote_reconcile_report.is_some();
      if has_activity {
          ui.add_space(12.0);
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label(egui::RichText::new("Activity").strong().size(18.0));
              if let Some(worker) = &self.run_worker {
                  ui.label(format!(
                      "{} running for `{}` with settings revision {}.",
                      worker.action.label(), worker.project_id, worker.revision
                  ));
                  ui.small(format!("Data Root: {}", worker.data_root.display()));
              }
              if let Some(worker) = &self.flow_retry_worker {
                  ui.label(format!(
                      "{} retry running for `{}` with settings revision {}.",
                      worker.target.label(), worker.project_id, worker.revision
                  ));
              }
              if let Some(worker) = &self.remote_reconcile_worker {
                  ui.label(format!(
                      "Remote audio reconciliation running for `{}` with settings revision {}.",
                      worker.project_id, worker.revision
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
                  ui.separator();
                  ui.strong(format!(
                      "Last {} | settings revision {}",
                      report.action.label(), report.settings_revision
                  ));
                  render_flow_report(ui, "Visual", &report.visual);
                  render_flow_report(ui, "Audio", &report.audio);
              }
              if !self.remote_reconcile_status.is_empty() {
                  if self.remote_reconcile_status_is_error {
                      ui.colored_label(
                          ui.visuals().error_fg_color,
                          &self.remote_reconcile_status,
                      );
                  } else {
                      ui.label(&self.remote_reconcile_status);
                  }
              }
              if let Some(report) = &self.last_remote_reconcile_report {
                  ui.separator();
                  render_remote_reconciliation_report(ui, report);
              }
          });
      }

      if !inspection.problems.is_empty() {
          ui.add_space(12.0);
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label(egui::RichText::new("Needs attention").strong().size(18.0));
              ui.label(
                  egui::RichText::new(
                      "Resolve these items or retry the affected flow before considering the project complete.",
                  )
                  .weak(),
              );
              ui.add_space(6.0);
              let next_scene = inspection
                  .problems
                  .iter()
                  .find_map(|problem| problem.scene_id.clone());
              if let Some(scene_id) = next_scene {
                  if ui.button("Fix next issue").clicked() {
                      self.selected_scene_id = Some(scene_id);
                      self.screen = Screen::Scene;
                  }
                  ui.add_space(4.0);
              }
              for problem in &inspection.problems {
                  let scene_target = problem.scene_id.clone();
                  egui::Frame::group(ui.style()).show(ui, |ui| {
                      ui.set_min_width(ui.available_width());
                      ui.horizontal_wrapped(|ui| {
                          status_badge(ui, &problem.area, problem.state);
                          ui.strong(&problem.scope);
                          ui.label(&problem.message);
                          if let Some(scene_id) = scene_target.clone() {
                              if ui.button("Open scene").clicked() {
                                  self.selected_scene_id = Some(scene_id);
                                  self.screen = Screen::Scene;
                              }
                          }
                      });
                  });
              }
          });
      }

      ui.add_space(16.0);
      ui.horizontal(|ui| {
          ui.heading(egui::RichText::new("Scenes").size(20.0));
          ui.label(
              egui::RichText::new(format!("{} total", inspection.scenes.len())).weak(),
          );
      });
      ui.add_space(6.0);

      for scene in inspection.scenes {
          let scene_id = scene.id.clone();
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.horizontal(|ui| {
                  ui.vertical(|ui| {
                      ui.label(
                          egui::RichText::new(format!(
                              "{}  |  {} to {}",
                              scene.id, scene.start_time, scene.end_time
                          ))
                          .strong()
                          .size(17.0),
                      );
                      ui.horizontal_wrapped(|ui| {
                          status_badge(ui, "Visual", scene.visual_state);
                          status_badge(ui, "Audio", scene.audio_state);
                      });
                      if scene.visual_detail_available {
                          let total = scene.visual_requests.len();
                          let completed = scene
                              .visual_requests
                              .iter()
                              .filter(|request| {
                                  request.state == crate::TaskState::Completed
                              })
                              .count();
                          let progress = if total == 0 {
                              0.0
                          } else {
                              completed as f32 / total as f32
                          };
                          ui.add(
                              egui::ProgressBar::new(progress)
                                  .desired_width(280.0)
                                  .text(format!(
                                      "{completed}/{total} visual requests complete"
                                  )),
                          );
                      } else {
                          ui.colored_label(
                              ui.visuals().error_fg_color,
                              "Visual request detail unavailable",
                          );
                      }
                  });
                  ui.with_layout(
                      egui::Layout::right_to_left(egui::Align::Center),
                      |ui| {
                          if ui.button("Inspect scene").clicked() {
                              self.selected_scene_id = Some(scene_id.clone());
                              self.screen = Screen::Scene;
                          }
                      },
                  );
              });
          });
          ui.add_space(6.0);
      }
  });
    }

    fn scene_detail_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
  .auto_shrink([false, false])
  .show(ui, |ui| {
      ui.set_max_width(1040.0);
      ui.heading(egui::RichText::new("Scene detail").size(24.0));
      ui.label(
          egui::RichText::new(
              "Inspect one scene, review asset requests, and take over manually when automation cannot finish the job.",
          )
          .weak(),
      );
      ui.add_space(12.0);

      let Some(inspection) = self.inspection.clone() else {
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label("No inspected project is selected.");
              if ui.button("Go to Project").clicked() {
                  self.screen = Screen::Dashboard;
              }
          });
          return;
      };
      let Some(scene_id) = self.selected_scene_id.clone() else {
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label("Choose a scene from the Project workspace first.");
              if ui.button("Go to Project").clicked() {
                  self.screen = Screen::Dashboard;
              }
          });
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
      let scene_index = inspection.scenes.iter().position(|item| item.id == scene.id);
      let previous_scene_id = scene_index
          .and_then(|index| index.checked_sub(1))
          .and_then(|index| inspection.scenes.get(index))
          .map(|item| item.id.clone());
      let next_scene_id = scene_index
          .and_then(|index| inspection.scenes.get(index + 1))
          .map(|item| item.id.clone());
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal_wrapped(|ui| {
              if ui.button("← Project").clicked() {
                  self.screen = Screen::Dashboard;
              }
              if ui.add_enabled(!mutation_busy, egui::Button::new("Refresh")).clicked() {
                  self.reload_selected_project();
              }
              ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                  if ui.add_enabled(next_scene_id.is_some(), egui::Button::new("Next ›")).clicked() {
                      self.selected_scene_id = next_scene_id.clone();
                  }
                  if ui.add_enabled(previous_scene_id.is_some(), egui::Button::new("‹ Previous")).clicked() {
                      self.selected_scene_id = previous_scene_id.clone();
                  }
              });
              if mutation_busy {
                  ui.spinner();
              }
          });
          ui.add_space(8.0);
          ui.label(
              egui::RichText::new(format!("{}  /  {}", inspection.title, scene.id))
                  .strong()
                  .size(20.0),
          );
          ui.label(
              egui::RichText::new(format!(
                  "Narration window: {} to {}",
                  scene.start_time, scene.end_time
              ))
              .weak(),
          );
          ui.horizontal_wrapped(|ui| {
              status_badge(ui, "Visual", scene.visual_state);
              status_badge(ui, "Audio", scene.audio_state);
          });
      });

      ui.add_space(12.0);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(egui::RichText::new("Flow recovery").strong().size(18.0));
          ui.label(
              egui::RichText::new(
                  "Retries are project-level flow actions. They may touch more than this scene.",
              )
              .weak(),
          );
          ui.add_space(8.0);
          ui.horizontal_wrapped(|ui| {
              let visual_retryable = retryable_flow_state(inspection.visual_flow);
              if ui
                  .add_enabled(
                      !mutation_busy && visual_retryable,
                      egui::Button::new("Retry visual flow"),
                  )
                  .clicked()
              {
                  self.start_flow_retry(FlowTarget::Visual);
              }
              let audio_retryable = retryable_flow_state(inspection.audio_flow);
              if ui
                  .add_enabled(
                      !mutation_busy && audio_retryable,
                      egui::Button::new("Retry audio flow"),
                  )
                  .clicked()
              {
                  self.start_flow_retry(FlowTarget::Audio);
              }
              status_badge(ui, "Visual flow", inspection.visual_flow);
              status_badge(ui, "Audio flow", inspection.audio_flow);
          });
          if let Some(worker) = &self.flow_retry_worker {
              ui.add_space(6.0);
              ui.label(format!(
                  "{} retry running with settings revision {}.",
                  worker.target.label(), worker.revision
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
              ui.separator();
              render_flow_report(ui, report.target.label(), &report.flow);
          }
      });

      if !self.manual_visual_status.is_empty() {
          ui.add_space(8.0);
          if self.manual_visual_status_is_error {
              ui.colored_label(ui.visuals().error_fg_color, &self.manual_visual_status);
          } else {
              ui.label(&self.manual_visual_status);
          }
      }

      ui.add_space(16.0);
      ui.heading(egui::RichText::new("Visual requests").size(20.0));
      ui.label(
          egui::RichText::new(
              "Downloaded and manually imported assets show their full local path here. Preview opens the file with the system default app; Go to folder reveals it in the file manager.",
          )
          .weak(),
      );
      if !self.asset_action_status.is_empty() {
          if self.asset_action_status_is_error {
              ui.colored_label(
                  ui.visuals().error_fg_color,
                  &self.asset_action_status,
              );
          } else {
              ui.label(&self.asset_action_status);
          }
      }
      ui.add_space(6.0);
      if !scene.visual_detail_available {
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.colored_label(
                  ui.visuals().error_fg_color,
                  "Visual status detail could not be loaded. See Dashboard for project-level errors.",
              );
          });
      } else if scene.visual_requests.is_empty() {
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label("This scene has no visual requests.");
          });
      } else {
          for request in &scene.visual_requests {
              egui::Frame::group(ui.style()).show(ui, |ui| {
                  ui.set_min_width(ui.available_width());
                  ui.horizontal(|ui| {
                      ui.label(
                          egui::RichText::new(&request.id).strong().size(17.0),
                      );
                      ui.with_layout(
                          egui::Layout::right_to_left(egui::Align::Center),
                          |ui| status_badge(ui, "Status", request.state),
                      );
                  });
                  let target = request.target_count as usize;
                  let progress = if target == 0 {
                      1.0
                  } else {
                      request.completed_assets as f32 / target as f32
                  };
                  ui.add(
                      egui::ProgressBar::new(progress.clamp(0.0, 1.0))
                          .desired_width(320.0)
                          .text(format!(
                              "{}/{} assets ready",
                              request.completed_assets, target
                          )),
                  );
                  if !request.attempted_queries.is_empty() || request.successful_query.is_some() {
                      ui.collapsing("Search details", |ui| {
                          if !request.attempted_queries.is_empty() {
                              ui.small(format!(
                                  "Attempted queries: {}",
                                  request.attempted_queries.join(" | ")
                              ));
                          }
                          if let Some(query) = &request.successful_query {
                              ui.small(format!("Successful query: {query}"));
                          }
                      });
                  }
                  if let Some(error) = &request.last_error {
                      ui.colored_label(ui.visuals().error_fg_color, error);
                  }
                  if !request.assets.is_empty() {
                      ui.add_space(8.0);
                      ui.label(egui::RichText::new("Local assets").strong());
                      for asset in &request.assets {
                          egui::Frame::group(ui.style()).show(ui, |ui| {
                              ui.set_min_width(ui.available_width());
                              let file_name = asset
                                  .absolute_path
                                  .file_name()
                                  .and_then(|name| name.to_str())
                                  .unwrap_or("local asset");
                              let (media_icon, media_label) = match asset.kind {
                                  crate::PersistedAssetKind::Image => ("▣", "IMAGE"),
                                  crate::PersistedAssetKind::Video => ("▶", "VIDEO"),
                              };
                              let thumbnail = if asset.kind == crate::PersistedAssetKind::Image {
                                  Some(self.image_thumbnail(ui.ctx(), &asset.absolute_path))
                              } else {
                                  None
                              };
                              ui.horizontal(|ui| {
                                  egui::Frame::group(ui.style()).show(ui, |ui| {
                                      ui.set_min_size(egui::vec2(188.0, 112.0));
                                      ui.vertical_centered(|ui| {
                                          match &thumbnail {
                                              Some(Ok((texture, _, _))) => {
                                                  ui.image((texture.id(), texture.size_vec2()));
                                              }
                                              Some(Err(error)) => {
                                                  ui.add_space(12.0);
                                                  ui.label(egui::RichText::new(media_icon).size(28.0));
                                                  ui.label(
                                                      egui::RichText::new("PREVIEW UNAVAILABLE")
                                                          .strong()
                                                          .small(),
                                                  )
                                                  .on_hover_text(error);
                                              }
                                              None => {
                                                  ui.add_space(12.0);
                                                  ui.label(egui::RichText::new(media_icon).size(28.0));
                                                  ui.label(
                                                      egui::RichText::new(media_label).strong().small(),
                                                  );
                                              }
                                          }
                                      });
                                  });
                                  ui.vertical(|ui| {
                                      ui.strong(file_name);
                                      let dimensions = thumbnail
                                          .as_ref()
                                          .and_then(|result| result.as_ref().ok())
                                          .map(|(_, width, height)| format!(" · {width}×{height}"))
                                          .unwrap_or_default();
                                      ui.label(
                                          egui::RichText::new(format!(
                                              "Slot {} · {}{}",
                                              asset.slot,
                                              format_bytes(asset.bytes),
                                              dimensions
                                          ))
                                          .weak(),
                                      );
                                      ui.add_space(4.0);
                                      ui.horizontal_wrapped(|ui| {
                                          if ui
                                              .button("Open preview")
                                              .on_hover_text("Open with the system default viewer or player")
                                              .clicked()
                                          {
                                              match open_asset_preview(&asset.absolute_path) {
                                                  Ok(()) => {
                                                      self.asset_action_status = format!(
                                                          "Opened preview: {}",
                                                          asset.absolute_path.display()
                                                      );
                                                      self.asset_action_status_is_error = false;
                                                  }
                                                  Err(error) => {
                                                      self.asset_action_status = error;
                                                      self.asset_action_status_is_error = true;
                                                  }
                                              }
                                          }
                                          if ui.button("Show in folder").clicked() {
                                              match reveal_asset_in_folder(&asset.absolute_path) {
                                                  Ok(()) => {
                                                      self.asset_action_status = format!(
                                                          "Revealed asset: {}",
                                                          asset.absolute_path.display()
                                                      );
                                                      self.asset_action_status_is_error = false;
                                                  }
                                                  Err(error) => {
                                                      self.asset_action_status = error;
                                                      self.asset_action_status_is_error = true;
                                                  }
                                              }
                                          }
                                      });
                                  });
                              });
                              ui.collapsing("Technical details", |ui| {
                                  ui.small(format!("Provider: {}", asset.provider));
                                  ui.small(format!(
                                      "Provider asset ID: {}",
                                      asset.provider_asset_id
                                  ));
                                  ui.small(format!("Relative path: {}", asset.relative_path));
                                  ui.add(
                                      egui::Label::new(
                                          egui::RichText::new(
                                              asset.absolute_path.display().to_string(),
                                          )
                                          .monospace(),
                                      )
                                      .wrap(),
                                  );
                              });
                          });
                          ui.add_space(6.0);
                      }
                  }
                  if request.completed_assets < target {
                      ui.add_space(6.0);
                      if ui
                          .add_enabled(
                              !mutation_busy,
                              egui::Button::new(format!("Choose local file for {}", request.id)),
                          )
                          .clicked()
                      {
                          match pick_visual_asset_file() {
                              Ok(Some(path)) => {
                                  self.manual_visual_path = path.to_string_lossy().into_owned();
                                  self.start_manual_visual_import(&scene.id, &request.id);
                              }
                              Ok(None) => {}
                              Err(error) => {
                                  self.manual_visual_status = error;
                                  self.manual_visual_status_is_error = true;
                              }
                          }
                      }
                      ui.label(
                          egui::RichText::new("Use a local image/video when stock search cannot finish this request.")
                              .weak(),
                      );
                  }
              });
              ui.add_space(6.0);
          }
      }

      ui.add_space(16.0);
      ui.heading(egui::RichText::new("Audio flow").size(20.0));
      ui.add_space(6.0);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal_wrapped(|ui| {
              status_badge(ui, "Flow", inspection.audio.state);
              ui.label(
                  egui::RichText::new(format!(
                      "{} remote attempt(s)",
                      inspection.audio.attempts.len()
                  ))
                  .weak(),
              );
          });
          if !inspection.audio.detail_available {
              ui.colored_label(
                  ui.visuals().error_fg_color,
                  "Audio status detail could not be loaded. See Dashboard for project-level errors.",
              );
          } else if inspection.audio.attempts.is_empty() {
              ui.label("No remote audio attempt has been submitted yet.");
          } else {
              for attempt in &inspection.audio.attempts {
                  ui.separator();
                  ui.horizontal_wrapped(|ui| {
                      ui.strong("Remote audio attempt");
                      status_badge(ui, "Status", attempt.state);
                  });
                  if let Some(error) = &attempt.last_error {
                      ui.colored_label(ui.visuals().error_fg_color, error);
                  }
                  ui.collapsing("Technical details", |ui| {
                      ui.small(format!("Attempt: {}", attempt.attempt_id));
                      ui.small(format!("Server: {}", attempt.server_base_url));
                      ui.small(format!(
                          "Remote project: {}",
                          attempt.remote_project_id
                      ));
                      ui.small(format!(
                          "Job: {}",
                          attempt.job_id.as_deref().unwrap_or("not assigned")
                      ));
                  });
              }
          }
      });
  });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
  .auto_shrink([false, false])
  .show(ui, |ui| {
      ui.set_max_width(1040.0);
      ui.heading(egui::RichText::new("Settings").size(24.0));
      ui.label(
          egui::RichText::new(
              "Configure the Data Root and runtime integrations used by Visual and Audio flows.",
          )
          .weak(),
      );
      let draft_dirty = self.draft != self.settings.draft();
      ui.add_space(8.0);
      ui.horizontal_wrapped(|ui| {
          if draft_dirty {
              ui.label(
                  egui::RichText::new("● Unsaved changes")
                      .color(egui::Color32::from_rgb(250, 204, 21)),
              );
              ui.label(
                  egui::RichText::new("Save settings when you are ready to use this configuration.")
                      .weak(),
              );
          } else {
              ui.label(
                  egui::RichText::new("● Saved")
                      .color(egui::Color32::from_rgb(134, 239, 172)),
              );
          }
      });
      ui.collapsing("Advanced / diagnostics", |ui| {
          ui.monospace(format!("Applied revision {}", self.settings.current().revision));
          if let Some(path) = self.settings.persistence_path() {
              if self.settings.loaded_from_disk() {
                  ui.small(format!("Preferences: {}", path.display()));
              } else {
                  ui.small(format!("Preferences will be saved to: {}", path.display()));
              }
          }
          ui.small("Data Root, flow toggles, provider URL, narration defaults, quality, and concurrency are restored on next launch.");
          if self.settings.system_secret_persistence_enabled() {
              if self.settings.secrets_restored() {
                  ui.small("Provider credentials were restored from the OS credential store.");
              } else {
                  ui.small("Provider credentials are saved in the OS credential store when settings are saved.");
              }
          } else {
              ui.small("Secure API-key persistence is unavailable on this platform; secrets remain session-only.");
          }
          if let Some(warning) = self.settings.secret_persistence_warning() {
              ui.colored_label(ui.visuals().warn_fg_color, warning);
          }
      });

      let connection_busy = self.connection_worker.is_some();

      ui.add_space(12.0);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(egui::RichText::new("Project storage").strong().size(18.0));
          ui.label(
              egui::RichText::new(
                  "All project state, generated assets, and resume metadata live under this Data Root.",
              )
              .weak(),
          );
          ui.add_space(8.0);
          ui.label(egui::RichText::new("Data Root").strong());
          ui.horizontal(|ui| {
              let browse_width = 120.0;
              let field_width =
                  (ui.available_width() - browse_width - ui.spacing().item_spacing.x).max(220.0);
              ui.add_sized(
                  [field_width, 34.0],
                  egui::TextEdit::singleline(&mut self.draft.data_root)
                      .interactive(false)
                      .hint_text("Choose a folder..."),
              );

              if ui
                  .add_sized([browse_width, 34.0], egui::Button::new("Choose folder"))
                  .clicked()
              {
                  match pick_data_root_folder(&self.draft.data_root) {
                      Ok(Some(path)) => {
                          self.draft.data_root = path.to_string_lossy().into_owned();
                          self.status =
                              "Data Root selected. Save settings to use it.".to_owned();
                          self.status_is_error = false;
                      }
                      Ok(None) => {}
                      Err(error) => {
                          self.status = format!("Could not open folder picker: {error}");
                          self.status_is_error = true;
                      }
                  }
              }
          });
          ui.small(
              "Choose the Data Root with the system folder browser. The path is read-only here and is saved after Apply settings.",
          );
      });

      ui.add_space(12.0);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(egui::RichText::new("Flows").strong().size(18.0));
          ui.label(
              egui::RichText::new(
                  "Disable a flow when you want to prepare only visuals or only narration.",
              )
              .weak(),
          );
          ui.add_space(8.0);
          ui.horizontal_wrapped(|ui| {
              ui.checkbox(&mut self.draft.visual_flow_enabled, "Visual Flow");
              ui.checkbox(&mut self.draft.audio_flow_enabled, "Audio Flow");
          });
      });

      ui.add_space(12.0);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(
              egui::RichText::new("Visual provider - Pexels")
                  .strong()
                  .size(18.0),
          );
          ui.label(
              egui::RichText::new(
                  "Used to search and download stock images and videos for visual requests.",
              )
              .weak(),
          );
          ui.add_space(8.0);
          ui.label(egui::RichText::new("Pexels API Key").strong());
          ui.add_sized(
              [ui.available_width(), 34.0],
              egui::TextEdit::singleline(&mut self.draft.pexels_api_key)
                  .password(true)
                  .hint_text("Paste API key"),
          );
          ui.add_space(8.0);
          ui.horizontal_wrapped(|ui| {
              if ui
                  .add_enabled(!connection_busy, egui::Button::new("Test Pexels"))
                  .clicked()
              {
                  self.start_connection_test(ConnectionTestTarget::Pexels);
              }
              if self
                  .connection_worker
                  .as_ref()
                  .is_some_and(|worker| worker.target == ConnectionTestTarget::Pexels)
              {
                  ui.spinner();
              }
          });
          ui.collapsing("Advanced download settings", |ui| {
              ui.horizontal_wrapped(|ui| {
                  ui.label(egui::RichText::new("Download concurrency").strong());
                  ui.add(
                      egui::DragValue::new(&mut self.draft.download_concurrency)
                          .range(1..=32),
                  );
              });
          });
          ui.small("The API key is kept out of preferences.json and uses the OS credential store on macOS/Windows.");
      });

      ui.add_space(12.0);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(
              egui::RichText::new("Audio provider - OmniVoice")
                  .strong()
                  .size(18.0),
          );
          ui.label(
              egui::RichText::new(
                  "Configure the current OmniVoice Studio endpoint and the narration defaults sent with project jobs.",
              )
              .weak(),
          );
          ui.add_space(8.0);

          ui.label(egui::RichText::new("OmniVoice URL").strong());
          ui.add_sized(
              [ui.available_width(), 34.0],
              egui::TextEdit::singleline(&mut self.draft.omnivoice_url)
                  .hint_text("https://your-current-domain.example/api/v1"),
          );
          ui.add_space(6.0);
          ui.label(egui::RichText::new("API Token").strong());
          ui.add_sized(
              [ui.available_width(), 34.0],
              egui::TextEdit::singleline(&mut self.draft.omnivoice_token)
                  .password(true)
                  .hint_text("Optional token"),
          );
          ui.add_space(8.0);
          ui.horizontal_wrapped(|ui| {
              if ui
                  .add_enabled(!connection_busy, egui::Button::new("Test OmniVoice"))
                  .clicked()
              {
                  self.start_connection_test(ConnectionTestTarget::OmniVoice);
              }
              if self
                  .connection_worker
                  .as_ref()
                  .is_some_and(|worker| worker.target == ConnectionTestTarget::OmniVoice)
              {
                  ui.spinner();
              }
          });
          ui.collapsing("Narration defaults", |ui| {
              ui.label(egui::RichText::new("Voice").strong());
              ui.add_sized(
                  [ui.available_width(), 34.0],
                  egui::TextEdit::singleline(&mut self.draft.voice_name)
                      .hint_text("Narrator"),
              );
              ui.add_space(6.0);
              ui.label(egui::RichText::new("Variant").strong());
              ui.add_sized(
                  [ui.available_width(), 34.0],
                  egui::TextEdit::singleline(&mut self.draft.voice_variant)
                      .hint_text("AUTO"),
              );
              ui.add_space(6.0);
              ui.label(egui::RichText::new("Language").strong());
              ui.add_sized(
                  [ui.available_width(), 34.0],
                  egui::TextEdit::singleline(&mut self.draft.language).hint_text("en"),
              );
              ui.add_space(8.0);
              ui.horizontal_wrapped(|ui| {
                  ui.label(egui::RichText::new("Quality").strong());
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
                  ui.checkbox(
                      &mut self.draft.read_section_titles,
                      "Read section titles",
                  );
              });
          });
      });

      if self.connection_worker.is_some()
          || !self.connection_status.is_empty()
          || self.last_connection_report.is_some()
      {
          ui.add_space(12.0);
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label(egui::RichText::new("Connection test").strong().size(18.0));
              if let Some(worker) = &self.connection_worker {
                  ui.label(format!(
                      "Testing {} with a captured draft. Applied revision at start: {}.",
                      worker.target.label(), worker.applied_revision_at_start
                  ));
              }
              if !self.connection_status.is_empty() {
                  if self.connection_status_is_error {
                      ui.colored_label(
                          ui.visuals().error_fg_color,
                          &self.connection_status,
                      );
                  } else {
                      ui.label(&self.connection_status);
                  }
              }
              if let Some(report) = &self.last_connection_report {
                  ui.separator();
                  render_connection_report(ui, report);
              }
          });
      }

      ui.add_space(12.0);
      let mut refresh_catalog = false;
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal_wrapped(|ui| {
              if ui
                  .add_enabled(draft_dirty, egui::Button::new("Discard changes"))
                  .clicked()
              {
                  self.draft = self.settings.draft();
                  self.status = "Unsaved changes discarded.".to_owned();
                  self.status_is_error = false;
              }
              if ui
                  .add_enabled(
                      draft_dirty,
                      egui::Button::new(egui::RichText::new("Save settings").strong()),
                  )
                  .clicked()
              {
                  let previous_root = self.settings.current().safe.data_root.clone();
                  match self.settings.apply(&self.draft) {
                      Ok(snapshot) => {
                          self.draft = RuntimeSettingsDraft::from_snapshot(&snapshot);
                          self.status = match self.settings.persistence_path() {
                              Some(path) => {
                                  let mut message = format!(
                                      "Applied runtime settings revision {} and saved preferences to {}.",
                                      snapshot.revision,
                                      path.display()
                                  );
                                  if self.settings.system_secret_persistence_enabled()
                                      && self.settings.secret_persistence_warning().is_none()
                                  {
                                      message.push_str(" API credentials were saved to the OS credential store.");
                                  }
                                  if let Some(warning) = self.settings.secret_persistence_warning() {
                                      message.push_str(&format!(" {warning}"));
                                  }
                                  message
                              }
                              None => format!(
                                  "Applied runtime settings revision {}. Persistent storage is unavailable.",
                                  snapshot.revision
                              ),
                          };
                          self.status_is_error = false;
                          refresh_catalog = previous_root != snapshot.safe.data_root;
                      }
                      Err(error) => {
                          self.status = error.to_string();
                          self.status_is_error = true;
                      }
                  }
              }
              ui.label(
                  egui::RichText::new(
                      "Saving updates the active runtime configuration and persists non-secret preferences for the next launch.",
                  )
                  .weak(),
              );
          });
          if !self.status.is_empty() {
              ui.add_space(6.0);
              if self.status_is_error {
                  ui.colored_label(ui.visuals().error_fg_color, &self.status);
              } else {
                  ui.label(
                      egui::RichText::new(&self.status)
                          .color(egui::Color32::from_rgb(134, 239, 172)),
                  );
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
  });
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
        let script_text = self.create_script_text.trim().to_owned();
        let script_path = self.create_script_path.trim().to_owned();

        if project_id.is_empty() {
            self.project_status = "Project ID is required.".to_owned();
            self.project_status_is_error = true;
            return;
        }
        if script_text.is_empty() && script_path.is_empty() {
            self.project_status = "Paste a script or provide a .vprep file path.".to_owned();
            self.project_status_is_error = true;
            return;
        }

        let data_root = self.settings.current().safe.data_root.clone();
        let (result, source) = if !script_text.is_empty() {
            (
                create_project_from_script_text(&data_root, &project_id, &script_text),
                "pasted script",
            )
        } else {
            (
                create_project_from_script_path(&data_root, &project_id, &script_path),
                ".vprep file",
            )
        };

        match result {
            Ok(project) => {
                let created_id = project.metadata.project_id.clone();
                self.create_project_id.clear();
                self.create_script_text.clear();
                self.create_script_path.clear();
                self.show_create_project = false;
                self.select_project(project);
                self.refresh_projects();
                self.project_status = format!("Created project `{created_id}` from {source}.");
                self.project_status_is_error = false;
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
        self.asset_action_status.clear();
        self.asset_action_status_is_error = false;
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
                let client =
                    OmniVoiceClient::new(base_url, token).map_err(|error| error.to_string())?;
                let mut project =
                    open_project_from_data_root(&worker_data_root, &worker_project_id)
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
        self.omnivoice_quick_apply_pending = false;
        self.start_connection_test_with_draft(target, self.draft.clone());
    }

    fn start_quick_omnivoice_connection(&mut self) {
        if self.connection_worker.is_some() {
            return;
        }
        let endpoint = match extract_omnivoice_url(&self.omnivoice_quick_input) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                self.omnivoice_quick_status =
                    format!("Could not detect OmniVoice REST endpoint: {error}");
                self.omnivoice_quick_status_is_error = true;
                return;
            }
        };

        let mut captured = self.settings.draft();
        captured.omnivoice_url = endpoint.clone();
        self.omnivoice_quick_status = format!("Testing {endpoint} before saving it...");
        self.omnivoice_quick_status_is_error = false;
        self.omnivoice_quick_apply_pending = true;
        self.start_connection_test_with_draft(ConnectionTestTarget::OmniVoice, captured);
    }

    fn start_connection_test_with_draft(
        &mut self,
        target: ConnectionTestTarget,
        captured_draft: RuntimeSettingsDraft,
    ) {
        if self.connection_worker.is_some() {
            return;
        }

        let applied_revision_at_start = self.settings.current().revision;
        let (sender, receiver) = mpsc::channel();
        match target {
            ConnectionTestTarget::Pexels => {
                let api_key = captured_draft.pexels_api_key.clone();
                thread::spawn(move || {
                    let result =
                        test_pexels_connection(&api_key).map_err(|error| error.to_string());
                    let _ = sender.send(result);
                });
            }
            ConnectionTestTarget::OmniVoice => {
                let base_url = captured_draft.omnivoice_url.clone();
                let token = Some(captured_draft.omnivoice_token.clone());
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
            captured_draft,
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
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(250));
                    None
                }
                Err(TryRecvError::Disconnected) => Some(Err(
                    "connection-test worker disconnected before returning a result".to_owned(),
                )),
            },
        };
        let Some(outcome) = outcome else {
            return;
        };

        let worker = self.connection_worker.take().expect("checked above");
        let target = worker.target;
        let applied_revision_at_start = worker.applied_revision_at_start;
        let quick_apply =
            self.omnivoice_quick_apply_pending && target == ConnectionTestTarget::OmniVoice;
        self.omnivoice_quick_apply_pending = false;

        match outcome {
            Ok(report) => {
                self.connection_status = format!(
                    "{} connection test passed. It used the captured draft; applied revision at start was {}.",
                    target.label(),
                    applied_revision_at_start
                );
                self.connection_status_is_error = false;
                self.last_connection_report = Some(report);

                if quick_apply {
                    match self.settings.apply(&worker.captured_draft) {
                        Ok(snapshot) => {
                            self.draft.omnivoice_url = snapshot.safe.omnivoice_url.clone();
                            self.omnivoice_quick_input = snapshot.safe.omnivoice_url.clone();
                            self.omnivoice_quick_status = format!(
                                "OmniVoice connection verified and saved as {}.",
                                snapshot.safe.omnivoice_url
                            );
                            self.omnivoice_quick_status_is_error = false;
                            self.show_omnivoice_quick_connect = false;
                        }
                        Err(error) => {
                            self.omnivoice_quick_status = format!(
                                "Connection passed, but the verified endpoint could not be saved: {error}"
                            );
                            self.omnivoice_quick_status_is_error = true;
                        }
                    }
                }
            }
            Err(error) => {
                self.connection_status =
                    format!("{} connection test failed: {error}", target.label());
                self.connection_status_is_error = true;
                self.last_connection_report = None;
                if quick_apply {
                    self.omnivoice_quick_status = format!(
                        "OmniVoice endpoint was not changed because the connection test failed: {error}"
                    );
                    self.omnivoice_quick_status_is_error = true;
                }
            }
        }
    }

    fn handle_dropped_vprep(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        let Some(path) = dropped
            .into_iter()
            .filter_map(|file| file.path)
            .find(|path| is_vprep_path(path))
        else {
            return;
        };

        self.load_vprep_into_create_form(path);
    }

    fn load_vprep_into_create_form(&mut self, path: PathBuf) {
        self.screen = Screen::Projects;
        self.show_create_project = true;
        self.create_script_path = path.to_string_lossy().into_owned();

        match fs::read_to_string(&path) {
            Ok(script_text) => {
                self.create_script_text = script_text;
                match crate::parse_script(self.create_script_text.trim()) {
                    Ok(script) => {
                        if self.create_project_id.trim().is_empty() {
                            self.create_project_id = slugify_project_id(&script.omnivoice.title);
                        }
                        self.project_status = format!(
                            "Loaded {} into the editor. Review it, then create the project when ready.",
                            path.display()
                        );
                        self.project_status_is_error = false;
                    }
                    Err(error) => {
                        self.project_status = format!(
                            "Loaded {}, but validation needs attention: {} · {error}",
                            path.display(),
                            error.code()
                        );
                        self.project_status_is_error = true;
                    }
                }
            }
            Err(error) => {
                self.project_status = format!(
                    "Could not read dropped .vprep file {}: {error}",
                    path.display()
                );
                self.project_status_is_error = true;
            }
        }
    }

    fn image_thumbnail(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
    ) -> Result<(egui::TextureHandle, u32, u32), String> {
        if !self.thumbnail_cache.contains_key(path) {
            const MAX_THUMBNAIL_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
            let result = (|| -> Result<(egui::TextureHandle, u32, u32), String> {
                let metadata = fs::metadata(path).map_err(|error| {
                    format!(
                        "Could not read image metadata for {}: {error}",
                        path.display()
                    )
                })?;
                if metadata.len() > MAX_THUMBNAIL_SOURCE_BYTES {
                    return Err(format!(
                        "Image preview skipped because {} is larger than 32 MB.",
                        path.display()
                    ));
                }

                let decoded = image::ImageReader::open(path)
                    .map_err(|error| format!("Could not open image {}: {error}", path.display()))?
                    .with_guessed_format()
                    .map_err(|error| {
                        format!(
                            "Could not detect image format for {}: {error}",
                            path.display()
                        )
                    })?
                    .decode()
                    .map_err(|error| {
                        format!("Could not decode image {}: {error}", path.display())
                    })?;
                let source_width = decoded.width();
                let source_height = decoded.height();
                let thumbnail = decoded.thumbnail(180, 104).to_rgba8();
                let size = [thumbnail.width() as usize, thumbnail.height() as usize];
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied(size, thumbnail.as_raw());
                let texture = ctx.load_texture(
                    format!("asset-thumbnail:{}", path.display()),
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                Ok((texture, source_width, source_height))
            })();

            let entry = match result {
                Ok((texture, source_width, source_height)) => ThumbnailCacheEntry::Ready {
                    texture,
                    source_width,
                    source_height,
                },
                Err(error) => ThumbnailCacheEntry::Failed(error),
            };
            self.thumbnail_cache.insert(path.to_path_buf(), entry);
        }

        match self.thumbnail_cache.get(path) {
            Some(ThumbnailCacheEntry::Ready {
                texture,
                source_width,
                source_height,
            }) => Ok((texture.clone(), *source_width, *source_height)),
            Some(ThumbnailCacheEntry::Failed(error)) => Err(error.clone()),
            None => Err("thumbnail cache entry disappeared unexpectedly".to_owned()),
        }
    }
}

fn open_asset_preview(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("Asset file does not exist: {}", path.display()));
    }

    #[cfg(target_os = "macos")]
    {
        return Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to open asset preview: {error}"));
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("cmd.exe")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to open asset preview: {error}"));
    }

    #[cfg(target_os = "linux")]
    {
        return Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to open asset preview: {error}"));
    }

    #[allow(unreachable_code)]
    Err("asset preview is not supported on this platform".to_owned())
}

fn reveal_asset_in_folder(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("Asset file does not exist: {}", path.display()));
    }

    #[cfg(target_os = "macos")]
    {
        return Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to reveal asset in Finder: {error}"));
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("explorer.exe")
            .arg("/select,")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to reveal asset in Explorer: {error}"));
    }

    #[cfg(target_os = "linux")]
    {
        let parent = path
            .parent()
            .ok_or_else(|| format!("asset has no parent folder: {}", path.display()))?;
        return Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to open asset folder: {error}"));
    }

    #[allow(unreachable_code)]
    Err("revealing an asset is not supported on this platform".to_owned())
}

fn open_folder(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("folder no longer exists: {}", path.display()));
    }

    #[cfg(target_os = "macos")]
    {
        return Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to open folder in Finder: {error}"));
    }

    #[cfg(target_os = "windows")]
    {
        return Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to open folder in Explorer: {error}"));
    }

    #[cfg(target_os = "linux")]
    {
        return Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("failed to open folder: {error}"));
    }

    #[allow(unreachable_code)]
    Err("opening a folder is not supported on this platform".to_owned())
}

fn pick_vprep_file() -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("osascript")
            .args([
                "-e",
                r#"POSIX path of (choose file with prompt "Choose .vprep script" of type {"vprep"})"#,
            ])
            .output()
            .map_err(|error| format!("failed to launch macOS file chooser: {error}"))?;
        if output.status.success() {
            return selected_folder_from_stdout(&output.stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("(-128)") {
            return Ok(None);
        }
        return Err(format!("macOS file chooser failed: {}", stderr.trim()));
    }

    #[cfg(target_os = "windows")]
    {
        const SCRIPT: &str = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = 'Choose .vprep script'
$dialog.Filter = 'Video Prepare scripts|*.vprep|All files|*.*'
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::Out.Write($dialog.FileName)
}
"#;
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", SCRIPT])
            .output()
            .map_err(|error| format!("failed to launch Windows file chooser: {error}"))?;
        if output.status.success() {
            return selected_folder_from_stdout(&output.stdout);
        }
        return Err(format!(
            "Windows file chooser failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    #[cfg(target_os = "linux")]
    {
        match Command::new("zenity")
            .args([
                "--file-selection",
                "--title=Choose .vprep script",
                "--file-filter=Video Prepare scripts | *.vprep",
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                return selected_folder_from_stdout(&output.stdout)
            }
            Ok(output) if output.status.code() == Some(1) => return Ok(None),
            Ok(_) => {}
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("failed to launch Linux file chooser: {error}"));
            }
            Err(_) => {}
        }
        match Command::new("kdialog")
            .args([
                "--getopenfilename",
                ".",
                "*.vprep|Video Prepare scripts",
                "--title",
                "Choose .vprep script",
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                return selected_folder_from_stdout(&output.stdout)
            }
            Ok(output) if output.status.code() == Some(1) => return Ok(None),
            Ok(output) => {
                return Err(format!(
                    "Linux file chooser failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(
                    "no supported system file chooser found (tried zenity and kdialog)".to_owned(),
                );
            }
            Err(error) => return Err(format!("failed to launch Linux file chooser: {error}")),
        }
    }

    #[allow(unreachable_code)]
    Err("system file chooser is not supported on this platform".to_owned())
}

fn pick_visual_asset_file() -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("osascript")
            .args([
                "-e",
                r#"POSIX path of (choose file with prompt "Choose local image or video")"#,
            ])
            .output()
            .map_err(|error| format!("failed to launch macOS file chooser: {error}"))?;
        if output.status.success() {
            return selected_folder_from_stdout(&output.stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("User canceled") || stderr.contains("(-128)") {
            return Ok(None);
        }
        return Err(format!("macOS file chooser failed: {}", stderr.trim()));
    }

    #[cfg(target_os = "windows")]
    {
        const SCRIPT: &str = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.OpenFileDialog
$dialog.Title = 'Choose local image or video'
$dialog.Filter = 'Media files|*.jpg;*.jpeg;*.png;*.webp;*.gif;*.mp4;*.mov;*.mkv;*.webm|All files|*.*'
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::Out.Write($dialog.FileName)
}
"#;
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", SCRIPT])
            .output()
            .map_err(|error| format!("failed to launch Windows file chooser: {error}"))?;
        if output.status.success() {
            return selected_folder_from_stdout(&output.stdout);
        }
        return Err(format!(
            "Windows file chooser failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    #[cfg(target_os = "linux")]
    {
        match Command::new("zenity")
            .args(["--file-selection", "--title=Choose local image or video"])
            .output()
        {
            Ok(output) if output.status.success() => {
                return selected_folder_from_stdout(&output.stdout)
            }
            Ok(output) if output.status.code() == Some(1) => return Ok(None),
            Ok(_) => {}
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("failed to launch Linux file chooser: {error}"));
            }
            Err(_) => {}
        }
        match Command::new("kdialog")
            .args([
                "--getopenfilename",
                ".",
                "--title",
                "Choose local image or video",
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                return selected_folder_from_stdout(&output.stdout)
            }
            Ok(output) if output.status.code() == Some(1) => return Ok(None),
            Ok(output) => {
                return Err(format!(
                    "Linux file chooser failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(
                    "no supported system file chooser found (tried zenity and kdialog)".to_owned(),
                );
            }
            Err(error) => return Err(format!("failed to launch Linux file chooser: {error}")),
        }
    }

    #[allow(unreachable_code)]
    Err("system file chooser is not supported on this platform".to_owned())
}

fn is_vprep_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vprep"))
}

fn format_duration(total_seconds: u64) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

fn normalize_project_id_input(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut separator_pending = false;

    for character in value.chars() {
        if character.is_whitespace() {
            separator_pending = !normalized.is_empty();
            continue;
        }

        if separator_pending && character != '-' && !normalized.ends_with('-') {
            normalized.push('-');
        }
        separator_pending = false;
        normalized.push(character);
    }

    normalized
}

fn slugify_project_id(title: &str) -> String {
    let mut slug = String::new();
    let mut separator_pending = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            if separator_pending && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(ch.to_ascii_lowercase());
            separator_pending = false;
        } else if !slug.is_empty() {
            separator_pending = true;
        }
    }
    if slug.is_empty() {
        "video-project".to_owned()
    } else {
        slug
    }
}

fn pick_data_root_folder(current: &str) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = current;
        return pick_data_root_folder_macos();
    }

    #[cfg(target_os = "windows")]
    {
        return pick_data_root_folder_windows(current);
    }

    #[cfg(target_os = "linux")]
    {
        return pick_data_root_folder_linux(current);
    }

    #[allow(unreachable_code)]
    Err("system folder picker is not supported on this platform".to_owned())
}

#[cfg(target_os = "macos")]
fn pick_data_root_folder_macos() -> Result<Option<PathBuf>, String> {
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"POSIX path of (choose folder with prompt "Choose Video Prepare Data Root")"#,
        ])
        .output()
        .map_err(|error| format!("failed to launch macOS folder chooser: {error}"))?;

    if output.status.success() {
        return selected_folder_from_stdout(&output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("User canceled") || stderr.contains("(-128)") {
        Ok(None)
    } else {
        Err(format!("macOS folder chooser failed: {}", stderr.trim()))
    }
}

#[cfg(target_os = "windows")]
fn pick_data_root_folder_windows(current: &str) -> Result<Option<PathBuf>, String> {
    const SCRIPT: &str = r#"
Add-Type -AssemblyName System.Windows.Forms
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog
$dialog.Description = 'Choose Video Prepare Data Root'
$dialog.ShowNewFolderButton = $true
if ($env:VIDEO_PREPARE_INITIAL_ROOT -and (Test-Path -LiteralPath $env:VIDEO_PREPARE_INITIAL_ROOT)) {
    $dialog.SelectedPath = $env:VIDEO_PREPARE_INITIAL_ROOT
}
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {
    [Console]::Out.Write($dialog.SelectedPath)
}
"#;

    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-Command", SCRIPT])
        .env("VIDEO_PREPARE_INITIAL_ROOT", current)
        .output()
        .map_err(|error| format!("failed to launch Windows folder browser: {error}"))?;

    if output.status.success() {
        selected_folder_from_stdout(&output.stdout)
    } else {
        Err(format!(
            "Windows folder browser failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(target_os = "linux")]
fn pick_data_root_folder_linux(current: &str) -> Result<Option<PathBuf>, String> {
    let mut zenity = Command::new("zenity");
    zenity.args([
        "--file-selection",
        "--directory",
        "--title=Choose Video Prepare Data Root",
    ]);
    if !current.trim().is_empty() {
        zenity.arg(format!("--filename={}/", current.trim_end_matches('/')));
    }

    match zenity.output() {
        Ok(output) if output.status.success() => {
            return selected_folder_from_stdout(&output.stdout)
        }
        Ok(output) if output.status.code() == Some(1) => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("failed to launch Linux folder browser: {error}"));
        }
        Err(_) => {}
    }

    let initial = if current.trim().is_empty() {
        "."
    } else {
        current
    };
    match Command::new("kdialog")
        .args([
            "--getexistingdirectory",
            initial,
            "--title",
            "Choose Video Prepare Data Root",
        ])
        .output()
    {
        Ok(output) if output.status.success() => selected_folder_from_stdout(&output.stdout),
        Ok(output) if output.status.code() == Some(1) => Ok(None),
        Ok(output) => Err(format!(
            "Linux folder browser failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err("no supported system folder browser found (tried zenity and kdialog)".to_owned())
        }
        Err(error) => Err(format!("failed to launch Linux folder browser: {error}")),
    }
}

fn selected_folder_from_stdout(stdout: &[u8]) -> Result<Option<PathBuf>, String> {
    let selected = String::from_utf8(stdout.to_vec())
        .map_err(|error| format!("folder browser returned invalid UTF-8: {error}"))?;
    let selected = selected.trim();
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(selected)))
    }
}

pub fn run_desktop() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([840.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Video Prepare",
        options,
        Box::new(|creation_context| {
            configure_style(&creation_context.egui_ctx);
            Ok(Box::new(VideoPrepareApp::default()))
        }),
    )
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 7.0);
    style.spacing.interact_size = egui::vec2(40.0, 32.0);
    ctx.set_style(style);
}

fn nav_button(ui: &mut egui::Ui, screen: &mut Screen, target: Screen, label: &str) {
    if ui
        .selectable_label(*screen == target, egui::RichText::new(label).size(15.0))
        .clicked()
    {
        *screen = target;
    }
}

fn status_badge(ui: &mut egui::Ui, label: &str, state: crate::TaskState) {
    let color = match state {
        crate::TaskState::Completed => egui::Color32::from_rgb(74, 222, 128),
        crate::TaskState::Failed => egui::Color32::from_rgb(248, 113, 113),
        crate::TaskState::Running
        | crate::TaskState::Partial
        | crate::TaskState::Interrupted
        | crate::TaskState::UnknownRemote => egui::Color32::from_rgb(250, 204, 21),
        crate::TaskState::Pending | crate::TaskState::Skipped => egui::Color32::from_gray(160),
    };
    ui.label(
        egui::RichText::new(format!("{label}: {}", state_text(state)))
            .monospace()
            .color(color),
    );
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

#[cfg(test)]
mod ui_logic_tests {
    use super::*;

    #[test]
    fn quick_connection_parser_is_available_to_desktop_flow() {
        let startup = "PUBLIC STUDIO UI: https://demo.example/ui\nPublic REST API: https://demo.example/api/v1\nPublic MCP: https://demo.example/mcp";
        assert_eq!(
            extract_omnivoice_url(startup).unwrap(),
            "https://demo.example"
        );
    }

    #[test]
    fn smart_workspace_action_prefers_remote_reconciliation_when_available() {
        assert_eq!(
            recommended_workspace_action(
                crate::TaskState::Partial,
                crate::TaskState::UnknownRemote,
                true,
            ),
            WorkspaceAction::CheckAudio
        );
    }

    #[test]
    fn workspace_action_maps_project_states_to_one_primary_action() {
        assert_eq!(
            recommended_workspace_action(
                crate::TaskState::Pending,
                crate::TaskState::Pending,
                false,
            ),
            WorkspaceAction::Start
        );
        assert_eq!(
            recommended_workspace_action(
                crate::TaskState::Partial,
                crate::TaskState::Completed,
                false,
            ),
            WorkspaceAction::Resume
        );
        assert_eq!(
            recommended_workspace_action(crate::TaskState::Failed, crate::TaskState::Failed, false,),
            WorkspaceAction::RetryFailed
        );
        assert_eq!(
            recommended_workspace_action(
                crate::TaskState::Completed,
                crate::TaskState::Completed,
                false,
            ),
            WorkspaceAction::Ready
        );
    }

    #[test]
    fn project_id_whitespace_is_normalized_to_hyphens() {
        assert_eq!(
            normalize_project_id_input("HashMap Deep Dive"),
            "HashMap-Deep-Dive"
        );
        assert_eq!(
            normalize_project_id_input("  HashMap   Deep\tDive  "),
            "HashMap-Deep-Dive"
        );
        assert_eq!(
            normalize_project_id_input("HashMap - Deep Dive"),
            "HashMap-Deep-Dive"
        );
    }

    #[test]
    fn byte_labels_are_human_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MB");
    }

    #[test]
    fn drag_drop_accepts_vprep_extension_case_insensitively() {
        assert!(is_vprep_path(Path::new("script.vprep")));
        assert!(is_vprep_path(Path::new("SCRIPT.VPREP")));
        assert!(!is_vprep_path(Path::new("script.yaml")));
    }

    #[test]
    fn timeline_duration_is_compact_and_readable() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(65), "1:05");
        assert_eq!(format_duration(3661), "1:01:01");
    }
}

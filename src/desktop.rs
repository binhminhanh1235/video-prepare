use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
};

use eframe::egui;

use crate::{
    create_project_from_script_path, create_project_from_script_text, discover_projects,
    execute_flow_retry, execute_project_run, import_manual_visual_asset, inspect_project,
    load_audio_status, open_project_from_data_root, reconcile_remote_audio,
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

struct ManualVisualWorker {
    receiver: Receiver<Result<ManualVisualImportSummary, String>>,
    project_id: String,
    data_root: PathBuf,
    scene_id: String,
    visual_id: String,
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
    manual_visual_path: String,
    manual_visual_worker: Option<ManualVisualWorker>,
    last_manual_visual_import: Option<ManualVisualImportSummary>,
    manual_visual_status: String,
    manual_visual_status_is_error: bool,
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
            create_script_text: String::new(),
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
            manual_visual_path: String::new(),
            manual_visual_worker: None,
            last_manual_visual_import: None,
            manual_visual_status: String::new(),
            manual_visual_status_is_error: false,
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

        egui::TopBottomPanel::top("top-nav").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("Video Prepare").size(22.0));
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(4.0);
                nav_button(ui, &mut self.screen, Screen::Projects, "Projects");
                nav_button(ui, &mut self.screen, Screen::Dashboard, "Dashboard");
                nav_button(ui, &mut self.screen, Screen::Scene, "Scene");
                nav_button(ui, &mut self.screen, Screen::Settings, "Settings");
            });
            ui.add_space(6.0);
        });

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
                ui.heading("Projects");
                ui.label(
                    egui::RichText::new(
                        "Start from a pasted script, then prepare visual and audio assets from one workspace.",
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

                ui.add_space(12.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.heading(egui::RichText::new("Create a project").size(20.0));
                    ui.label(
                        egui::RichText::new(
                            "Paste is the preferred input. File import stays available as a fallback.",
                        )
                        .weak(),
                    );
                    ui.add_space(10.0);

                    ui.label(egui::RichText::new("Project ID").strong());
                    ui.add_sized(
                        [420.0, 34.0],
                        egui::TextEdit::singleline(&mut self.create_project_id)
                            .hint_text("e.g. hashmap-deep-dive"),
                    );

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

                    ui.add_space(8.0);
                    ui.collapsing("Import a .vprep file instead", |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Optional fallback for scripts already saved on disk.",
                            )
                            .weak(),
                        );
                        ui.add_sized(
                            [ui.available_width(), 34.0],
                            egui::TextEdit::singleline(&mut self.create_script_path)
                                .hint_text("/path/to/script.vprep"),
                        );
                    });

                    let pasted_ready = !self.create_script_text.trim().is_empty();
                    let file_ready = !self.create_script_path.trim().is_empty();
                    let id_ready = !self.create_project_id.trim().is_empty();
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
                            id_ready && (pasted_ready || file_ready),
                            egui::Button::new(egui::RichText::new("Create project").strong()),
                        )
                        .clicked()
                    {
                        self.create_project_from_script();
                    }
                });

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
                } else {
                    let projects = self.catalog.projects.clone();
                    for project in projects {
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
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} scene{}",
                                            project.scene_count,
                                            if project.scene_count == 1 { "" } else { "s" }
                                        ))
                                        .weak(),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("Open project").clicked() {
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
      ui.heading(egui::RichText::new("Dashboard").size(24.0));
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

      ui.add_space(12.0);
      let worker_active = self.mutation_worker_active();
      let remote_available = self.remote_reconciliation_available(true);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(egui::RichText::new("Project actions").strong().size(18.0));
          ui.label(
              egui::RichText::new(
                  "Run all enabled flows, continue interrupted work, or retry failed work without leaving this screen.",
              )
              .weak(),
          );
          ui.add_space(8.0);
          ui.horizontal_wrapped(|ui| {
              if ui
                  .add_enabled(!worker_active, egui::Button::new("Run project"))
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
                  .add_enabled(!worker_active, egui::Button::new("Retry failed"))
                  .clicked()
              {
                  self.start_run(RunAction::RetryFailed);
              }
              if ui
                  .add_enabled(
                      !worker_active && remote_available,
                      egui::Button::new("Reconcile remote audio"),
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
              if worker_active {
                  ui.spinner();
                  ui.label(egui::RichText::new("Background work in progress").weak());
              }
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
              for problem in &inspection.problems {
                  ui.horizontal_wrapped(|ui| {
                      status_badge(ui, &problem.area, problem.state);
                      ui.strong(&problem.scope);
                      ui.label(&problem.message);
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
              if ui.button("Go to Dashboard").clicked() {
                  self.screen = Screen::Dashboard;
              }
          });
          return;
      };
      let Some(scene_id) = self.selected_scene_id.clone() else {
          egui::Frame::group(ui.style()).show(ui, |ui| {
              ui.set_min_width(ui.available_width());
              ui.label("Choose a scene from the Dashboard first.");
              if ui.button("Go to Dashboard").clicked() {
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
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal_wrapped(|ui| {
              if ui.button("Back to Dashboard").clicked() {
                  self.screen = Screen::Dashboard;
              }
              if ui
                  .add_enabled(!mutation_busy, egui::Button::new("Refresh from disk"))
                  .clicked()
              {
                  self.reload_selected_project();
              }
              if mutation_busy {
                  ui.spinner();
                  ui.label(egui::RichText::new("Background work in progress").weak());
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

      ui.add_space(12.0);
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.label(
              egui::RichText::new("Manual visual takeover")
                  .strong()
                  .size(18.0),
          );
          ui.label(
              egui::RichText::new(
                  "Provide a local image or video when a stock request cannot be satisfied. The original absolute path is not persisted.",
              )
              .weak(),
          );
          ui.add_space(8.0);
          ui.label(egui::RichText::new("Local image / video file").strong());
          ui.add_sized(
              [ui.available_width(), 34.0],
              egui::TextEdit::singleline(&mut self.manual_visual_path)
                  .hint_text("/path/to/local/asset.mp4"),
          );
          if let Some(worker) = &self.manual_visual_worker {
              ui.add_space(6.0);
              ui.label(format!(
                  "Importing {} / {} for project `{}`...",
                  worker.scene_id, worker.visual_id, worker.project_id
              ));
          }
          if !self.manual_visual_status.is_empty() {
              if self.manual_visual_status_is_error {
                  ui.colored_label(
                      ui.visuals().error_fg_color,
                      &self.manual_visual_status,
                  );
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

      ui.add_space(16.0);
      ui.heading(egui::RichText::new("Visual requests").size(20.0));
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
                  if !request.attempted_queries.is_empty() {
                      ui.small(format!(
                          "Attempted queries: {}",
                          request.attempted_queries.join(" | ")
                      ));
                  }
                  if let Some(query) = &request.successful_query {
                      ui.small(format!("Successful query: {query}"));
                  }
                  if let Some(error) = &request.last_error {
                      ui.colored_label(ui.visuals().error_fg_color, error);
                  }
                  if request.completed_assets < target {
                      ui.add_space(6.0);
                      let source_ready = !self.manual_visual_path.trim().is_empty();
                      if ui
                          .add_enabled(
                              !mutation_busy && source_ready,
                              egui::Button::new(format!(
                                  "Use local file for {}",
                                  request.id
                              )),
                          )
                          .clicked()
                      {
                          self.start_manual_visual_import(&scene.id, &request.id);
                      }
                      if !source_ready {
                          ui.label(
                              egui::RichText::new(
                                  "Choose a local file above to enable manual takeover.",
                              )
                              .weak(),
                          );
                      }
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
                      ui.strong(&attempt.attempt_id);
                      status_badge(ui, "Status", attempt.state);
                  });
                  ui.small(format!("Server: {}", attempt.server_base_url));
                  ui.small(format!(
                      "Remote project: {}",
                      attempt.remote_project_id
                  ));
                  ui.small(format!(
                      "Job: {}",
                      attempt.job_id.as_deref().unwrap_or("not assigned")
                  ));
                  if let Some(error) = &attempt.last_error {
                      ui.colored_label(ui.visuals().error_fg_color, error);
                  }
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
      ui.add_space(8.0);
      ui.horizontal_wrapped(|ui| {
          ui.label(
              egui::RichText::new(format!(
                  "Applied revision {}",
                  self.settings.current().revision
              ))
              .monospace(),
          );
          ui.label(
              egui::RichText::new(
                  "Changes below remain a draft until you click Apply settings.",
              )
              .weak(),
          );
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
          ui.add_sized(
              [ui.available_width(), 34.0],
              egui::TextEdit::singleline(&mut self.draft.data_root)
                  .hint_text("video-prepare-data"),
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
              ui.label(egui::RichText::new("Download concurrency").strong());
              ui.add(
                  egui::DragValue::new(&mut self.draft.download_concurrency)
                      .range(1..=32),
              );
              if ui
                  .add_enabled(!connection_busy, egui::Button::new("Test Pexels"))
                  .clicked()
              {
                  self.start_connection_test(ConnectionTestTarget::Pexels);
              }
              if connection_busy {
                  ui.spinner();
              }
          });
          ui.small("Connection tests use a captured copy of the draft. Secrets stay memory-only.");
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
          ui.add_space(6.0);
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
              if ui
                  .add_enabled(!connection_busy, egui::Button::new("Test OmniVoice"))
                  .clicked()
              {
                  self.start_connection_test(ConnectionTestTarget::OmniVoice);
              }
              if connection_busy {
                  ui.spinner();
              }
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
              if ui.button("Discard draft").clicked() {
                  self.draft = self.settings.draft();
                  self.status =
                      "Draft restored from the last applied runtime snapshot.".to_owned();
                  self.status_is_error = false;
              }
              if ui
                  .button(egui::RichText::new("Apply settings").strong())
                  .clicked()
              {
                  let previous_root = self.settings.current().safe.data_root.clone();
                  match self.settings.apply(&self.draft) {
                      Ok(snapshot) => {
                          self.draft = RuntimeSettingsDraft::from_snapshot(&snapshot);
                          self.status = format!(
                              "Applied runtime settings revision {}.",
                              snapshot.revision
                          );
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
                      "Applying updates the in-memory runtime snapshot used by new work.",
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

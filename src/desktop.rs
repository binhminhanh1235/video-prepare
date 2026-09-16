use std::path::PathBuf;

use eframe::egui;

use crate::{
    create_project_from_script_path, discover_projects, open_project_from_data_root, ProjectCatalog,
    QualityPreset, RuntimeSettingsDraft, RuntimeSettingsStore, StoredProject,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Projects,
    Dashboard,
    Scene,
    Settings,
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
    create_project_id: String,
    create_script_path: String,
    project_status: String,
    project_status_is_error: bool,
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
            create_project_id: String::new(),
            create_script_path: String::new(),
            project_status: String::new(),
            project_status_is_error: false,
        };
        app.refresh_projects();
        app
    }
}

impl eframe::App for VideoPrepareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            Screen::Scene => placeholder(
                ui,
                "Scene detail",
                "Scene-level assets, audio attempts and retry controls are not wired yet.",
            ),
            Screen::Settings => self.settings_ui(ui),
        });
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
        let Some(project) = self.selected_project.as_ref() else {
            ui.label("No project selected. Open a project from Projects first.");
            if ui.button("Go to Projects").clicked() {
                self.screen = Screen::Projects;
            }
            return;
        };

        ui.strong(&project.metadata.title);
        ui.label(format!("Project ID: {}", project.metadata.project_id));
        ui.label(format!("Root: {}", project.root.display()));
        ui.label(format!("Overall: {:?}", project.status.overall));
        ui.label(format!("Visual Flow: {:?}", project.status.visual_flow));
        ui.label(format!("Audio Flow: {:?}", project.status.audio_flow));
        ui.label(format!("Scenes: {}", project.status.scenes.len()));
        ui.add_space(8.0);
        ui.label("Run, Resume and Retry orchestration is intentionally not wired in P4.02.");
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Runtime Settings");
        ui.label(format!(
            "Applied revision: {}",
            self.settings.current().revision
        ));
        ui.add_space(8.0);

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
            ui.small("Secrets remain memory-only. Test Connection is wired in a later P4 task.");
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
        });

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
                self.selected_project = Some(project);
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
                self.selected_project = Some(project);
                self.screen = Screen::Dashboard;
            }
            Err(error) => {
                self.project_status = format!("Project open failed: {error}");
                self.project_status_is_error = true;
                self.refresh_projects();
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

fn placeholder(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.heading(title);
    ui.label(detail);
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.text_edit_singleline(value);
    });
}

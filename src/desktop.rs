use eframe::egui;

use crate::{QualityPreset, RuntimeSettingsDraft, RuntimeSettingsStore};

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
}

impl Default for VideoPrepareApp {
    fn default() -> Self {
        let settings = RuntimeSettingsStore::default();
        let draft = settings.draft();
        Self {
            settings,
            draft,
            screen: Screen::Settings,
            status: "Runtime settings are memory-only until applied.".to_owned(),
            status_is_error: false,
        }
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
            Screen::Projects => placeholder(
                ui,
                "Project list",
                "Project create/open wiring starts in P4.02.",
            ),
            Screen::Dashboard => placeholder(
                ui,
                "Project dashboard",
                "Run, Resume and Retry actions are not wired in P4.01.",
            ),
            Screen::Scene => placeholder(
                ui,
                "Scene detail",
                "Scene-level assets, audio attempts and retry controls are not wired in P4.01.",
            ),
            Screen::Settings => self.settings_ui(ui),
        });
    }
}

impl VideoPrepareApp {
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
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                self.draft = self.settings.draft();
                self.status = "Draft restored from the last applied runtime snapshot.".to_owned();
                self.status_is_error = false;
            }
            if ui.button("Apply").clicked() {
                match self.settings.apply(&self.draft) {
                    Ok(snapshot) => {
                        self.draft = RuntimeSettingsDraft::from_snapshot(&snapshot);
                        self.status =
                            format!("Applied runtime settings revision {}.", snapshot.revision);
                        self.status_is_error = false;
                    }
                    Err(error) => {
                        self.status = error.to_string();
                        self.status_is_error = true;
                    }
                }
            }
        });

        ui.add_space(8.0);
        if self.status_is_error {
            ui.colored_label(ui.visuals().error_fg_color, &self.status);
        } else {
            ui.label(&self.status);
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

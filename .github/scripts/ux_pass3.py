from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing patch anchor: {label}")
    return text.replace(old, new, 1)


# settings.rs: accept either a direct OmniVoice URL or the full Studio startup output.
path = Path("src/settings.rs")
text = path.read_text()
text = replace_once(
    text,
    '''pub fn normalize_omnivoice_url(input: &str, required: bool) -> Result<String, SettingsError> {
    let raw = input.trim();''',
    '''pub fn extract_omnivoice_url(input: &str) -> Result<String, SettingsError> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(SettingsError::MissingOmniVoiceUrl);
    }

    if !raw.contains('\\n') && !raw.contains('\\r') {
        if let Ok(url) = normalize_omnivoice_url(raw, true) {
            return Ok(url);
        }
    }

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some((label, value)) = trimmed.split_once(':') {
            if label.trim().eq_ignore_ascii_case("Public REST API") {
                return normalize_omnivoice_url(value.trim(), true);
            }
        }
    }

    for token in raw.split_whitespace() {
        let candidate = token.trim_matches(|ch: char| {
            matches!(ch, '`' | '\"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']')
        });
        if (candidate.starts_with("http://") || candidate.starts_with("https://"))
            && candidate
                .trim_end_matches('/')
                .to_ascii_lowercase()
                .ends_with("/api/v1")
        {
            if let Ok(url) = normalize_omnivoice_url(candidate, true) {
                return Ok(url);
            }
        }
    }

    Err(SettingsError::InvalidOmniVoiceUrl)
}

pub fn normalize_omnivoice_url(input: &str, required: bool) -> Result<String, SettingsError> {
    let raw = input.trim();''',
    "omnivoice startup-output parser",
)
text = replace_once(
    text,
    '''    #[test]
    fn public_rest_api_url_is_accepted_and_normalized_to_service_root() {
        assert_eq!(
            normalize_omnivoice_url("https://studio.example/api/v1", true).unwrap(),
            "https://studio.example"
        );
        assert_eq!(
            normalize_omnivoice_url("https://studio.example/api/v1/", true).unwrap(),
            "https://studio.example"
        );
    }
''',
    '''    #[test]
    fn public_rest_api_url_is_accepted_and_normalized_to_service_root() {
        assert_eq!(
            normalize_omnivoice_url("https://studio.example/api/v1", true).unwrap(),
            "https://studio.example"
        );
        assert_eq!(
            normalize_omnivoice_url("https://studio.example/api/v1/", true).unwrap(),
            "https://studio.example"
        );
    }

    #[test]
    fn omnivoice_quick_connection_extracts_public_rest_api_from_startup_output() {
        let startup = r#"PUBLIC STUDIO UI: https://neo-station.example/ui
Public REST API: https://neo-station.example/api/v1
Public MCP: https://neo-station.example/mcp"#;
        assert_eq!(
            extract_omnivoice_url(startup).unwrap(),
            "https://neo-station.example"
        );
    }

    #[test]
    fn omnivoice_quick_connection_accepts_direct_service_or_rest_url() {
        assert_eq!(
            extract_omnivoice_url("https://studio.example").unwrap(),
            "https://studio.example"
        );
        assert_eq!(
            extract_omnivoice_url("https://studio.example/api/v1").unwrap(),
            "https://studio.example"
        );
    }

    #[test]
    fn omnivoice_quick_connection_does_not_mistake_ui_or_mcp_url_for_rest_api() {
        let startup = r#"PUBLIC STUDIO UI: https://neo-station.example/ui
Public MCP: https://neo-station.example/mcp"#;
        assert_eq!(
            extract_omnivoice_url(startup),
            Err(SettingsError::InvalidOmniVoiceUrl)
        );
    }
''',
    "omnivoice parser tests",
)
path.write_text(text)


# lib.rs: export the parser for desktop UX.
path = Path("src/lib.rs")
text = path.read_text()
text = replace_once(
    text,
    '''pub use settings::{
    normalize_omnivoice_url, QualityPreset, RuntimeSecrets, RuntimeSettingsDraft,''',
    '''pub use settings::{
    extract_omnivoice_url, normalize_omnivoice_url, QualityPreset, RuntimeSecrets, RuntimeSettingsDraft,''',
    "export omnivoice extractor",
)
path.write_text(text)


# desktop.rs: quick connection, cleaner settings, and richer media-first asset cards.
path = Path("src/desktop.rs")
text = path.read_text()
text = replace_once(
    text,
    '''    create_project_from_script_path, create_project_from_script_text, discover_projects,
    execute_flow_retry, execute_project_run, import_manual_visual_asset, inspect_project,''',
    '''    create_project_from_script_path, create_project_from_script_text, discover_projects,
    execute_flow_retry, execute_project_run, extract_omnivoice_url, import_manual_visual_asset, inspect_project,''',
    "desktop omnivoice extractor import",
)
text = replace_once(
    text,
    '''struct ConnectionWorker {
    receiver: Receiver<Result<ConnectionTestReport, String>>,
    target: ConnectionTestTarget,
    applied_revision_at_start: u64,
}''',
    '''struct ConnectionWorker {
    receiver: Receiver<Result<ConnectionTestReport, String>>,
    target: ConnectionTestTarget,
    applied_revision_at_start: u64,
    captured_draft: RuntimeSettingsDraft,
}''',
    "captured connection draft",
)
text = replace_once(
    text,
    '''    connection_status: String,
    connection_status_is_error: bool,
    manual_visual_path: String,''',
    '''    connection_status: String,
    connection_status_is_error: bool,
    show_omnivoice_quick_connect: bool,
    omnivoice_quick_input: String,
    omnivoice_quick_status: String,
    omnivoice_quick_status_is_error: bool,
    omnivoice_quick_apply_pending: bool,
    manual_visual_path: String,''',
    "quick connection fields",
)
text = replace_once(
    text,
    '''            connection_status: String::new(),
            connection_status_is_error: false,
            manual_visual_path: String::new(),''',
    '''            connection_status: String::new(),
            connection_status_is_error: false,
            show_omnivoice_quick_connect: false,
            omnivoice_quick_input: String::new(),
            omnivoice_quick_status: String::new(),
            omnivoice_quick_status_is_error: false,
            omnivoice_quick_apply_pending: false,
            manual_visual_path: String::new(),''',
    "quick connection defaults",
)

# Workspace quick connection card, directly before Next step.
text = replace_once(
    text,
    '''      ui.add_space(12.0);
      let worker_active = self.mutation_worker_active();
      let remote_available = self.remote_reconciliation_available(true);''',
    '''      ui.add_space(12.0);
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
      let remote_available = self.remote_reconciliation_available(true);''',
    "workspace omnivoice quick connection",
)

# Richer media-first local asset tile without pulling in a video/image decoding dependency.
old_asset_card = '''                              let file_name = asset
                                  .absolute_path
                                  .file_name()
                                  .and_then(|name| name.to_str())
                                  .unwrap_or("local asset");
                              ui.horizontal_wrapped(|ui| {
                                  ui.strong(file_name);
                                  ui.label(
                                      egui::RichText::new(format!(
                                          "Slot {} · {:?} · {}",
                                          asset.slot,
                                          asset.kind,
                                          format_bytes(asset.bytes)
                                      ))
                                      .weak(),
                                  );
                              });
                              ui.horizontal_wrapped(|ui| {
                                  if ui.button("Preview").clicked() {
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
                              });'''
new_asset_card = '''                              let file_name = asset
                                  .absolute_path
                                  .file_name()
                                  .and_then(|name| name.to_str())
                                  .unwrap_or("local asset");
                              let (media_icon, media_label) = match asset.kind {
                                  crate::PersistedAssetKind::Image => ("▣", "IMAGE"),
                                  crate::PersistedAssetKind::Video => ("▶", "VIDEO"),
                              };
                              ui.horizontal(|ui| {
                                  egui::Frame::group(ui.style()).show(ui, |ui| {
                                      ui.set_min_size(egui::vec2(118.0, 74.0));
                                      ui.vertical_centered(|ui| {
                                          ui.add_space(6.0);
                                          ui.label(egui::RichText::new(media_icon).size(28.0));
                                          ui.label(egui::RichText::new(media_label).strong().small());
                                      });
                                  });
                                  ui.vertical(|ui| {
                                      ui.strong(file_name);
                                      ui.label(
                                          egui::RichText::new(format!(
                                              "Slot {} · {}",
                                              asset.slot,
                                              format_bytes(asset.bytes)
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
                              });'''
text = replace_once(text, old_asset_card, new_asset_card, "media-first local asset tile")

# Settings header: user-facing saved/dirty state first, implementation details collapsed.
old_settings_header = '''      ui.add_space(8.0);
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
      if let Some(path) = self.settings.persistence_path() {
          if self.settings.loaded_from_disk() {
              ui.small(format!("Loaded from: {}", path.display()));
          } else {
              ui.small(format!("Will save to: {}", path.display()));
          }
      }
      ui.small("Data Root, flow toggles, provider URL, voice options, quality, and concurrency are restored on next launch.");
      if self.settings.system_secret_persistence_enabled() {
          if self.settings.secrets_restored() {
              ui.small("Pexels API Key and OmniVoice API Token were restored from the OS credential store.");
          } else {
              ui.small("Pexels API Key and OmniVoice API Token are saved in the OS credential store when you Apply settings.");
          }
      } else {
          ui.small("Secure API-key persistence is unavailable on this platform; secrets remain session-only.");
      }
      if let Some(warning) = self.settings.secret_persistence_warning() {
          ui.colored_label(ui.visuals().warn_fg_color, warning);
      }

      let connection_busy = self.connection_worker.is_some();'''
new_settings_header = '''      let draft_dirty = self.draft != self.settings.draft();
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

      let connection_busy = self.connection_worker.is_some();'''
text = replace_once(text, old_settings_header, new_settings_header, "settings progressive diagnostics")

# Pexels: keep common task visible, move tuning into advanced disclosure.
old_pexels_controls = '''          ui.add_space(8.0);
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
          ui.small("Connection tests use a captured copy of the draft. The API key is never written to preferences.json; on macOS/Windows it is stored in the OS credential store after Apply settings.");'''
new_pexels_controls = '''          ui.add_space(8.0);
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
          ui.small("The API key is kept out of preferences.json and uses the OS credential store on macOS/Windows.");'''
text = replace_once(text, old_pexels_controls, new_pexels_controls, "simplify pexels settings")

# OmniVoice: keep connection fields visible, narration knobs under one disclosure.
old_omnivoice_options = '''          ui.add_space(6.0);
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
          });'''
new_omnivoice_options = '''          ui.add_space(8.0);
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
          });'''
text = replace_once(text, old_omnivoice_options, new_omnivoice_options, "collapse narration defaults")

# Settings footer: clear dirty state and conventional Save / Discard wording.
text = text.replace('"Data Root selected. Click Apply settings to save it."', '"Data Root selected. Save settings to use it."')
old_footer_buttons = '''          ui.horizontal_wrapped(|ui| {
              if ui.button("Discard draft").clicked() {
                  self.draft = self.settings.draft();
                  self.status =
                      "Draft restored from the last applied runtime snapshot.".to_owned();
                  self.status_is_error = false;
              }
              if ui
                  .button(egui::RichText::new("Apply settings").strong())
                  .clicked()
              {'''
new_footer_buttons = '''          ui.horizontal_wrapped(|ui| {
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
              {'''
text = replace_once(text, old_footer_buttons, new_footer_buttons, "save settings footer")
text = text.replace(
    '"Applying updates the runtime snapshot and saves non-secret preferences for the next launch."',
    '"Saving updates the active runtime configuration and persists non-secret preferences for the next launch."',
)

# Connection testing now retains the exact draft used so quick-connect can safely persist only a tested snapshot.
old_start_connection = '''    fn start_connection_test(&mut self, target: ConnectionTestTarget) {
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
    }'''
new_start_connection = '''    fn start_connection_test(&mut self, target: ConnectionTestTarget) {
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
                self.omnivoice_quick_status = format!("Could not detect OmniVoice REST endpoint: {error}");
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
        let quick_apply = self.omnivoice_quick_apply_pending
            && target == ConnectionTestTarget::OmniVoice;
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
                            self.draft = RuntimeSettingsDraft::from_snapshot(&snapshot);
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
    }'''
text = replace_once(text, old_start_connection, new_start_connection, "safe quick-connect worker flow")

# Regression tests for UI-independent smart behavior remain colocated with desktop helpers.
text = replace_once(
    text,
    '''    fn smart_workspace_action_prefers_remote_reconciliation_when_available() {''',
    '''    fn quick_connection_parser_is_available_to_desktop_flow() {
        let startup = "PUBLIC STUDIO UI: https://demo.example/ui\\nPublic REST API: https://demo.example/api/v1\\nPublic MCP: https://demo.example/mcp";
        assert_eq!(
            extract_omnivoice_url(startup).unwrap(),
            "https://demo.example"
        );
    }

    #[test]
    fn smart_workspace_action_prefers_remote_reconciliation_when_available() {''',
    "desktop quick parser regression",
)

path.write_text(text)

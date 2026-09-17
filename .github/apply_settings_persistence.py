from pathlib import Path

p = Path("src/desktop.rs")
s = p.read_text()

replacements = [
    (
        '''        let settings = RuntimeSettingsStore::default();
        let draft = settings.draft();
        let catalog_data_root = settings.current().safe.data_root.clone();
        let mut app = Self {
            settings,
            draft,
            screen: Screen::Projects,
            status: "Runtime settings are memory-only until applied.".to_owned(),
            status_is_error: false,
''',
        '''        let (settings, load_warning) = RuntimeSettingsStore::load_persistent_or_default();
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
                Some(path) => (
                    format!(
                        "Settings persistence is enabled at {}. API keys and tokens remain session-only.",
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
''',
    ),
    (
        '''      ui.horizontal_wrapped(|ui| {
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
''',
        '''      ui.horizontal_wrapped(|ui| {
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
          ui.small(format!("Saved preferences: {}", path.display()));
      }
      ui.small("Data Root, flow toggles, provider URL, voice options, quality, and concurrency are restored on next launch. API keys and tokens are intentionally session-only.");
''',
    ),
    (
        '          ui.small("Connection tests use a captured copy of the draft. Secrets stay memory-only.");\n',
        '          ui.small("Connection tests use a captured copy of the draft. The API key is session-only and is never written to the preferences file.");\n',
    ),
    (
        '''                          self.status = format!(
                              "Applied runtime settings revision {}.",
                              snapshot.revision
                          );
''',
        '''                          self.status = match self.settings.persistence_path() {
                              Some(path) => format!(
                                  "Applied runtime settings revision {} and saved preferences to {}. API keys and tokens remain session-only.",
                                  snapshot.revision,
                                  path.display()
                              ),
                              None => format!(
                                  "Applied runtime settings revision {}. Persistent storage is unavailable.",
                                  snapshot.revision
                              ),
                          };
''',
    ),
    (
        '''              ui.label(
                  egui::RichText::new(
                      "Applying updates the in-memory runtime snapshot used by new work.",
                  )
                  .weak(),
              );
''',
        '''              ui.label(
                  egui::RichText::new(
                      "Applying updates the runtime snapshot and saves non-secret preferences for the next launch.",
                  )
                  .weak(),
              );
''',
    ),
]

for old, new in replacements:
    if old not in s:
        raise SystemExit("expected desktop snippet not found")
    s = s.replace(old, new, 1)
p.write_text(s)

p = Path("docs/runtime-settings.md")
s = p.read_text()
start = s.index("## 8. Persistence cua settings")
end = s.index("\n## 9. UI sketch", start)
section = '''## 8. Persistence cua settings

Safe preferences duoc persist tu dong khi user bam `Apply settings` va duoc load lai khi Video Prepare khoi dong lan sau:

```text
Data Root
flow toggles
OmniVoice URL
voice name
voice variant
language
quality preset
read section titles
download concurrency
```

Preferences file nam trong application config directory cua OS:

```text
macOS   ~/Library/Application Support/video-prepare/preferences.json
Windows %APPDATA%/video-prepare/preferences.json
Linux   $XDG_CONFIG_HOME/video-prepare/preferences.json
        hoac ~/.config/video-prepare/preferences.json
```

Co the override config directory bang `VIDEO_PREPARE_CONFIG_DIR`, huu ich cho test hoac portable deployment.

Secrets van memory-only va KHONG ghi vao preferences file:

```text
Pexels API Key
OmniVoice API Token
```

Neu can remember secret trong future, dung OS credential store thay vi plaintext app config.
'''
p.write_text(s[:start] + section + s[end:])

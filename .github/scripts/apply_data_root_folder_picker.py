from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"expected {label} snippet not found")
    return text.replace(old, new, 1)


desktop = Path("src/desktop.rs")
text = desktop.read_text()

text = replace_once(
    text,
    """use std::{\n    path::PathBuf,\n""",
    """use std::{\n    path::PathBuf,\n    process::Command,\n""",
    "desktop imports",
)

text = replace_once(
    text,
    '''          ui.label(egui::RichText::new("Data Root").strong());
          ui.add_sized(
              [ui.available_width(), 34.0],
              egui::TextEdit::singleline(&mut self.draft.data_root)
                  .hint_text("video-prepare-data"),
          );
''',
    '''          ui.label(egui::RichText::new("Data Root").strong());
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
                              "Data Root selected. Click Apply settings to save it.".to_owned();
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
''',
    "Data Root UI",
)

helper = r'''
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
            r#"POSIX path of (choose folder with prompt \"Choose Video Prepare Data Root\")"#,
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
        Ok(output) if output.status.success() => return selected_folder_from_stdout(&output.stdout),
        Ok(output) if output.status.code() == Some(1) => return Ok(None),
        Ok(_) => {}
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("failed to launch Linux folder browser: {error}"));
        }
        Err(_) => {}
    }

    let initial = if current.trim().is_empty() { "." } else { current };
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(
            "no supported system folder browser found (tried zenity and kdialog)".to_owned(),
        ),
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

'''

text = replace_once(
    text,
    "pub fn run_desktop() -> eframe::Result<()> {\n",
    helper + "pub fn run_desktop() -> eframe::Result<()> {\n",
    "desktop helper insertion point",
)

desktop.write_text(text)

docs = Path("docs/runtime-settings.md")
text = docs.read_text()
text = replace_once(
    text,
    '''`Data Root`

- user chon thu muc luu toan bo Video Prepare data;
- app tao `projects/` ben trong Data Root;
''',
    '''`Data Root`

- user chon thu muc bang system folder browser (Finder tren macOS, File Explorer/folder dialog tren Windows, system chooser tren Linux), khong can go path thu cong;
- field Data Root tren UI la read-only va duoc cap nhat tu folder browser;
- app tao `projects/` ben trong Data Root;
''',
    "Data Root docs",
)
docs.write_text(text)

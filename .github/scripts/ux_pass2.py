from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"missing patch anchor: {label}")
    return text.replace(old, new, 1)


# inspection.rs: make issue navigation typed instead of parsing display strings.
path = Path("src/inspection.rs")
text = path.read_text()
text = replace_once(
    text,
    '''pub struct InspectionProblem {
    pub area: String,
    pub scope: String,
    pub state: TaskState,
    pub message: String,
}''',
    '''pub struct InspectionProblem {
    pub area: String,
    pub scope: String,
    pub scene_id: Option<String>,
    pub visual_id: Option<String>,
    pub state: TaskState,
    pub message: String,
}''',
    "typed inspection problem fields",
)
text = text.replace(
    '''            scope: "project".to_owned(),
            state:''',
    '''            scope: "project".to_owned(),
            scene_id: None,
            visual_id: None,
            state:''',
)
text = replace_once(
    text,
    '''                                scope: format!("{}/{}", scene.id, request.id),
                                state: stored.state,''',
    '''                                scope: format!("{}/{}", scene.id, request.id),
                                scene_id: Some(scene.id.clone()),
                                visual_id: Some(request.id.clone()),
                                state: stored.state,''',
    "visual problem typed target",
)
text = replace_once(
    text,
    '''                    scope: "flow".to_owned(),
                    state: status.state,''',
    '''                    scope: "flow".to_owned(),
                    scene_id: None,
                    visual_id: None,
                    state: status.state,''',
    "audio problem typed target",
)
text = replace_once(
    text,
    '''        assert!(inspection
            .problems
            .iter()
            .any(|problem| problem.scope == "S01/V02" && problem.message == "rate limited"));''',
    '''        assert!(inspection
            .problems
            .iter()
            .any(|problem| problem.scope == "S01/V02" && problem.message == "rate limited"));
        let targeted = inspection
            .problems
            .iter()
            .find(|problem| problem.scope == "S01/V02")
            .unwrap();
        assert_eq!(targeted.scene_id.as_deref(), Some("S01"));
        assert_eq!(targeted.visual_id.as_deref(), Some("V02"));''',
    "typed target regression assertions",
)
path.write_text(text)


# project_catalog.rs: expose user-facing progress/attention summary for cards and filters.
path = Path("src/project_catalog.rs")
text = path.read_text()
text = replace_once(
    text,
    '''use crate::{
    parse_script, reconcile_local_project, LocalReconciliationError, ProjectError, ProjectStore,
    StoredProject, TaskState,
};''',
    '''use crate::{
    inspection::inspect_project, parse_script, reconcile_local_project, LocalReconciliationError,
    ProjectError, ProjectStore, StoredProject, TaskState,
};''',
    "catalog inspection import",
)
text = replace_once(
    text,
    '''    pub visual_flow: TaskState,
    pub audio_flow: TaskState,
}''',
    '''    pub visual_flow: TaskState,
    pub audio_flow: TaskState,
    pub visual_assets_ready: usize,
    pub visual_assets_total: usize,
    pub attention_count: usize,
}''',
    "project summary progress fields",
)
text = replace_once(
    text,
    '''impl From<&StoredProject> for ProjectSummary {
    fn from(project: &StoredProject) -> Self {
        Self {
            project_id: project.metadata.project_id.clone(),
            title: project.metadata.title.clone(),
            root: project.root.clone(),
            created_unix_ms: project.metadata.created_unix_ms,
            scene_count: project.metadata.scene_ids.len(),
            overall: project.status.overall,
            visual_flow: project.status.visual_flow,
            audio_flow: project.status.audio_flow,
        }
    }
}''',
    '''impl From<&StoredProject> for ProjectSummary {
    fn from(project: &StoredProject) -> Self {
        let inspection = inspect_project(project);
        let visual_assets_total = inspection
            .scenes
            .iter()
            .flat_map(|scene| scene.visual_requests.iter())
            .map(|request| request.target_count as usize)
            .sum();
        let visual_assets_ready = inspection
            .scenes
            .iter()
            .flat_map(|scene| scene.visual_requests.iter())
            .map(|request| request.completed_assets)
            .sum();
        Self {
            project_id: project.metadata.project_id.clone(),
            title: project.metadata.title.clone(),
            root: project.root.clone(),
            created_unix_ms: project.metadata.created_unix_ms,
            scene_count: project.metadata.scene_ids.len(),
            overall: inspection.overall,
            visual_flow: inspection.visual_flow,
            audio_flow: inspection.audio_flow,
            visual_assets_ready,
            visual_assets_total,
            attention_count: inspection.problems.len(),
        }
    }
}''',
    "project summary derived progress",
)
text = replace_once(
    text,
    '''        assert_eq!(catalog.errors.len(), 1);
        assert_eq!(catalog.errors[0].root.file_name().unwrap(), "broken");''',
    '''        assert_eq!(catalog.errors.len(), 1);
        assert!(catalog.projects.iter().all(|project| project.visual_assets_total > 0));
        assert!(catalog.projects.iter().all(|project| project.visual_assets_ready == 0));
        assert!(catalog.projects.iter().all(|project| project.attention_count > 0));
        assert_eq!(catalog.errors[0].root.file_name().unwrap(), "broken");''',
    "project summary regression assertions",
)
path.write_text(text)


# desktop.rs: workflow-oriented project discovery, safer smart action, file pickers, and progressive disclosure.
path = Path("src/desktop.rs")
text = path.read_text()
text = replace_once(
    text,
    '''enum Screen {
    Projects,
    Dashboard,
    Scene,
    Settings,
}
''',
    '''enum Screen {
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
    const ALL: [Self; 4] = [Self::All, Self::InProgress, Self::NeedsAttention, Self::Ready];

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
''',
    "project filter and typed workspace action",
)
text = replace_once(
    text,
    '''    show_create_project: bool,
    project_status: String,''',
    '''    show_create_project: bool,
    project_search: String,
    project_filter: ProjectFilter,
    project_status: String,''',
    "project filter fields",
)
text = replace_once(
    text,
    '''    asset_action_status: String,
    asset_action_status_is_error: bool,
}''',
    '''    asset_action_status: String,
    asset_action_status_is_error: bool,
    workspace_action_status: String,
    workspace_action_status_is_error: bool,
}''',
    "workspace file action fields",
)
text = replace_once(
    text,
    '''            show_create_project: false,
            project_status: String::new(),''',
    '''            show_create_project: false,
            project_search: String::new(),
            project_filter: ProjectFilter::All,
            project_status: String::new(),''',
    "project filter defaults",
)
text = replace_once(
    text,
    '''            asset_action_status: String::new(),
            asset_action_status_is_error: false,
        };''',
    '''            asset_action_status: String::new(),
            asset_action_status_is_error: false,
            workspace_action_status: String::new(),
            workspace_action_status_is_error: false,
        };''',
    "workspace file action defaults",
)

text = replace_once(
    text,
    '''                    ui.collapsing("Import a .vprep file instead", |ui| {
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
                    });''',
    '''                    ui.collapsing("Import a .vprep file instead", |ui| {
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
                                        self.create_script_path = path.to_string_lossy().into_owned();
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        self.project_status = error;
                                        self.project_status_is_error = true;
                                    }
                                }
                            }
                        });
                    });''',
    "vprep system picker UI",
)

text = replace_once(
    text,
    '''                ui.add_space(6.0);

                if self.catalog.projects.is_empty() {''',
    '''                ui.add_space(6.0);
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

                if self.catalog.projects.is_empty() {''',
    "project search and filters",
)
text = replace_once(
    text,
    '''                } else {
                    let projects = self.catalog.projects.clone();
                    for project in projects {''',
    '''                } else if filtered_projects.is_empty() {
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
                    for project in filtered_projects {''',
    "filtered project list empty state",
)
text = replace_once(
    text,
    '''                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} scene{}",
                                            project.scene_count,
                                            if project.scene_count == 1 { "" } else { "s" }
                                        ))
                                        .weak(),
                                    );''',
    '''                                    if project.visual_assets_total > 0 {
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
                                    });''',
    "project card progress",
)

text = replace_once(
    text,
    '''      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal(|ui| {''',
    '''      let project_root = self
          .selected_project
          .as_ref()
          .map(|project| project.root.clone());
      egui::Frame::group(ui.style()).show(ui, |ui| {
          ui.set_min_width(ui.available_width());
          ui.horizontal(|ui| {''',
    "workspace project root capture",
)
text = replace_once(
    text,
    '''              ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                  let incomplete = inspection.incomplete_count();''',
    '''              ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                  let incomplete = inspection.incomplete_count();''',
    "workspace open folder action",
)
text = replace_once(
    text,
    '''          ui.horizontal_wrapped(|ui| {
              status_badge(ui, "Overall", inspection.overall);
              status_badge(ui, "Visual", inspection.visual_flow);
              status_badge(ui, "Audio", inspection.audio_flow);
          });
      });

      ui.add_space(12.0);''',
    '''          ui.horizontal_wrapped(|ui| {
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

      ui.add_space(12.0);''',
    "workspace action status",
)

old_smart = '''          let smart_action = if remote_available
              && matches!(inspection.audio_flow, crate::TaskState::Running | crate::TaskState::UnknownRemote)
          {
              3
          } else if inspection.overall == crate::TaskState::Completed {
              4
          } else if inspection.overall == crate::TaskState::Failed {
              2
          } else if matches!(
              inspection.overall,
              crate::TaskState::Partial | crate::TaskState::Interrupted | crate::TaskState::Running
          ) {
              1
          } else {
              0
          };
          let primary_label = match smart_action {
              1 => "Continue preparation",
              2 => "Retry failed items",
              3 => "Check audio status",
              4 => "Project ready ✓",
              _ => "Start preparation",
          };
          if ui
              .add_enabled(
                  !worker_active && smart_action != 4,
                  egui::Button::new(egui::RichText::new(primary_label).strong()),
              )
              .clicked()
          {
              match smart_action {
                  1 => self.start_run(RunAction::Resume),
                  2 => self.start_run(RunAction::RetryFailed),
                  3 => self.start_remote_audio_reconciliation(true),
                  _ => self.start_run(RunAction::Run),
              }
          }'''
new_smart = '''          let smart_action = recommended_workspace_action(
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
          }'''
text = replace_once(text, old_smart, new_smart, "typed smart workspace action")

text = replace_once(
    text,
    '''              let next_scene = inspection.problems.iter().find_map(|problem| {
                  inspection.scenes.iter().find(|scene| problem.scope.contains(&scene.id)).map(|scene| scene.id.clone())
              });''',
    '''              let next_scene = inspection
                  .problems
                  .iter()
                  .find_map(|problem| problem.scene_id.clone());''',
    "typed next issue target",
)
text = replace_once(
    text,
    '''                  let scene_target = inspection
                      .scenes
                      .iter()
                      .find(|scene| problem.scope.contains(&scene.id))
                      .map(|scene| scene.id.clone());''',
    '''                  let scene_target = problem.scene_id.clone();''',
    "typed issue card target",
)

text = replace_once(
    text,
    '''                  if !request.attempted_queries.is_empty() {
                      ui.small(format!(
                          "Attempted queries: {}",
                          request.attempted_queries.join(" | ")
                      ));
                  }
                  if let Some(query) = &request.successful_query {
                      ui.small(format!("Successful query: {query}"));
                  }''',
    '''                  if !request.attempted_queries.is_empty() || request.successful_query.is_some() {
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
                  }''',
    "progressive search details",
)

old_asset = '''                              ui.horizontal_wrapped(|ui| {
                                  ui.strong(format!(
                                      "Slot {} · {:?}",
                                      asset.slot, asset.kind
                                  ));
                                  ui.label(
                                      egui::RichText::new(format!(
                                          "{} · {} bytes · provider asset {}",
                                          asset.provider, asset.bytes, asset.provider_asset_id
                                      ))
                                      .weak(),
                                  );
                              });
                              ui.small(format!("Relative path: {}", asset.relative_path));
                              ui.label(egui::RichText::new("Full path").strong());
                              ui.add(
                                  egui::Label::new(
                                      egui::RichText::new(
                                          asset.absolute_path.display().to_string(),
                                      )
                                      .monospace(),
                                  )
                                  .wrap(),
                              );
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
                                  if ui.button("Go to folder").clicked() {
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
new_asset = '''                              let file_name = asset
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
                              });'''
text = replace_once(text, old_asset, new_asset, "content-first asset cards")

old_audio = '''                  ui.horizontal_wrapped(|ui| {
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
                  }'''
new_audio = '''                  ui.horizontal_wrapped(|ui| {
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
                  });'''
text = replace_once(text, old_audio, new_audio, "progressive audio attempt details")

# Helper functions: project folder opener, .vprep picker, and human-readable bytes.
text = replace_once(
    text,
    '''fn pick_visual_asset_file() -> Result<Option<PathBuf>, String> {''',
    '''fn open_folder(path: &Path) -> Result<(), String> {
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

fn pick_visual_asset_file() -> Result<Option<PathBuf>, String> {''',
    "project folder and vprep picker helpers",
)
text = replace_once(
    text,
    '''fn slugify_project_id(title: &str) -> String {''',
    '''fn format_bytes(bytes: u64) -> String {
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

fn slugify_project_id(title: &str) -> String {''',
    "format bytes helper",
)

# Add lightweight tests for pure UI decision logic.
text = replace_once(
    text,
    '''fn optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}''',
    '''fn optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod ui_logic_tests {
    use super::*;

    #[test]
    fn workspace_action_prefers_remote_reconciliation_when_audio_is_unknown() {
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
            recommended_workspace_action(
                crate::TaskState::Failed,
                crate::TaskState::Failed,
                false,
            ),
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
    fn byte_labels_are_human_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MB");
    }
}''',
    "desktop UI logic tests",
)
path.write_text(text)

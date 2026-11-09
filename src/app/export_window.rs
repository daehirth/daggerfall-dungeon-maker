//! The "Export to Daggerfall Unity" window.
//!
//! It describes the mod (saved with the project), lists the project's
//! dungeons to tick for export (only those placed on the world map can be),
//! and writes the mod's files. It can then build a `.dfmod` with the Unity
//! editor in the background (see [`crate::export::unity`]). Where the files go
//! and how to build are remembered between runs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::arena2::Arena2Library;
use crate::export::unity::{
    default_build_project, editor_version, find_unity_editors, is_compatible_version, BuildEvent, BuildJob,
    BuildRequest, BuildTarget, UNITY_VERSION,
};
use crate::export::{build_mod, check_project, new_guid, write_mod, DungeonCheck, WORLD_DATA_FOLDER};
use crate::project::{ItemId, Project};

/// Width of the window's text fields, in points.
const FIELD_WIDTH: f32 = 340.0;
/// Width of the window's contents, in points.
const WINDOW_WIDTH: f32 = 580.0;
/// Most height the dungeon list and the export's outcome take before they scroll, in points.
const SCROLLING_LIST_HEIGHT: f32 = 180.0;
/// How often the window looks for news from a running build.
const BUILD_POLL_SECONDS: f32 = 0.25;

/// Whether, and how, to build a `.dfmod` after writing the mod's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DfmodBuild {
    /// Only write the files. (Settings saved when there was a build without
    /// Unity read as this.)
    #[default]
    #[serde(alias = "Direct")]
    None,
    /// Run the Unity editor, as Daggerfall Unity's Mod Builder does.
    Unity,
}

impl DfmodBuild {
    const ALL: [Self; 2] = [Self::None, Self::Unity];

    fn label(self) -> &'static str {
        match self {
            Self::None => "Files only",
            Self::Unity => "Build a .dfmod with Unity",
        }
    }

    fn explanation(self) -> &'static str {
        match self {
            Self::None => "Writes the WorldData files and the manifest, without packing them into a .dfmod",
            Self::Unity => "Runs the Unity editor in the background to pack the files, as Daggerfall Unity's Mod Builder does",
        }
    }
}

/// Export settings remembered between runs (not part of the project).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    /// Folder the mod's folder is written into.
    pub output_folder: String,
    /// Whether, and how, to build a `.dfmod` after writing the files.
    pub dfmod: DfmodBuild,
    /// The Unity editor program.
    pub unity_editor: String,
    pub targets: Vec<BuildTarget>,
    /// Daggerfall Unity's `Mods` folder, to copy the `.dfmod` into; empty for none.
    pub mods_folder: String,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            output_folder: String::new(),
            dfmod: DfmodBuild::None,
            unity_editor: find_unity_editors()
                .first()
                .map(|editor| editor.display().to_string())
                .unwrap_or_default(),
            targets: vec![BuildTarget::current()],
            mods_folder: String::new(),
        }
    }
}

/// What the window asks the application to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportRequest {
    /// Check the dungeons again (the project may have changed).
    Check,
    /// Write the mod's files, then build if asked to.
    Export,
}

/// How the last export went.
enum Outcome {
    /// Running: the steps so far.
    Building(Vec<String>),
    Done {
        summary: String,
        folder: PathBuf,
        files: Vec<PathBuf>,
    },
    Failed(String),
}

/// State of the export window.
#[derive(Default)]
pub struct ExportWindow {
    pub is_open: bool,
    pub settings: ExportSettings,
    /// The dungeons as last checked; `None` until checked.
    checks: Option<Vec<DungeonCheck>>,
    /// Placed dungeons the user unticked. (Placed dungeons are ticked unless
    /// unticked, so new ones are included; unplaced ones never are.)
    unticked: HashSet<ItemId>,
    job: Option<BuildJob>,
    /// The export's folder and summary, while its build runs.
    pending: Option<(PathBuf, String)>,
    outcome: Option<Outcome>,
}

impl ExportWindow {
    /// Opens the window; the dungeons are checked on the next frame.
    pub fn open(&mut self) {
        self.is_open = true;
        self.checks = None;
    }

    /// Whether the dungeons need checking before the window can show them.
    pub fn needs_check(&self) -> bool {
        self.is_open && self.checks.is_none()
    }

    /// Checks every project dungeon for export.
    pub fn run_checks(&mut self, project: &Project, library: &mut Arena2Library) {
        self.checks = Some(check_project(project, library));
    }

    /// Draws the window. Returns what the application should do.
    pub fn show(
        &mut self,
        context: &egui::Context,
        project: &mut Project,
        project_title: &str,
    ) -> Option<ExportRequest> {
        self.poll_build(context);
        if !self.is_open {
            return None;
        }
        let mut request = None;
        let mut is_open = self.is_open;
        egui::Window::new("Export to Daggerfall Unity")
            .open(&mut is_open)
            // Sized to fit its content; only the dungeon list and the outcome scroll.
            .resizable(false)
            .default_width(WINDOW_WIDTH)
            .show(context, |ui| {
                ui.set_width(WINDOW_WIDTH);
                self.show_mod_details(ui, project, project_title);
                ui.add_space(8.0);
                if self.show_dungeons(ui) {
                    request = Some(ExportRequest::Check);
                }
                ui.add_space(8.0);
                self.show_output(ui);
                ui.separator();
                if self.show_export_bar(ui) {
                    request = Some(ExportRequest::Export);
                }
                egui::ScrollArea::vertical()
                    .id_salt("export_outcome")
                    .max_height(SCROLLING_LIST_HEIGHT)
                    .show(ui, |ui| self.show_outcome(ui));
            });
        self.is_open = is_open;
        request
    }

    /// Writes the mod's files and, if asked, starts building the `.dfmod`.
    pub fn export(&mut self, project: &mut Project, library: &mut Arena2Library, project_title: &str) {
        if project.mod_details.guid.is_empty() {
            project.mod_details.guid = new_guid(project_title);
        }
        let selected = self.selected();
        let result = build_mod(project, library, &project.mod_details, project_title, &selected);
        self.checks = Some(check_project(project, library));
        let mod_files = match result {
            Ok(mod_files) => mod_files,
            Err(error) => {
                self.outcome = Some(Outcome::Failed(error));
                return;
            }
        };
        let folder = PathBuf::from(self.settings.output_folder.trim());
        let mod_folder = match write_mod(&mod_files, &folder) {
            Ok(mod_folder) => mod_folder,
            Err(error) => {
                self.outcome = Some(Outcome::Failed(error));
                return;
            }
        };
        let count = |number: usize, one: &str, many: &str| {
            format!("{number} {}", if number == 1 { one } else { many })
        };
        let summary = format!(
            "Exported {} and {} as \"{}\".",
            count(mod_files.dungeons.len(), "dungeon", "dungeons"),
            count(mod_files.cells.len(), "new cell", "new cells"),
            mod_files.title,
        );
        let mods_folder = self.settings.mods_folder.trim();
        let mods_folder = (!mods_folder.is_empty()).then(|| PathBuf::from(mods_folder));
        let targets: Vec<BuildTarget> =
            BuildTarget::ALL.into_iter().filter(|target| self.settings.targets.contains(target)).collect();
        match self.settings.dfmod {
            DfmodBuild::None => {
                self.outcome = Some(Outcome::Done { summary, folder: mod_folder, files: Vec::new() });
                return;
            }
            DfmodBuild::Unity => {}
        }
        self.job = Some(BuildJob::start(BuildRequest {
            unity_editor: PathBuf::from(self.settings.unity_editor.trim()),
            build_project: default_build_project(),
            mod_folder: mod_folder.clone(),
            file_name: mod_files.file_name.clone(),
            targets,
            mods_folder,
        }));
        self.pending = Some((mod_folder, summary));
        self.outcome = Some(Outcome::Building(vec!["Files written.".to_string()]));
    }

    /// The dungeons ticked for export: placed ones the user has not unticked.
    fn selected(&self) -> Vec<ItemId> {
        self.checks
            .iter()
            .flatten()
            .filter(|check| check.placed && !self.unticked.contains(&check.id))
            .map(|check| check.id)
            .collect()
    }

    /// Takes in news from a running build.
    fn poll_build(&mut self, context: &egui::Context) {
        let Some(job) = &self.job else { return };
        context.request_repaint_after(std::time::Duration::from_secs_f32(BUILD_POLL_SECONDS));
        for event in job.poll() {
            match event {
                BuildEvent::Step(step) => {
                    if let Some(Outcome::Building(steps)) = &mut self.outcome {
                        steps.push(step);
                    }
                }
                BuildEvent::Finished(result) => {
                    let (folder, summary) = self.pending.take().unwrap_or_default();
                    self.outcome = Some(match result {
                        Ok(files) => Outcome::Done { summary, folder, files },
                        Err(error) => Outcome::Failed(format!(
                            "{summary} The files were written to '{}', but the .dfmod could not be built:\n{error}",
                            folder.display()
                        )),
                    });
                    self.job = None;
                    return;
                }
            }
        }
    }

    /// The mod's title, version, author, contact and description.
    fn show_mod_details(&mut self, ui: &mut egui::Ui, project: &mut Project, project_title: &str) {
        ui.heading("Mod");
        let details = &mut project.mod_details;
        egui::Grid::new("export_mod_details").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            let field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str, tip: &str| {
                ui.label(label).on_hover_text(tip);
                ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(FIELD_WIDTH))
                    .on_hover_text(tip);
                ui.end_row();
            };
            field(ui, "Title", &mut details.title, project_title, "Shown in Daggerfall Unity's list of mods");
            field(ui, "Version", &mut details.version, "1.0", "Raise it with each release");
            field(ui, "Author", &mut details.author, "", "Who made the mod");
            field(ui, "Contact", &mut details.contact, "", "A web page or e-mail address for players");
            ui.label("Description").on_hover_text("Shown in Daggerfall Unity's list of mods");
            ui.add(
                egui::TextEdit::multiline(&mut details.description)
                    .desired_rows(2)
                    .desired_width(FIELD_WIDTH),
            );
            ui.end_row();
        });
        ui.label(egui::RichText::new("These details are saved with the project.").weak().small());
    }

    /// The dungeons, each with a tick box for export, and what stops each
    /// being exported. Returns whether the user asked to check again.
    fn show_dungeons(&mut self, ui: &mut egui::Ui) -> bool {
        let mut check_again = false;
        ui.horizontal(|ui| {
            ui.heading("Dungeons");
            if ui
                .small_button("Check again")
                .on_hover_text("Check the dungeons again after changing them")
                .clicked()
            {
                check_again = true;
            }
        });
        let Some(checks) = &self.checks else {
            ui.spinner();
            return check_again;
        };
        if checks.is_empty() {
            ui.label("The project has no dungeons. Create one in the project panel first.");
        }
        let bad = egui::Color32::from_rgb(230, 90, 80);
        let unticked = &mut self.unticked;
        egui::ScrollArea::vertical().id_salt("export_dungeons").max_height(SCROLLING_LIST_HEIGHT).show(
            ui,
            |ui| {
                for check in checks {
                    let mut ticked = check.placed && !unticked.contains(&check.id);
                    ui.horizontal_wrapped(|ui| {
                        let response = ui
                            .add_enabled(check.placed, egui::Checkbox::new(&mut ticked, &check.name))
                            .on_hover_text("Include this dungeon in the mod")
                            .on_disabled_hover_text(
                                "Only dungeons placed on the world map can be exported. Place it under \
                                 Location in the dungeon's properties first.",
                            );
                        if response.changed() {
                            if ticked {
                                unticked.remove(&check.id);
                            } else {
                                unticked.insert(check.id);
                            }
                        }
                        if !check.placed {
                            ui.label(egui::RichText::new("not placed on the world map").weak());
                        } else if !ticked {
                            ui.label(egui::RichText::new("left out").weak());
                        } else if check.problems.is_empty() {
                            ui.label(egui::RichText::new("ready").weak());
                        }
                        if let Some(replaced) = &check.replaces {
                            ui.label(egui::RichText::new(format!("replaces {replaced}")).weak());
                        }
                    });
                    if ticked {
                        for problem in &check.problems {
                            ui.label(egui::RichText::new(format!("      {problem}")).color(bad));
                        }
                    }
                }
            },
        );
        check_again
    }

    /// Where the files go, and the `.dfmod` build settings.
    fn show_output(&mut self, ui: &mut egui::Ui) {
        ui.heading("Output");
        // Ticked dungeons with music of their own, which loose files cannot play.
        let selected = self.selected();
        let custom_music: Vec<String> = self
            .checks
            .iter()
            .flatten()
            .filter(|check| check.custom_music && selected.contains(&check.id))
            .map(|check| check.name.clone())
            .collect();
        // And those with a skybox, which loose files cannot show either.
        let skyboxes: Vec<String> = self
            .checks
            .iter()
            .flatten()
            .filter(|check| check.skybox && selected.contains(&check.id))
            .map(|check| check.name.clone())
            .collect();
        let settings = &mut self.settings;
        ui.horizontal(|ui| {
            ui.label("Folder");
            ui.add(
                egui::TextEdit::singleline(&mut settings.output_folder)
                    .hint_text("Choose a folder")
                    .desired_width(FIELD_WIDTH),
            );
            if ui.button("Browse…").clicked() {
                if let Some(folder) = pick_folder("Export the mod into", &settings.output_folder) {
                    settings.output_folder = folder.display().to_string();
                }
            }
        });
        ui.label(
            egui::RichText::new(format!(
                "A folder named after the mod is made here, with the {WORLD_DATA_FOLDER} files and the mod's manifest. \
                 The {WORLD_DATA_FOLDER} files also work loose in Daggerfall Unity's StreamingAssets/{WORLD_DATA_FOLDER} folder."
            ))
            .weak()
            .small(),
        );
        ui.add_space(4.0);
        for choice in DfmodBuild::ALL {
            ui.radio_value(&mut settings.dfmod, choice, choice.label()).on_hover_text(choice.explanation());
        }
        if settings.dfmod == DfmodBuild::None {
            if !custom_music.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠ Custom music is not supported in loose files: {} will fall back to the game's own music \
                         (the song chosen under Music). Build a .dfmod with Unity to play it.",
                        custom_music.join(", ")
                    ))
                    .color(ui.visuals().warn_fg_color),
                );
            }
            if !skyboxes.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠ Skyboxes do not work in loose files: {} will show black behind the dungeon, as \
                         Daggerfall's dungeons do. Build a .dfmod with Unity to show the skybox.",
                        skyboxes.join(", ")
                    ))
                    .color(ui.visuals().warn_fg_color),
                );
            }
            return;
        }
        ui.indent("export_dfmod", |ui| {
            let weak = |text: String| egui::RichText::new(text).weak().small();
            if settings.dfmod == DfmodBuild::Unity {
            ui.horizontal(|ui| {
                ui.label("Unity editor");
                ui.add(egui::TextEdit::singleline(&mut settings.unity_editor).hint_text("Unity.exe").desired_width(FIELD_WIDTH - 60.0));
                if ui.button("Browse…").clicked() {
                    if let Some(file) = rfd::FileDialog::new().set_title("Unity editor program").pick_file() {
                        settings.unity_editor = file.display().to_string();
                    }
                }
                if ui.button("Find").on_hover_text("Look for editors installed with Unity Hub").clicked() {
                    match find_unity_editors().first() {
                        Some(editor) => settings.unity_editor = editor.display().to_string(),
                        None => settings.unity_editor.clear(),
                    }
                }
            });
            let editor = Path::new(settings.unity_editor.trim());
            if settings.unity_editor.trim().is_empty() {
                ui.label(weak(format!(
                    "No Unity editor found. Install Unity {UNITY_VERSION} with Unity Hub, then press Find."
                )));
            } else if !editor.is_file() {
                ui.label(egui::RichText::new("There is no program at this path.").color(ui.visuals().warn_fg_color).small());
            } else {
                match editor_version(editor) {
                    Some(version) if !is_compatible_version(&version) => {
                        ui.label(
                            egui::RichText::new(format!(
                                "This is Unity {version}; Daggerfall Unity needs mods built with Unity 2019.4 ({UNITY_VERSION})."
                            ))
                            .color(ui.visuals().warn_fg_color)
                            .small(),
                        );
                    }
                    Some(version) => {
                        ui.label(weak(format!("Unity {version}")));
                    }
                    None => {}
                }
            }
            }
            ui.horizontal(|ui| {
                ui.label("Platforms");
                for target in BuildTarget::ALL {
                    let mut chosen = settings.targets.contains(&target);
                    let tip = match settings.dfmod {
                        DfmodBuild::Unity => {
                            format!("Needs {} build support installed in the Unity editor", target.name())
                        }
                        _ => format!("Make a .dfmod for Daggerfall Unity on {}", target.name()),
                    };
                    if ui.checkbox(&mut chosen, target.name()).on_hover_text(tip).changed() {
                        settings.targets.retain(|other| *other != target);
                        if chosen {
                            settings.targets.push(target);
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Copy to Mods folder");
                ui.add(
                    egui::TextEdit::singleline(&mut settings.mods_folder)
                        .hint_text("Optional: …/DaggerfallUnity_Data/StreamingAssets/Mods")
                        .desired_width(FIELD_WIDTH - 60.0),
                );
                if ui.button("Browse…").clicked() {
                    if let Some(folder) = pick_folder("Daggerfall Unity's Mods folder", &settings.mods_folder) {
                        settings.mods_folder = folder.display().to_string();
                    }
                }
            });
            if settings.dfmod == DfmodBuild::Unity {
                ui.label(weak(format!(
                    "The first build makes a small Unity project in {}; later builds reuse it.",
                    default_build_project().display()
                )));
            }
        });
    }

    /// The Export button, with what stops it. Returns whether it was pressed.
    fn show_export_bar(&self, ui: &mut egui::Ui) -> bool {
        let checks = self.checks.as_deref().unwrap_or_default();
        let mut reasons = Vec::new();
        if self.job.is_some() {
            reasons.push("A build is running".to_string());
        }
        if self.settings.output_folder.trim().is_empty() {
            reasons.push("Choose the folder to export into".to_string());
        }
        let selected = self.selected();
        let chosen: Vec<&DungeonCheck> = checks.iter().filter(|check| selected.contains(&check.id)).collect();
        if chosen.iter().any(|check| !check.is_ready()) {
            reasons.push("Fix the problems listed under the ticked dungeons, or untick them".to_string());
        } else if chosen.is_empty() && self.checks.is_some() {
            reasons.push("Tick at least one dungeon placed on the world map".to_string());
        }
        if self.settings.dfmod == DfmodBuild::Unity && !Path::new(self.settings.unity_editor.trim()).is_file()
        {
            reasons.push("Choose the Unity editor, or choose another way to build".to_string());
        }
        if self.settings.dfmod != DfmodBuild::None && self.settings.targets.is_empty() {
            reasons.push("Choose a platform to build for".to_string());
        }
        let mut pressed = false;
        ui.horizontal(|ui| {
            let button = egui::Button::new(egui::RichText::new("Export").strong());
            pressed = ui
                .add_enabled(reasons.is_empty() && self.checks.is_some(), button)
                .on_disabled_hover_text(reasons.join("\n"))
                .clicked();
            if let Some(reason) = reasons.first() {
                ui.label(egui::RichText::new(reason).weak());
            }
        });
        pressed
    }

    /// How the last export went.
    fn show_outcome(&self, ui: &mut egui::Ui) {
        let Some(outcome) = &self.outcome else { return };
        ui.add_space(6.0);
        match outcome {
            Outcome::Building(steps) => {
                for step in steps {
                    ui.label(step);
                }
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Unity is building the mod. This can take a minute or two.");
                });
            }
            Outcome::Done { summary, folder, files } => {
                ui.label(egui::RichText::new(summary).strong());
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Files are in {}", folder.display()));
                    if ui.small_button("Open folder").clicked() {
                        open_in_file_manager(folder);
                    }
                });
                for file in files {
                    ui.label(format!("Built {}", file.display()));
                }
                if !self.settings.mods_folder.trim().is_empty() && !files.is_empty() {
                    ui.label(format!(
                        "The {} build was copied to the Mods folder.",
                        BuildTarget::current().name()
                    ));
                }
            }
            Outcome::Failed(error) => {
                ui.label(egui::RichText::new(error).color(ui.visuals().error_fg_color));
            }
        }
    }
}

/// Asks for a folder, starting at `current` when it exists.
fn pick_folder(title: &str, current: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    let current = Path::new(current.trim());
    if current.is_dir() {
        dialog = dialog.set_directory(current);
    }
    dialog.pick_folder()
}

/// Shows `folder` in the system's file manager.
fn open_in_file_manager(folder: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(error) = std::process::Command::new(program).arg(folder).spawn() {
        log::warn!("could not open {}: {error}", folder.display());
    }
}

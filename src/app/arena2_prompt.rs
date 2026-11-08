//! The window that asks the user where their Daggerfall `ARENA2` folder is.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::arena2::Arena2Library;

/// Install locations relative to a Windows drive root (or a mounted Windows drive on Linux),
/// checked to pre-fill the folder field.
const WINDOWS_INSTALL_PATHS: [&str; 4] = [
    "SteamLibrary/steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2",
    "Program Files (x86)/Steam/steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2",
    "Program Files/Steam/steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2",
    "GOG Games/Daggerfall/DAGGER/ARENA2",
];

/// Install locations relative to the home folder on Linux: native, Flatpak and Snap
/// Steam, and a GOG copy.
#[cfg(not(windows))]
const LINUX_INSTALL_PATHS: [&str; 6] = [
    ".local/share/Steam/steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2",
    ".steam/steam/steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2",
    ".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2",
    "snap/steam/common/.local/share/Steam/steamapps/common/The Elder Scrolls Daggerfall/DF/DAGGER/ARENA2",
    "GOG Games/Daggerfall/DAGGER/ARENA2",
    "Games/Daggerfall/DAGGER/ARENA2",
];

/// Example shown in the empty folder field.
#[cfg(windows)]
const FOLDER_HINT: &str = r"...\DAGGER\ARENA2";
#[cfg(not(windows))]
const FOLDER_HINT: &str = ".../DAGGER/ARENA2";

/// State of the ARENA2 folder prompt.
pub struct Arena2Prompt {
    /// Whether the window is shown.
    pub is_open: bool,
    /// Folder path typed or browsed by the user.
    folder_text: String,
    /// Validation message shown below the path field.
    problem: Option<String>,
}

impl Arena2Prompt {
    /// Creates a prompt pre-filled with `initial_folder`, or a detected install if none is given.
    pub fn new(is_open: bool, initial_folder: Option<&Path>) -> Self {
        let folder_text = initial_folder
            .map(Path::to_path_buf)
            .or_else(detect_installed_folder)
            .map(|folder| folder.display().to_string())
            .unwrap_or_default();
        Self { is_open, folder_text, problem: None }
    }

    /// Opens the prompt, optionally showing why it was opened.
    pub fn open(&mut self, problem: Option<String>) {
        self.is_open = true;
        self.problem = problem;
    }

    /// Draws the prompt. Returns the chosen folder when the user confirms a valid one.
    ///
    /// `can_cancel` is false until a folder has been loaded, because the
    /// program cannot do anything without game data.
    pub fn show(&mut self, context: &egui::Context, can_cancel: bool) -> Option<PathBuf> {
        if !self.is_open {
            return None;
        }

        let mut chosen_folder = None;
        egui::Window::new("Daggerfall data folder")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.set_width(520.0);
                ui.label("Select the ARENA2 folder of your Daggerfall installation.");
                ui.label(
                    egui::RichText::new("Game files are read from this folder each time the program starts; nothing is copied.")
                        .weak(),
                );
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    let path_field = egui::TextEdit::singleline(&mut self.folder_text)
                        .hint_text(FOLDER_HINT)
                        .desired_width(400.0);
                    let field_response = ui.add(path_field);
                    if field_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                        chosen_folder = self.validate();
                    }
                    if ui.button("Browse…").clicked() {
                        if let Some(folder) = rfd::FileDialog::new()
                            .set_title("Select the Daggerfall ARENA2 folder")
                            .pick_folder()
                        {
                            self.folder_text = folder.display().to_string();
                            self.problem = None;
                        }
                    }
                });

                if let Some(problem) = &self.problem {
                    ui.colored_label(ui.visuals().error_fg_color, problem);
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Use this folder").clicked() {
                        chosen_folder = self.validate();
                    }
                    if can_cancel && ui.button("Cancel").clicked() {
                        self.is_open = false;
                    }
                });
            });

        if chosen_folder.is_some() {
            self.is_open = false;
        }
        chosen_folder
    }

    /// Checks the typed folder and returns the resolved ARENA2 path if it is usable.
    fn validate(&mut self) -> Option<PathBuf> {
        let typed_folder = PathBuf::from(self.folder_text.trim());
        match Arena2Library::resolve_folder(&typed_folder) {
            Some(folder) => {
                self.problem = None;
                Some(folder)
            }
            None => {
                let missing = Arena2Library::missing_files(&typed_folder);
                self.problem = Some(if typed_folder.is_dir() {
                    format!("This folder is missing {}.", missing.join(", "))
                } else {
                    "This folder does not exist.".to_string()
                });
                None
            }
        }
    }
}

/// Looks for Daggerfall in a few common install locations.
fn detect_installed_folder() -> Option<PathBuf> {
    install_candidates().find(|candidate| Arena2Library::missing_files(candidate).is_empty())
}

/// Every drive letter, each joined with the common Windows install locations.
#[cfg(windows)]
fn install_candidates() -> impl Iterator<Item = PathBuf> {
    windows_drive_candidates(|drive| PathBuf::from(format!("{drive}:\\")))
}

/// The common Linux install locations under the home folder, then Windows
/// drives mounted under `/mnt` (as WSL does), so a Windows install is found too.
#[cfg(not(windows))]
fn install_candidates() -> impl Iterator<Item = PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let home_candidates =
        home.into_iter().flat_map(|home| LINUX_INSTALL_PATHS.iter().map(move |relative| home.join(relative)));
    home_candidates.chain(windows_drive_candidates(|drive| {
        PathBuf::from(format!("/mnt/{}", drive.to_ascii_lowercase()))
    }))
}

/// Joins each drive root produced by `drive_root` with every Windows install location.
fn windows_drive_candidates(drive_root: fn(char) -> PathBuf) -> impl Iterator<Item = PathBuf> {
    ('C'..='Z').flat_map(move |drive| {
        let root = drive_root(drive);
        WINDOWS_INSTALL_PATHS.iter().map(move |relative| root.join(relative))
    })
}

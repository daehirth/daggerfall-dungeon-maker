//! The egui application: windows, menus, dialogs and shortcuts.
//!
//! The main window shows one workspace at a time: a single room in a
//! [`RoomEditor`] (File > Open room) or a whole dungeon in a [`DungeonView`]
//! (File > Open dungeon). Double-clicking a room in the dungeon view opens it
//! in its own window running a full room editor; edits made there are copied
//! back into the dungeon view.

mod arena2_prompt;
mod dungeon_browser;
mod dungeon_view;
mod room_browser;
mod room_editor;
mod viewport_tools;

use std::path::{Path, PathBuf};

use eframe::{egui, egui_wgpu};

use crate::arena2::Arena2Library;
use crate::catalog::AssetCatalog;
use crate::custom_names::CustomNames;
use crate::render::SceneRenderer;
use crate::scene::Scene;
use arena2_prompt::Arena2Prompt;
use dungeon_browser::DungeonBrowser;
use dungeon_view::DungeonView;
use room_browser::RoomBrowser;
use room_editor::RoomEditor;
use viewport_tools::EditorResources;

/// Storage key under which the chosen ARENA2 folder is remembered between runs.
const ARENA2_FOLDER_STORAGE_KEY: &str = "arena2_folder";
/// Storage key under which the user's room and model names are saved.
const CUSTOM_NAMES_STORAGE_KEY: &str = "custom_names";
/// Name shown in window titles.
const APPLICATION_NAME: &str = "Daggerfall Dungeon Maker";
/// Initial size of a room editor window, in points.
const ROOM_WINDOW_SIZE: [f32; 2] = [1280.0, 820.0];

/// Optional settings supplied on the command line.
#[derive(Debug, Clone, Default)]
pub struct StartupOptions {
    /// ARENA2 folder to load instead of the remembered one.
    pub arena2_folder: Option<PathBuf>,
    /// Room to open once the game data has loaded, for example `N0000004.RDB`.
    pub room_name: Option<String>,
    /// Dungeon to open once the game data has loaded, for example `Privateer's Hold`.
    pub dungeon_name: Option<String>,
}

/// What the main window is showing.
enum Workspace {
    /// Nothing is open yet.
    Empty,
    /// A single room opened from the room list.
    Room(Box<RoomEditor>),
    /// A whole dungeon opened from the dungeon list.
    Dungeon(Box<DungeonView>),
}

/// A room editor running in its own operating system window.
struct RoomWindow {
    viewport_id: egui::ViewportId,
    editor: RoomEditor,
    /// Index of the room in the main window's dungeon that this window edits, if it is still open there.
    dungeon_room_index: Option<usize>,
    /// Cleared when the user closes the window.
    is_open: bool,
    show_controls_window: bool,
}

/// The dungeon maker application.
pub struct DungeonMakerApp {
    /// Loaded game data; `None` until the user has chosen an ARENA2 folder.
    library: Option<Arena2Library>,
    /// Handles to the GPU device shared with egui; `None` only if wgpu failed to initialise.
    render_state: Option<egui_wgpu::RenderState>,
    /// GPU renderer shared by every window; `None` only if wgpu failed to initialise.
    renderer: Option<SceneRenderer>,
    /// Placeable assets, built from the game data when it is loaded.
    catalog: AssetCatalog,
    /// The user's own names for rooms and models, saved with the settings.
    custom_names: CustomNames,
    workspace: Workspace,
    room_windows: Vec<RoomWindow>,
    /// Counter used to give every room window a unique id.
    next_room_window_number: u64,
    arena2_prompt: Arena2Prompt,
    room_browser: RoomBrowser,
    dungeon_browser: DungeonBrowser,
    status_message: String,
    error_message: Option<String>,
    show_controls_window: bool,
    /// Title last sent to the main window, to avoid resending it every frame.
    window_title: String,
}

impl DungeonMakerApp {
    /// Creates the application.
    ///
    /// The ARENA2 folder given on the command line is used if present,
    /// otherwise the folder remembered from the last run. If neither is
    /// valid the user is asked to choose one.
    pub fn new(creation_context: &eframe::CreationContext<'_>, startup_options: StartupOptions) -> Self {
        let render_state = creation_context.wgpu_render_state.clone();
        let renderer = render_state.as_ref().map(SceneRenderer::new);
        let saved_folder = startup_options.arena2_folder.clone().or_else(|| {
            creation_context
                .storage
                .and_then(|storage| storage.get_string(ARENA2_FOLDER_STORAGE_KEY))
                .map(PathBuf::from)
        });
        let custom_names = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, CUSTOM_NAMES_STORAGE_KEY))
            .unwrap_or_default();

        let mut app = Self {
            library: None,
            render_state,
            renderer,
            catalog: AssetCatalog::default(),
            custom_names,
            workspace: Workspace::Empty,
            room_windows: Vec::new(),
            next_room_window_number: 0,
            arena2_prompt: Arena2Prompt::new(true, saved_folder.as_deref()),
            room_browser: RoomBrowser::default(),
            dungeon_browser: DungeonBrowser::default(),
            status_message: "Choose your Daggerfall ARENA2 folder to begin.".to_string(),
            error_message: None,
            show_controls_window: false,
            window_title: String::new(),
        };

        if app.renderer.is_none() {
            app.error_message =
                Some("The GPU renderer could not be initialised, so the 3D view is unavailable.".into());
        }
        if let Some(folder) = saved_folder.and_then(|folder| Arena2Library::resolve_folder(&folder)) {
            app.load_library(&folder);
        }
        if app.library.is_some() {
            if let Some(room_name) = startup_options.room_name {
                app.room_browser.is_open = false;
                app.open_room(&room_name.to_ascii_uppercase());
            }
            if let Some(dungeon_name) = startup_options.dungeon_name {
                app.room_browser.is_open = false;
                app.open_dungeon_named(&dungeon_name);
            }
        }
        app
    }

    /// Loads the game data in `folder`, replacing any previously loaded data.
    fn load_library(&mut self, folder: &Path) {
        match Arena2Library::open(folder) {
            Ok(mut library) => {
                self.close_everything();
                self.room_browser.set_room_names(library.rdb_block_names());
                self.catalog = AssetCatalog::build(&mut library);
                self.status_message = format!("Loaded game data from {}", folder.display());
                self.library = Some(library);
                // Cached GPU resources belong to the old data, so start afresh.
                if let (Some(renderer), Some(render_state)) =
                    (self.renderer.as_mut(), self.render_state.as_ref())
                {
                    renderer.clear_cache(render_state);
                }
                self.arena2_prompt.is_open = false;
                self.room_browser.is_open = true;
            }
            Err(error) => self.arena2_prompt.open(Some(error.to_string())),
        }
    }

    /// Opens the RDB room called `room_name` in the main window.
    fn open_room(&mut self, room_name: &str) {
        let Some(library) = self.library.as_mut() else { return };
        match library.rdb_block(room_name) {
            Ok(block) => {
                let scene = Scene::from_rdb_block(&block);
                let title = format!("{room_name} ({})", room_browser::category_description(room_name));
                let editor = RoomEditor::new(scene, title, egui::Id::new("main_room_editor"), library);
                self.status_message = editor.status_message().to_string();
                self.set_workspace(Workspace::Room(Box::new(editor)));
            }
            Err(error) => self.error_message = Some(format!("Could not open {room_name}: {error}")),
        }
    }

    /// Opens the dungeon at `dungeon_index` in the library's dungeon list in the main window.
    fn open_dungeon(&mut self, dungeon_index: usize) {
        let Some(library) = self.library.as_mut() else { return };
        let dungeons = library.dungeons();
        let Some(definition) = dungeons.get(dungeon_index) else { return };

        let (view, problems) = DungeonView::load(definition, library);
        if !problems.is_empty() {
            self.error_message = Some(format!("Some rooms could not be loaded:\n{}", problems.join("\n")));
        }
        self.status_message = view.status_text(&self.custom_names);
        self.set_workspace(Workspace::Dungeon(Box::new(view)));
    }

    /// Opens the first dungeon whose name matches `dungeon_name` (case-insensitive).
    fn open_dungeon_named(&mut self, dungeon_name: &str) {
        let Some(library) = self.library.as_mut() else { return };
        let dungeon_index =
            library.dungeons().iter().position(|dungeon| dungeon.name.eq_ignore_ascii_case(dungeon_name));
        match dungeon_index {
            Some(index) => self.open_dungeon(index),
            None => self.error_message = Some(format!("There is no dungeon called '{dungeon_name}'.")),
        }
    }

    /// Opens room `room_index` of the open dungeon in its own room editor window.
    ///
    /// Only one room window is open at a time: any other room window is closed
    /// first. Its edits are not lost, because every frame copies them back into
    /// the dungeon view. If the room is already open, its window is brought to the front.
    fn open_room_window(&mut self, context: &egui::Context, room_index: usize) {
        if let Some(window) =
            self.room_windows.iter().find(|window| window.dungeon_room_index == Some(room_index))
        {
            context.send_viewport_cmd_to(window.viewport_id, egui::ViewportCommand::Focus);
            return;
        }
        self.close_room_windows();

        let (Workspace::Dungeon(view), Some(library)) = (&self.workspace, self.library.as_mut()) else {
            return;
        };
        let Some(room) = view.room(room_index) else { return };

        let window_number = self.next_room_window_number;
        self.next_room_window_number += 1;
        let title = format!("{} in {}", room.block_name, view.name);
        let editor_id = egui::Id::new(("room_window_editor", window_number));
        let editor = RoomEditor::new(room.scene.clone(), title, editor_id, library);
        self.status_message = format!("Opened {} in a new window", room.block_name);
        self.room_windows.push(RoomWindow {
            viewport_id: egui::ViewportId::from_hash_of(("room_window", window_number)),
            editor,
            dungeon_room_index: Some(room_index),
            is_open: true,
            show_controls_window: false,
        });
    }

    /// Replaces the main window's workspace, releasing the old one's GPU image.
    ///
    /// Room windows opened from a dungeon stay open but stop syncing back to it.
    fn set_workspace(&mut self, workspace: Workspace) {
        if let Some(render_state) = self.render_state.as_ref() {
            match &mut self.workspace {
                Workspace::Room(editor) => editor.release(render_state),
                Workspace::Dungeon(view) => view.release(render_state),
                Workspace::Empty => {}
            }
        }
        for window in &mut self.room_windows {
            window.dungeon_room_index = None;
        }
        self.workspace = workspace;
    }

    /// Closes the workspace and every room window.
    fn close_everything(&mut self) {
        self.set_workspace(Workspace::Empty);
        self.close_room_windows();
    }

    /// Closes every room window, releasing their GPU images.
    ///
    /// A window's OS window disappears on the next frame, because it is no longer shown.
    fn close_room_windows(&mut self) {
        if let Some(render_state) = self.render_state.as_ref() {
            for window in &mut self.room_windows {
                window.editor.release(render_state);
            }
        }
        self.room_windows.clear();
    }

    /// Handles keyboard shortcuts of the main window.
    fn handle_shortcuts(&mut self, context: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};

        let shortcut = |modifiers, key| {
            context.input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        let has_library = self.library.is_some();
        if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::O) && self.can_open_dungeons() {
            self.dungeon_browser.is_open = true;
        }
        if shortcut(Modifiers::COMMAND, Key::O) && has_library {
            self.room_browser.is_open = true;
        }

        let Some(library) = self.library.as_mut() else { return };
        match &mut self.workspace {
            Workspace::Room(editor) => editor.handle_shortcuts(context, library),
            Workspace::Dungeon(view) => view.handle_shortcuts(context, library),
            Workspace::Empty => {}
        }
    }

    /// Whether the loaded game data includes dungeon layouts.
    fn can_open_dungeons(&self) -> bool {
        self.library.as_ref().is_some_and(Arena2Library::has_dungeons)
    }

    /// Draws the main window's menu bar.
    fn show_menu_bar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(context, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| self.file_menu(ui));
                let library = self.library.as_mut();
                match (&mut self.workspace, library) {
                    (Workspace::Room(editor), Some(library)) => {
                        ui.menu_button("Edit", |ui| editor.edit_menu(ui));
                        ui.menu_button("View", |ui| editor.view_menu(ui, library));
                    }
                    (Workspace::Dungeon(view), Some(library)) => {
                        ui.menu_button("View", |ui| view.view_menu(ui, library));
                    }
                    _ => {}
                }
                ui.menu_button("Help", |ui| {
                    if ui.button("Viewport controls").clicked() {
                        self.show_controls_window = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    /// Adds the File menu entries.
    fn file_menu(&mut self, ui: &mut egui::Ui) {
        let has_library = self.library.is_some();
        if ui.add_enabled(has_library, egui::Button::new("Open room…").shortcut_text("Ctrl+O")).clicked() {
            self.room_browser.is_open = true;
            ui.close_menu();
        }
        let dungeon_button = egui::Button::new("Open dungeon…").shortcut_text("Ctrl+Shift+O");
        let dungeon_response = ui.add_enabled(self.can_open_dungeons(), dungeon_button);
        if dungeon_response.clicked() {
            self.dungeon_browser.is_open = true;
            ui.close_menu();
        }
        if has_library && !self.can_open_dungeons() {
            dungeon_response.on_disabled_hover_text("MAPS.BSA was not found in the ARENA2 folder.");
        }
        let has_workspace = !matches!(self.workspace, Workspace::Empty);
        if ui.add_enabled(has_workspace, egui::Button::new("Close")).clicked() {
            self.set_workspace(Workspace::Empty);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Set ARENA2 folder…").clicked() {
            self.arena2_prompt.open(None);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Exit").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Draws the main window's status bar.
    fn show_status_bar(&mut self, context: &egui::Context) {
        let status = match &self.workspace {
            Workspace::Room(editor) => editor.status_message().to_string(),
            Workspace::Dungeon(view) => view.status_text(&self.custom_names),
            Workspace::Empty => self.status_message.clone(),
        };
        egui::TopBottomPanel::bottom("status_bar").show(context, |ui| ui.label(status));
    }

    /// Draws the main window's workspace.
    fn show_workspace(&mut self, context: &egui::Context) {
        let Some(library) = self.library.as_mut() else {
            show_centered_hint(context, "Choose your ARENA2 folder to begin.");
            return;
        };
        let mut resources = EditorResources {
            library,
            render_state: self.render_state.as_ref(),
            renderer: self.renderer.as_mut(),
            catalog: &self.catalog,
            custom_names: &mut self.custom_names,
        };

        let mut room_to_open = None;
        match &mut self.workspace {
            Workspace::Room(editor) => editor.show(context, &mut resources),
            Workspace::Dungeon(view) => {
                egui::CentralPanel::default().frame(egui::Frame::none()).show(context, |ui| {
                    room_to_open = view.show(ui, &mut resources);
                });
            }
            Workspace::Empty => show_centered_hint(
                context,
                "File > Open room… (Ctrl+O) or File > Open dungeon… (Ctrl+Shift+O)",
            ),
        }
        if let Some(room_index) = room_to_open {
            self.open_room_window(context, room_index);
        }
    }

    /// Draws every room editor window and syncs their edits back into the dungeon view.
    fn show_room_windows(&mut self, context: &egui::Context) {
        let Some(library) = self.library.as_mut() else { return };
        let mut resources = EditorResources {
            library,
            render_state: self.render_state.as_ref(),
            renderer: self.renderer.as_mut(),
            catalog: &self.catalog,
            custom_names: &mut self.custom_names,
        };

        for window in &mut self.room_windows {
            // egui compares the builder with the previous frame's, so a renamed room updates the title.
            let builder = egui::ViewportBuilder::default()
                .with_title(editor_window_title(&window.editor, resources.custom_names))
                .with_inner_size(ROOM_WINDOW_SIZE);
            context.show_viewport_immediate(window.viewport_id, builder, |window_context, _class| {
                show_room_window_contents(window_context, window, &mut resources);
            });

            if let (Some(room_index), Workspace::Dungeon(view)) =
                (window.dungeon_room_index, &mut self.workspace)
            {
                view.update_room_objects(room_index, &window.editor.scene.objects);
            }
        }

        // Release closed windows.
        let render_state = self.render_state.as_ref();
        self.room_windows.retain_mut(|window| {
            if !window.is_open {
                if let Some(render_state) = render_state {
                    window.editor.release(render_state);
                }
            }
            window.is_open
        });
    }

    /// Draws the error popup, if there is an error to report.
    fn show_error_window(&mut self, context: &egui::Context) {
        let Some(message) = self.error_message.clone() else { return };
        egui::Window::new("Error")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label(message);
                if ui.button("OK").clicked() {
                    self.error_message = None;
                }
            });
    }

    /// Updates the main window title to show what is open.
    fn update_window_title(&mut self, context: &egui::Context) {
        let title = match &self.workspace {
            Workspace::Room(editor) => editor_window_title(editor, &self.custom_names),
            Workspace::Dungeon(view) => format!("{} ({}) - {APPLICATION_NAME}", view.name, view.region_name),
            Workspace::Empty => APPLICATION_NAME.to_string(),
        };
        if title != self.window_title {
            context.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}

impl eframe::App for DungeonMakerApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(context);
        self.show_menu_bar(context);
        self.show_status_bar(context);
        self.show_workspace(context);

        if let Some(folder) = self.arena2_prompt.show(context, self.library.is_some()) {
            self.load_library(&folder);
        }
        if let Some(room_name) = self.room_browser.show(context, &mut self.custom_names) {
            self.open_room(&room_name);
        }
        if self.dungeon_browser.is_open {
            if let Some(library) = self.library.as_mut() {
                let dungeons = library.dungeons();
                if let Some(dungeon_index) = self.dungeon_browser.show(context, &dungeons) {
                    self.open_dungeon(dungeon_index);
                }
            }
        }
        show_controls_window(context, &mut self.show_controls_window);
        self.show_error_window(context);
        self.show_room_windows(context);
        self.update_window_title(context);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(library) = &self.library {
            storage.set_string(ARENA2_FOLDER_STORAGE_KEY, library.root().display().to_string());
        }
        eframe::set_value(storage, CUSTOM_NAMES_STORAGE_KEY, &self.custom_names);
    }
}

/// Window title for a room editor, including the user's name for the room if it has one.
fn editor_window_title(editor: &RoomEditor, custom_names: &CustomNames) -> String {
    match custom_names.room_name(&editor.scene.block_name) {
        Some(room_name) => format!("{room_name} - {} - {APPLICATION_NAME}", editor.title()),
        None => format!("{} - {APPLICATION_NAME}", editor.title()),
    }
}

/// Draws the menu bar, status bar and editor inside a room editor window.
fn show_room_window_contents(
    context: &egui::Context,
    window: &mut RoomWindow,
    resources: &mut EditorResources<'_>,
) {
    let editor_id = window.editor.id();
    window.editor.handle_shortcuts(context, resources.library);

    egui::TopBottomPanel::top(editor_id.with("menu_bar")).show(context, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("Edit", |ui| window.editor.edit_menu(ui));
            ui.menu_button("View", |ui| window.editor.view_menu(ui, resources.library));
            ui.menu_button("Help", |ui| {
                if ui.button("Viewport controls").clicked() {
                    window.show_controls_window = true;
                    ui.close_menu();
                }
            });
        });
    });
    egui::TopBottomPanel::bottom(editor_id.with("status_bar")).show(context, |ui| {
        let sync_note = if window.dungeon_room_index.is_some() {
            "Changes appear in the dungeon view."
        } else {
            "Not linked to an open dungeon."
        };
        ui.horizontal(|ui| {
            ui.label(window.editor.status_message());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(sync_note).weak());
            });
        });
    });
    window.editor.show(context, resources);
    show_controls_window(context, &mut window.show_controls_window);

    if context.input(|input| input.viewport().close_requested()) {
        window.is_open = false;
    }
}

/// Draws a hint in the middle of an otherwise empty window.
fn show_centered_hint(context: &egui::Context, hint: &str) {
    egui::CentralPanel::default().show(context, |ui| {
        ui.centered_and_justified(|ui| ui.label(egui::RichText::new(hint).size(15.0).weak()));
    });
}

/// Draws the viewport controls help window.
fn show_controls_window(context: &egui::Context, is_open: &mut bool) {
    const CONTROLS: [(&str, &str); 19] = [
        ("Hover a room (dungeon view)", "Highlight the whole room"),
        ("Double-click a room (dungeon view)", "Open it in a room editor window"),
        ("Click an asset tile", "Arm that asset for placement"),
        ("Left click (while placing)", "Place the asset; hold Shift to keep placing"),
        ("R / Esc (while placing)", "Rotate the asset 90° / stop placing"),
        ("Left click", "Select object"),
        ("Left drag on arrow", "Move selection along that axis"),
        ("Ctrl while dragging", "Toggle grid snapping"),
        ("Right drag", "Orbit"),
        ("Middle drag / Shift + right drag", "Pan"),
        ("Mouse wheel", "Zoom"),
        ("Right mouse + W A S D", "Fly forward / left / back / right"),
        ("Right mouse + Q / E", "Fly down / up (hold Shift to go faster)"),
        ("F", "Frame selection"),
        ("Home", "Frame room or dungeon"),
        ("R / Shift + R", "Rotate selection 90° clockwise / anticlockwise"),
        ("Ctrl + D", "Duplicate selection"),
        ("Delete", "Delete selection"),
        ("Ctrl + Z / Ctrl + Y", "Undo / redo"),
    ];
    egui::Window::new("Viewport controls").open(is_open).resizable(false).show(context, |ui| {
        egui::Grid::new("controls_grid").striped(true).show(ui, |ui| {
            for (input, action) in CONTROLS {
                ui.strong(input);
                ui.label(action);
                ui.end_row();
            }
        });
    });
}

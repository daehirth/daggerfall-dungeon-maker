//! The egui application: windows, menus, dialogs and shortcuts.
//!
//! The main window shows the project panel and one workspace at a time: a
//! single room in a [`RoomEditor`] or a whole dungeon in a [`DungeonView`].
//! Double-clicking a room in the dungeon view opens it in its own window
//! running a full room editor; edits made there are copied back into the
//! dungeon view.
//!
//! Rooms and dungeons from Daggerfall open read-only. Trying to change one
//! offers to copy it into the [`Project`], where it can be edited, renamed and
//! saved in a `.ddm` project file.

mod arena2_prompt;
mod dungeon_browser;
mod dungeon_view;
mod project_panel;
mod room_browser;
mod room_editor;
mod viewport_tools;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::{egui, egui_wgpu};
use glam::Vec3;

use crate::arena2::{Arena2Library, RDB_BLOCK_SIDE};
use crate::catalog::AssetCatalog;
use crate::custom_names::CustomNames;
use crate::project::{
    ItemId, Project, ProjectDungeon, ProjectDungeonRoom, ProjectEntry, ProjectRoom, PROJECT_FILE_EXTENSION,
};
use crate::render::{RoomOutline, SceneRenderer};
use crate::scene::Scene;
use arena2_prompt::Arena2Prompt;
use dungeon_browser::DungeonBrowser;
use dungeon_view::{DungeonDetails, DungeonPropertiesAction, DungeonView};
use project_panel::{OpenedItem, PanelAction, ProjectPanel};
use room_browser::RoomBrowser;
use room_editor::{ContextCell, DungeonContext, RoomEditor, UsedInEntry};
use viewport_tools::EditorResources;

/// Storage key under which the chosen ARENA2 folder is remembered between runs.
const ARENA2_FOLDER_STORAGE_KEY: &str = "arena2_folder";
/// Storage key under which the user's room and model names are saved.
const CUSTOM_NAMES_STORAGE_KEY: &str = "custom_names";
/// Storage key under which the last project file is remembered, to reopen it on the next run.
const PROJECT_FILE_STORAGE_KEY: &str = "project_file";
/// How long the pointer must rest on something before its explanation appears.
const TOOLTIP_DELAY_SECONDS: f32 = 0.5;
/// Name shown in window titles.
const APPLICATION_NAME: &str = "Daggerfall Dungeon Maker";
/// Name of a project that has not been saved yet.
const UNTITLED_PROJECT_NAME: &str = "Untitled project";
/// Initial size of a room editor window, in points.
const ROOM_WINDOW_SIZE: [f32; 2] = [1280.0, 820.0];

/// Optional settings supplied on the command line.
#[derive(Debug, Clone, Default)]
pub struct StartupOptions {
    /// ARENA2 folder to load instead of the remembered one.
    pub arena2_folder: Option<PathBuf>,
    /// Project file to open instead of the one open last time.
    pub project_file: Option<PathBuf>,
    /// Room to open once the game data has loaded, for example `N0000004.RDB`.
    pub room_name: Option<String>,
    /// Dungeon to open once the game data has loaded, for example `Privateer's Hold`.
    pub dungeon_name: Option<String>,
}

/// What the main window is showing.
enum Workspace {
    /// Nothing is open yet.
    Empty,
    /// A single room.
    Room(Box<RoomEditor>),
    /// A whole dungeon.
    Dungeon(Box<DungeonView>),
}

/// A room editor running in its own operating system window.
struct RoomWindow {
    viewport_id: egui::ViewportId,
    editor: RoomEditor,
    /// Index of the room in the main window's dungeon that this window edits.
    dungeon_room_index: Option<usize>,
    /// Cleared when the user closes the window.
    is_open: bool,
    show_controls_window: bool,
}

/// What the user tried to change that comes from Daggerfall, so what a copy is made of.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CopySubject {
    /// A Daggerfall room open on its own: copy the room.
    Room,
    /// A property of a Daggerfall dungeon: copy the dungeon.
    Dungeon,
    /// A room in a Daggerfall dungeon: copy the dungeon and the room.
    DungeonAndRoom,
    /// A Daggerfall room in a project dungeon: copy the room for that dungeon.
    RoomInDungeon,
}

/// The "You cannot edit … from Daggerfall" question, shown after the user tried to change one.
#[derive(Debug, Clone, Copy)]
struct CopyPrompt {
    subject: CopySubject,
    /// Window the attempt was made in: `None` for the main window, otherwise a room window.
    viewport: Option<egui::ViewportId>,
}

/// The answer to the copy prompt.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CopyChoice {
    CreateCopy,
    Cancel,
}

/// Something that would discard unsaved project changes, waiting for the user to decide.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DiscardingAction {
    NewProject,
    OpenProject,
    Exit,
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
    /// For each Daggerfall cell, the indices of the Daggerfall dungeons that use it.
    cell_usage: HashMap<String, Vec<usize>>,
    /// The user's own names for rooms and models, saved with the settings.
    custom_names: CustomNames,
    /// The user's own rooms and dungeons.
    project: Project,
    /// Where the project was last saved or opened from; `None` until it is saved.
    project_path: Option<PathBuf>,
    /// The project as last saved or opened, to tell whether it has unsaved changes.
    saved_project_text: String,
    workspace: Workspace,
    /// What the workspace shows; `None` when it is empty.
    opened_item: Option<OpenedItem>,
    room_windows: Vec<RoomWindow>,
    /// Counter used to give every room window a unique id.
    next_room_window_number: u64,
    arena2_prompt: Arena2Prompt,
    room_browser: RoomBrowser,
    dungeon_browser: DungeonBrowser,
    project_panel: ProjectPanel,
    copy_prompt: Option<CopyPrompt>,
    /// Room or dungeon the user asked to delete, waiting for confirmation.
    delete_prompt: Option<ItemId>,
    /// Action waiting for the user to decide what to do with unsaved changes.
    unsaved_changes_prompt: Option<DiscardingAction>,
    /// Set once the user agreed to close the program, so the close is not stopped again.
    exit_confirmed: bool,
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
    /// valid the user is asked to choose one. The project given on the command
    /// line, or else the one open last time, is reopened.
    pub fn new(creation_context: &eframe::CreationContext<'_>, startup_options: StartupOptions) -> Self {
        // Property explanations appear after the pointer has rested on them for half a second.
        creation_context.egui_ctx.style_mut(|style| style.interaction.tooltip_delay = TOOLTIP_DELAY_SECONDS);
        let render_state = creation_context.wgpu_render_state.clone();
        let renderer = render_state.as_ref().map(SceneRenderer::new);
        let stored_string = |key: &str| creation_context.storage.and_then(|storage| storage.get_string(key));
        let saved_folder = startup_options
            .arena2_folder
            .clone()
            .or_else(|| stored_string(ARENA2_FOLDER_STORAGE_KEY).map(PathBuf::from));
        let project_file = startup_options.project_file.clone().or_else(|| {
            stored_string(PROJECT_FILE_STORAGE_KEY).map(PathBuf::from).filter(|path| path.is_file())
        });
        let custom_names = creation_context
            .storage
            .and_then(|storage| eframe::get_value(storage, CUSTOM_NAMES_STORAGE_KEY))
            .unwrap_or_default();

        let project = Project::default();
        let saved_project_text = project.to_file_text();
        let mut app = Self {
            library: None,
            render_state,
            renderer,
            catalog: AssetCatalog::default(),
            cell_usage: HashMap::new(),
            custom_names,
            project,
            project_path: None,
            saved_project_text,
            workspace: Workspace::Empty,
            opened_item: None,
            room_windows: Vec::new(),
            next_room_window_number: 0,
            arena2_prompt: Arena2Prompt::new(true, saved_folder.as_deref()),
            room_browser: RoomBrowser::default(),
            dungeon_browser: DungeonBrowser::default(),
            project_panel: ProjectPanel::default(),
            copy_prompt: None,
            delete_prompt: None,
            unsaved_changes_prompt: None,
            exit_confirmed: false,
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
        if let Some(path) = project_file {
            app.open_project_file(&path);
        }
        if app.library.is_some() {
            if let Some(room_name) = startup_options.room_name {
                app.open_room(&room_name.to_ascii_uppercase());
            }
            if let Some(dungeon_name) = startup_options.dungeon_name {
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
                let room_names = library.rdb_block_names();
                self.room_browser.set_room_names(room_names.clone());
                let dungeons = library.dungeons();
                self.cell_usage.clear();
                for (index, dungeon) in dungeons.iter().enumerate() {
                    for block in &dungeon.blocks {
                        let users = self.cell_usage.entry(block.block_name.clone()).or_default();
                        if users.last() != Some(&index) {
                            users.push(index);
                        }
                    }
                }
                self.project_panel.set_daggerfall_contents(room_names, dungeons);
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
            }
            Err(error) => self.arena2_prompt.open(Some(error.to_string())),
        }
    }

    // -----------------------------------------------------------------------
    // Opening rooms and dungeons
    // -----------------------------------------------------------------------

    /// Opens `item` in the main window.
    fn open_item(&mut self, item: &OpenedItem) {
        match item {
            OpenedItem::DaggerfallRoom(room_name) => self.open_room(room_name),
            OpenedItem::DaggerfallDungeon(dungeon_index) => self.open_dungeon(*dungeon_index),
            OpenedItem::Project(id) => self.open_project_item(*id),
        }
    }

    /// Opens the Daggerfall room called `room_name`, read-only, in the main window.
    fn open_room(&mut self, room_name: &str) {
        let Some(library) = self.library.as_mut() else { return };
        match library.rdb_block(room_name) {
            Ok(block) => {
                let scene = Scene::from_rdb_block(&block);
                let title = format!("{room_name} ({})", room_browser::category_description(room_name));
                let mut editor = RoomEditor::new(scene, title, egui::Id::new("main_room_editor"), library);
                editor.set_read_only(true);
                self.status_message = editor.status_message().to_string();
                self.set_workspace(
                    Workspace::Room(Box::new(editor)),
                    Some(OpenedItem::DaggerfallRoom(room_name.to_string())),
                );
            }
            Err(error) => self.error_message = Some(format!("Could not open {room_name}: {error}")),
        }
    }

    /// Opens the Daggerfall dungeon at `dungeon_index` in the library's dungeon list, read-only.
    fn open_dungeon(&mut self, dungeon_index: usize) {
        let Some(library) = self.library.as_mut() else { return };
        let dungeons = library.dungeons();
        let Some(definition) = dungeons.get(dungeon_index) else { return };

        let layout = ProjectDungeonRoom::layout_of(definition);
        let (view, problems) = DungeonView::load(
            &definition.name,
            definition.region_name,
            definition.dungeon_type,
            &layout,
            &self.project,
            library,
        );
        self.show_dungeon(view, problems, OpenedItem::DaggerfallDungeon(dungeon_index));
    }

    /// Opens the first Daggerfall dungeon whose name matches `dungeon_name` (case-insensitive).
    fn open_dungeon_named(&mut self, dungeon_name: &str) {
        let Some(library) = self.library.as_mut() else { return };
        let dungeon_index =
            library.dungeons().iter().position(|dungeon| dungeon.name.eq_ignore_ascii_case(dungeon_name));
        match dungeon_index {
            Some(index) => self.open_dungeon(index),
            None => self.error_message = Some(format!("There is no dungeon called '{dungeon_name}'.")),
        }
    }

    /// Opens the project room or dungeon `id` in the main window.
    fn open_project_item(&mut self, id: ItemId) {
        if self.library.is_none() {
            self.error_message =
                Some("Choose your ARENA2 folder first: cells are drawn from the game data.".into());
            return;
        }
        // Store the open item's changes first, in case it is the one being reopened.
        self.set_workspace(Workspace::Empty, None);
        let entry = self.project.find(id).cloned();
        let Some(library) = self.library.as_mut() else { return };
        match entry {
            Some(ProjectEntry::Room(room)) => {
                let scene = Scene {
                    block_name: room.based_on.clone().unwrap_or_default(),
                    objects: room.objects.clone(),
                };
                let mut editor =
                    RoomEditor::new(scene, room.name.clone(), egui::Id::new("main_room_editor"), library);
                editor.set_project_room_name(Some(room.name.clone()));
                self.status_message = editor.status_message().to_string();
                self.set_workspace(Workspace::Room(Box::new(editor)), Some(OpenedItem::Project(id)));
            }
            Some(ProjectEntry::Dungeon(dungeon)) => {
                let (view, problems) = DungeonView::load(
                    &dungeon.name,
                    &dungeon.region_name,
                    dungeon.dungeon_type,
                    &dungeon.rooms,
                    &self.project,
                    library,
                );
                self.show_dungeon(view, problems, OpenedItem::Project(id));
            }
            _ => {}
        }
    }

    /// Shows a loaded dungeon in the main window, reporting rooms that could not be loaded.
    fn show_dungeon(&mut self, view: DungeonView, problems: Vec<String>, item: OpenedItem) {
        if !problems.is_empty() {
            self.error_message = Some(format!("Some cells could not be loaded:\n{}", problems.join("\n")));
        }
        self.status_message = view.status_text(&self.custom_names);
        self.set_workspace(Workspace::Dungeon(Box::new(view)), Some(item));
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
        let mut editor = RoomEditor::new(room.scene.clone(), title, editor_id, library);
        // Daggerfall's own rooms stay read-only, even in a project dungeon.
        editor.set_read_only(room.project_room.is_none());
        editor.set_dungeon_type(view.dungeon_type);
        // The dungeon it was opened from is selected under "Used in", so its
        // other cells show faded around this one.
        if let Some(dungeon) = self.opened_item.clone() {
            editor.select_used_in(Some(dungeon));
        }
        self.status_message = format!("Opened {} in a new window", room.block_name);
        self.room_windows.push(RoomWindow {
            viewport_id: egui::ViewportId::from_hash_of(("room_window", window_number)),
            editor,
            dungeon_room_index: Some(room_index),
            is_open: true,
            show_controls_window: false,
        });
    }

    /// Replaces the main window's workspace, which now shows `item`.
    ///
    /// Changes to the project item that was open are stored in the project
    /// first. Room windows belong to the old workspace, so they are closed.
    fn set_workspace(&mut self, workspace: Workspace, item: Option<OpenedItem>) {
        self.store_workspace_changes();
        if let Some(render_state) = self.render_state.as_ref() {
            match &mut self.workspace {
                Workspace::Room(editor) => editor.release(render_state),
                Workspace::Dungeon(view) => view.release(render_state),
                Workspace::Empty => {}
            }
        }
        self.close_room_windows();
        self.workspace = workspace;
        self.opened_item = item;
        self.copy_prompt = None;
    }

    /// Closes the workspace and every room window.
    fn close_everything(&mut self) {
        self.set_workspace(Workspace::Empty, None);
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

    // -----------------------------------------------------------------------
    // Project items
    // -----------------------------------------------------------------------

    /// Copies the open project room's or dungeon's edits into the project.
    fn store_workspace_changes(&mut self) {
        let Some(OpenedItem::Project(id)) = self.opened_item else { return };
        match &self.workspace {
            Workspace::Room(editor) => {
                if let Some(room) = self.project.room_mut(id) {
                    if room.objects != editor.scene.objects {
                        room.objects = editor.scene.objects.clone();
                    }
                }
            }
            Workspace::Dungeon(view) => {
                if self.project.dungeon(id).is_some() {
                    view.store_edited_rooms(&mut self.project);
                }
            }
            Workspace::Empty => {}
        }
    }

    /// The dungeons that use a cell: those using the project cell `project_room`,
    /// or, for a Daggerfall cell, the Daggerfall dungeons and project dungeons
    /// that use the original `block_name`. Project dungeons come first.
    fn used_in(&mut self, block_name: &str, project_room: Option<ItemId>) -> Vec<UsedInEntry> {
        let mut entries: Vec<UsedInEntry> = self
            .project
            .dungeons()
            .into_iter()
            .filter(|dungeon| {
                dungeon.rooms.iter().any(|placement| match project_room {
                    Some(room_id) => placement.room == Some(room_id),
                    None => placement.room.is_none() && placement.block_name == block_name,
                })
            })
            .map(|dungeon| UsedInEntry {
                label: format!("{} · your dungeon", dungeon.name),
                item: OpenedItem::Project(dungeon.id),
            })
            .collect();
        if project_room.is_none() {
            if let (Some(library), Some(users)) = (self.library.as_mut(), self.cell_usage.get(block_name)) {
                let dungeons = library.dungeons();
                entries.extend(users.iter().filter_map(|&index| {
                    let dungeon = dungeons.get(index)?;
                    Some(UsedInEntry {
                        label: format!("{} · {}", dungeon.name, dungeon.region_name),
                        item: OpenedItem::DaggerfallDungeon(index),
                    })
                }));
            }
        }
        entries
    }

    /// The cells of `dungeon` around this cell, positioned relative to it, for
    /// showing faded around the cell. The cell is found as in [`Self::used_in`];
    /// `layout_index` picks its place when the cell appears more than once and
    /// `dungeon` is the dungeon open in the main window.
    fn dungeon_context(
        &mut self,
        dungeon: &OpenedItem,
        block_name: &str,
        project_room: Option<ItemId>,
        layout_index: Option<usize>,
    ) -> Option<DungeonContext> {
        let library = self.library.as_mut()?;
        let layout = match dungeon {
            OpenedItem::DaggerfallDungeon(index) => {
                ProjectDungeonRoom::layout_of(library.dungeons().get(*index)?)
            }
            OpenedItem::Project(id) => self.project.dungeon(*id)?.rooms.clone(),
            OpenedItem::DaggerfallRoom(_) => return None,
        };
        let is_this_cell = |placement: &ProjectDungeonRoom| match project_room {
            Some(room_id) => placement.room == Some(room_id),
            None => placement.room.is_none() && placement.block_name == block_name,
        };
        let preferred = layout_index.filter(|&index| layout.get(index).is_some_and(is_this_cell));
        let here_index = preferred.or_else(|| layout.iter().position(is_this_cell))?;
        let here = &layout[here_index];

        let mut objects = Vec::new();
        let mut outlines = vec![RoomOutline { origin: Vec3::ZERO, highlighted: false }];
        let mut cells = Vec::new();
        for (index, placement) in layout.iter().enumerate() {
            if index == here_index {
                continue;
            }
            let offset = Vec3::new(
                (placement.grid_x - here.grid_x) as f32 * RDB_BLOCK_SIDE,
                0.0,
                (placement.grid_z - here.grid_z) as f32 * RDB_BLOCK_SIDE,
            );
            let Ok((scene, _)) = dungeon_view::load_room_scene(placement, &self.project, library) else {
                continue;
            };
            let cell_index = cells.len();
            objects.extend(scene.objects.into_iter().map(|mut object| {
                object.position += offset;
                object.group_index = cell_index;
                object
            }));
            outlines.push(RoomOutline { origin: offset, highlighted: false });
            let label = match placement.room.and_then(|room_id| self.project.room(room_id)) {
                Some(room) => format!("{} (your cell)", room.name),
                None => placement.block_name.clone(),
            };
            cells.push(ContextCell { label, layout_index: index });
        }
        Some(DungeonContext {
            dungeon: dungeon.clone(),
            scene: Scene { block_name: String::new(), objects },
            outlines,
            cells,
        })
    }

    /// The rooms of `dungeon`, in layout order.
    fn dungeon_layout(&mut self, dungeon: &OpenedItem) -> Option<Vec<ProjectDungeonRoom>> {
        match dungeon {
            OpenedItem::DaggerfallDungeon(index) => {
                Some(ProjectDungeonRoom::layout_of(self.library.as_mut()?.dungeons().get(*index)?))
            }
            OpenedItem::Project(id) => Some(self.project.dungeon(*id)?.rooms.clone()),
            OpenedItem::DaggerfallRoom(_) => None,
        }
    }

    /// Opens the cell at `layout_index` in `dungeon`, double-clicked among the
    /// faded cells around another one, keeping `dungeon` around it.
    ///
    /// From a cell window of the dungeon open in the main window, the cell opens
    /// in a cell window the same way; otherwise it opens in the main window.
    fn open_context_cell(
        &mut self,
        context: &egui::Context,
        dungeon: OpenedItem,
        layout_index: usize,
        from_window: bool,
    ) {
        if from_window && self.opened_item.as_ref() == Some(&dungeon) {
            if let Workspace::Dungeon(view) = &self.workspace {
                let room_index = (0..)
                    .map_while(|index| view.room(index).map(|_| index))
                    .find(|&index| view.layout_index(index) == Some(layout_index));
                if let Some(room_index) = room_index {
                    self.open_room_window(context, room_index);
                    return;
                }
            }
        }
        let Some(placement) =
            self.dungeon_layout(&dungeon).and_then(|layout| layout.get(layout_index).cloned())
        else {
            return;
        };
        match placement.room {
            Some(room_id) => self.open_project_item(room_id),
            None => self.open_room(&placement.block_name),
        }
        if let Workspace::Room(editor) = &mut self.workspace {
            editor.set_home_placement(dungeon.clone(), layout_index);
            editor.select_used_in(Some(dungeon));
        }
    }

    /// The cell open in the main window: its block name and project room.
    fn main_cell(&self) -> Option<(String, Option<ItemId>)> {
        match self.opened_item.clone()? {
            OpenedItem::DaggerfallRoom(block_name) => Some((block_name, None)),
            OpenedItem::Project(id) if matches!(self.workspace, Workspace::Room(_)) => {
                let block_name =
                    self.project.room(id).and_then(|room| room.based_on.clone()).unwrap_or_default();
                Some((block_name, Some(id)))
            }
            _ => None,
        }
    }

    /// "Used in" list for the cell open in the main window.
    fn main_used_in(&mut self) -> Vec<UsedInEntry> {
        match self.opened_item.clone() {
            Some(OpenedItem::DaggerfallRoom(block_name)) => self.used_in(&block_name, None),
            Some(OpenedItem::Project(id)) if matches!(self.workspace, Workspace::Room(_)) => {
                let block_name =
                    self.project.room(id).and_then(|room| room.based_on.clone()).unwrap_or_default();
                self.used_in(&block_name, Some(id))
            }
            _ => Vec::new(),
        }
    }

    /// "Used in" lists for each room window, in window order.
    fn room_window_used_in(&mut self) -> Vec<Vec<UsedInEntry>> {
        let cells: Vec<Option<(String, Option<ItemId>)>> = self
            .room_windows
            .iter()
            .map(|window| {
                let Workspace::Dungeon(view) = &self.workspace else { return None };
                let room = window.dungeon_room_index.and_then(|index| view.room(index))?;
                Some((room.block_name.clone(), room.project_room))
            })
            .collect();
        cells
            .into_iter()
            .map(|cell| match cell {
                Some((block_name, project_room)) => self.used_in(&block_name, project_room),
                None => Vec::new(),
            })
            .collect()
    }

    /// Shows the project's current names in the open workspace and room windows.
    fn show_current_names(&mut self) {
        let Some(OpenedItem::Project(id)) = self.opened_item else { return };
        let Some(name) = self.project.find(id).map(|entry| entry.name().to_string()) else { return };
        match &mut self.workspace {
            Workspace::Room(editor) if editor.title() != name => {
                editor.set_title(name.clone());
                editor.set_project_room_name(Some(name));
            }
            Workspace::Dungeon(view) => {
                view.name = name;
                // The type can change in the properties panel; room windows preview enemies for it.
                view.dungeon_type = self.project.dungeon(id).and_then(|dungeon| dungeon.dungeon_type);
                for window in &mut self.room_windows {
                    window.editor.set_dungeon_type(view.dungeon_type);
                    let Some(room) = window.dungeon_room_index.and_then(|index| view.room(index)) else {
                        continue;
                    };
                    let room_name = room
                        .project_room
                        .and_then(|room_id| self.project.room(room_id))
                        .map(|room| room.name.clone());
                    let title =
                        format!("{} in {}", room_name.as_deref().unwrap_or(&room.block_name), view.name);
                    if window.editor.title() != title {
                        window.editor.set_title(title);
                        window.editor.set_project_room_name(room_name);
                    }
                }
            }
            _ => {}
        }
    }

    /// Name a copy of the Daggerfall room `block_name` is based on: the user's
    /// name for the room if it has one, otherwise the block name without `.RDB`.
    fn room_copy_base_name(&self, block_name: &str) -> String {
        match self.custom_names.room_name(block_name) {
            Some(custom_name) => custom_name.to_string(),
            None => block_name.trim_end_matches(".RDB").to_string(),
        }
    }

    /// Adds a copy of the Daggerfall room or dungeon `item` to the project and returns its id.
    ///
    /// A room's objects come from `room_scene` if given (the open room),
    /// otherwise from the game data. The copy goes in `folder`, or the top level.
    fn add_copy_of_daggerfall_item(
        &mut self,
        item: &OpenedItem,
        room_scene: Option<&Scene>,
        folder: Option<ItemId>,
    ) -> Option<ItemId> {
        let library = self.library.as_mut()?;
        let entry = match item {
            OpenedItem::DaggerfallRoom(room_name) => {
                let scene = match room_scene {
                    Some(scene) => scene.clone(),
                    None => match library.rdb_block(room_name) {
                        Ok(block) => Scene::from_rdb_block(&block),
                        Err(error) => {
                            self.error_message = Some(format!("Could not copy {room_name}: {error}"));
                            return None;
                        }
                    },
                };
                let base_name = self.room_copy_base_name(room_name);
                ProjectEntry::Room(ProjectRoom {
                    id: self.project.new_id(),
                    name: self.project.copy_name(&base_name),
                    based_on: Some(room_name.clone()),
                    objects: scene.objects,
                })
            }
            OpenedItem::DaggerfallDungeon(dungeon_index) => {
                let dungeons = library.dungeons();
                let definition = dungeons.get(*dungeon_index)?;
                ProjectEntry::Dungeon(ProjectDungeon {
                    id: self.project.new_id(),
                    name: self.project.copy_name(&definition.name),
                    region_name: definition.region_name.to_string(),
                    dungeon_type: definition.dungeon_type,
                    based_on: Some(definition.name.clone()),
                    rooms: ProjectDungeonRoom::layout_of(definition),
                })
            }
            OpenedItem::Project(_) => return None,
        };
        let id = entry.id();
        self.status_message = format!("Created {} in the project", entry.name());
        self.project.add(entry, folder);
        Some(id)
    }

    /// Turns the open Daggerfall room or dungeon into a copy in the project.
    ///
    /// The open editor and room windows keep their camera and selection; only
    /// what they edit changes. A copied dungeon's rooms are still Daggerfall's
    /// own, so they stay read-only until they are copied too.
    fn convert_open_item_to_copy(&mut self) {
        let Some(item) = self.opened_item.clone() else { return };
        let open_room_scene = match &self.workspace {
            Workspace::Room(editor) => Some(editor.scene.clone()),
            _ => None,
        };
        let Some(id) = self.add_copy_of_daggerfall_item(&item, open_room_scene.as_ref(), None) else {
            return;
        };
        self.opened_item = Some(OpenedItem::Project(id));
        let message = format!("{}. You can now edit it.", self.status_message);
        match &mut self.workspace {
            Workspace::Room(editor) => {
                editor.set_read_only(false);
                editor.set_status_message(message);
            }
            Workspace::Dungeon(_) | Workspace::Empty => {}
        }
        // Titles and names follow the copy from the next frame on.
        self.show_current_names();
    }

    /// Copies the Daggerfall room edited in the room window `viewport` for the
    /// open dungeon, copying the dungeon first if it is Daggerfall's own too.
    /// The room window then edits the copy.
    fn copy_room_in_open_dungeon(&mut self, viewport: egui::ViewportId) {
        if matches!(self.opened_item, Some(OpenedItem::DaggerfallDungeon(_))) {
            self.convert_open_item_to_copy();
        }
        let Some(OpenedItem::Project(dungeon_id)) = self.opened_item else { return };
        let Some(window) = self.room_windows.iter().find(|window| window.viewport_id == viewport) else {
            return;
        };
        let Some(room_index) = window.dungeon_room_index else { return };
        let objects = window.editor.scene.objects.clone();
        let Workspace::Dungeon(view) = &self.workspace else { return };
        let (Some(layout_index), Some(room)) = (view.layout_index(room_index), view.room(room_index)) else {
            return;
        };
        let block_name = room.block_name.clone();

        let room_id = self.project.new_id();
        let name = self.project.copy_name(&self.room_copy_base_name(&block_name));
        let dungeon_name =
            self.project.dungeon(dungeon_id).map(|dungeon| dungeon.name.clone()).unwrap_or_default();
        self.project.add(
            ProjectEntry::Room(ProjectRoom {
                id: room_id,
                name: name.clone(),
                based_on: Some(block_name),
                objects,
            }),
            None,
        );
        if let Some(placement) =
            self.project.dungeon_mut(dungeon_id).and_then(|dungeon| dungeon.rooms.get_mut(layout_index))
        {
            placement.room = Some(room_id);
        }
        if let Workspace::Dungeon(view) = &mut self.workspace {
            view.set_project_room(room_index, room_id);
        }
        if let Some(window) = self.room_windows.iter_mut().find(|window| window.viewport_id == viewport) {
            window.editor.set_read_only(false);
            window
                .editor
                .set_status_message(format!("Created {name} for {dungeon_name}. You can now edit it."));
        }
        self.status_message = format!("Created {name} for {dungeon_name}");
        self.show_current_names();
    }

    /// Carries out what the user asked for in the project panel.
    fn handle_panel_action(&mut self, action: PanelAction) {
        match action {
            PanelAction::Open(item) => self.open_item(&item),
            PanelAction::CopyFromDaggerfall { item, folder, open } => {
                if open && self.opened_item.as_ref() == Some(&item) {
                    self.convert_open_item_to_copy();
                } else if let Some(id) = self.add_copy_of_daggerfall_item(&item, None, folder) {
                    if open {
                        self.open_project_item(id);
                    }
                }
            }
            PanelAction::Duplicate(id) => {
                self.store_workspace_changes();
                if let Some(copy_id) = self.project.duplicate(id) {
                    let name =
                        self.project.find(copy_id).map(|entry| entry.name().to_string()).unwrap_or_default();
                    self.status_message = format!("Created {name}");
                }
            }
            PanelAction::Delete(id) => self.delete_prompt = Some(id),
        }
    }

    /// Deletes the project room or dungeon `id`, closing it first if it is open.
    ///
    /// Dungeons that used a deleted room use the Daggerfall room again; one
    /// that is open is closed first so it does not keep showing the room.
    fn delete_project_item(&mut self, id: ItemId) {
        let open_dungeon_uses_room = match self.opened_item {
            Some(OpenedItem::Project(open_id)) => self
                .project
                .dungeon(open_id)
                .is_some_and(|dungeon| dungeon.rooms.iter().any(|placement| placement.room == Some(id))),
            _ => false,
        };
        if self.opened_item == Some(OpenedItem::Project(id)) || open_dungeon_uses_room {
            self.set_workspace(Workspace::Empty, None);
        }
        self.project.stop_using_room(id);
        if let Some(entry) = self.project.remove(id) {
            self.status_message = format!("Deleted {}", entry.name());
        }
    }

    // -----------------------------------------------------------------------
    // Project files
    // -----------------------------------------------------------------------

    /// Name of the project, from its file name.
    fn project_title(&self) -> String {
        self.project_path
            .as_deref()
            .and_then(Path::file_stem)
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| UNTITLED_PROJECT_NAME.to_string())
    }

    /// Whether the project changed since it was last saved or opened.
    fn has_unsaved_changes(&mut self) -> bool {
        self.store_workspace_changes();
        self.project.to_file_text() != self.saved_project_text
    }

    /// Runs `action`, first asking what to do with unsaved changes if there are any.
    fn request(&mut self, action: DiscardingAction) {
        if self.has_unsaved_changes() {
            self.unsaved_changes_prompt = Some(action);
        } else {
            self.perform(action);
        }
    }

    /// Runs `action` without asking about unsaved changes.
    fn perform(&mut self, action: DiscardingAction) {
        match action {
            DiscardingAction::NewProject => {
                self.set_workspace(Workspace::Empty, None);
                self.replace_project(Project::default(), None);
                self.status_message = "Started a new project".to_string();
            }
            DiscardingAction::OpenProject => {
                let dialog = rfd::FileDialog::new()
                    .set_title("Open project")
                    .add_filter("Daggerfall Dungeon Maker project", &[PROJECT_FILE_EXTENSION]);
                if let Some(path) = dialog.pick_file() {
                    self.open_project_file(&path);
                }
            }
            DiscardingAction::Exit => {
                self.exit_confirmed = true;
            }
        }
    }

    /// Opens the project saved at `path`, replacing the open one.
    fn open_project_file(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(project) => {
                self.set_workspace(Workspace::Empty, None);
                self.replace_project(project, Some(path.to_path_buf()));
                self.status_message = format!("Opened project {}", path.display());
            }
            Err(error) => self.error_message = Some(error),
        }
    }

    /// Makes `project` the open project, saved at `path`.
    fn replace_project(&mut self, project: Project, path: Option<PathBuf>) {
        self.saved_project_text = project.to_file_text();
        self.project = project;
        self.project_path = path;
    }

    /// Saves the project to its file, asking for one if it has never been saved.
    /// Returns `false` if it was not saved.
    fn save_project(&mut self) -> bool {
        match self.project_path.clone() {
            Some(path) => self.save_project_to(&path),
            None => self.save_project_as(),
        }
    }

    /// Asks for a file and saves the project to it. Returns `false` if it was not saved.
    fn save_project_as(&mut self) -> bool {
        let dialog = rfd::FileDialog::new()
            .set_title("Save project")
            .add_filter("Daggerfall Dungeon Maker project", &[PROJECT_FILE_EXTENSION])
            .set_file_name(format!("{}.{PROJECT_FILE_EXTENSION}", self.project_title()));
        let Some(mut path) = dialog.save_file() else { return false };
        if path.extension().is_none_or(|extension| !extension.eq_ignore_ascii_case(PROJECT_FILE_EXTENSION)) {
            path.as_mut_os_string().push(format!(".{PROJECT_FILE_EXTENSION}"));
        }
        self.save_project_to(&path)
    }

    /// Saves the project to `path`. Returns `false` (and reports the error) if it could not be written.
    fn save_project_to(&mut self, path: &Path) -> bool {
        self.store_workspace_changes();
        match self.project.save(path) {
            Ok(()) => {
                self.saved_project_text = self.project.to_file_text();
                self.project_path = Some(path.to_path_buf());
                self.status_message = format!("Saved project {}", path.display());
                true
            }
            Err(error) => {
                self.error_message = Some(error);
                false
            }
        }
    }

    // -----------------------------------------------------------------------
    // Menus, shortcuts and windows
    // -----------------------------------------------------------------------

    /// Handles keyboard shortcuts of the main window.
    fn handle_shortcuts(&mut self, context: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};

        let shortcut = |modifiers, key| {
            context.input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            self.save_project_as();
        }
        if shortcut(Modifiers::COMMAND, Key::S) {
            self.save_project();
        }
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
                if let (Workspace::Room(editor), Some(_)) = (&mut self.workspace, library.as_ref()) {
                    ui.menu_button("Edit", |ui| editor.edit_menu(ui));
                }
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.project_panel.is_open, "Project panel");
                    match (&mut self.workspace, library) {
                        (Workspace::Room(editor), Some(library)) => {
                            ui.separator();
                            editor.view_menu(ui, library);
                        }
                        (Workspace::Dungeon(view), Some(library)) => {
                            ui.separator();
                            view.view_menu(ui, library);
                        }
                        _ => {}
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Viewport controls").clicked() {
                        self.show_controls_window = true;
                        ui.close_menu();
                    }
                });
                if let Workspace::Room(editor) = &mut self.workspace {
                    ui.separator();
                    editor.toolbar(ui);
                }
            });
        });
    }

    /// Adds the File menu entries.
    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("New project").clicked() {
            self.request(DiscardingAction::NewProject);
            ui.close_menu();
        }
        if ui.button("Open project…").clicked() {
            self.request(DiscardingAction::OpenProject);
            ui.close_menu();
        }
        if ui.add(egui::Button::new("Save project").shortcut_text("Ctrl+S")).clicked() {
            self.save_project();
            ui.close_menu();
        }
        if ui.add(egui::Button::new("Save project as…").shortcut_text("Ctrl+Shift+S")).clicked() {
            self.save_project_as();
            ui.close_menu();
        }
        ui.separator();
        let has_library = self.library.is_some();
        if ui.add_enabled(has_library, egui::Button::new("Open cell…").shortcut_text("Ctrl+O")).clicked() {
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
            self.set_workspace(Workspace::Empty, None);
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

    /// Draws the project panel and carries out what the user asks for in it.
    fn show_project_panel(&mut self, context: &egui::Context) {
        let title = self.project_title();
        let action = self.project_panel.show(
            context,
            &mut self.project,
            &title,
            &self.custom_names,
            self.opened_item.as_ref(),
        );
        if let Some(action) = action {
            self.handle_panel_action(action);
        }
    }

    /// Draws the main window's workspace.
    fn show_workspace(&mut self, context: &egui::Context) {
        let mut main_used_in = self.main_used_in();
        let mut open_request = None;
        let mut context_request = None;
        let mut cell_open_request = None;
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
        let mut properties_action = None;
        match &mut self.workspace {
            Workspace::Room(editor) => {
                editor.set_used_in(std::mem::take(&mut main_used_in));
                editor.show(context, &mut resources);
                open_request = editor.take_open_request();
                context_request =
                    editor.take_context_request().map(|dungeon| {
                        let home = dungeon.as_ref().and_then(|dungeon| editor.home_layout_index(dungeon));
                        (dungeon, home)
                    });
                cell_open_request = editor.take_cell_open_request();
                if editor.take_blocked_edit() && self.copy_prompt.is_none() {
                    self.copy_prompt = Some(CopyPrompt { subject: CopySubject::Room, viewport: None });
                }
            }
            Workspace::Dungeon(view) => {
                let open_dungeon = match self.opened_item {
                    Some(OpenedItem::Project(id)) => self.project.dungeon(id),
                    _ => None,
                };
                let details = DungeonDetails {
                    read_only: open_dungeon.is_none(),
                    based_on: open_dungeon.and_then(|dungeon| dungeon.based_on.as_deref()),
                    project: &self.project,
                    custom_names: resources.custom_names,
                };
                properties_action = view.show_properties(context, &details);
                egui::CentralPanel::default().frame(egui::Frame::none()).show(context, |ui| {
                    room_to_open = view.show(ui, &mut resources);
                });
                if let Some(room_index) = view.take_make_start_request() {
                    properties_action = Some(DungeonPropertiesAction::MakeStart(room_index));
                }
            }
            Workspace::Empty => show_centered_hint(
                context,
                "Click a dungeon or double-click a cell in the project panel, or use File > Open cell… / Open dungeon…",
            ),
        }
        if let Some(room_index) = room_to_open {
            self.open_room_window(context, room_index);
        }
        if let Some(action) = properties_action {
            self.handle_dungeon_properties_action(context, action);
        }
        // Load the surrounding cells of a dungeon newly selected under "Used in".
        if let Some((dungeon, home)) = context_request {
            let context = match (dungeon, self.main_cell()) {
                (Some(dungeon), Some((block_name, project_room))) => {
                    self.dungeon_context(&dungeon, &block_name, project_room, home)
                }
                _ => None,
            };
            if let Workspace::Room(editor) = &mut self.workspace {
                editor.set_dungeon_context(context);
            }
        }
        if let Some((dungeon, layout_index)) = cell_open_request {
            self.open_context_cell(context, dungeon, layout_index, false);
        }
        if let Some(item) = open_request {
            self.open_item(&item);
        }
    }

    /// Carries out what the user asked for in the dungeon properties panel.
    fn handle_dungeon_properties_action(&mut self, context: &egui::Context, action: DungeonPropertiesAction) {
        let open_project_dungeon = match self.opened_item {
            Some(OpenedItem::Project(id)) => self.project.dungeon_mut(id),
            _ => None,
        };
        match (action, open_project_dungeon) {
            (DungeonPropertiesAction::Rename(name), Some(dungeon)) => dungeon.name = name,
            (DungeonPropertiesAction::SetType(dungeon_type), Some(dungeon)) => {
                dungeon.dungeon_type = Some(dungeon_type);
                self.status_message = format!("{} is now a {}", dungeon.name, dungeon_type.name());
            }
            (DungeonPropertiesAction::OpenRoom(room_index), _) => self.open_room_window(context, room_index),
            (DungeonPropertiesAction::MakeStart(room_index), Some(_)) => self.make_start_room(room_index),
            (DungeonPropertiesAction::MakeStart(_), None) if self.copy_prompt.is_none() => {
                self.copy_prompt = Some(CopyPrompt { subject: CopySubject::Dungeon, viewport: None });
            }
            (DungeonPropertiesAction::EditBlocked, _) if self.copy_prompt.is_none() => {
                self.copy_prompt = Some(CopyPrompt { subject: CopySubject::Dungeon, viewport: None });
            }
            _ => {}
        }
    }

    /// Makes room `room_index` of the open project dungeon its starting cell,
    /// in the project and in the view; whichever cell was the start stops being it.
    fn make_start_room(&mut self, room_index: usize) {
        let Some(OpenedItem::Project(id)) = self.opened_item else { return };
        let Workspace::Dungeon(view) = &mut self.workspace else { return };
        let Some(layout_index) = view.layout_index(room_index) else { return };
        if let Some(problems) = view.room(room_index).map(|room| room.start_problems()) {
            if !problems.is_empty() {
                self.status_message = format!("{}, so it cannot be the start", problems.join(" and "));
                return;
            }
        }
        let Some(dungeon) = self.project.dungeon_mut(id) else { return };
        for (index, placement) in dungeon.rooms.iter_mut().enumerate() {
            placement.is_starting_block = index == layout_index;
        }
        let block_name = dungeon.rooms.get(layout_index).map(|placement| placement.block_name.clone());
        // Cell windows hold their cell's objects and copy them back every frame,
        // which would undo the change of exit doors, so they are closed.
        self.close_room_windows();
        let (Workspace::Dungeon(view), Some(library)) = (&mut self.workspace, self.library.as_mut()) else {
            return;
        };
        view.set_start_room(room_index, library);
        self.status_message = format!("{} is now the start cell", block_name.unwrap_or_default());
    }

    /// Draws every room editor window and syncs their edits back into the dungeon view.
    fn show_room_windows(&mut self, context: &egui::Context) {
        let window_used_in = self.room_window_used_in();
        let mut open_request = None;
        let mut context_requests = Vec::new();
        let mut cell_open_request = None;
        let Some(library) = self.library.as_mut() else { return };
        let mut resources = EditorResources {
            library,
            render_state: self.render_state.as_ref(),
            renderer: self.renderer.as_mut(),
            catalog: &self.catalog,
            custom_names: &mut self.custom_names,
        };

        let prompt = self.copy_prompt;
        let mut copy_choice = None;
        for (window, used_in) in self.room_windows.iter_mut().zip(window_used_in) {
            window.editor.set_used_in(used_in);
            // egui compares the builder with the previous frame's, so a renamed room updates the title.
            let builder = egui::ViewportBuilder::default()
                .with_title(editor_window_title(&window.editor, resources.custom_names))
                .with_inner_size(ROOM_WINDOW_SIZE);
            context.show_viewport_immediate(window.viewport_id, builder, |window_context, _class| {
                show_room_window_contents(window_context, window, &mut resources);
                if let Some(prompt) = prompt.filter(|prompt| prompt.viewport == Some(window.viewport_id)) {
                    copy_choice = show_copy_prompt(window_context, prompt.subject).or(copy_choice);
                }
            });

            if let Some(item) = window.editor.take_open_request() {
                open_request = Some(item);
            }
            if let Some(dungeon) = window.editor.take_context_request() {
                context_requests.push((window.viewport_id, dungeon));
            }
            if let Some(request) = window.editor.take_cell_open_request() {
                cell_open_request = Some(request);
            }
            if window.editor.take_blocked_edit() && self.copy_prompt.is_none() {
                let subject = match self.opened_item {
                    Some(OpenedItem::DaggerfallDungeon(_)) => CopySubject::DungeonAndRoom,
                    _ => CopySubject::RoomInDungeon,
                };
                self.copy_prompt = Some(CopyPrompt { subject, viewport: Some(window.viewport_id) });
            }
            if let (Some(room_index), Workspace::Dungeon(view)) =
                (window.dungeon_room_index, &mut self.workspace)
            {
                view.update_room_objects(room_index, &window.editor.scene.objects);
            }
        }

        // The prompt belongs to a window that has since closed.
        if let Some(CopyPrompt { viewport: Some(viewport), .. }) = self.copy_prompt {
            if !self.room_windows.iter().any(|window| window.viewport_id == viewport && window.is_open) {
                self.copy_prompt = None;
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

        // Load the surrounding cells of dungeons newly selected under "Used in".
        for (viewport_id, dungeon) in context_requests {
            let Some(window_index) =
                self.room_windows.iter().position(|window| window.viewport_id == viewport_id)
            else {
                continue;
            };
            let room_index = self.room_windows[window_index].dungeon_room_index;
            let cell = match &self.workspace {
                Workspace::Dungeon(view) => room_index.and_then(|index| {
                    let room = view.room(index)?;
                    Some((room.block_name.clone(), room.project_room, view.layout_index(index)))
                }),
                _ => None,
            };
            let context = match (dungeon, cell) {
                (Some(dungeon), Some((block_name, project_room, layout_index))) => {
                    // The window's own place counts only in the dungeon it was opened from.
                    let layout_index = layout_index.filter(|_| self.opened_item.as_ref() == Some(&dungeon));
                    self.dungeon_context(&dungeon, &block_name, project_room, layout_index)
                }
                _ => None,
            };
            self.room_windows[window_index].editor.set_dungeon_context(context);
        }

        if let Some(choice) = copy_choice {
            self.answer_copy_prompt(choice);
        }
        if let Some((dungeon, layout_index)) = cell_open_request {
            self.open_context_cell(context, dungeon, layout_index, true);
        }
        if let Some(item) = open_request {
            self.open_item(&item);
        }
    }

    /// Acts on the answer to the copy prompt.
    fn answer_copy_prompt(&mut self, choice: CopyChoice) {
        let Some(prompt) = self.copy_prompt.take() else { return };
        if choice == CopyChoice::Cancel {
            return;
        }
        match prompt.viewport {
            Some(viewport) => self.copy_room_in_open_dungeon(viewport),
            None => self.convert_open_item_to_copy(),
        }
    }

    /// Draws the copy prompt in the main window, if it belongs there.
    fn show_main_copy_prompt(&mut self, context: &egui::Context) {
        let Some(prompt) = self.copy_prompt.filter(|prompt| prompt.viewport.is_none()) else { return };
        if let Some(choice) = show_copy_prompt(context, prompt.subject) {
            self.answer_copy_prompt(choice);
        }
    }

    /// Asks the user to confirm deleting a project room or dungeon.
    fn show_delete_prompt(&mut self, context: &egui::Context) {
        let Some(id) = self.delete_prompt else { return };
        let Some(name) = self.project.find(id).map(|entry| entry.name().to_string()) else {
            self.delete_prompt = None;
            return;
        };
        let users = self.project.dungeons_using(id).join(", ");
        let mut answer = None;
        modal_window(context, "Delete", |ui| {
            ui.label(format!("Delete \"{name}\" from the project?"));
            if !users.is_empty() {
                ui.label(format!("{users} will use the Daggerfall cell again."));
            }
            ui.label(egui::RichText::new("This cannot be undone once the project is saved.").weak());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(confirmed) = answer {
            self.delete_prompt = None;
            if confirmed {
                self.delete_project_item(id);
            }
        }
    }

    /// Asks whether to save unsaved changes before discarding them.
    fn show_unsaved_changes_prompt(&mut self, context: &egui::Context) {
        let Some(action) = self.unsaved_changes_prompt else { return };
        let title = self.project_title();
        let mut answer = None;
        modal_window(context, "Unsaved changes", |ui| {
            ui.label(format!("Save changes to \"{title}\"?"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    answer = Some(Some(true));
                }
                if ui.button("Don't save").clicked() {
                    answer = Some(Some(false));
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(None);
                }
            });
        });
        let Some(answer) = answer else { return };
        self.unsaved_changes_prompt = None;
        match answer {
            Some(true) => {
                if self.save_project() {
                    self.perform(action);
                }
            }
            Some(false) => self.perform(action),
            None => {}
        }
        if self.exit_confirmed {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Stops the program closing while the project has unsaved changes, and asks first.
    fn handle_close_request(&mut self, context: &egui::Context) {
        if !context.input(|input| input.viewport().close_requested()) || self.exit_confirmed {
            return;
        }
        if self.has_unsaved_changes() {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.unsaved_changes_prompt = Some(DiscardingAction::Exit);
        }
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
        let item = match &self.workspace {
            Workspace::Room(editor) => Some(editor_title(editor, &self.custom_names)),
            Workspace::Dungeon(view) => Some(view.description()),
            Workspace::Empty => None,
        };
        let project = self.project_title();
        let title = match item {
            Some(item) => format!("{item} - {project} - {APPLICATION_NAME}"),
            None => format!("{project} - {APPLICATION_NAME}"),
        };
        if title != self.window_title {
            context.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }
}

impl eframe::App for DungeonMakerApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_close_request(context);
        self.handle_shortcuts(context);
        self.show_current_names();
        self.show_menu_bar(context);
        self.show_status_bar(context);
        self.show_project_panel(context);
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
        self.show_room_windows(context);
        self.show_main_copy_prompt(context);
        self.show_delete_prompt(context);
        self.show_unsaved_changes_prompt(context);
        self.show_error_window(context);
        self.update_window_title(context);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(library) = &self.library {
            storage.set_string(ARENA2_FOLDER_STORAGE_KEY, library.root().display().to_string());
        }
        let project_file =
            self.project_path.as_deref().map(|path| path.display().to_string()).unwrap_or_default();
        storage.set_string(PROJECT_FILE_STORAGE_KEY, project_file);
        eframe::set_value(storage, CUSTOM_NAMES_STORAGE_KEY, &self.custom_names);
    }
}

/// Short title of the room in an editor: the project room's name, or the
/// Daggerfall room with the user's name for it.
fn editor_title(editor: &RoomEditor, custom_names: &CustomNames) -> String {
    match custom_names.room_name(&editor.scene.block_name).filter(|_| !editor.is_project_room()) {
        Some(room_name) => format!("{room_name} - {}", editor.title()),
        None => editor.title().to_string(),
    }
}

/// Window title of a room editor window.
fn editor_window_title(editor: &RoomEditor, custom_names: &CustomNames) -> String {
    format!("{} - {APPLICATION_NAME}", editor_title(editor, custom_names))
}

/// Draws the "You cannot edit … from Daggerfall" question. Returns the user's answer once given.
fn show_copy_prompt(context: &egui::Context, subject: CopySubject) -> Option<CopyChoice> {
    let (message, explanation) = match subject {
        CopySubject::Room => ("You cannot edit cells from Daggerfall", None),
        CopySubject::Dungeon => ("You cannot edit dungeons from Daggerfall", None),
        CopySubject::DungeonAndRoom => (
            "You cannot edit dungeons from Daggerfall",
            Some("Create a copy copies the dungeon and this cell."),
        ),
        CopySubject::RoomInDungeon => (
            "You cannot edit cells from Daggerfall",
            Some("Create a copy copies this cell for this dungeon."),
        ),
    };
    let mut choice = None;
    modal_window(context, "Read-only", |ui| {
        ui.label(message);
        if let Some(explanation) = explanation {
            ui.label(egui::RichText::new(explanation).weak());
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Create a copy").clicked() {
                choice = Some(CopyChoice::CreateCopy);
            }
            if ui.button("Cancel").clicked() {
                choice = Some(CopyChoice::Cancel);
            }
        });
    });
    choice
}

/// Draws a small dialog in the middle of the screen over a dimmed background
/// that swallows clicks, so the rest of the window cannot be used until it is answered.
fn modal_window(context: &egui::Context, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let screen = context.screen_rect();
    egui::Area::new(egui::Id::new(("modal_background", title)))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(context, |ui| {
            ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(110));
            ui.allocate_rect(screen, egui::Sense::click_and_drag());
        });
    let dialog = egui::Area::new(egui::Id::new(("modal_dialog", title)))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(context, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.set_min_width(260.0);
                ui.strong(title);
                ui.separator();
                add_contents(ui);
            });
        });
    // Keep the dialog above its dimmed background.
    context.move_to_top(dialog.response.layer_id);
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
            ui.separator();
            window.editor.toolbar(ui);
        });
    });
    egui::TopBottomPanel::bottom(editor_id.with("status_bar")).show(context, |ui| {
        ui.horizontal(|ui| {
            ui.label(window.editor.status_message());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Changes appear in the dungeon view.").weak());
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
    const CONTROLS: [(&str, &str); 20] = [
        ("Hover a cell (dungeon view)", "Highlight the whole cell"),
        ("Double-click a cell (dungeon view)", "Open it in a cell editor window"),
        ("Click an asset tile", "Arm that asset for placement"),
        ("Left click (while placing)", "Place the asset; hold Shift to keep placing"),
        ("R / Esc (while placing)", "Rotate the asset 90° / stop placing"),
        ("Left click", "Select object"),
        ("Left drag on arrow", "Move selection along that axis"),
        ("G", "Turn grid lock on or off"),
        ("Ctrl while dragging", "Switch grid lock for that drag"),
        ("Right drag", "Orbit"),
        ("Middle drag / Shift + right drag", "Pan"),
        ("Mouse wheel", "Zoom"),
        ("Right mouse + W A S D", "Fly forward / left / back / right"),
        ("Right mouse + Q / E", "Fly down / up (hold Shift to go faster)"),
        ("F", "Frame selection"),
        ("Home", "Frame cell or dungeon"),
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

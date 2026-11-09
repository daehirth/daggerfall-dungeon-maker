//! A complete editor for one room: outliner, properties, asset palette and 3D viewport.
//!
//! A [`RoomEditor`] owns everything specific to the room being edited
//! (scene, camera, selection, undo history) and borrows the shared game data
//! and renderer through [`EditorResources`]. That lets the same editor run in
//! the main window or in its own window opened from the dungeon view.

mod asset_palette;
mod history;
mod panels;
mod viewport;

use eframe::{egui, egui_wgpu};
use glam::Vec3;

use super::project_panel::OpenedItem;
use super::viewport_tools::EditorResources;
use crate::arena2::{Arena2Library, DoorKind, RDB_BLOCK_SIDE};
use crate::camera::OrbitCamera;
use crate::encounters::{enemy_chances, DungeonType, EncounterTable, EnemyChance};
use crate::render::{RoomOutline, ViewSettings, ViewportTarget};
use crate::scene::{
    ActionMotion, ObjectAction, ObjectId, Scene, SceneObject, SceneObjectKind, ADDED_OBJECTS_GROUP,
    OPEN_DOOR_ACTION, OUTSIDE_CELL_TOLERANCE,
};
use asset_palette::AssetPalette;
use history::{EditHistory, SceneEdit};
use viewport::ViewportState;

/// How far a duplicated object is offset from the original, in world units.
const DUPLICATE_OFFSET: Vec3 = Vec3::new(1.0, 0.0, 0.0);
/// Numbers every editor made, so copied objects know which editor they came from.
static NEXT_EDITOR_INSTANCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Objects copied with Ctrl+C. They are kept in egui's memory, so every cell
/// editor (the main window's and each cell window's) can paste them.
#[derive(Clone)]
struct CopiedObjects {
    objects: Vec<SceneObject>,
    /// The editor they were copied in (see [`RoomEditor::instance`]).
    source: u64,
    /// The text put on the system clipboard with them. Ctrl+V only reaches
    /// the program with the clipboard's text, so a paste counts when it
    /// brings this text back, and not after something else was copied.
    marker: String,
    /// The editor pasted into last, and how many times in a row, so each
    /// paste there lands a little further along.
    pasted_into: Option<u64>,
    pastes: u32,
}

/// Where [`CopiedObjects`] are kept in egui's memory.
fn copied_objects_id() -> egui::Id {
    egui::Id::new("ddm_copied_objects")
}

/// Radius used to frame objects without a mesh (flats and lights), in world units.
const DEFAULT_FOCUS_RADIUS: f32 = 2.0;
/// Highest player level offered when previewing random enemies; past it the tables no longer change.
const MAX_PREVIEW_LEVEL: u32 = 30;

/// Snap steps offered for grid lock, in world units, with their labels. Each
/// divides the 3.2 m floor grid evenly, so snap lines meet its lines.
const SNAP_STEPS: [(f32, &str); 4] = [
    (0.4, "0.4 m (16 units)"),
    (0.8, "0.8 m (32 units)"),
    (1.6, "1.6 m (64 units)"),
    (3.2, "3.2 m (128 units)"),
];

/// Grid lock: snapping objects to a grid when moving or placing them.
#[derive(Debug, Clone, Copy)]
pub struct SnapSettings {
    /// Whether grid lock is on. Holding Ctrl while dragging inverts this for that drag.
    pub enabled: bool,
    /// Snap increment in world units, one of [`SNAP_STEPS`].
    pub step: f32,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            step: 0.4, // 16 native units.
        }
    }
}

/// Editor state for one room.
pub struct RoomEditor {
    /// The room being edited.
    pub scene: Scene,
    /// Title shown in window titles, for example the room name and its dungeon.
    title: String,
    /// Prefix for widget ids so several editors can be open at once.
    id: egui::Id,
    /// Number of this editor, unique for as long as the program runs.
    instance: u64,
    /// The selected object; with several selected, the one selected last.
    selected_object: Option<usize>,
    /// Every selected object when several are (including `selected_object`). It
    /// only counts while it contains `selected_object`, so selecting one object
    /// any other way ends the multiple selection.
    selection_set: Vec<usize>,
    camera: OrbitCamera,
    view_settings: ViewSettings,
    snap: SnapSettings,
    history: EditHistory,
    /// Object currently being edited in the properties panel, so a drag records one undo step.
    property_edit_session: Option<usize>,
    viewport: ViewportState,
    palette: AssetPalette,
    /// Height of the floor grid, placed just under the room.
    grid_height: f32,
    target: ViewportTarget,
    status_message: String,
    /// Set for rooms from Daggerfall, which can be looked at but not changed.
    read_only: bool,
    /// Set when the user tried to change a read-only room, so the application can offer a copy.
    edit_blocked: bool,
    /// Name of the project room being edited; `None` for rooms from Daggerfall.
    project_room_name: Option<String>,
    /// Type of the dungeon the room was opened from, which decides what random enemy markers become.
    dungeon_type: Option<DungeonType>,
    /// Player level used to preview which enemies random enemy markers can become.
    enemy_preview_level: u32,
    /// Dungeons that use this cell, listed under "Used in"; set by the application.
    used_in: Vec<UsedInEntry>,
    /// A dungeon the user double-clicked under "Used in", for the application to open.
    open_request: Option<OpenedItem>,
    /// An action being played back in the viewport, if any.
    preview: Option<ActionPreview>,
    /// Open faces of the models placed or being placed, for snapping, by model id.
    open_faces: std::collections::HashMap<u32, std::sync::Arc<[crate::connectors::OpenFace]>>,
    /// Openings of the cell's models that lead to empty space, shown as yellow panes.
    unclosed: Vec<crate::connectors::UnclosedFace>,
    /// What the openings were last checked for (a hash of the objects), and when.
    unclosed_checked: Option<(u64, f64)>,
    /// The dungeon selected under "Used in", whose other cells are shown faded around this one.
    selected_dungeon: Option<OpenedItem>,
    /// The dungeon and layout place this cell was opened at, so the cells around
    /// that place (not another use of the same cell) are shown for that dungeon.
    home_placement: Option<(OpenedItem, usize)>,
    /// How far each object reaches outside the cell's square, refreshed every frame.
    outside_cell: Vec<f32>,
    /// A surrounding cell the user double-clicked: its dungeon and layout place.
    cell_open_request: Option<(OpenedItem, usize)>,
    /// Set when the dungeon was selected for the user, so its row scrolls into view once.
    scroll_to_selected_dungeon: bool,
    /// Set when `selected_dungeon` changes, for the application to load its cells.
    context_request: Option<Option<OpenedItem>>,
    /// The selected dungeon's other cells, placed around this one, and their outlines.
    dungeon_context: Option<DungeonContext>,
}

/// The cells around the edited one in a dungeon, shown faded for context.
pub struct DungeonContext {
    /// The dungeon they belong to.
    pub dungeon: OpenedItem,
    /// Their objects, positioned relative to the edited cell. Each object's
    /// `group_index` is the index of its cell in `cells`.
    pub scene: Scene,
    /// Outlines of the edited cell (first, at the origin) and of each surrounding
    /// cell, in the order of `cells`.
    pub outlines: Vec<RoomOutline>,
    /// The surrounding cells.
    pub cells: Vec<ContextCell>,
}

/// One cell shown faded around the edited one.
pub struct ContextCell {
    /// Name shown when the pointer is over it.
    pub label: String,
    /// Its place in the dungeon's layout.
    pub layout_index: usize,
}

/// How long a finished preview stays in its end state before everything returns.
const PREVIEW_HOLD_SECONDS: f64 = 0.5;

/// An action chain being played back in the viewport, without changing the cell.
struct ActionPreview {
    /// Time the preview started, from egui's clock.
    started: f64,
    /// Each moving object and how it moves; they all start together, as in the game.
    motions: Vec<(usize, ActionMotion)>,
}

/// A dungeon listed under "Used in".
pub struct UsedInEntry {
    pub label: String,
    pub item: OpenedItem,
}

/// What the right-click menu of an object offers.
pub(super) struct ObjectMenuInfo {
    name: String,
    /// Whether the object can have an action (models and flats).
    can_tie: bool,
    /// How many objects' actions trigger this object.
    triggered_by: usize,
}

/// What the user chose in an object's right-click menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObjectMenuChoice {
    TieAction,
    RemoveActions,
}

/// Draws the entries of an object's right-click menu. Returns the user's choice.
pub(super) fn object_menu(ui: &mut egui::Ui, info: &ObjectMenuInfo) -> Option<ObjectMenuChoice> {
    let mut choice = None;
    ui.label(egui::RichText::new(&info.name).weak());
    if info.can_tie
        && ui.button("Tie action to…").on_hover_text("Choose the object this one's action triggers").clicked()
    {
        choice = Some(ObjectMenuChoice::TieAction);
    }
    if info.triggered_by > 0 {
        let hover = format!("Remove the action of the {} object(s) that trigger this one", info.triggered_by);
        if ui.button("Remove actions").on_hover_text(hover).clicked() {
            choice = Some(ObjectMenuChoice::RemoveActions);
        }
    }
    if !info.can_tie && info.triggered_by == 0 {
        ui.label(egui::RichText::new("Lights cannot have actions").weak());
    }
    if choice.is_some() {
        ui.close_menu();
    }
    choice
}

impl RoomEditor {
    /// Creates an editor for `scene` and frames the camera on it.
    pub fn new(scene: Scene, title: String, id: egui::Id, library: &mut Arena2Library) -> Self {
        let status_message = format!("{title}: {} objects", scene.objects.len());
        let mut editor = Self {
            scene,
            title,
            id,
            instance: NEXT_EDITOR_INSTANCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            selected_object: None,
            selection_set: Vec::new(),
            camera: OrbitCamera::default(),
            view_settings: ViewSettings::default(),
            snap: SnapSettings::default(),
            history: EditHistory::default(),
            property_edit_session: None,
            viewport: ViewportState::default(),
            palette: AssetPalette::default(),
            grid_height: 0.0,
            target: ViewportTarget::default(),
            status_message,
            read_only: false,
            edit_blocked: false,
            project_room_name: None,
            dungeon_type: None,
            enemy_preview_level: 1,
            used_in: Vec::new(),
            open_request: None,
            preview: None,
            open_faces: std::collections::HashMap::new(),
            unclosed: Vec::new(),
            unclosed_checked: None,
            selected_dungeon: None,
            scroll_to_selected_dungeon: false,
            home_placement: None,
            cell_open_request: None,
            outside_cell: Vec::new(),
            context_request: None,
            dungeon_context: None,
        };
        editor.frame_room(library);
        if let Some((minimum, _)) = editor.scene.bounds(library) {
            editor.grid_height = minimum.y - 0.05;
        }
        editor
    }

    /// Title of the room, for window titles.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Changes the title of the room, for example after it was renamed.
    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    /// Makes the room read-only (a room from Daggerfall) or editable (a project room).
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
        self.edit_blocked = false;
    }

    /// Sets the project room name shown above the outliner; `None` for rooms from Daggerfall.
    pub fn set_project_room_name(&mut self, name: Option<String>) {
        self.project_room_name = name;
    }

    /// Sets the type of the dungeon the room is in, to preview what random enemy markers become.
    pub fn set_dungeon_type(&mut self, dungeon_type: Option<DungeonType>) {
        self.dungeon_type = dungeon_type;
    }

    /// The table the random enemy marker `object_index` picks from: the
    /// underwater table below the cell's water level, otherwise the dungeon's.
    /// `None` above water in a cell that is not open from a dungeon.
    fn encounter_table(&self, object_index: usize) -> Option<EncounterTable> {
        let object = self.scene.objects.get(object_index)?;
        if self.scene.is_under_water(object) {
            return Some(EncounterTable::Underwater);
        }
        self.dungeon_type.map(EncounterTable::Dungeon)
    }

    /// The enemies the random enemy marker `object_index` can become at the
    /// preview level, most likely first; `None` if that depends on an unknown dungeon.
    fn possible_enemies(&self, object_index: usize) -> Option<Vec<EnemyChance>> {
        self.encounter_table(object_index).map(|table| enemy_chances(table, self.enemy_preview_level))
    }

    /// What moves when object `start`'s action is triggered: every object down
    /// its action chain, to the end, whose action moves it, each with its motion.
    /// Doors whose action opens them swing open. Empty if nothing moves.
    pub(super) fn preview_motions(
        &self,
        start: usize,
        library: &mut Arena2Library,
    ) -> Vec<(usize, ActionMotion)> {
        self.scene
            .action_chain(start)
            .into_iter()
            .filter_map(|index| Some((index, self.object_motion(index, library)?)))
            .collect()
    }

    /// How object `index` moves when its action plays, if it moves.
    pub(super) fn object_motion(&self, index: usize, library: &mut Arena2Library) -> Option<ActionMotion> {
        match self.scene.objects.get(index)?.kind {
            SceneObjectKind::Model { model_id, data } => {
                let action = data.action?;
                if let Some(motion) = action.motion() {
                    Some(motion)
                } else if action.action_type == OPEN_DOOR_ACTION
                    && library.model(model_id).and_then(|mesh| mesh.door_kind()) == Some(DoorKind::Building)
                {
                    Some(ActionMotion::door_swing())
                } else {
                    None
                }
            }
            SceneObjectKind::Flat { data, .. } if data.action != 0 => {
                // Flats' actions use their magnitude byte as both axis and magnitude, with no duration.
                let [magnitude, _] = data.faction_or_mobile_id.to_le_bytes();
                let action = ObjectAction {
                    action_type: data.action,
                    axis: magnitude,
                    duration: 0,
                    magnitude: u16::from(magnitude),
                };
                action.motion()
            }
            _ => None,
        }
    }

    /// Starts playing object `index`'s action chain in the viewport.
    pub(super) fn start_preview(&mut self, index: usize, now: f64, library: &mut Arena2Library) {
        let motions = self.preview_motions(index, library);
        if !motions.is_empty() {
            self.status_message = format!("Previewing {} moving object(s)", motions.len());
            self.preview = Some(ActionPreview { started: now, motions });
        }
    }

    /// The scene as it looks at time `now` during a preview, or `None` when no
    /// preview is playing. A preview ends half a second after its slowest motion.
    pub(super) fn preview_scene(&mut self, now: f64) -> Option<Scene> {
        let preview = self.preview.as_ref()?;
        let elapsed = now - preview.started;
        let longest = preview.motions.iter().map(|(_, motion)| f64::from(motion.seconds)).fold(0.0, f64::max);
        if elapsed > longest + PREVIEW_HOLD_SECONDS {
            self.preview = None;
            return None;
        }
        let mut scene = self.scene.clone();
        for (index, motion) in &preview.motions {
            let fraction = if motion.seconds <= 0.0 {
                1.0
            } else {
                (elapsed / f64::from(motion.seconds)).min(1.0) as f32
            };
            if let Some(object) = scene.objects.get_mut(*index) {
                *object = object.moved_by(motion, fraction);
            }
        }
        Some(scene)
    }

    /// Sets the dungeons listed under "Used in".
    pub fn set_used_in(&mut self, used_in: Vec<UsedInEntry>) {
        self.used_in = used_in;
    }

    /// Returns, once, the dungeon newly selected under "Used in" (or `Some(None)`
    /// when the selection was cleared), so its cells can be loaded with
    /// [`Self::set_dungeon_context`].
    pub fn take_context_request(&mut self) -> Option<Option<OpenedItem>> {
        self.context_request.take()
    }

    /// How far object `index` reaches outside the cell's square, in world units.
    pub(super) fn distance_outside_cell(&self, index: usize, library: &mut Arena2Library) -> f32 {
        let Some(object) = self.scene.objects.get(index) else { return 0.0 };
        object_distance_outside_cell(object, library)
    }

    /// Whether object `index` reaches noticeably outside the cell's square (as of this frame).
    pub(super) fn is_outside_cell(&self, index: usize) -> bool {
        self.outside_cell.get(index).is_some_and(|&distance| distance > OUTSIDE_CELL_TOLERANCE)
    }

    /// Records which place in `dungeon`'s layout this cell was opened at.
    pub fn set_home_placement(&mut self, dungeon: OpenedItem, layout_index: usize) {
        self.home_placement = Some((dungeon, layout_index));
    }

    /// The layout place this cell was opened at in `dungeon`, if it was opened from it.
    pub fn home_layout_index(&self, dungeon: &OpenedItem) -> Option<usize> {
        self.home_placement.as_ref().filter(|(home, _)| home == dungeon).map(|&(_, index)| index)
    }

    /// Returns, once, a surrounding cell the user double-clicked to open.
    pub fn take_cell_open_request(&mut self) -> Option<(OpenedItem, usize)> {
        self.cell_open_request.take()
    }

    /// Sets the cells shown faded around this one, or removes them.
    pub fn set_dungeon_context(&mut self, context: Option<DungeonContext>) {
        self.dungeon_context = context;
    }

    /// Selects `dungeon` under "Used in" (or clears the selection) and asks for its cells.
    pub(super) fn select_used_in(&mut self, dungeon: Option<OpenedItem>) {
        if self.selected_dungeon != dungeon {
            self.scroll_to_selected_dungeon = dungeon.is_some();
            self.selected_dungeon = dungeon.clone();
            self.context_request = Some(dungeon);
            if self.selected_dungeon.is_none() {
                self.dungeon_context = None;
            }
        }
    }

    /// Returns, once, a dungeon the user asked to open from "Used in".
    pub fn take_open_request(&mut self) -> Option<OpenedItem> {
        self.open_request.take()
    }

    /// What the right-click menu of object `index` offers.
    pub(super) fn object_menu_info(
        &self,
        index: usize,
        resources: &EditorResources<'_>,
    ) -> Option<ObjectMenuInfo> {
        let object = self.scene.objects.get(index)?;
        Some(ObjectMenuInfo {
            name: panels::object_display_name(object, resources.custom_names, resources.catalog),
            can_tie: object.can_have_action(),
            triggered_by: self.scene.objects_triggering(object.id).len(),
        })
    }

    /// Carries out the choice made in the right-click menu of object `index`.
    pub(super) fn apply_object_menu(
        &mut self,
        choice: ObjectMenuChoice,
        index: usize,
        resources: &EditorResources<'_>,
    ) {
        match choice {
            ObjectMenuChoice::TieAction => self.start_tying(index, resources),
            ObjectMenuChoice::RemoveActions => self.remove_actions_triggering(index, resources),
        }
    }

    /// Starts choosing the object that object `index`'s action triggers.
    fn start_tying(&mut self, index: usize, resources: &EditorResources<'_>) {
        if !self.can_edit() {
            return;
        }
        let Some(object) = self.scene.objects.get(index) else { return };
        self.palette.armed = None;
        self.viewport.tying_from = Some(object.id);
        self.selected_object = Some(index);
        let name = panels::object_display_name(object, resources.custom_names, resources.catalog);
        self.status_message =
            format!("Tying the action of {name}: click the object it should trigger (Esc to cancel)");
    }

    /// Stops choosing an action target without changing anything.
    fn cancel_tying(&mut self) {
        if self.viewport.tying_from.take().is_some() {
            self.status_message = "Tying cancelled".to_string();
        }
    }

    /// Makes the action of object `source_id` trigger object `target_index`.
    fn tie_action(&mut self, source_id: ObjectId, target_index: usize, resources: &EditorResources<'_>) {
        self.viewport.tying_from = None;
        let Some(source_index) = self.scene.objects.iter().position(|object| object.id == source_id) else {
            return;
        };
        let Some(target_id) = self.scene.objects.get(target_index).map(|object| object.id) else { return };
        if source_index == target_index || !self.can_edit() {
            return;
        }
        let object = &mut self.scene.objects[source_index];
        self.history.record(SceneEdit::Replace { index: source_index, object: object.clone() });
        object.tie_action_to(target_id);
        let describe = |index: usize| {
            panels::object_display_name(&self.scene.objects[index], resources.custom_names, resources.catalog)
        };
        self.status_message = format!("{} now triggers {}", describe(source_index), describe(target_index));
        self.selected_object = Some(source_index);
    }

    /// Removes the actions of every object that triggers object `index`, as one undo step.
    fn remove_actions_triggering(&mut self, index: usize, resources: &EditorResources<'_>) {
        let Some(target_id) = self.scene.objects.get(index).map(|object| object.id) else { return };
        let sources = self.scene.objects_triggering(target_id);
        if sources.is_empty() || !self.can_edit() {
            return;
        }
        let reverts = sources
            .iter()
            .map(|&source| SceneEdit::Replace { index: source, object: self.scene.objects[source].clone() })
            .collect();
        for &source in &sources {
            self.scene.objects[source].remove_action();
        }
        self.history.record(SceneEdit::Group(reverts));
        let name = panels::object_display_name(
            &self.scene.objects[index],
            resources.custom_names,
            resources.catalog,
        );
        self.status_message = format!("Removed {} action(s) that triggered {name}", sources.len());
    }

    /// Whether the room belongs to the project (rather than being a Daggerfall room).
    pub fn is_project_room(&self) -> bool {
        self.project_room_name.is_some()
    }

    /// Returns `true` once after the user tried to change a read-only room.
    pub fn take_blocked_edit(&mut self) -> bool {
        std::mem::take(&mut self.edit_blocked)
    }

    /// Whether the room may be changed. For a read-only room this records the
    /// attempt (see [`Self::take_blocked_edit`]) and returns `false`.
    fn can_edit(&mut self) -> bool {
        if self.read_only {
            self.edit_blocked = true;
        }
        !self.read_only
    }

    /// Base id of this editor's widgets, unique among open editors.
    pub fn id(&self) -> egui::Id {
        self.id
    }

    /// Latest status message, for the status bar.
    pub fn status_message(&self) -> &str {
        &self.status_message
    }

    /// Replaces the status message, for example to report something the application did.
    pub fn set_status_message(&mut self, message: String) {
        self.status_message = message;
    }

    /// Frees the GPU image this editor renders into. Call when the editor is closed.
    pub fn release(&mut self, render_state: &egui_wgpu::RenderState) {
        self.target.release(render_state);
    }

    /// Draws the side panels, asset palette and viewport into `context`.
    ///
    /// Call this after any top or bottom bars of the window have been added.
    pub fn show(&mut self, context: &egui::Context, resources: &mut EditorResources<'_>) {
        self.outside_cell = (0..self.scene.objects.len())
            .map(|index| self.distance_outside_cell(index, resources.library))
            .collect();
        self.show_outliner(context, resources);
        self.show_properties(context, resources);
        self.palette.show(context, self.id, resources);
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
            .show(context, |ui| self.show_viewport(ui, resources));
    }

    /// Handles editor keyboard shortcuts in `context`.
    pub fn handle_shortcuts(&mut self, context: &egui::Context, library: &mut Arena2Library) {
        use egui::{Key, KeyboardShortcut, Modifiers};

        let shortcut = |modifiers, key| {
            context.input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        if shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || shortcut(Modifiers::COMMAND, Key::Y) {
            self.redo();
        }
        if shortcut(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
        if shortcut(Modifiers::COMMAND, Key::D) {
            self.duplicate_selection();
        }

        // Single key shortcuts must not fire while the user is typing in a text field.
        if context.wants_keyboard_input() {
            return;
        }
        // Ctrl+C and Ctrl+V arrive as copy and paste events, not as keys.
        let (copy, pasted) = context.input(|input| {
            let mut copy = false;
            let mut pasted = None;
            for event in &input.events {
                match event {
                    egui::Event::Copy => copy = true,
                    egui::Event::Paste(text) => pasted = Some(text.clone()),
                    _ => {}
                }
            }
            (copy, pasted)
        });
        if copy {
            self.copy_selection(context);
        }
        if let Some(text) = pasted {
            let ours = context
                .data(|data| data.get_temp::<CopiedObjects>(copied_objects_id()))
                .is_some_and(|copied| copied.marker == text);
            if ours {
                self.paste(context);
            }
        }
        if shortcut(Modifiers::NONE, Key::Delete) {
            self.delete_selection();
        }
        if shortcut(Modifiers::NONE, Key::F) {
            self.focus_selection(library);
        }
        if shortcut(Modifiers::NONE, Key::Home) {
            self.frame_room(library);
        }
        if shortcut(Modifiers::NONE, Key::G) {
            self.toggle_grid_lock();
        }

        // While an asset is armed for placement, Escape and R act on it instead of the selection.
        let is_placing = self.palette.armed.is_some();
        if shortcut(Modifiers::NONE, Key::Escape) {
            if self.viewport.tying_from.is_some() {
                self.cancel_tying();
            } else if is_placing {
                self.palette.armed = None;
            } else {
                self.selected_object = None;
            }
        }
        // R (Shift+R the other way), and Q and E, turn the object being placed or
        // the one selected object by a quarter turn. Q and E are left to the
        // camera while the right mouse button is held (flying).
        let flying = context.input(|input| input.pointer.secondary_down());
        let mut turns = vec![(Modifiers::SHIFT, Key::R, -90.0), (Modifiers::NONE, Key::R, 90.0)];
        if !flying {
            turns.extend([(Modifiers::NONE, Key::Q, -90.0), (Modifiers::NONE, Key::E, 90.0)]);
        }
        for (modifiers, key, degrees) in turns {
            if shortcut(modifiers, key) {
                if is_placing {
                    self.palette.rotate_armed(degrees);
                } else {
                    self.rotate_selection(degrees);
                }
            }
        }
    }

    /// Adds the Edit menu entries.
    pub fn edit_menu(&mut self, ui: &mut egui::Ui) {
        let has_selection = self.selected_object.is_some();
        // Several objects can be copied, duplicated, deleted and moved, but not turned.
        let has_one = has_selection && !self.has_multiple_selection();
        let has_copied = ui.ctx().data(|data| data.get_temp::<CopiedObjects>(copied_objects_id())).is_some();
        let menu_button = |ui: &mut egui::Ui, enabled: bool, label: &str, shortcut: &str| {
            let clicked = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut)).clicked();
            if clicked {
                ui.close_menu();
            }
            clicked
        };

        if menu_button(ui, self.history.can_undo(), "Undo", "Ctrl+Z") {
            self.undo();
        }
        if menu_button(ui, self.history.can_redo(), "Redo", "Ctrl+Y") {
            self.redo();
        }
        ui.separator();
        if menu_button(ui, has_selection, "Copy", "Ctrl+C") {
            self.copy_selection(ui.ctx());
        }
        if menu_button(ui, has_copied, "Paste", "Ctrl+V") {
            self.paste(ui.ctx());
        }
        if menu_button(ui, has_selection, "Duplicate", "Ctrl+D") {
            self.duplicate_selection();
        }
        if menu_button(ui, has_one, "Rotate 90°", "R") {
            self.rotate_selection(90.0);
        }
        if menu_button(ui, has_selection, "Delete", "Del") {
            self.delete_selection();
        }
        ui.separator();
        let grid_lock = egui::Checkbox::new(&mut self.snap.enabled, "Grid lock");
        ui.add(grid_lock).on_hover_text("Snap objects to the grid while moving and placing them (G)");
        ui.horizontal(|ui| {
            ui.label("Snap step");
            snap_step_selector(ui, self.id.with("menu_snap_step"), &mut self.snap.step);
        });
    }

    /// Adds the grid lock button and its snap step to a menu bar.
    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        let label = if self.snap.enabled { "Grid lock: on" } else { "Grid lock: off" };
        let response = ui.selectable_label(self.snap.enabled, label).on_hover_text(
            "Snap objects to the grid while moving and placing them (G).\nHold Ctrl while dragging to switch it for that drag.",
        );
        if response.clicked() {
            self.toggle_grid_lock();
        }
        snap_step_selector(ui, self.id.with("toolbar_snap_step"), &mut self.snap.step);
    }

    /// Turns grid lock on or off and says so in the status bar.
    fn toggle_grid_lock(&mut self) {
        self.snap.enabled = !self.snap.enabled;
        self.status_message = if self.snap.enabled {
            format!("Grid lock on: objects snap to a {} m grid", self.snap.step)
        } else {
            "Grid lock off".to_string()
        };
    }

    /// Adds the View menu entries.
    pub fn view_menu(&mut self, ui: &mut egui::Ui, library: &mut Arena2Library) {
        if ui.add(egui::Button::new("Frame cell").shortcut_text("Home")).clicked() {
            self.frame_room(library);
            ui.close_menu();
        }
        let has_selection = self.selected_object.is_some();
        if ui.add_enabled(has_selection, egui::Button::new("Frame selection").shortcut_text("F")).clicked() {
            self.focus_selection(library);
            ui.close_menu();
        }
        ui.separator();
        ui.checkbox(&mut self.palette.is_open, "Asset palette");
        ui.separator();
        view_settings_menu(ui, &mut self.view_settings);
    }

    /// Frames the whole room.
    fn frame_room(&mut self, library: &mut Arena2Library) {
        match self.scene.bounds(library) {
            Some((minimum, maximum)) => self.camera.frame_bounds(minimum, maximum),
            None => self
                .camera
                .frame_bounds(Vec3::new(0.0, -5.0, 0.0), Vec3::new(RDB_BLOCK_SIDE, 5.0, RDB_BLOCK_SIDE)),
        }
    }

    /// Centres the camera on the selected object (the last selected, with several).
    fn focus_selection(&mut self, library: &mut Arena2Library) {
        let Some(object) = self.selected_object.and_then(|index| self.scene.objects.get(index)) else {
            return;
        };
        let radius = match object.kind {
            SceneObjectKind::Model { model_id, .. } => {
                library.model(model_id).map_or(DEFAULT_FOCUS_RADIUS, |mesh| mesh.bounding_radius())
            }
            _ => DEFAULT_FOCUS_RADIUS,
        };
        let center = object.position;
        self.camera.frame_bounds(center - Vec3::splat(radius), center + Vec3::splat(radius));
    }

    /// Every selected object, the last selected last. Empty if none is.
    pub(super) fn selected_indices(&self) -> Vec<usize> {
        let count = self.scene.objects.len();
        match self.selected_object.filter(|&index| index < count) {
            Some(primary) if self.selection_set.contains(&primary) => {
                self.selection_set.iter().copied().filter(|&index| index < count).collect()
            }
            Some(primary) => vec![primary],
            None => Vec::new(),
        }
    }

    /// Whether more than one object is selected; then they can only be moved.
    pub(super) fn has_multiple_selection(&self) -> bool {
        self.selected_indices().len() > 1
    }

    /// Selects exactly `indices` (the last one becomes the main selection).
    pub(super) fn select_objects(&mut self, mut indices: Vec<usize>) {
        let mut seen = std::collections::HashSet::new();
        indices.retain(|index| seen.insert(*index));
        self.selected_object = indices.last().copied();
        self.selection_set = if indices.len() > 1 { indices } else { Vec::new() };
    }

    /// Adds object `index` to the selection, or takes it out if it is selected (Shift+click).
    pub(super) fn toggle_selected(&mut self, index: usize) {
        let mut indices = self.selected_indices();
        if let Some(position) = indices.iter().position(|&selected| selected == index) {
            indices.remove(position);
        } else {
            indices.push(index);
        }
        self.select_objects(indices);
    }

    /// Says in the status bar that this cannot be done with several objects
    /// selected (they can be moved, copied, duplicated and deleted, not turned),
    /// and returns `true` if that is the case.
    fn refuse_for_multiple_selection(&mut self, action: &str) -> bool {
        let count = self.selected_indices().len();
        if count > 1 {
            self.status_message = format!(
                "Cannot {action} {count} objects at once; several objects can be moved, copied, duplicated \
                 or deleted"
            );
        }
        count > 1
    }

    /// The selected objects, as many as there are.
    fn selected_objects(&self) -> Vec<SceneObject> {
        self.selected_indices()
            .into_iter()
            .filter_map(|index| self.scene.objects.get(index).cloned())
            .collect()
    }

    /// A short description of `count` objects, the only one by name.
    fn count_description(objects: &[SceneObject]) -> String {
        match objects {
            [object] => object.name.clone(),
            several => format!("{} objects", several.len()),
        }
    }

    /// Removes the selected objects, as one undo step.
    fn delete_selection(&mut self) {
        let mut indices = self.selected_indices();
        if indices.is_empty() || !self.can_edit() {
            return;
        }
        indices.sort_unstable();
        indices.dedup();
        let mut removed = Vec::with_capacity(indices.len());
        for &index in indices.iter().rev() {
            removed.push((index, self.scene.objects.remove(index)));
        }
        removed.reverse();
        let objects: Vec<SceneObject> = removed.iter().map(|(_, object)| object.clone()).collect();
        self.status_message = format!("Deleted {}", Self::count_description(&objects));
        // Undoing puts them back from the first place up, so each goes where it was.
        self.history.record(SceneEdit::Group(
            removed.into_iter().map(|(index, object)| SceneEdit::Insert { index, object }).collect(),
        ));
        self.selected_object = None;
        self.selection_set.clear();
    }

    /// Copies the selected objects next to them, and selects the copies.
    fn duplicate_selection(&mut self) {
        let originals = self.selected_objects();
        if originals.is_empty() || !self.can_edit() {
            return;
        }
        self.insert_copies(&originals, DUPLICATE_OFFSET, true);
        self.status_message = format!("Duplicated {}", Self::count_description(&originals));
    }

    /// Copies the selected objects (Ctrl+C), to paste them into this cell or
    /// another with Ctrl+V.
    fn copy_selection(&mut self, context: &egui::Context) {
        let objects = self.selected_objects();
        if objects.is_empty() {
            self.status_message = "Select objects to copy them".to_string();
            return;
        }
        let description = Self::count_description(&objects);
        // A different text each time, so an older copy on the clipboard is not mistaken for this one.
        let marker = format!(
            "Daggerfall Dungeon Maker: {description} copied ({:.3})",
            context.input(|input| input.time)
        );
        context.copy_text(marker.clone());
        context.data_mut(|data| {
            data.insert_temp(
                copied_objects_id(),
                CopiedObjects { objects, source: self.instance, marker, pasted_into: None, pastes: 0 },
            )
        });
        self.status_message = format!("Copied {description}; Ctrl+V pastes it into this cell or another");
    }

    /// Pastes the objects copied last (Ctrl+V), and selects them. Pasted
    /// into the cell they were copied from, they land next to the originals;
    /// into another cell, where they were (each further paste a little further along).
    fn paste(&mut self, context: &egui::Context) {
        let Some(mut copied) = context.data(|data| data.get_temp::<CopiedObjects>(copied_objects_id()))
        else {
            self.status_message = "Nothing is copied: select objects and press Ctrl+C first".to_string();
            return;
        };
        if !self.can_edit() {
            return;
        }
        let same_cell = copied.source == self.instance;
        let pastes = match copied.pasted_into == Some(self.instance) {
            true => copied.pastes + 1,
            false => u32::from(same_cell),
        };
        self.insert_copies(&copied.objects, DUPLICATE_OFFSET * pastes as f32, same_cell);
        self.status_message = format!("Pasted {}", Self::count_description(&copied.objects));
        copied.pasted_into = Some(self.instance);
        copied.pastes = pastes;
        context.data_mut(|data| data.insert_temp(copied_objects_id(), copied));
    }

    /// Adds copies of `originals`, moved by `offset`, with new ids, as one
    /// undo step, and selects them. Links between the copies follow them;
    /// links to other objects are kept only in the cell they came from
    /// (`same_cell`), where those objects are. In another cell the copies go
    /// into the list of added objects, as the cell's own lists differ.
    fn insert_copies(&mut self, originals: &[SceneObject], offset: Vec3, same_cell: bool) {
        let mut next_id = self.scene.new_object_id();
        let ids: std::collections::HashMap<ObjectId, ObjectId> = originals
            .iter()
            .map(|object| {
                let id = next_id;
                next_id += 1;
                (object.id, id)
            })
            .collect();
        let start = self.scene.objects.len();
        for original in originals {
            let mut copy = original.clone();
            copy.id = ids[&original.id];
            copy.next = original.next.and_then(|next| ids.get(&next).copied().or(same_cell.then_some(next)));
            copy.position += offset;
            if !same_cell {
                copy.group_index = ADDED_OBJECTS_GROUP;
            }
            self.scene.objects.push(copy);
        }
        let end = self.scene.objects.len();
        // Undoing removes them from the last one back.
        self.history
            .record(SceneEdit::Group((start..end).rev().map(|index| SceneEdit::Remove { index }).collect()));
        self.select_objects((start..end).collect());
        self.viewport.selection_changed_in_viewport = true;
    }

    /// Turns the selected object around the vertical axis by `degrees`.
    fn rotate_selection(&mut self, degrees: f32) {
        if self.refuse_for_multiple_selection("rotate") {
            return;
        }
        let Some(index) = self.selected_object.filter(|&index| index < self.scene.objects.len()) else {
            return;
        };
        if !self.can_edit() {
            return;
        }
        let Some(object) = self.scene.objects.get_mut(index) else { return };
        self.history.record(SceneEdit::Replace { index, object: object.clone() });
        object.rotation_degrees.y = (object.rotation_degrees.y + degrees).rem_euclid(360.0);
    }

    /// Reverts the last edit.
    fn undo(&mut self) {
        if self.history.undo(&mut self.scene.objects) {
            self.after_history_change();
            self.status_message = "Undo".into();
        }
    }

    /// Re-applies the last undone edit.
    fn redo(&mut self) {
        if self.history.redo(&mut self.scene.objects) {
            self.after_history_change();
            self.status_message = "Redo".into();
        }
    }

    /// Keeps the selection valid after the object list changed underneath it.
    fn after_history_change(&mut self) {
        self.property_edit_session = None;
        if self.selected_object.is_some_and(|index| index >= self.scene.objects.len()) {
            self.selected_object = None;
        }
        let count = self.scene.objects.len();
        self.selection_set.retain(|&index| index < count);
    }
}

/// How far `object` reaches outside its cell's square, using its model's box.
pub(super) fn object_distance_outside_cell(object: &SceneObject, library: &mut Arena2Library) -> f32 {
    let bounds = match object.kind {
        SceneObjectKind::Model { model_id, .. } => {
            library.model(model_id).map(|mesh| (mesh.bounds_min, mesh.bounds_max))
        }
        _ => None,
    };
    object.distance_outside_cell(bounds)
}

/// Tells the user how far an object reaches outside the cell's square.
pub(super) fn outside_cell_warning(distance: f32) -> String {
    format!(
        "⚠ Reaches {distance:.2} m outside the cell's {RDB_BLOCK_SIDE} m square, so in a dungeon \
         it can overlap the next cell"
    )
}

/// Draws a drop-down choosing the grid lock snap step from [`SNAP_STEPS`].
fn snap_step_selector(ui: &mut egui::Ui, id: egui::Id, step: &mut f32) {
    let selected = SNAP_STEPS
        .iter()
        .find(|(value, _)| (*value - *step).abs() < 1e-4)
        .map_or_else(|| format!("{step} m"), |(_, label)| (*label).to_string());
    egui::ComboBox::from_id_salt(id).selected_text(selected).show_ui(ui, |ui| {
        for (value, label) in SNAP_STEPS {
            ui.selectable_value(step, value, label);
        }
    });
}

/// Adds checkboxes for every [`ViewSettings`] option.
pub fn view_settings_menu(ui: &mut egui::Ui, settings: &mut ViewSettings) {
    ui.checkbox(&mut settings.cull_back_faces, "Hide back faces (see into cells)");
    ui.checkbox(&mut settings.show_grid, "Grid");
    ui.checkbox(&mut settings.show_room_outline, "Block outlines");
    ui.separator();
    ui.checkbox(&mut settings.show_models, "Models");
    ui.checkbox(&mut settings.show_flats, "Flats");
    ui.checkbox(&mut settings.show_markers, "Editor markers");
    ui.checkbox(&mut settings.show_lights, "Lights");
    ui.checkbox(&mut settings.show_action_links, "Action links")
        .on_hover_text("Red lines from each object to the object its action triggers");
}

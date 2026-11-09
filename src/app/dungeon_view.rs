//! Shows a whole pre-built dungeon: every RDB block (room) placed on its grid.
//!
//! Hovering a room highlights all of its objects and its block outline;
//! double-clicking a room asks the application to open it in a room editor
//! window. Edits made in those windows are copied back with
//! [`DungeonView::update_room_objects`] so the dungeon always shows the latest version.
//!
//! Shift+click selects several cells (or Shift+drag a box over them); their
//! right-click menu can merge them into one cell (see [`crate::merge`]).
//!
//! The properties panel on the right shows the dungeon's name, region, type,
//! location and rooms. The name, type and location of a project dungeon can
//! be changed there; its spot on the world map is chosen in a map window.

use eframe::{egui, egui_wgpu};
use glam::Vec3;

use super::entrance_picker::{entrance_label, EntrancePicker, EntranceRequest};
use super::project_panel::{DragPayload, OpenedItem};
use super::room_browser::{category_description, show_room_name_field};
use super::viewport_tools::{
    aspect_ratio, draw_unclosed_openings, find_unclosed_openings, handle_camera_input, paint_scene,
    size_in_pixels, EditorResources, ViewportProjection, UNCLOSED_CHECK_INTERVAL,
};
use super::world_map::{occupant, MapChoice, MapPickRequest, WorldMapPicker};
use crate::arena2::{Arena2Library, DungeonDefinition, LocationType, MapPixel, WorldMap, RDB_BLOCK_SIDE};
use crate::camera::OrbitCamera;
use crate::custom_names::CustomNames;
use crate::encounters::DungeonType;
use crate::music::{DungeonSong, MusicPlayer, DUNGEON_SONGS};
use crate::picking::{intersect_horizontal_plane, pick_object, PickOptions};
use crate::project::{DungeonLocation, ItemId, Project, ProjectDungeonRoom};
use crate::render::{FrameRequest, RoomOutline, ViewSettings, ViewportTarget};
use crate::scene::{ObjectId, Scene, SceneObject};

/// Something the user did in the dungeon's properties panel, for the application to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum DungeonPropertiesAction {
    Rename(String),
    SetType(DungeonType),
    /// Open room `index` in a room editor window.
    OpenRoom(usize),
    /// Make room `index` the dungeon's starting cell, instead of any other.
    MakeStart(usize),
    /// Change where the dungeon goes in the world.
    SetLocation(DungeonLocation),
    /// The user tried to change a dungeon from Daggerfall, which cannot be edited.
    EditBlocked,
    /// Play `song` to preview it (see [`DungeonView::play_song`]).
    PlaySong(DungeonSong),
}

/// What the properties panel needs besides the view itself.
pub struct DungeonDetails<'details> {
    /// Set for dungeons from Daggerfall, whose properties cannot be changed.
    pub read_only: bool,
    /// The Daggerfall dungeon a project dungeon was copied from.
    pub based_on: Option<&'details str>,
    /// For the names of the project rooms the dungeon uses.
    pub project: &'details Project,
    pub custom_names: &'details CustomNames,
    /// Where the dungeon goes in the world (for Daggerfall's own dungeons, where it is).
    pub location: DungeonLocation,
    /// The world map, or `None` without `MAPS.BSA`.
    pub world: Option<&'details WorldMap>,
    /// The project's other placed dungeons, which cannot share the dungeon's spot.
    pub other_dungeons: &'details [(&'details str, MapPixel)],
    /// Daggerfall's dungeons, which the entrances come from.
    pub daggerfall_dungeons: &'details [DungeonDefinition],
    /// The song the game plays in the dungeon when none is chosen, if known:
    /// a Daggerfall dungeon's own, or that of the dungeon a project dungeon replaces.
    pub game_song: Option<DungeonSong>,
}

/// Colour of warnings in the properties panel.
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 190, 70);
/// Colour of "all is well" notes in the properties panel.
const GOOD_COLOR: egui::Color32 = egui::Color32::from_rgb(120, 230, 140);

/// What the world map and entrance windows need, besides the view itself.
pub struct LocationWindowsRequest<'request> {
    /// Set for dungeons from Daggerfall, which are only shown.
    pub read_only: bool,
    pub location: &'request DungeonLocation,
    pub world: Option<&'request WorldMap>,
    pub other_dungeons: &'request [(&'request str, MapPixel)],
    pub daggerfall_dungeons: &'request [DungeonDefinition],
}

/// One room of the dungeon.
pub struct DungeonRoom {
    /// RDB block name, for example `N0000012.RDB`.
    pub block_name: String,
    /// World position of the block's minimum corner.
    pub origin: Vec3,
    /// Whether the player enters the dungeon in this room.
    pub is_starting_block: bool,
    /// The room's objects in block-local coordinates, exactly as a room editor sees them.
    pub scene: Scene,
    /// The project room used here, or `None` for the Daggerfall room (which cannot be edited).
    pub project_room: Option<ItemId>,
    /// Index of the room in the dungeon layout it was loaded from.
    layout_index: usize,
    /// Whether the room's objects were changed since the dungeon was loaded.
    edited: bool,
    /// Whether the cell itself has an exit door, so it can be the start cell.
    /// (Rooms other than the start are shown without their exit doors, so this
    /// is worked out from the cell, not from `scene`.)
    pub has_exit: bool,
    /// Whether the cell has a start marker, where the player is placed on
    /// entering the dungeon, so it can be the start cell.
    pub has_start_marker: bool,
}

/// State of the dungeon view.
pub struct DungeonView {
    /// Dungeon name as shown in game.
    pub name: String,
    /// Region the dungeon belongs to.
    pub region_name: String,
    /// The kind of dungeon, which decides the enemies at random enemy markers.
    pub dungeon_type: Option<DungeonType>,
    rooms: Vec<DungeonRoom>,
    /// Every room's objects moved into world space, with `group_index` set to the room index.
    combined_scene: Scene,
    room_outlines: Vec<RoomOutline>,
    /// The water surface of each cell that has water.
    water_planes: Vec<Vec3>,
    camera: OrbitCamera,
    view_settings: ViewSettings,
    grid_height: f32,
    target: ViewportTarget,
    hovered_room: Option<usize>,
    /// Room whose row is hovered in the properties panel, highlighted like a hovered room.
    list_hovered_room: Option<usize>,
    /// The name typed in the properties panel, applied when the field loses focus.
    name_edit: Option<String>,
    /// Room whose right-click menu (for naming it) is open.
    renamed_room: Option<usize>,
    /// Room the user chose "Make Start" for in the viewport's menu.
    make_start_request: Option<usize>,
    last_pointer_position: Option<egui::Pos2>,
    /// The window for choosing the dungeon's spot on the world map.
    map_picker: WorldMapPicker,
    /// The window for choosing the dungeon's entrance.
    entrance_picker: EntrancePicker,
    /// A cell being dragged in from the project panel, shown where it would go.
    drop_preview: Option<DropPreview>,
    /// A cell dropped into the view, for the application to add to the dungeon.
    add_room_request: Option<ProjectDungeonRoom>,
    /// A cell the user pressed Delete on, for the application to take out of the dungeon.
    delete_room_request: Option<usize>,
    /// A cell being dragged to another grid spot.
    room_drag: Option<RoomDrag>,
    /// A cell dragged to a free spot, for the application to move in the dungeon:
    /// its index, and the grid spots it came from and went to.
    move_room_request: Option<RoomMove>,
    /// Openings that lead to empty space, shown as yellow panes; `None` until
    /// found (the openings Daggerfall leaves open are learned in the background first).
    unclosed: Option<Vec<crate::connectors::UnclosedFace>>,
    /// Whether the dungeon changed since the openings were last checked, and when they were.
    unclosed_stale: bool,
    unclosed_checked_at: f64,
    /// Plays a song to preview the dungeon's music.
    music_player: MusicPlayer,
    /// Openings of each model, by model id, for checking them.
    open_faces: std::collections::HashMap<u32, std::sync::Arc<[crate::connectors::OpenFace]>>,
    /// Selected cells (Shift+click, or Shift+drag a box), which can be merged.
    selected_rooms: Vec<usize>,
    /// Where a selection box being dragged started.
    selection_box_start: Option<egui::Pos2>,
    /// The cells inside the selection box being dragged.
    box_rooms: Vec<usize>,
    /// Cells the user chose to merge, for the application to merge.
    merge_request: Option<Vec<usize>>,
    /// Whether the dungeon's skybox is drawn (View > Skybox).
    show_skybox: bool,
    /// Whether the dungeon has a skybox to draw.
    has_skybox: bool,
}

/// Colours of the box dragged to select cells.
const SELECTION_BOX_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(40, 70, 110, 40);
const SELECTION_BOX_STROKE: egui::Color32 = egui::Color32::from_rgb(110, 170, 255);
/// Smallest box, in points, that selects; anything smaller is a click.
const SELECTION_BOX_MIN_SIZE: f32 = 4.0;

/// A cell moved by dragging: its index, and the grid spots it came from and went to.
pub type RoomMove = (usize, (i32, i32), (i32, i32));

/// A cell of the dungeon being dragged to another grid spot.
struct RoomDrag {
    room_index: usize,
    /// Where on the floor the cell was grabbed, from its corner.
    grab_offset: Vec3,
    /// Its grid spot before the drag.
    from: (i32, i32),
    /// The grid spot it is shown at now.
    to: (i32, i32),
}

/// A cell dragged over the view from the project panel.
struct DropPreview {
    /// Its block name and, for a project cell, its id.
    cell: (String, Option<ItemId>),
    /// Name shown next to the pointer.
    label: String,
    /// Its objects in block coordinates, as a cell other than the start shows them.
    scene: Scene,
    /// The grid spot under the pointer, if the pointer is over the floor.
    spot: Option<(i32, i32)>,
    /// The cell moved to `spot`, for drawing.
    placed: Scene,
}

impl DungeonView {
    /// Loads the dungeon called `name` made of the rooms in `layout`.
    ///
    /// Rooms that use a project room show it; the others are read from the game data.
    /// Blocks that cannot be loaded are skipped; their errors are returned so they can be reported.
    pub fn load(
        name: &str,
        region_name: &str,
        dungeon_type: Option<DungeonType>,
        layout: &[ProjectDungeonRoom],
        project: &Project,
        library: &mut Arena2Library,
    ) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let rooms: Vec<DungeonRoom> = layout
            .iter()
            .enumerate()
            .filter_map(|(layout_index, placement)| match load_room_scene(placement, project, library) {
                Ok((scene, project_room)) => Some(DungeonRoom {
                    has_exit: match project_room {
                        Some(_) => scene.objects.iter().any(SceneObject::is_exit_door),
                        None => cell_has_exit(&placement.block_name, library),
                    },
                    has_start_marker: scene.start_marker_index().is_some(),
                    block_name: placement.block_name.clone(),
                    origin: Vec3::new(
                        placement.grid_x as f32 * RDB_BLOCK_SIDE,
                        0.0,
                        placement.grid_z as f32 * RDB_BLOCK_SIDE,
                    ),
                    is_starting_block: placement.is_starting_block,
                    scene,
                    project_room,
                    layout_index,
                    edited: false,
                }),
                Err(error) => {
                    problems.push(format!("{}: {error}", placement.block_name));
                    None
                }
            })
            .collect();

        let mut view = Self {
            name: name.to_string(),
            region_name: region_name.to_string(),
            dungeon_type,
            rooms,
            combined_scene: Scene::default(),
            room_outlines: Vec::new(),
            water_planes: Vec::new(),
            camera: OrbitCamera::default(),
            view_settings: ViewSettings::default(),
            grid_height: 0.0,
            target: ViewportTarget::default(),
            hovered_room: None,
            list_hovered_room: None,
            name_edit: None,
            renamed_room: None,
            make_start_request: None,
            last_pointer_position: None,
            map_picker: WorldMapPicker::default(),
            entrance_picker: EntrancePicker::default(),
            drop_preview: None,
            add_room_request: None,
            delete_room_request: None,
            room_drag: None,
            move_room_request: None,
            unclosed: None,
            music_player: MusicPlayer::default(),
            unclosed_stale: true,
            unclosed_checked_at: f64::NEG_INFINITY,
            open_faces: std::collections::HashMap::new(),
            selected_rooms: Vec::new(),
            selection_box_start: None,
            box_rooms: Vec::new(),
            merge_request: None,
            show_skybox: true,
            has_skybox: false,
        };
        view.rebuild_combined_scene();
        view.frame_dungeon(library);
        if let Some((minimum, _)) = view.combined_scene.bounds(library) {
            view.grid_height = minimum.y - 0.05;
        }
        (view, problems)
    }

    /// Whether the player can leave the dungeon: its start cell has an exit door.
    fn has_usable_exit(&self) -> bool {
        self.rooms.iter().any(|room| room.is_starting_block && room.has_exit)
    }

    /// What stops the dungeon from being placed on the world map: it needs an
    /// entrance (to get in from the world) and an exit (to get back out).
    fn placement_problems(&self, location: &DungeonLocation) -> Vec<&'static str> {
        let mut problems = Vec::new();
        if location.entrance.is_none() {
            problems.push("it needs an entrance");
        }
        if !self.has_usable_exit() {
            problems.push("it needs an exit (an exit door in its start cell)");
        }
        problems
    }

    /// Starts playing `song` from the HMI file `data`, stopping any song playing.
    pub fn play_song(&mut self, song: DungeonSong, data: &[u8]) -> Result<(), String> {
        self.music_player.play(song.file, data)
    }

    /// Returns, once, a cell the user dragged to a free grid spot: its index,
    /// and the spots it came from and went to. The view already shows it there.
    pub fn take_move_room_request(&mut self) -> Option<RoomMove> {
        self.move_room_request.take()
    }

    /// Puts room `room_index` at grid spot `spot` in the view.
    pub fn set_room_spot(&mut self, room_index: usize, spot: (i32, i32)) {
        let Some(room) = self.rooms.get_mut(room_index) else { return };
        let origin = grid_origin(spot.0, spot.1);
        if room.origin != origin {
            room.origin = origin;
            self.rebuild_combined_scene();
        }
    }

    /// The point on the floor grid under `pointer`, if the pointer is over it.
    fn floor_point(&self, projection: &ViewportProjection, pointer: egui::Pos2) -> Option<Vec3> {
        let ray = self.camera.ray_through(projection.screen_to_ndc(pointer), aspect_ratio(projection.rect));
        Some(ray.point_at(intersect_horizontal_plane(&ray, self.grid_height)?))
    }

    /// Dragging a cell with the left mouse button moves it from grid spot to
    /// grid spot. Dropped on a free spot it stays there (and the application is
    /// asked to move it in the dungeon); dropped on a taken spot it goes back.
    fn handle_room_drag(&mut self, response: &egui::Response, projection: &ViewportProjection) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            let start =
                response.interact_pointer_pos().and_then(|pointer| self.floor_point(projection, pointer));
            if let (Some(room_index), Some(start)) = (self.hovered_room, start) {
                if let Some(room) = self.rooms.get(room_index) {
                    let from = grid_spot(room.origin + Vec3::splat(RDB_BLOCK_SIDE * 0.5));
                    self.room_drag =
                        Some(RoomDrag { room_index, grab_offset: start - room.origin, from, to: from });
                }
            }
        }
        let Some(drag) = &self.room_drag else { return };
        let (room_index, from, mut to) = (drag.room_index, drag.from, drag.to);
        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(point) =
                response.interact_pointer_pos().and_then(|pointer| self.floor_point(projection, pointer))
            {
                let corner = point - drag.grab_offset;
                to = ((corner.x / RDB_BLOCK_SIDE).round() as i32, (corner.z / RDB_BLOCK_SIDE).round() as i32);
            }
            if let Some(drag) = self.room_drag.as_mut() {
                drag.to = to;
            }
            self.set_room_spot(room_index, to);
            return;
        }
        // Released.
        self.room_drag = None;
        if to != from && self.spot_is_free_for(room_index, to) {
            self.move_room_request = Some((room_index, from, to));
        } else {
            self.set_room_spot(room_index, from);
        }
        self.last_pointer_position = None;
    }

    /// Whether no cell but `room_index` is at grid spot `spot`.
    fn spot_is_free_for(&self, room_index: usize, spot: (i32, i32)) -> bool {
        !self.rooms.iter().enumerate().any(|(index, room)| {
            index != room_index && grid_spot(room.origin + Vec3::splat(RDB_BLOCK_SIDE * 0.5)) == spot
        })
    }

    /// Returns, once, the cells the user chose to merge.
    pub fn take_merge_request(&mut self) -> Option<Vec<usize>> {
        self.merge_request.take()
    }

    /// Keeps the camera and view options of `old`, a view of the same dungeon
    /// this one replaces (for example after its cells were merged).
    pub fn keep_view_of(&mut self, old: &DungeonView) {
        self.camera = old.camera.clone();
        self.view_settings = old.view_settings;
        self.grid_height = old.grid_height;
        self.show_skybox = old.show_skybox;
    }

    /// Selects the cells inside a selection box while it is dragged (with
    /// Shift held when it started), adding them to the selection on release.
    fn handle_selection_box(&mut self, response: &egui::Response, projection: &ViewportProjection) {
        let (Some(start), Some(pointer)) = (self.selection_box_start, response.interact_pointer_pos()) else {
            self.selection_box_start = None;
            self.box_rooms.clear();
            return;
        };
        let selection_box = egui::Rect::from_two_pos(start, pointer);
        self.box_rooms = if selection_box.width().max(selection_box.height()) >= SELECTION_BOX_MIN_SIZE {
            self.rooms
                .iter()
                .enumerate()
                .filter(|(_, room)| {
                    let center =
                        room.origin + Vec3::new(RDB_BLOCK_SIDE * 0.5, self.grid_height, RDB_BLOCK_SIDE * 0.5);
                    projection
                        .world_to_screen(center)
                        .is_some_and(|position| selection_box.contains(position))
                })
                .map(|(index, _)| index)
                .collect()
        } else {
            Vec::new()
        };
        if response.drag_stopped() || !response.dragged() {
            for room_index in std::mem::take(&mut self.box_rooms) {
                if !self.selected_rooms.contains(&room_index) {
                    self.selected_rooms.push(room_index);
                }
            }
            self.selection_box_start = None;
            self.last_pointer_position = None;
        }
    }

    /// Selects only `room_index` (or nothing), or with `add` adds it to the
    /// selection, or takes it out if it is selected.
    fn click_room(&mut self, room_index: Option<usize>, add: bool) {
        match (room_index, add) {
            (Some(room_index), true) => {
                match self.selected_rooms.iter().position(|&index| index == room_index) {
                    Some(position) => {
                        self.selected_rooms.remove(position);
                    }
                    None => self.selected_rooms.push(room_index),
                }
            }
            (Some(room_index), false) => self.selected_rooms = vec![room_index],
            (None, true) => {}
            (None, false) => self.selected_rooms.clear(),
        }
    }

    /// Returns, once, the index of a cell the user pressed Delete on.
    pub fn take_delete_room_request(&mut self) -> Option<usize> {
        self.delete_room_request.take()
    }

    /// Takes room `room_index` out of the view. The rooms after it in the
    /// dungeon's layout move up one place, as they do in the layout.
    pub fn remove_room(&mut self, room_index: usize) {
        if room_index >= self.rooms.len() {
            return;
        }
        let removed = self.rooms.remove(room_index);
        self.selected_rooms.clear();
        for room in &mut self.rooms {
            if room.layout_index > removed.layout_index {
                room.layout_index -= 1;
            }
        }
        self.hovered_room = None;
        self.list_hovered_room = None;
        self.renamed_room = None;
        self.last_pointer_position = None;
        self.rebuild_combined_scene();
    }

    /// Returns, once, a cell the user dropped into the view, placed where it was dropped.
    pub fn take_add_room_request(&mut self) -> Option<ProjectDungeonRoom> {
        self.add_room_request.take()
    }

    /// Adds the cell `placement`, which is entry `layout_index` of the
    /// dungeon's layout, to the view.
    pub fn add_room(
        &mut self,
        placement: &ProjectDungeonRoom,
        layout_index: usize,
        project: &Project,
        library: &mut Arena2Library,
    ) -> Result<(), String> {
        let (scene, project_room) = load_room_scene(placement, project, library)?;
        let was_empty = self.rooms.is_empty();
        self.rooms.push(DungeonRoom {
            has_exit: match project_room {
                Some(_) => scene.objects.iter().any(SceneObject::is_exit_door),
                None => cell_has_exit(&placement.block_name, library),
            },
            has_start_marker: scene.start_marker_index().is_some(),
            block_name: placement.block_name.clone(),
            origin: grid_origin(placement.grid_x, placement.grid_z),
            is_starting_block: placement.is_starting_block,
            scene,
            project_room,
            layout_index,
            edited: false,
        });
        self.rebuild_combined_scene();
        if was_empty {
            self.frame_dungeon(library);
            if let Some((minimum, _)) = self.combined_scene.bounds(library) {
                self.grid_height = minimum.y - 0.05;
            }
        }
        Ok(())
    }

    /// Whether a cell of the dungeon is at grid spot `spot`.
    fn spot_is_taken(&self, spot: (i32, i32)) -> bool {
        self.rooms.iter().any(|room| grid_spot(room.origin) == spot)
    }

    /// Follows a cell dragged over the view from the project panel: loads it
    /// once, and moves it to the grid spot under the pointer.
    fn update_drop_preview(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        projection: &ViewportProjection,
        project: &Project,
        library: &mut Arena2Library,
    ) {
        let payload = egui::DragAndDrop::payload::<DragPayload>(ui.ctx());
        let dragged = payload
            .and_then(|payload| dragged_cell(&payload, project))
            .filter(|_| response.contains_pointer());
        let Some((cell, label)) = dragged else {
            self.drop_preview = None;
            return;
        };
        if self.drop_preview.as_ref().map(|preview| &preview.cell) != Some(&cell) {
            let placement = ProjectDungeonRoom {
                block_name: cell.0.clone(),
                grid_x: 0,
                grid_z: 0,
                is_starting_block: false,
                room: cell.1,
            };
            let Ok((scene, _)) = load_room_scene(&placement, project, library) else {
                self.drop_preview = None;
                return;
            };
            self.drop_preview =
                Some(DropPreview { cell, label, scene, spot: None, placed: Scene::default() });
        }
        let spot = ui.ctx().pointer_hover_pos().and_then(|pointer| {
            let ray =
                self.camera.ray_through(projection.screen_to_ndc(pointer), aspect_ratio(projection.rect));
            let floor_point = ray.point_at(intersect_horizontal_plane(&ray, self.grid_height)?);
            Some(grid_spot(floor_point))
        });
        let Some(preview) = self.drop_preview.as_mut() else { return };
        if preview.spot != spot {
            preview.spot = spot;
            preview.placed = match spot {
                Some((grid_x, grid_z)) => {
                    let origin = grid_origin(grid_x, grid_z);
                    Scene {
                        block_name: preview.scene.block_name.clone(),
                        objects: preview
                            .scene
                            .objects
                            .iter()
                            .map(|object| SceneObject {
                                position: object.position + origin,
                                ..object.clone()
                            })
                            .collect(),
                    }
                }
                None => Scene::default(),
            };
        }
    }

    /// The free grid spot the dragged cell would be dropped on, if any.
    fn drop_spot(&self) -> Option<(i32, i32)> {
        self.drop_preview.as_ref()?.spot.filter(|&spot| !self.spot_is_taken(spot))
    }

    /// Returns, once, the room the user chose "Make Start" for in the viewport.
    pub fn take_make_start_request(&mut self) -> Option<usize> {
        self.make_start_request.take()
    }

    /// Makes room `room_index` the starting cell and every other room not.
    ///
    /// As in the game, only the starting cell keeps its exit doors, so the
    /// Daggerfall cells that gain or lose the start are loaded again with or
    /// without them. (Project cells are shown as saved.)
    pub fn set_start_room(&mut self, room_index: usize, library: &mut Arena2Library) {
        for (index, room) in self.rooms.iter_mut().enumerate() {
            let is_start = index == room_index;
            if room.is_starting_block == is_start {
                continue;
            }
            room.is_starting_block = is_start;
            if room.project_room.is_none() {
                if let Ok(block) = library.rdb_block(&room.block_name) {
                    let mut scene = Scene::from_rdb_block(&block);
                    if !is_start {
                        scene.remove_exit_doors();
                    }
                    room.scene = scene;
                }
            }
        }
        self.rebuild_combined_scene();
    }

    /// Returns room `room_index`, if it exists.
    pub fn room(&self, room_index: usize) -> Option<&DungeonRoom> {
        self.rooms.get(room_index)
    }

    /// Text for the status bar.
    pub fn status_text(&self, custom_names: &CustomNames) -> String {
        let hint = match self.hovered_room.and_then(|index| self.rooms.get(index)) {
            Some(room) => format!(
                "{} ({}) - double-click to edit, right-click to name it",
                custom_names.room_label(&room.block_name),
                category_description(&room.block_name)
            ),
            None => "Hover a cell to highlight it, double-click to open it in a cell editor".to_string(),
        };
        format!("{}: {} cells. {hint}", self.description(), self.rooms.len())
    }

    /// Name, region and type, for example "Privateer's Hold (Daggerfall, Human Stronghold)".
    pub fn description(&self) -> String {
        // A new dungeon has no region or type until they are chosen.
        let details: Vec<&str> = [self.region_name.as_str(), self.dungeon_type.map_or("", DungeonType::name)]
            .into_iter()
            .filter(|detail| !detail.is_empty())
            .collect();
        match details.is_empty() {
            true => self.name.clone(),
            false => format!("{} ({})", self.name, details.join(", ")),
        }
    }

    /// Replaces the objects of room `room_index` if they differ, for example after it was edited.
    pub fn update_room_objects(&mut self, room_index: usize, objects: &[SceneObject]) {
        let Some(room) = self.rooms.get_mut(room_index) else { return };
        if room.scene.objects != objects {
            room.scene.objects = objects.to_vec();
            room.edited = true;
            if room.project_room.is_some() {
                room.has_exit = objects.iter().any(SceneObject::is_exit_door);
            }
            room.has_start_marker = room.scene.start_marker_index().is_some();
            self.rebuild_combined_scene();
        }
    }

    /// Copies the objects of every project room changed since loading into `project`.
    pub fn store_edited_rooms(&self, project: &mut Project) {
        for room in self.rooms.iter().filter(|room| room.edited) {
            if let Some(project_room) = room.project_room.and_then(|room_id| project.room_mut(room_id)) {
                if project_room.objects != room.scene.objects {
                    project_room.objects = room.scene.objects.clone();
                }
            }
        }
    }

    /// Index of room `room_index` in the layout the dungeon was loaded from.
    pub fn layout_index(&self, room_index: usize) -> Option<usize> {
        self.rooms.get(room_index).map(|room| room.layout_index)
    }

    /// Makes room `room_index` use the project room `room_id`, for example after copying it.
    pub fn set_project_room(&mut self, room_index: usize, room_id: ItemId) {
        if let Some(room) = self.rooms.get_mut(room_index) {
            room.project_room = Some(room_id);
        }
    }

    /// Frees the GPU image this view renders into. Call when the view is closed.
    pub fn release(&mut self, render_state: &egui_wgpu::RenderState) {
        self.target.release(render_state);
        self.entrance_picker.release(render_state);
    }

    /// Adds the View menu entries.
    pub fn view_menu(&mut self, ui: &mut egui::Ui, library: &mut Arena2Library) {
        if ui.add(egui::Button::new("Frame dungeon").shortcut_text("Home")).clicked() {
            self.frame_dungeon(library);
            ui.close_menu();
        }
        ui.separator();
        ui.add_enabled(self.has_skybox, egui::Checkbox::new(&mut self.show_skybox, "Skybox"))
            .on_hover_text(
                "Draws the dungeon's skybox behind it, as Daggerfall Unity shows it with the .dfmod",
            )
            .on_disabled_hover_text(
                "The dungeon has no skybox: tick Skybox in its properties and choose an image",
            );
        super::room_editor::view_settings_menu(ui, &mut self.view_settings);
    }

    /// Handles keyboard shortcuts for the dungeon view.
    pub fn handle_shortcuts(&mut self, context: &egui::Context, library: &mut Arena2Library) {
        let frame_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Home);
        if !context.wants_keyboard_input()
            && context.input_mut(|input| input.consume_shortcut(&frame_shortcut))
        {
            self.frame_dungeon(library);
        }
        let escape = egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Escape);
        if !context.wants_keyboard_input()
            && !self.selected_rooms.is_empty()
            && context.input_mut(|input| input.consume_shortcut(&escape))
        {
            self.selected_rooms.clear();
        }
    }

    /// Draws the properties panel on the right of `context`. Call it before [`Self::show`].
    ///
    /// Returns what the user asked for: a new name or type (or an attempt to
    /// change a read-only dungeon), or a room to open.
    pub fn show_properties(
        &mut self,
        context: &egui::Context,
        details: &DungeonDetails<'_>,
    ) -> Option<DungeonPropertiesAction> {
        let mut action = None;
        egui::SidePanel::right("dungeon_properties").resizable(true).default_width(280.0).show(
            context,
            |ui| {
                ui.heading("Properties");
                ui.separator();
                egui::Grid::new("dungeon_property_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Name").on_hover_text("The dungeon's name. It can be changed for your own dungeons.");
                    self.show_name_field(ui, details.read_only, &mut action);
                    ui.end_row();

                    ui.label("Type").on_hover_text("Decides which enemies appear at random enemy markers");
                    self.show_type_field(ui, details.read_only, &mut action);
                    ui.end_row();

                    ui.label("Source").on_hover_text("Where the dungeon comes from: Daggerfall's own data (read-only), or a copy in your project.");
                    match (details.read_only, details.based_on) {
                        (true, _) => ui.label("Daggerfall (read-only)"),
                        (false, Some(original)) => ui.label(format!("Copy of {original}")),
                        (false, None) => ui.label("Project"),
                    };
                    ui.end_row();

                    ui.label("Cells").on_hover_text("Number of cells (RDB blocks) the dungeon is built from.");
                    ui.label(self.rooms.len().to_string());
                    ui.end_row();
                });

                self.show_location(ui, details, &mut action);

                ui.add_space(8.0);
                self.show_unclosed_warning(ui, details);
                ui.add_space(4.0);
                ui.strong("Cells").on_hover_text("Every cell in the dungeon. Hover one to highlight it in the view, double-click to open it.");
                ui.label(egui::RichText::new("Hover to highlight, double-click to edit").weak());
                let mut hovered_row = None;
                let mut deleted_row = None;
                let mut clicked_row = None;
                let shift = ui.input(|input| input.modifiers.shift);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (room_index, room) in self.rooms.iter().enumerate() {
                        let room_name =
                            match room.project_room.and_then(|room_id| details.project.room(room_id)) {
                                Some(project_room) => project_room.name.clone(),
                                None => details.custom_names.room_label(&room.block_name),
                            };
                        let response = ui
                            .horizontal(|ui| {
                                let highlighted = self.hovered_room == Some(room_index)
                                    || self.selected_rooms.contains(&room_index);
                                let response = ui.selectable_label(highlighted, room_name);
                                let source = if room.project_room.is_some() { "copy" } else { "Daggerfall" };
                                ui.label(egui::RichText::new(source).weak().small());
                                if room.is_starting_block {
                                    ui.label(
                                        egui::RichText::new("Start")
                                            .color(egui::Color32::from_rgb(120, 230, 140)),
                                    );
                                }
                                response
                            })
                            .inner
                            .on_hover_text(format!(
                                "{} ({})\nDouble-click to open it in a cell editor, Shift+click to select several, \
                                 right-click for more",
                                room.block_name,
                                category_description(&room.block_name)
                            ));
                        if response.hovered() {
                            hovered_row = Some(room_index);
                        }
                        if response.double_clicked() {
                            action = Some(DungeonPropertiesAction::OpenRoom(room_index));
                        } else if response.clicked() && shift {
                            clicked_row = Some(room_index);
                        }
                        let merge_count = self.selected_rooms.len();
                        let in_selection = self.selected_rooms.contains(&room_index);
                        response.context_menu(|ui| {
                            if in_selection && merge_button(ui, merge_count) {
                                self.merge_request = Some(self.selected_rooms.clone());
                            }
                            if make_start_button(ui, room) {
                                action = Some(DungeonPropertiesAction::MakeStart(room_index));
                            }
                            if delete_cell_button(ui) {
                                deleted_row = Some(room_index);
                            }
                        });
                    }
                });
                self.list_hovered_room = hovered_row;
                if clicked_row.is_some() {
                    self.click_room(clicked_row, true);
                }
                if deleted_row.is_some() {
                    self.delete_room_request = deleted_row;
                }
            },
        );

        action
    }

    /// Draws the world map and entrance windows, each in its own OS window,
    /// when they are open. Returns the change the user made in them.
    pub fn show_location_windows(
        &mut self,
        context: &egui::Context,
        request: &LocationWindowsRequest<'_>,
        resources: &mut EditorResources<'_>,
    ) -> Option<DungeonPropertiesAction> {
        let mut action = None;
        if let Some(world) = request.world {
            let map_request = MapPickRequest {
                world,
                dungeon_name: &self.name,
                current: request.location.map_pixel,
                current_is_project_dungeon: !request.read_only,
                other_dungeons: request.other_dungeons,
                read_only: request.read_only,
                placement_blocked: {
                    let problems = self.placement_problems(request.location);
                    (!problems.is_empty()).then(|| problems.join(" and "))
                },
                daggerfall_dungeons: request.daggerfall_dungeons,
            };
            if let Some(choice) = self.map_picker.show(context, &map_request) {
                let replaces = match &choice {
                    MapChoice::Spot(_) => None,
                    MapChoice::Replace(replaced, _) => Some(replaced.clone()),
                };
                action = Some(DungeonPropertiesAction::SetLocation(DungeonLocation {
                    map_pixel: Some(choice.pixel()),
                    replaces,
                    ..request.location.clone()
                }));
            }
        }
        let entrance_request = EntranceRequest {
            dungeon_name: &self.name,
            daggerfall_dungeons: request.daggerfall_dungeons,
            current: request.location.entrance.as_ref(),
            read_only: request.read_only,
        };
        if let Some(entrance) = self.entrance_picker.show(context, &entrance_request, resources) {
            action = Some(DungeonPropertiesAction::SetLocation(DungeonLocation {
                entrance: Some(entrance),
                ..request.location.clone()
            }));
        }
        action
    }

    /// Draws the "Location" section: where the dungeon is (or goes) in the
    /// world, and for a project dungeon what is still missing before it can be
    /// added to the game.
    fn show_location(
        &mut self,
        ui: &mut egui::Ui,
        details: &DungeonDetails<'_>,
        action: &mut Option<DungeonPropertiesAction>,
    ) {
        let location = &details.location;
        let mut edited = location.clone();
        let placement_problems = self.placement_problems(location);
        let can_place = placement_problems.is_empty();
        ui.add_space(8.0);
        ui.strong("Location").on_hover_text(
            "Where the dungeon is in the world. Your own dungeons are added to the game as new locations, \
             at the spot you choose.",
        );
        egui::Grid::new("dungeon_location_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Map position").on_hover_text(
                "The dungeon's spot on the world map, in map pixels: x from 0 (west) to 999 (east), y from \
                 0 (north) to 499 (south). The dungeon belongs to the region of that spot.",
            );
            ui.horizontal(|ui| {
                match (details.read_only, location.map_pixel) {
                    (false, Some(pixel)) => {
                        let mut x = pixel.x;
                        let mut y = pixel.y;
                        // A replacement stays at the replaced dungeon's spot.
                        let can_move = can_place && location.replaces.is_none();
                        ui.add_enabled(
                            can_move,
                            egui::DragValue::new(&mut x).range(0..=crate::arena2::MAP_WIDTH - 1).prefix("x "),
                        );
                        ui.add_enabled(
                            can_move,
                            egui::DragValue::new(&mut y)
                                .range(0..=crate::arena2::MAP_HEIGHT - 1)
                                .prefix("y "),
                        );
                        let moved = MapPixel::new(x, y);
                        let taken = moved != pixel
                            && details
                                .world
                                .and_then(|world| occupant(world, details.other_dungeons, moved))
                                .is_some();
                        if !taken {
                            edited.map_pixel = Some(moved);
                        }
                    }
                    (_, Some(pixel)) => {
                        ui.label(pixel.to_string());
                    }
                    (_, None) => {
                        ui.label("Not placed");
                    }
                }
                let (text, hover) = match details.read_only {
                    true => ("Show map", "Shows where the dungeon is on the world map"),
                    false => ("Map…", "Opens the world map to choose the dungeon's spot"),
                };
                let mut button = ui
                    .add_enabled(details.world.is_some(), egui::Button::new(text))
                    .on_hover_text(hover)
                    .on_disabled_hover_text("The world map needs MAPS.BSA in the ARENA2 folder");
                if !details.read_only && !can_place {
                    button = button.on_hover_text(format!(
                        "It cannot be placed yet: {}. The map can still be looked at.",
                        placement_problems.join(" and ")
                    ));
                }
                if button.clicked() {
                    self.map_picker.open(location.map_pixel);
                }
                if !details.read_only && location.map_pixel.is_some() {
                    let remove = ui.button("Remove").on_hover_text(
                        "Takes the dungeon off the world map. Its other location settings are kept.",
                    );
                    if remove.clicked() {
                        edited.map_pixel = None;
                        edited.replaces = None;
                    }
                }
            });
            ui.end_row();

            if let Some(replaced) = location.replaces.as_ref().filter(|_| !details.read_only) {
                ui.label("Replaces").on_hover_text(
                    "The Daggerfall dungeon this one takes the place of. On export it replaces that dungeon in \
                     the game instead of being added as a new location.",
                );
                ui.horizontal(|ui| {
                    ui.label(replaced.label()).on_hover_text(format!(
                        "In the game the dungeon keeps the name {}, which quests and the travel map know it by, \
                         and its spot.",
                        replaced.name
                    ));
                    let stop = ui
                        .button("Stop replacing")
                        .on_hover_text("Takes the dungeon off the map, so it can be placed on a free spot instead");
                    if stop.clicked() {
                        edited.map_pixel = None;
                        edited.replaces = None;
                    }
                });
                ui.end_row();
            }

            ui.label("Region").on_hover_text(
                "The region of the Iliac Bay the dungeon belongs to: the region of its spot on the map.",
            );
            ui.label(&self.region_name);
            ui.end_row();

            ui.label("Location type").on_hover_text("Decides the dungeon's icon on the travel map.");
            if details.read_only {
                ui.label(location.location_type.name());
            } else {
                egui::ComboBox::from_id_salt("dungeon_location_type")
                    .selected_text(location.location_type.name())
                    .show_ui(ui, |ui| {
                        for location_type in LocationType::DUNGEON_TYPES {
                            ui.selectable_value(
                                &mut edited.location_type,
                                location_type,
                                location_type.name(),
                            );
                        }
                    });
            }
            ui.end_row();

            if !details.read_only {
                ui.label("Discovered").on_hover_text(
                    "Shown on the travel map from the start of the game. Otherwise the player has to hear of \
                     it (or find it) first.",
                );
                ui.checkbox(&mut edited.discovered, "From the start");
                ui.end_row();
            }

            ui.label("Entrance").on_hover_text(
                "What stands above ground around the way down: ruins, a tower, a graveyard… It is taken \
                 from one of Daggerfall's dungeons.",
            );
            let text = match (details.read_only, &location.entrance) {
                (true, _) => "View…".to_string(),
                (false, Some(entrance)) => entrance_label(entrance, details.daggerfall_dungeons),
                (false, None) => "Choose…".to_string(),
            };
            let hover = match details.read_only {
                true => "Shows the dungeon's entrance in a window of its own",
                false => "Opens a window of every entrance a dungeon can have, with a preview of each",
            };
            let mut button = ui.add(egui::Button::new(text).truncate()).on_hover_text(hover);
            if let Some(entrance) = location.entrance.as_ref().filter(|_| !details.read_only) {
                button = button.on_hover_text(format!("Taken from {}", entrance.label()));
            }
            if button.clicked() {
                self.entrance_picker.open();
            }
            ui.end_row();

            self.show_music_row(ui, details, &mut edited, action);
            ui.end_row();

            if !details.read_only {
                show_custom_music_rows(ui, &mut edited);
                show_skybox_rows(ui, &mut edited);
            }
        });

        if !details.read_only {
            let mut problems = location.problems(details.world, details.other_dungeons);
            problems.extend(crate::export::custom_music_problem(location));
            problems.extend(crate::export::skybox_problem(location));
            if !self.has_usable_exit() {
                problems.push(
                    "It has no exit: its start cell needs an exit door (model 70300) for the player to leave by"
                        .to_string(),
                );
            }
            if problems.is_empty() {
                ui.colored_label(GOOD_COLOR, "✔ The location is complete");
            }
            for problem in problems {
                ui.colored_label(WARNING_COLOR, format!("⚠ {problem}"));
            }
        }

        if edited != *location {
            *action = Some(DungeonPropertiesAction::SetLocation(edited));
        }
    }

    /// Draws the "Music" row: the song played inside (a choice for a project
    /// dungeon), with a button that plays it, which turns into Stop while it plays.
    fn show_music_row(
        &mut self,
        ui: &mut egui::Ui,
        details: &DungeonDetails<'_>,
        edited: &mut DungeonLocation,
        action: &mut Option<DungeonPropertiesAction>,
    ) {
        ui.label("Music").on_hover_text(
            "The song played inside the dungeon (in General MIDI; with FM music the game plays the FM version \
             of the same song). Music mods may play something else.",
        );
        let location = &details.location;
        // The song that will play: the one chosen, or else the game's choice when known.
        let song = location.music.or(details.game_song);
        ui.horizontal(|ui| {
            // With custom music the game's songs are not used.
            if location.custom_music && !details.read_only {
                ui.disable();
            }
            if details.read_only {
                ui.label(song.map_or_else(|| "Unknown".to_string(), DungeonSong::label));
            } else {
                let game_choice = match details.game_song {
                    Some(game_song) => format!("Chosen by the game: {}", game_song.label()),
                    None => "Chosen by the game".to_string(),
                };
                let selected = location.music.map_or(game_choice.clone(), DungeonSong::label);
                egui::ComboBox::from_id_salt("dungeon_music").selected_text(selected).width(190.0).show_ui(
                    ui,
                    |ui| {
                        ui.selectable_value(&mut edited.music, None, game_choice).on_hover_text(
                            match location.replaces {
                                Some(_) => "Keeps the song the replaced dungeon has",
                                None => "The game picks one of its dungeon songs from the location's data",
                            },
                        );
                        for choice in DUNGEON_SONGS {
                            ui.selectable_value(&mut edited.music, Some(choice), choice.label());
                        }
                    },
                );
            }
            // Choosing another song stops the one playing, so Play is offered for the new one.
            let song = match edited.music != location.music {
                true => {
                    self.music_player.stop();
                    edited.music.or(details.game_song)
                }
                false => song,
            };
            let playing = self.music_player.playing_file().map(str::to_string);
            if playing.is_some() {
                // Turn Stop back into Play when the song ends.
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
            match song {
                Some(song) if playing.as_deref() == Some(song.file) => {
                    if ui.button("■ Stop").on_hover_text("Stops the song").clicked() {
                        self.music_player.stop();
                    }
                }
                Some(song) => {
                    if ui
                        .button("▶ Play")
                        .on_hover_text(format!("Plays {} to preview it", song.label()))
                        .clicked()
                    {
                        *action = Some(DungeonPropertiesAction::PlaySong(song));
                    }
                }
                None => {
                    ui.add_enabled(false, egui::Button::new("▶ Play")).on_disabled_hover_text(
                        "The game picks the song when the dungeon is exported; choose one to hear it",
                    );
                }
            }
        });
    }

    /// Draws the name field. A new name is applied when the field loses focus
    /// (Escape cancels it); typing in a read-only dungeon's name is refused.
    fn show_name_field(
        &mut self,
        ui: &mut egui::Ui,
        read_only: bool,
        action: &mut Option<DungeonPropertiesAction>,
    ) {
        let mut text = self.name_edit.clone().unwrap_or_else(|| self.name.clone());
        let response = ui.text_edit_singleline(&mut text);
        if response.changed() {
            if read_only {
                *action = Some(DungeonPropertiesAction::EditBlocked);
            } else {
                self.name_edit = Some(text);
            }
        }
        if response.lost_focus() {
            let cancelled = ui.input(|input| input.key_pressed(egui::Key::Escape));
            if let Some(name) = self.name_edit.take().filter(|_| !cancelled) {
                let name = name.trim();
                if !name.is_empty() && name != self.name {
                    *action = Some(DungeonPropertiesAction::Rename(name.to_string()));
                }
            }
        }
    }

    /// Draws the dungeon type drop-down; choosing a type for a read-only dungeon is refused.
    fn show_type_field(
        &self,
        ui: &mut egui::Ui,
        read_only: bool,
        action: &mut Option<DungeonPropertiesAction>,
    ) {
        let selected = self.dungeon_type.map_or("Unknown", DungeonType::name);
        egui::ComboBox::from_id_salt("dungeon_type").selected_text(selected).width(180.0).show_ui(ui, |ui| {
            for dungeon_type in DungeonType::all() {
                let is_current = self.dungeon_type == Some(dungeon_type);
                if ui.selectable_label(is_current, dungeon_type.name()).clicked() && !is_current {
                    *action = Some(if read_only {
                        DungeonPropertiesAction::EditBlocked
                    } else {
                        DungeonPropertiesAction::SetType(dungeon_type)
                    });
                }
            }
        });
    }

    /// Draws the dungeon and returns the index of a room the user double-clicked.
    ///
    /// A cell dragged in from the project panel is shown, faded, on the grid
    /// spot under the pointer; dropping it there asks for it to be added (see
    /// [`Self::take_add_room_request`]).
    ///
    /// `skybox` is the dungeon's skybox image, drawn behind it unless View >
    /// Skybox is off.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        resources: &mut EditorResources<'_>,
        project: &Project,
        skybox: Option<&std::path::Path>,
    ) -> Option<usize> {
        self.has_skybox = skybox.is_some();
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        // When the camera moves, what is under a still pointer changes too.
        if handle_camera_input(&mut self.camera, ui, &response) {
            self.last_pointer_position = None;
        }
        let projection = ViewportProjection::new(&self.camera, rect);
        // Shift+drag draws a box selecting cells; a drag without Shift moves the cell under the pointer.
        let shift = ui.input(|input| input.modifiers.shift);
        if response.drag_started_by(egui::PointerButton::Primary) && shift {
            self.selection_box_start = response.interact_pointer_pos();
        }
        if self.selection_box_start.is_some() {
            self.handle_selection_box(&response, &projection);
        } else {
            self.handle_room_drag(&response, &projection);
        }
        if response.clicked_by(egui::PointerButton::Primary) {
            self.click_room(self.hovered_room, shift);
        }
        self.update_drop_preview(ui, &response, &projection, project, resources.library);
        let drop_spot = self.drop_spot();
        let mut room_outlines = self.room_outlines.clone();
        if let Some((grid_x, grid_z)) = drop_spot {
            room_outlines.push(RoomOutline { origin: grid_origin(grid_x, grid_z), highlighted: true });
        }

        if !response.dragged() {
            let pointer_position = response.hover_pos();
            if pointer_position != self.last_pointer_position {
                self.last_pointer_position = pointer_position;
                self.hovered_room = pointer_position
                    .and_then(|pointer| self.room_under_pointer(&projection, pointer, resources.library));
            }
        }

        let dragged_room = self.room_drag.as_ref().map(|drag| drag.room_index);
        let highlighted_room = dragged_room.or(self.list_hovered_room).or(self.hovered_room);
        for (room_index, outline) in self.room_outlines.iter_mut().enumerate() {
            outline.highlighted = highlighted_room == Some(room_index)
                || self.selected_rooms.contains(&room_index)
                || self.box_rooms.contains(&room_index);
        }
        let sky = skybox.filter(|_| self.show_skybox);
        let request = FrameRequest {
            scene: Some(&self.combined_scene),
            camera: &self.camera,
            selected_object: None,
            hovered_object: None,
            selected_objects: &[],
            hovered_objects: &[],
            highlighted_group: highlighted_room,
            highlighted_groups: &self.box_rooms,
            selected_groups: &self.selected_rooms,
            placement_preview: None,
            context_scene: drop_spot.and(self.drop_preview.as_ref()).map(|preview| &preview.placed),
            ground_squares: &[],
            water_planes: &self.water_planes,
            room_outlines: &room_outlines,
            settings: &self.view_settings,
            grid_height: self.grid_height,
            snap_grid_spacing: None,
            size_in_pixels: size_in_pixels(ui, rect),
            sky,
        };
        paint_scene(ui, rect, resources, &mut self.target, &request);
        if let Some(start) = self.selection_box_start {
            if let Some(pointer) = response.interact_pointer_pos() {
                let painter = ui.painter_at(rect);
                let selection_box = egui::Rect::from_two_pos(start, pointer);
                painter.rect_filled(selection_box, 0.0, SELECTION_BOX_FILL);
                painter.rect_stroke(selection_box, 0.0, egui::Stroke::new(1.0_f32, SELECTION_BOX_STROKE));
            }
        }
        // A skybox image that cannot be read is said so in the view.
        let sky_error = sky.and_then(|path| {
            let error = resources.renderer.as_deref()?.sky_error(path)?;
            Some(format!("Skybox {}: {error}", path.display()))
        });
        if let Some(error) = sky_error {
            ui.painter_at(rect).text(
                rect.left_top() + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                error,
                egui::FontId::proportional(13.0),
                WARNING_COLOR,
            );
        }
        self.update_unclosed_openings(ui, resources);
        if let Some(unclosed) = &self.unclosed {
            draw_unclosed_openings(&ui.painter_at(rect), &projection, unclosed);
        }
        self.draw_room_labels(ui, &projection, &response, resources.custom_names);
        self.draw_drop_label(ui, &projection, drop_spot);
        self.draw_room_drag_label(ui, &projection, resources.custom_names);

        // Dropping a dragged cell on a free spot adds it there.
        if self.drop_preview.is_some() && ui.input(|input| input.pointer.any_released()) {
            if let (Some((grid_x, grid_z)), Some(preview)) = (drop_spot, self.drop_preview.take()) {
                self.add_room_request = Some(ProjectDungeonRoom {
                    block_name: preview.cell.0,
                    grid_x,
                    grid_z,
                    is_starting_block: false,
                    room: preview.cell.1,
                });
            }
            egui::DragAndDrop::clear_payload(ui.ctx());
            self.drop_preview = None;
        }

        // Delete takes out the cell under the pointer, in the view or in the cell list.
        let target = match response.contains_pointer() {
            true => self.hovered_room,
            false => self.list_hovered_room,
        };
        if let Some(room_index) = target.filter(|_| self.drop_preview.is_none()) {
            if !ui.ctx().wants_keyboard_input()
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            {
                self.delete_room_request = Some(room_index);
            }
        }

        // Right-click (without dragging, which orbits) names the room under the pointer;
        // with several cells selected, it can merge them.
        if response.secondary_clicked() {
            self.renamed_room = self.hovered_room;
        }
        let merge_count = self.selected_rooms.len();
        let can_merge = merge_count >= 2
            && self.renamed_room.is_none_or(|room_index| self.selected_rooms.contains(&room_index));
        response.context_menu(|ui| {
            if can_merge && merge_button(ui, merge_count) {
                self.merge_request = Some(self.selected_rooms.clone());
            }
            let Some(room) = self.renamed_room.and_then(|index| self.rooms.get(index)) else {
                if !can_merge {
                    ui.close_menu();
                }
                return;
            };
            if can_merge {
                ui.separator();
            }
            ui.strong(&room.block_name);
            show_room_name_field(ui, &room.block_name, resources.custom_names);
            ui.separator();
            if make_start_button(ui, room) {
                self.make_start_request = self.renamed_room;
            }
            if delete_cell_button(ui) {
                self.delete_room_request = self.renamed_room;
            }
        });

        let double_clicked = response.double_clicked_by(egui::PointerButton::Primary);
        double_clicked.then_some(self.hovered_room).flatten()
    }

    /// Finds the room under the pointer: the room owning the object hit, or failing
    /// that the block whose footprint the pointer is over.
    fn room_under_pointer(
        &self,
        projection: &ViewportProjection,
        pointer: egui::Pos2,
        library: &mut Arena2Library,
    ) -> Option<usize> {
        let ray = self.camera.ray_through(projection.screen_to_ndc(pointer), aspect_ratio(projection.rect));
        let settings = &self.view_settings;
        let options = PickOptions {
            ignore_back_faces: settings.cull_back_faces,
            include_models: settings.show_models,
            include_flats: settings.show_flats || settings.show_markers,
            include_lights: false,
            camera_right: self.camera.right(),
        };
        if let Some(hit) = pick_object(&self.combined_scene, library, &ray, &options) {
            return Some(self.combined_scene.objects[hit.object_index].group_index);
        }

        let floor_point = ray.point_at(intersect_horizontal_plane(&ray, self.grid_height)?);
        self.rooms.iter().position(|room| {
            let local = floor_point - room.origin;
            (0.0..RDB_BLOCK_SIDE).contains(&local.x) && (0.0..RDB_BLOCK_SIDE).contains(&local.z)
        })
    }

    /// Labels the starting room, and the hovered room next to the pointer.
    fn draw_room_labels(
        &self,
        ui: &egui::Ui,
        projection: &ViewportProjection,
        response: &egui::Response,
        custom_names: &CustomNames,
    ) {
        let painter = ui.painter_at(projection.rect);
        let block_center = |room: &DungeonRoom| {
            room.origin + Vec3::new(RDB_BLOCK_SIDE * 0.5, self.grid_height, RDB_BLOCK_SIDE * 0.5)
        };

        if let Some(start_room) = self.rooms.iter().find(|room| room.is_starting_block) {
            if let Some(position) = projection.world_to_screen(block_center(start_room)) {
                painter.text(
                    position,
                    egui::Align2::CENTER_CENTER,
                    "Start",
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_rgb(120, 230, 140),
                );
            }
        }

        let hovered =
            self.hovered_room.filter(|_| self.room_drag.is_none()).and_then(|index| self.rooms.get(index));
        if let (Some(room), Some(pointer)) = (hovered, response.hover_pos()) {
            let label = format!(
                "{} ({})\nDouble-click to edit this cell · drag to move it · Delete to remove it\n\
                 Shift+click or Shift+drag a box to select several, then right-click to merge them",
                custom_names.room_label(&room.block_name),
                category_description(&room.block_name)
            );
            painter.text(
                pointer + egui::vec2(16.0, 16.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }
    }

    /// Finds again the openings that lead to empty space when the dungeon
    /// changed, at most every [`UNCLOSED_CHECK_INTERVAL`] while it keeps changing.
    fn update_unclosed_openings(&mut self, ui: &egui::Ui, resources: &mut EditorResources<'_>) {
        if !self.unclosed_stale && self.unclosed.is_some() {
            return;
        }
        let now = ui.input(|input| input.time);
        if now - self.unclosed_checked_at < UNCLOSED_CHECK_INTERVAL {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(UNCLOSED_CHECK_INTERVAL));
            return;
        }
        let objects = &self.combined_scene.objects;
        let found = find_unclosed_openings(
            objects,
            objects.len(),
            false,
            resources.library,
            resources.catalog,
            &mut self.open_faces,
        );
        if found.is_some() {
            self.unclosed = found;
            self.unclosed_stale = false;
            self.unclosed_checked_at = now;
        }
    }

    /// Warns, in the properties panel, of openings that lead to empty space.
    fn show_unclosed_warning(&self, ui: &mut egui::Ui, details: &DungeonDetails<'_>) {
        let Some(unclosed) = &self.unclosed else {
            ui.label(egui::RichText::new("Checking that every room is closed…").weak()).on_hover_text(
                "The openings Daggerfall's own dungeons leave open are being learned from the game data, \
                 the first time only",
            );
            return;
        };
        if unclosed.is_empty() {
            return;
        }
        let mut cells: Vec<String> = Vec::new();
        for face in unclosed {
            let Some(room_index) =
                self.combined_scene.objects.get(face.owner).map(|object| object.group_index)
            else {
                continue;
            };
            let Some(room) = self.rooms.get(room_index) else { continue };
            let name = match room.project_room.and_then(|room_id| details.project.room(room_id)) {
                Some(project_room) => project_room.name.clone(),
                None => details.custom_names.room_label(&room.block_name),
            };
            if !cells.contains(&name) {
                cells.push(name);
            }
        }
        let openings = if unclosed.len() == 1 { "1 opening leads" } else { "openings lead" };
        let count = if unclosed.len() == 1 { String::new() } else { format!("{} ", unclosed.len()) };
        ui.colored_label(WARNING_COLOR, "⚠ Not all rooms are closed").on_hover_text(
            "Pieces such as halls have openings that join other pieces. These lead out into empty space, \
             where the player could see or fall out of the dungeon. They are marked with yellow panes in the \
             view. This is only a warning: it does not stop saving or exporting.",
        );
        ui.label(
            egui::RichText::new(format!("{count}{openings} to empty space, in {}", cells.join(", ")))
                .color(WARNING_COLOR)
                .small(),
        );
    }

    /// Says next to the pointer where the cell being moved would go.
    fn draw_room_drag_label(
        &self,
        ui: &egui::Ui,
        projection: &ViewportProjection,
        custom_names: &CustomNames,
    ) {
        let (Some(drag), Some(pointer)) = (&self.room_drag, ui.ctx().pointer_interact_pos()) else { return };
        let Some(room) = self.rooms.get(drag.room_index) else { return };
        let (text, color) = if !self.spot_is_free_for(drag.room_index, drag.to) {
            (
                "A cell is already here: let go to put it back".to_string(),
                egui::Color32::from_rgb(255, 120, 100),
            )
        } else if drag.to == drag.from {
            (format!("Moving {}", custom_names.room_label(&room.block_name)), egui::Color32::WHITE)
        } else {
            (
                format!("Move {} to {}, {}", custom_names.room_label(&room.block_name), drag.to.0, drag.to.1),
                egui::Color32::WHITE,
            )
        };
        ui.painter_at(projection.rect).text(
            pointer + egui::vec2(16.0, 16.0),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::proportional(13.0),
            color,
        );
    }

    /// Says next to the pointer what dropping the dragged cell would do.
    fn draw_drop_label(&self, ui: &egui::Ui, projection: &ViewportProjection, drop_spot: Option<(i32, i32)>) {
        let (Some(preview), Some(pointer)) = (&self.drop_preview, ui.ctx().pointer_hover_pos()) else {
            return;
        };
        let (text, color) = match (drop_spot, preview.spot) {
            (Some(_), _) => (format!("Drop to add {} here", preview.label), egui::Color32::WHITE),
            (None, Some(_)) => ("A cell is already here".to_string(), egui::Color32::from_rgb(255, 120, 100)),
            (None, None) => {
                ("Point at the floor grid to place the cell".to_string(), egui::Color32::LIGHT_GRAY)
            }
        };
        // Below the panel's own label of the dragged entry.
        ui.painter_at(projection.rect).text(
            pointer + egui::vec2(16.0, 34.0),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::proportional(13.0),
            color,
        );
    }

    /// Frames the camera on the whole dungeon.
    fn frame_dungeon(&mut self, library: &mut Arena2Library) {
        if let Some((minimum, maximum)) = self.combined_scene.bounds(library) {
            self.camera.frame_bounds(minimum, maximum);
        }
    }

    /// Rebuilds the world space scene and outlines from the per-room scenes.
    fn rebuild_combined_scene(&mut self) {
        let objects = self
            .rooms
            .iter()
            .enumerate()
            .flat_map(|(room_index, room)| {
                let id_base = (room_index as ObjectId + 1) << 16;
                room.scene.objects.iter().map(move |object| SceneObject {
                    id: id_base | object.id,
                    next: object.next.map(|next| id_base | next),
                    position: object.position + room.origin,
                    group_index: room_index,
                    ..object.clone()
                })
            })
            .collect();
        self.combined_scene = Scene { block_name: self.name.clone(), objects };
        self.unclosed_stale = true;
        self.room_outlines =
            self.rooms.iter().map(|room| RoomOutline { origin: room.origin, highlighted: false }).collect();
        self.water_planes = self
            .rooms
            .iter()
            .filter_map(|room| Some(room.origin + Vec3::new(0.0, room.scene.water_level()?, 0.0)))
            .collect();
    }
}

/// Draws the "Custom music" rows: a tick box for playing a music file of the
/// user's own instead of a game song and, while it is ticked, the file with
/// **Browse…** to choose it.
fn show_custom_music_rows(ui: &mut egui::Ui, edited: &mut DungeonLocation) {
    ui.label("Custom music").on_hover_text(
        "Plays a music file of your own (OGG or MP3) inside the dungeon instead of a game song. Only a .dfmod \
         can do this: it carries a script that plays the file there. With the exported files used loose, the \
         game's music plays instead (the song chosen above).",
    );
    ui.checkbox(&mut edited.custom_music, "Play my own music file");
    ui.end_row();
    if !edited.custom_music {
        return;
    }
    ui.label("Music file")
        .on_hover_text("The OGG or MP3 file played inside the dungeon; it is copied into the mod");
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut edited.custom_music_file)
                .hint_text("Choose an OGG or MP3 file")
                .desired_width(170.0),
        );
        if ui.button("Browse…").clicked() {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Music for the dungeon")
                .add_filter("Music (OGG, MP3)", &crate::project::CUSTOM_MUSIC_EXTENSIONS);
            if let Some(folder) = std::path::Path::new(edited.custom_music_file.trim())
                .parent()
                .filter(|folder| folder.is_dir())
            {
                dialog = dialog.set_directory(folder);
            }
            if let Some(file) = dialog.pick_file() {
                edited.custom_music_file = file.display().to_string();
            }
        }
    });
    ui.end_row();
}

/// Draws the "Skybox" rows: a tick box for showing a sky behind the dungeon
/// and, while it is ticked, the image with **Browse…** to choose it.
fn show_skybox_rows(ui: &mut egui::Ui, edited: &mut DungeonLocation) {
    ui.label("Skybox").on_hover_text(
        "Shows a sky of your own behind the dungeon instead of black, seen through any openings to the \
         outside (a broken ceiling, a chasm, an open courtyard). Only a .dfmod can do this: it carries a \
         script that shows the sky there. With the exported files used loose, the dungeon stays black.",
    );
    ui.checkbox(&mut edited.skybox, "Show a sky");
    ui.end_row();
    if !edited.skybox {
        return;
    }
    ui.label("Sky image").on_hover_text(
        "A PNG or JPG panorama of the whole sky (360 degrees around, from straight up to straight down), \
         twice as wide as it is high, for example 4096 × 2048. It is copied into the mod.",
    );
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut edited.skybox_file)
                .hint_text("Choose a PNG or JPG image")
                .desired_width(170.0),
        );
        if ui.button("Browse…").clicked() {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Sky image for the dungeon")
                .add_filter("Images (PNG, JPG)", &crate::project::SKYBOX_EXTENSIONS);
            if let Some(folder) =
                std::path::Path::new(edited.skybox_file.trim()).parent().filter(|folder| folder.is_dir())
            {
                dialog = dialog.set_directory(folder);
            }
            if let Some(file) = dialog.pick_file() {
                edited.skybox_file = file.display().to_string();
            }
        }
    });
    ui.end_row();
}

/// Draws the "Merge" entry of a cell menu, for the `count` selected cells.
/// Returns whether it was clicked (the menu then closes).
fn merge_button(ui: &mut egui::Ui, count: usize) -> bool {
    let response = ui
        .add_enabled(count >= 2, egui::Button::new(format!("Merge {count} selected cells")))
        .on_hover_text(
            "Makes one new cell of your own from the selected cells, with all their objects where they are \
             now, and puts it in the dungeon in their place. The cells themselves are not changed: your own \
             cells stay in the project.",
        )
        .on_disabled_hover_text("Shift+click, or Shift+drag a box over, at least two cells to merge them");
    let clicked = response.clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

/// World position of the minimum corner of grid spot (`grid_x`, `grid_z`).
fn grid_origin(grid_x: i32, grid_z: i32) -> Vec3 {
    Vec3::new(grid_x as f32 * RDB_BLOCK_SIDE, 0.0, grid_z as f32 * RDB_BLOCK_SIDE)
}

/// The grid spot holding world position `point`.
fn grid_spot(point: Vec3) -> (i32, i32) {
    ((point.x / RDB_BLOCK_SIDE).floor() as i32, (point.z / RDB_BLOCK_SIDE).floor() as i32)
}

/// The cell `payload` drags, if it is one: its block name and project cell
/// id, and its name. A project cell made from nothing has no block name of its
/// own, so its name stands in.
fn dragged_cell(payload: &DragPayload, project: &Project) -> Option<((String, Option<ItemId>), String)> {
    match payload {
        DragPayload::Project(id) => {
            let room = project.room(*id)?;
            let block_name = room.based_on.clone().unwrap_or_else(|| room.name.clone());
            Some(((block_name, Some(room.id)), room.name.clone()))
        }
        DragPayload::Daggerfall(OpenedItem::DaggerfallRoom(name)) => {
            Some(((name.clone(), None), name.clone()))
        }
        DragPayload::Daggerfall(_) => None,
    }
}

/// Adds the "Delete" entry to a cell's menu. Returns `true` when chosen.
fn delete_cell_button(ui: &mut egui::Ui) -> bool {
    let clicked = ui
        .button("Delete")
        .on_hover_text(
            "Takes the cell out of the dungeon (Delete key while hovering it does the same). A project cell              stays in the project.",
        )
        .clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

/// Adds the "Make Start" entry to a cell's menu. Returns `true` when chosen.
///
/// A cell without an exit door (the player could not leave the dungeon) or
/// without a start marker (the player would have nowhere to arrive) cannot be
/// the start; the entry is then disabled and the menu says why.
fn make_start_button(ui: &mut egui::Ui, room: &DungeonRoom) -> bool {
    let response = ui
        .add_enabled(room.can_be_start(), egui::Button::new("Make Start"))
        .on_hover_text(
            "Makes this the cell the player enters the dungeon in. Only the start cell keeps its exit \
             doors; the cell that was the start before stops being it.",
        )
        .on_disabled_hover_text(if room.is_starting_block {
            "This is already the dungeon's start cell"
        } else {
            "A start cell needs an exit door (model 70300), so the player can leave the dungeon, and a \
             start marker, where the player arrives"
        });
    if !room.is_starting_block {
        for problem in room.start_problems() {
            ui.colored_label(egui::Color32::from_rgb(255, 190, 70), format!("⚠ {problem}"));
        }
    }
    if response.clicked() {
        ui.close_menu();
    }
    response.clicked()
}

impl DungeonRoom {
    /// Why the cell cannot be the dungeon's start cell; empty if it can.
    pub fn start_problems(&self) -> Vec<&'static str> {
        let mut problems = Vec::new();
        if !self.has_exit {
            problems.push("No exit exists for this cell");
        }
        if !self.has_start_marker {
            problems.push("No start marker exists for this cell");
        }
        problems
    }

    /// Whether "Make Start" can make this cell the start.
    fn can_be_start(&self) -> bool {
        !self.is_starting_block && self.start_problems().is_empty()
    }
}

/// Whether Daggerfall's cell `block_name` has an exit door of its own.
fn cell_has_exit(block_name: &str, library: &mut Arena2Library) -> bool {
    library
        .rdb_block(block_name)
        .is_ok_and(|block| Scene::from_rdb_block(&block).objects.iter().any(SceneObject::is_exit_door))
}

/// Builds the scene of one dungeon room: the project room it uses, or the
/// Daggerfall block from the game data. Returns the project room's id too.
pub(super) fn load_room_scene(
    placement: &ProjectDungeonRoom,
    project: &Project,
    library: &mut Arena2Library,
) -> Result<(Scene, Option<ItemId>), String> {
    if let Some(room) = placement.room.and_then(|room_id| project.room(room_id)) {
        let scene = Scene { block_name: placement.block_name.clone(), objects: room.objects.clone() };
        return Ok((scene, Some(room.id)));
    }
    let block = library.rdb_block(&placement.block_name).map_err(|error| error.to_string())?;
    let mut scene = Scene::from_rdb_block(&block);
    // Only the starting block keeps its way out, as in the game.
    if !placement.is_starting_block {
        scene.remove_exit_doors();
    }
    Ok((scene, None))
}

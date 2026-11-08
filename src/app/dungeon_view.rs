//! Shows a whole pre-built dungeon: every RDB block (room) placed on its grid.
//!
//! Hovering a room highlights all of its objects and its block outline;
//! double-clicking a room asks the application to open it in a room editor
//! window. Edits made in those windows are copied back with
//! [`DungeonView::update_room_objects`] so the dungeon always shows the latest version.
//!
//! The properties panel on the right shows the dungeon's name, region, type and
//! rooms. The name and type of a project dungeon can be changed there.

use eframe::{egui, egui_wgpu};
use glam::Vec3;

use super::room_browser::{category_description, show_room_name_field};
use super::viewport_tools::{
    aspect_ratio, handle_camera_input, paint_scene, size_in_pixels, EditorResources, ViewportProjection,
};
use crate::arena2::{Arena2Library, RDB_BLOCK_SIDE};
use crate::camera::OrbitCamera;
use crate::custom_names::CustomNames;
use crate::encounters::DungeonType;
use crate::picking::{intersect_horizontal_plane, pick_object, PickOptions};
use crate::project::{ItemId, Project, ProjectDungeonRoom};
use crate::render::{FrameRequest, RoomOutline, ViewSettings, ViewportTarget};
use crate::scene::{Scene, SceneObject};

/// Something the user did in the dungeon's properties panel, for the application to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum DungeonPropertiesAction {
    Rename(String),
    SetType(DungeonType),
    /// Open room `index` in a room editor window.
    OpenRoom(usize),
    /// The user tried to change a dungeon from Daggerfall, which cannot be edited.
    EditBlocked,
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
    last_pointer_position: Option<egui::Pos2>,
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
            camera: OrbitCamera::default(),
            view_settings: ViewSettings::default(),
            grid_height: 0.0,
            target: ViewportTarget::default(),
            hovered_room: None,
            list_hovered_room: None,
            name_edit: None,
            renamed_room: None,
            last_pointer_position: None,
        };
        view.rebuild_combined_scene();
        view.frame_dungeon(library);
        if let Some((minimum, _)) = view.combined_scene.bounds(library) {
            view.grid_height = minimum.y - 0.05;
        }
        (view, problems)
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
        match self.dungeon_type {
            Some(dungeon_type) => format!("{} ({}, {})", self.name, self.region_name, dungeon_type.name()),
            None => format!("{} ({})", self.name, self.region_name),
        }
    }

    /// Replaces the objects of room `room_index` if they differ, for example after it was edited.
    pub fn update_room_objects(&mut self, room_index: usize, objects: &[SceneObject]) {
        let Some(room) = self.rooms.get_mut(room_index) else { return };
        if room.scene.objects != objects {
            room.scene.objects = objects.to_vec();
            room.edited = true;
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
    }

    /// Adds the View menu entries.
    pub fn view_menu(&mut self, ui: &mut egui::Ui, library: &mut Arena2Library) {
        if ui.add(egui::Button::new("Frame dungeon").shortcut_text("Home")).clicked() {
            self.frame_dungeon(library);
            ui.close_menu();
        }
        ui.separator();
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
                    ui.label("Name");
                    self.show_name_field(ui, details.read_only, &mut action);
                    ui.end_row();

                    ui.label("Type").on_hover_text("Decides which enemies appear at random enemy markers");
                    self.show_type_field(ui, details.read_only, &mut action);
                    ui.end_row();

                    ui.label("Region");
                    ui.label(&self.region_name);
                    ui.end_row();

                    ui.label("Source");
                    match (details.read_only, details.based_on) {
                        (true, _) => ui.label("Daggerfall (read-only)"),
                        (false, Some(original)) => ui.label(format!("Copy of {original}")),
                        (false, None) => ui.label("Project"),
                    };
                    ui.end_row();

                    ui.label("Cells");
                    ui.label(self.rooms.len().to_string());
                    ui.end_row();
                });

                ui.add_space(8.0);
                ui.strong("Cells");
                ui.label(egui::RichText::new("Hover to highlight, double-click to edit").weak());
                let mut hovered_row = None;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (room_index, room) in self.rooms.iter().enumerate() {
                        let room_name =
                            match room.project_room.and_then(|room_id| details.project.room(room_id)) {
                                Some(project_room) => project_room.name.clone(),
                                None => details.custom_names.room_label(&room.block_name),
                            };
                        let response = ui
                            .horizontal(|ui| {
                                let highlighted = self.hovered_room == Some(room_index);
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
                                "{} ({})\nDouble-click to open it in a cell editor",
                                room.block_name,
                                category_description(&room.block_name)
                            ));
                        if response.hovered() {
                            hovered_row = Some(room_index);
                        }
                        if response.double_clicked() {
                            action = Some(DungeonPropertiesAction::OpenRoom(room_index));
                        }
                    }
                });
                self.list_hovered_room = hovered_row;
            },
        );
        action
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
    pub fn show(&mut self, ui: &mut egui::Ui, resources: &mut EditorResources<'_>) -> Option<usize> {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        handle_camera_input(&mut self.camera, ui, &response);
        let projection = ViewportProjection::new(&self.camera, rect);

        if !response.dragged() {
            let pointer_position = response.hover_pos();
            if pointer_position != self.last_pointer_position {
                self.last_pointer_position = pointer_position;
                self.hovered_room = pointer_position
                    .and_then(|pointer| self.room_under_pointer(&projection, pointer, resources.library));
            }
        }

        let highlighted_room = self.list_hovered_room.or(self.hovered_room);
        for (room_index, outline) in self.room_outlines.iter_mut().enumerate() {
            outline.highlighted = highlighted_room == Some(room_index);
        }
        let request = FrameRequest {
            scene: Some(&self.combined_scene),
            camera: &self.camera,
            selected_object: None,
            hovered_object: None,
            highlighted_group: highlighted_room,
            placement_preview: None,
            room_outlines: &self.room_outlines,
            settings: &self.view_settings,
            grid_height: self.grid_height,
            snap_grid_spacing: None,
            size_in_pixels: size_in_pixels(ui, rect),
        };
        paint_scene(ui, rect, resources, &mut self.target, &request);
        self.draw_room_labels(ui, &projection, &response, resources.custom_names);

        // Right-click (without dragging, which orbits) names the room under the pointer.
        if response.secondary_clicked() {
            self.renamed_room = self.hovered_room;
        }
        response.context_menu(|ui| {
            let Some(room) = self.renamed_room.and_then(|index| self.rooms.get(index)) else {
                ui.close_menu();
                return;
            };
            ui.strong(&room.block_name);
            show_room_name_field(ui, &room.block_name, resources.custom_names);
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

        let hovered = self.hovered_room.and_then(|index| self.rooms.get(index));
        if let (Some(room), Some(pointer)) = (hovered, response.hover_pos()) {
            let label = format!(
                "{} ({})\nDouble-click to edit this cell",
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
                room.scene.objects.iter().map(move |object| SceneObject {
                    position: object.position + room.origin,
                    group_index: room_index,
                    ..object.clone()
                })
            })
            .collect();
        self.combined_scene = Scene { block_name: self.name.clone(), objects };
        self.room_outlines =
            self.rooms.iter().map(|room| RoomOutline { origin: room.origin, highlighted: false }).collect();
    }
}

/// Builds the scene of one dungeon room: the project room it uses, or the
/// Daggerfall block from the game data. Returns the project room's id too.
fn load_room_scene(
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

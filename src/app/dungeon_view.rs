//! Shows a whole pre-built dungeon: every RDB block (room) placed on its grid.
//!
//! Hovering a room highlights all of its objects and its block outline;
//! double-clicking a room asks the application to open it in a room editor
//! window. Edits made in those windows are copied back with
//! [`DungeonView::update_room_objects`] so the dungeon always shows the latest version.

use eframe::{egui, egui_wgpu};
use glam::Vec3;

use super::room_browser::{category_description, show_room_name_field};
use super::viewport_tools::{
    aspect_ratio, handle_camera_input, paint_scene, size_in_pixels, EditorResources, ViewportProjection,
};
use crate::arena2::{Arena2Library, DungeonDefinition, RDB_BLOCK_SIDE};
use crate::camera::OrbitCamera;
use crate::custom_names::CustomNames;
use crate::picking::{intersect_horizontal_plane, pick_object, PickOptions};
use crate::render::{FrameRequest, RoomOutline, ViewSettings, ViewportTarget};
use crate::scene::{Scene, SceneObject};

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
}

/// State of the dungeon view.
pub struct DungeonView {
    /// Dungeon name as shown in game.
    pub name: String,
    /// Region the dungeon belongs to.
    pub region_name: &'static str,
    rooms: Vec<DungeonRoom>,
    /// Every room's objects moved into world space, with `group_index` set to the room index.
    combined_scene: Scene,
    room_outlines: Vec<RoomOutline>,
    camera: OrbitCamera,
    view_settings: ViewSettings,
    grid_height: f32,
    target: ViewportTarget,
    hovered_room: Option<usize>,
    /// Room whose right-click menu (for naming it) is open.
    renamed_room: Option<usize>,
    last_pointer_position: Option<egui::Pos2>,
}

impl DungeonView {
    /// Loads every block of `definition`.
    ///
    /// Blocks that cannot be loaded are skipped; their errors are returned so they can be reported.
    pub fn load(definition: &DungeonDefinition, library: &mut Arena2Library) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let rooms: Vec<DungeonRoom> = definition
            .blocks
            .iter()
            .filter_map(|placement| match library.rdb_block(&placement.block_name) {
                Ok(block) => {
                    let mut scene = Scene::from_rdb_block(&block);
                    // Only the starting block keeps its way out, as in the game.
                    if !placement.is_starting_block {
                        scene.remove_exit_doors();
                    }
                    Some(DungeonRoom {
                        block_name: placement.block_name.clone(),
                        origin: Vec3::new(
                            placement.grid_x as f32 * RDB_BLOCK_SIDE,
                            0.0,
                            placement.grid_z as f32 * RDB_BLOCK_SIDE,
                        ),
                        is_starting_block: placement.is_starting_block,
                        scene,
                    })
                }
                Err(error) => {
                    problems.push(format!("{}: {error}", placement.block_name));
                    None
                }
            })
            .collect();

        let mut view = Self {
            name: definition.name.clone(),
            region_name: definition.region_name,
            rooms,
            combined_scene: Scene::default(),
            room_outlines: Vec::new(),
            camera: OrbitCamera::default(),
            view_settings: ViewSettings::default(),
            grid_height: 0.0,
            target: ViewportTarget::default(),
            hovered_room: None,
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
            None => "Hover a room to highlight it, double-click to open it in a room editor".to_string(),
        };
        format!("{} ({}): {} rooms. {hint}", self.name, self.region_name, self.rooms.len())
    }

    /// Replaces the objects of room `room_index` if they differ, for example after it was edited.
    pub fn update_room_objects(&mut self, room_index: usize, objects: &[SceneObject]) {
        let Some(room) = self.rooms.get_mut(room_index) else { return };
        if room.scene.objects != objects {
            room.scene.objects = objects.to_vec();
            self.rebuild_combined_scene();
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

        for (room_index, outline) in self.room_outlines.iter_mut().enumerate() {
            outline.highlighted = self.hovered_room == Some(room_index);
        }
        let request = FrameRequest {
            scene: Some(&self.combined_scene),
            camera: &self.camera,
            selected_object: None,
            hovered_object: None,
            highlighted_group: self.hovered_room,
            placement_preview: None,
            room_outlines: &self.room_outlines,
            settings: &self.view_settings,
            grid_height: self.grid_height,
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
                "{} ({})\nDouble-click to edit this room",
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

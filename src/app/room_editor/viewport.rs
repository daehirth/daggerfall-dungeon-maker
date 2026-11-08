//! The room editor's 3D viewport: object picking, the move gizmo and placing
//! assets from the asset palette. Camera controls are shared with the dungeon view.

use eframe::egui;
use glam::{Vec2, Vec3};

use super::history::SceneEdit;
use super::panels::{object_display_name, percent};
use super::RoomEditor;
use crate::app::viewport_tools::{
    aspect_ratio, handle_camera_input, paint_scene, size_in_pixels, EditorResources, ViewportProjection,
};
use crate::arena2::Arena2Library;
use crate::picking::{intersect_horizontal_plane, pick_object, PickOptions};
use crate::render::{FrameRequest, RoomOutline};
use crate::scene::{SceneObject, SceneObjectKind};

/// Length of the gizmo arrows as a fraction of the camera distance to the object.
const GIZMO_SIZE_FACTOR: f32 = 0.15;
/// How close (in points) the pointer must be to an arrow to grab it.
const GIZMO_GRAB_DISTANCE: f32 = 8.0;
/// The single block outline drawn around a room, which always sits at the origin.
const ROOM_OUTLINE: [RoomOutline; 1] = [RoomOutline { origin: Vec3::ZERO, highlighted: false }];

/// A world axis the gizmo can move an object along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GizmoAxis {
    X,
    Y,
    Z,
}

impl GizmoAxis {
    const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    fn direction(self) -> Vec3 {
        match self {
            Self::X => Vec3::X,
            Self::Y => Vec3::Y,
            Self::Z => Vec3::Z,
        }
    }

    /// Index of the axis's component in a `Vec3`.
    fn component(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }

    fn color(self, is_active: bool) -> egui::Color32 {
        match (self, is_active) {
            (_, true) => egui::Color32::from_rgb(255, 220, 60),
            (Self::X, false) => egui::Color32::from_rgb(220, 70, 70),
            (Self::Y, false) => egui::Color32::from_rgb(100, 200, 80),
            (Self::Z, false) => egui::Color32::from_rgb(80, 130, 235),
        }
    }
}

/// One gizmo arrow on screen.
#[derive(Debug, Clone, Copy)]
struct GizmoHandle {
    axis: GizmoAxis,
    screen_start: egui::Pos2,
    screen_end: egui::Pos2,
    /// Length of the arrow in world units.
    world_length: f32,
}

/// An in-progress gizmo drag.
#[derive(Debug, Clone, Copy)]
struct GizmoDrag {
    axis: GizmoAxis,
    object_index: usize,
    start_position: Vec3,
    /// Total distance dragged along the axis, before snapping.
    distance_along_axis: f32,
}

/// Viewport interaction state that persists between frames.
#[derive(Debug, Default)]
pub struct ViewportState {
    /// Object under the mouse pointer.
    pub hovered_object: Option<usize>,
    /// Set when a click in the viewport changes the selection, so the outliner can scroll to it.
    pub selection_changed_in_viewport: bool,
    gizmo_drag: Option<GizmoDrag>,
    hovered_axis: Option<GizmoAxis>,
    last_pointer_position: Option<egui::Pos2>,
    /// Ghost of the armed asset at the position it would be placed.
    placement_preview: Option<SceneObject>,
}

impl RoomEditor {
    /// Draws the viewport and handles every interaction inside it.
    pub(super) fn show_viewport(&mut self, ui: &mut egui::Ui, resources: &mut EditorResources<'_>) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        handle_camera_input(&mut self.camera, ui, &response);

        let projection = ViewportProjection::new(&self.camera, rect);
        let is_placing = self.palette.armed.is_some();
        let gizmo_handles = if is_placing { Vec::new() } else { self.gizmo_handles(&projection) };
        if is_placing {
            self.handle_placement(ui, &response, &projection, resources);
        } else {
            self.viewport.placement_preview = None;
            self.handle_gizmo_and_picking(ui, &response, &projection, &gizmo_handles, resources);
        }

        // Render after input so the image reflects this frame's changes.
        let request = FrameRequest {
            scene: Some(&self.scene),
            camera: &self.camera,
            selected_object: self.selected_object,
            hovered_object: self.viewport.hovered_object,
            highlighted_group: None,
            placement_preview: self.viewport.placement_preview.as_ref(),
            room_outlines: &ROOM_OUTLINE,
            settings: &self.view_settings,
            grid_height: self.grid_height,
            snap_grid_spacing: self.snap.enabled.then_some(self.snap.step),
            size_in_pixels: size_in_pixels(ui, rect),
        };
        paint_scene(ui, rect, resources, &mut self.target, &request);

        let painter = ui.painter_at(rect);
        self.draw_gizmo(&painter, &gizmo_handles);
        self.draw_hover_label(&painter, &response, resources);
    }

    /// Name and type of object `index` for labels and messages, for example "Door 55000 (Action model)".
    fn describe_object(&self, index: usize, resources: &EditorResources<'_>) -> Option<String> {
        let object = self.scene.objects.get(index)?;
        let name = object_display_name(object, resources.custom_names, resources.catalog);
        Some(format!("{name} ({})", object.kind_label()))
    }

    /// Returns the on-screen arrows of the move gizmo for the selected object.
    fn gizmo_handles(&self, projection: &ViewportProjection) -> Vec<GizmoHandle> {
        let Some(object) = self
            .selected_object
            .and_then(|index| self.scene.objects.get(index))
            .filter(|object| object.visible)
        else {
            return Vec::new();
        };

        let origin = object.position;
        let world_length = (self.camera.eye_position() - origin).length() * GIZMO_SIZE_FACTOR;
        let Some(screen_start) = projection.world_to_screen(origin) else {
            return Vec::new();
        };
        GizmoAxis::ALL
            .into_iter()
            .filter_map(|axis| {
                let screen_end = projection.world_to_screen(origin + axis.direction() * world_length)?;
                Some(GizmoHandle { axis, screen_start, screen_end, world_length })
            })
            .collect()
    }

    /// Starts, continues and ends gizmo drags, and selects objects on click.
    fn handle_gizmo_and_picking(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        projection: &ViewportProjection,
        gizmo_handles: &[GizmoHandle],
        resources: &mut EditorResources<'_>,
    ) {
        let pointer_position = response.hover_pos().or(response.interact_pointer_pos());

        // Which arrow (if any) is under the pointer.
        if self.viewport.gizmo_drag.is_none() {
            self.viewport.hovered_axis = pointer_position.and_then(|pointer| {
                gizmo_handles
                    .iter()
                    .map(|handle| {
                        (handle.axis, distance_to_segment(pointer, handle.screen_start, handle.screen_end))
                    })
                    .filter(|(_, distance)| *distance <= GIZMO_GRAB_DISTANCE)
                    .min_by(|first, second| first.1.total_cmp(&second.1))
                    .map(|(axis, _)| axis)
            });
        }

        // Begin a drag on an arrow.
        if response.drag_started_by(egui::PointerButton::Primary) {
            let grabbed = self.viewport.hovered_axis.zip(self.selected_object);
            if let Some((axis, object_index)) = grabbed.filter(|_| self.can_edit()) {
                if let Some(object) = self.scene.objects.get(object_index) {
                    self.history.record(SceneEdit::Replace { index: object_index, object: object.clone() });
                    self.viewport.gizmo_drag = Some(GizmoDrag {
                        axis,
                        object_index,
                        start_position: object.position,
                        distance_along_axis: 0.0,
                    });
                }
            }
        }

        // Continue a drag: project the mouse movement onto the arrow's screen direction.
        if let Some(mut drag) = self.viewport.gizmo_drag {
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(handle) = gizmo_handles.iter().find(|handle| handle.axis == drag.axis) {
                    let screen_axis = handle.screen_end - handle.screen_start;
                    let screen_axis_length_squared = screen_axis.length_sq();
                    if screen_axis_length_squared > 1.0 {
                        let along = response.drag_delta().dot(screen_axis) / screen_axis_length_squared;
                        drag.distance_along_axis += along * handle.world_length;
                    }
                }
                // With grid lock on (or Ctrl held while it is off) the position along the
                // dragged axis lands on the snap grid itself, not just whole steps from the start.
                let invert_snap = ui.input(|input| input.modifiers.command);
                let snapping = self.snap.enabled != invert_snap;
                let mut position = drag.start_position + drag.axis.direction() * drag.distance_along_axis;
                if snapping && self.snap.step > 0.0 {
                    let component = &mut position[drag.axis.component()];
                    *component = (*component / self.snap.step).round() * self.snap.step;
                }
                if let Some(object) = self.scene.objects.get_mut(drag.object_index) {
                    object.position = position;
                }
                self.viewport.gizmo_drag = Some(drag);
            }
            if response.drag_stopped() {
                self.viewport.gizmo_drag = None;
            }
            return;
        }

        // Hover highlighting and click selection, ignored while orbiting or panning.
        if response.dragged() {
            return;
        }
        let pointer_moved = pointer_position != self.viewport.last_pointer_position;
        self.viewport.last_pointer_position = pointer_position;
        let clicked = response.clicked_by(egui::PointerButton::Primary);
        if !(pointer_moved || clicked) {
            return;
        }

        let picked = match pointer_position {
            Some(pointer) if self.viewport.hovered_axis.is_none() => self.pick_visible_object(
                projection.screen_to_ndc(pointer),
                aspect_ratio(projection.rect),
                resources.library,
            ),
            _ => None,
        };
        self.viewport.hovered_object = picked;

        if clicked && self.viewport.hovered_axis.is_none() {
            self.selected_object = picked;
            self.viewport.selection_changed_in_viewport = picked.is_some();
            if let Some(description) = picked.and_then(|index| self.describe_object(index, resources)) {
                self.status_message = format!("Selected {description}");
            }
        }
    }

    /// Returns the closest object under `ndc` that the current view settings show.
    fn pick_visible_object(
        &self,
        ndc: Vec2,
        aspect_ratio: f32,
        library: &mut Arena2Library,
    ) -> Option<usize> {
        let ray = self.camera.ray_through(ndc, aspect_ratio);
        let settings = &self.view_settings;
        let options = PickOptions {
            ignore_back_faces: settings.cull_back_faces,
            include_models: settings.show_models,
            include_flats: settings.show_flats || settings.show_markers,
            include_lights: settings.show_lights,
            camera_right: self.camera.right(),
        };
        pick_object(&self.scene, library, &ray, &options).map(|hit| hit.object_index).filter(|&index| {
            let object = &self.scene.objects[index];
            match object.kind {
                SceneObjectKind::Flat { .. } if object.is_marker() => settings.show_markers,
                SceneObjectKind::Flat { .. } => settings.show_flats,
                _ => true,
            }
        })
    }

    /// Moves the ghost of the armed asset under the mouse and places it on click.
    fn handle_placement(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        projection: &ViewportProjection,
        resources: &mut EditorResources<'_>,
    ) {
        self.viewport.hovered_object = None;
        self.viewport.hovered_axis = None;
        let Some(armed) = self.palette.armed else { return };
        let library = &mut *resources.library;

        // Ignore the pointer while orbiting or panning so the ghost does not jump around.
        if !response.dragged() {
            let surface_point = response.hover_pos().and_then(|pointer| {
                self.placement_surface_point(
                    projection.screen_to_ndc(pointer),
                    aspect_ratio(projection.rect),
                    library,
                )
            });
            self.viewport.placement_preview =
                surface_point.map(|point| armed.asset.create_object(library, point, armed.yaw_degrees));
        }

        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let Some(object) = self.viewport.placement_preview.clone() else { return };
        if !self.can_edit() {
            self.palette.armed = None;
            self.viewport.placement_preview = None;
            return;
        }
        let new_index = self.scene.objects.len();
        self.scene.objects.push(object);
        if let Some(description) = self.describe_object(new_index, resources) {
            self.status_message = format!("Placed {description}");
        }
        self.history.record(SceneEdit::Remove { index: new_index });
        self.selected_object = Some(new_index);
        self.viewport.selection_changed_in_viewport = true;

        // Shift+click keeps the asset armed for placing several copies.
        if !ui.input(|input| input.modifiers.shift) {
            self.palette.armed = None;
            self.viewport.placement_preview = None;
        }
    }

    /// Finds where a ray through `ndc` meets the room (or the grid plane if it misses every model).
    fn placement_surface_point(
        &self,
        ndc: Vec2,
        aspect_ratio: f32,
        library: &mut Arena2Library,
    ) -> Option<Vec3> {
        let ray = self.camera.ray_through(ndc, aspect_ratio);
        let options = PickOptions {
            ignore_back_faces: self.view_settings.cull_back_faces,
            include_models: true,
            include_flats: false,
            include_lights: false,
            camera_right: self.camera.right(),
        };
        let distance = pick_object(&self.scene, library, &ray, &options)
            .map(|hit| hit.distance)
            .or_else(|| intersect_horizontal_plane(&ray, self.grid_height))?;

        let mut point = ray.point_at(distance);
        if self.snap.enabled && self.snap.step > 0.0 {
            let step = self.snap.step;
            point.x = (point.x / step).round() * step;
            point.z = (point.z / step).round() * step;
        }
        Some(point)
    }

    /// Draws the move gizmo arrows over the viewport image.
    fn draw_gizmo(&self, painter: &egui::Painter, gizmo_handles: &[GizmoHandle]) {
        let active_axis = self.viewport.gizmo_drag.map(|drag| drag.axis).or(self.viewport.hovered_axis);
        for handle in gizmo_handles {
            let color = handle.axis.color(active_axis == Some(handle.axis));
            painter.line_segment([handle.screen_start, handle.screen_end], egui::Stroke::new(3.0_f32, color));
            painter.circle_filled(handle.screen_end, 5.0, color);
        }
        if let Some(handle) = gizmo_handles.first() {
            painter.circle_filled(handle.screen_start, 3.5, egui::Color32::WHITE);
        }
    }

    /// Draws the name of the object under the mouse next to the pointer.
    fn draw_hover_label(
        &self,
        painter: &egui::Painter,
        response: &egui::Response,
        resources: &EditorResources<'_>,
    ) {
        let (Some(index), Some(pointer)) = (self.viewport.hovered_object, response.hover_pos()) else {
            return;
        };
        if let Some(mut description) = self.describe_object(index, resources) {
            if let Some(fixed_enemy) = self.scene.objects[index].fixed_enemy() {
                description = format!("{description}\nSpawns: {}", fixed_enemy.description());
            }
            // A random enemy marker also shows its likeliest enemies in this dungeon.
            let possible_enemies =
                self.possible_enemies(index).filter(|_| self.scene.objects[index].is_random_enemy_marker());
            if let Some(possible_enemies) = possible_enemies {
                let likeliest: Vec<String> = possible_enemies
                    .iter()
                    .take(3)
                    .map(|chance| format!("{} {}", chance.enemy.name(), percent(chance.chance)))
                    .collect();
                description =
                    format!("{description}\nLevel {}: {}", self.enemy_preview_level, likeliest.join(", "));
            }
            painter.text(
                pointer + egui::vec2(14.0, 14.0),
                egui::Align2::LEFT_TOP,
                description,
                egui::FontId::proportional(13.0),
                egui::Color32::WHITE,
            );
        }
    }
}

/// Shortest distance from `point` to the segment between `start` and `end`.
fn distance_to_segment(point: egui::Pos2, start: egui::Pos2, end: egui::Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let fraction = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + segment * fraction)
}

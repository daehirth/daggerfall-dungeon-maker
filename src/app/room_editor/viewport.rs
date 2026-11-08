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
use crate::arena2::RDB_BLOCK_SIDE;
use crate::picking::{intersect_horizontal_plane, pick_object, PickOptions};
use crate::render::{FrameRequest, RoomOutline};
use crate::scene::OUTSIDE_CELL_TOLERANCE;
use crate::scene::{ObjectId, SceneObject, SceneObjectKind};

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

/// An in-progress gizmo drag, moving every selected object together.
#[derive(Debug, Clone)]
struct GizmoDrag {
    axis: GizmoAxis,
    /// Each moved object and where it started. The first one is the last
    /// selected, which lands on the snap grid; the others keep their offsets to it.
    objects: Vec<(usize, Vec3)>,
    /// Total distance dragged along the axis, before snapping.
    distance_along_axis: f32,
}

/// Smallest drag, in points, that counts as a selection box rather than a click.
const SELECTION_BOX_MIN_SIZE: f32 = 4.0;
const SELECTION_BOX_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(40, 70, 110, 40);
const SELECTION_BOX_STROKE: egui::Color32 = egui::Color32::from_rgb(110, 170, 255);

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
    /// Object whose action is being tied to a target chosen with the mouse.
    pub tying_from: Option<ObjectId>,
    /// Object whose right-click menu is open.
    context_object: Option<usize>,
    /// Screen corner where a selection box drag started.
    selection_box_start: Option<egui::Pos2>,
    /// Objects inside the selection box being dragged, highlighted until it is released.
    selection_box_objects: Vec<usize>,
    /// Faded surrounding cell under the pointer (index into the context's cells).
    hovered_context_cell: Option<usize>,
}

/// Colour of the line drawn from an object to the pointer while tying its action.
const TYING_LINE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 60, 60);

impl RoomEditor {
    /// Draws the viewport and handles every interaction inside it.
    pub(super) fn show_viewport(&mut self, ui: &mut egui::Ui, resources: &mut EditorResources<'_>) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        handle_camera_input(&mut self.camera, ui, &response);

        let projection = ViewportProjection::new(&self.camera, rect);
        let is_placing = self.palette.armed.is_some();
        let tying_from = self.viewport.tying_from;
        let gizmo_handles =
            if is_placing || tying_from.is_some() { Vec::new() } else { self.gizmo_handles(&projection) };
        if let Some(source) = tying_from {
            self.viewport.placement_preview = None;
            self.handle_tying(&response, &projection, source, resources);
        } else if is_placing {
            self.handle_placement(ui, &response, &projection, resources);
        } else {
            self.viewport.placement_preview = None;
            self.handle_gizmo_and_picking(ui, &response, &projection, &gizmo_handles, resources);
            self.handle_object_menu(&response, &projection, resources);
        }

        // Render after input so the image reflects this frame's changes. A playing
        // action preview is drawn instead of the cell, which it never changes.
        let preview = self.preview_scene(ui.input(|input| input.time));
        if preview.is_some() {
            ui.ctx().request_repaint();
        }
        let selected_objects = self.selected_indices();
        let water_planes: Vec<Vec3> =
            self.scene.water_level().map(|height| Vec3::new(0.0, height, 0.0)).into_iter().collect();
        let request = FrameRequest {
            scene: Some(preview.as_ref().unwrap_or(&self.scene)),
            camera: &self.camera,
            selected_object: self.selected_object,
            hovered_object: self.viewport.hovered_object,
            selected_objects: &selected_objects,
            hovered_objects: &self.viewport.selection_box_objects,
            highlighted_group: None,
            placement_preview: self.viewport.placement_preview.as_ref(),
            context_scene: self.dungeon_context.as_ref().map(|context| &context.scene),
            water_planes: &water_planes,
            room_outlines: self
                .dungeon_context
                .as_ref()
                .map_or(&ROOM_OUTLINE[..], |context| &context.outlines),
            settings: &self.view_settings,
            grid_height: self.grid_height,
            snap_grid_spacing: self.snap.enabled.then_some(self.snap.step),
            size_in_pixels: size_in_pixels(ui, rect),
        };
        paint_scene(ui, rect, resources, &mut self.target, &request);

        let painter = ui.painter_at(rect);
        if let (Some(start), Some(pointer)) =
            (self.viewport.selection_box_start, response.interact_pointer_pos())
        {
            let selection_box = egui::Rect::from_two_pos(start, pointer);
            painter.rect_filled(selection_box, 0.0, SELECTION_BOX_FILL);
            painter.rect_stroke(selection_box, 0.0, egui::Stroke::new(1.0_f32, SELECTION_BOX_STROKE));
        }
        self.draw_gizmo(&painter, &gizmo_handles);
        if let Some(source) = self.viewport.tying_from {
            self.draw_tying_line(&painter, &response, &projection, source, resources);
        }
        self.draw_hover_label(&painter, &response, resources);
    }

    /// Opens an object's menu on right-click (without dragging, which orbits) and carries out the choice.
    fn handle_object_menu(
        &mut self,
        response: &egui::Response,
        projection: &ViewportProjection,
        resources: &mut EditorResources<'_>,
    ) {
        if response.secondary_clicked() && self.has_multiple_selection() {
            self.status_message = "Several objects are selected; they can only be moved".into();
            self.viewport.context_object = None;
        } else if response.secondary_clicked() {
            let picked = response.interact_pointer_pos().and_then(|pointer| {
                self.pick_visible_object(
                    projection.screen_to_ndc(pointer),
                    aspect_ratio(projection.rect),
                    resources.library,
                )
            });
            self.viewport.context_object = picked;
            if let Some(index) = picked {
                self.select_objects(vec![index]);
                self.viewport.selection_changed_in_viewport = true;
            }
        }
        let info = self.viewport.context_object.and_then(|index| self.object_menu_info(index, resources));
        let mut choice = None;
        response.context_menu(|ui| match &info {
            Some(info) => choice = super::object_menu(ui, info),
            None => ui.close_menu(),
        });
        if let (Some(choice), Some(index)) = (choice, self.viewport.context_object) {
            self.apply_object_menu(choice, index, resources);
        }
    }

    /// While tying an action: highlights the object under the pointer and ties
    /// the action to it on click. Right-clicking or clicking empty space cancels.
    fn handle_tying(
        &mut self,
        response: &egui::Response,
        projection: &ViewportProjection,
        source: ObjectId,
        resources: &mut EditorResources<'_>,
    ) {
        self.viewport.hovered_axis = None;
        let pointer = response.hover_pos().or(response.interact_pointer_pos());
        if !response.dragged() && pointer != self.viewport.last_pointer_position {
            self.viewport.last_pointer_position = pointer;
            self.viewport.hovered_object = pointer
                .and_then(|pointer| {
                    self.pick_visible_object(
                        projection.screen_to_ndc(pointer),
                        aspect_ratio(projection.rect),
                        resources.library,
                    )
                })
                .filter(|&index| self.scene.objects[index].id != source);
        }
        if response.secondary_clicked() {
            self.cancel_tying();
        } else if response.clicked_by(egui::PointerButton::Primary) {
            match self.viewport.hovered_object {
                Some(target) => self.tie_action(source, target, resources),
                None => self.cancel_tying(),
            }
        }
    }

    /// Draws a line from the object being tied to the pointer.
    fn draw_tying_line(
        &self,
        painter: &egui::Painter,
        response: &egui::Response,
        projection: &ViewportProjection,
        source: ObjectId,
        resources: &mut EditorResources<'_>,
    ) {
        let (Some(pointer), Some(object)) = (response.hover_pos(), self.scene.object_by_id(source)) else {
            return;
        };
        let center = match object.kind {
            SceneObjectKind::Model { model_id, .. } => {
                resources.library.model(model_id).map_or(object.position, |mesh| {
                    object.model_matrix().transform_point3((mesh.bounds_min + mesh.bounds_max) * 0.5)
                })
            }
            _ => object.position,
        };
        let Some(start) = projection.world_to_screen(center) else { return };
        let stroke = egui::Stroke::new(2.0_f32, TYING_LINE_COLOR);
        painter.line_segment([start, pointer], stroke);
        painter.circle_filled(start, 4.0, TYING_LINE_COLOR);
        painter.circle_stroke(pointer, 6.0, stroke);
    }

    /// Like [`Self::describe_object`], for other parts of the editor.
    pub(super) fn describe_object_for_status(
        &self,
        index: usize,
        resources: &EditorResources<'_>,
    ) -> Option<String> {
        self.describe_object(index, resources)
    }

    /// Name and type of object `index` for labels and messages, for example "Door 55000 (Action model)".
    fn describe_object(&self, index: usize, resources: &EditorResources<'_>) -> Option<String> {
        let object = self.scene.objects.get(index)?;
        let name = object_display_name(object, resources.custom_names, resources.catalog);
        Some(format!("{name} ({})", object.kind_label()))
    }

    /// Returns the on-screen arrows of the move gizmo, at the middle of the
    /// visible selected objects.
    fn gizmo_handles(&self, projection: &ViewportProjection) -> Vec<GizmoHandle> {
        let positions: Vec<Vec3> = self
            .selected_indices()
            .into_iter()
            .filter_map(|index| self.scene.objects.get(index))
            .filter(|object| object.visible)
            .map(|object| object.position)
            .collect();
        if positions.is_empty() {
            return Vec::new();
        }

        let origin = positions.iter().copied().sum::<Vec3>() / positions.len() as f32;
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
        // Begin a drag on an arrow, or a selection box anywhere else.
        if response.drag_started_by(egui::PointerButton::Primary) {
            match self.viewport.hovered_axis {
                Some(axis) => {
                    if self.can_edit() {
                        // The last selected object leads, so it is the one that lands on the grid.
                        let mut indices = self.selected_indices();
                        indices.reverse();
                        let objects: Vec<(usize, Vec3)> = indices
                            .iter()
                            .filter_map(|&index| {
                                self.scene.objects.get(index).map(|object| (index, object.position))
                            })
                            .collect();
                        let edits = objects
                            .iter()
                            .map(|&(index, _)| SceneEdit::Replace {
                                index,
                                object: self.scene.objects[index].clone(),
                            })
                            .collect();
                        self.history.record(SceneEdit::Group(edits));
                        self.viewport.gizmo_drag =
                            Some(GizmoDrag { axis, objects, distance_along_axis: 0.0 });
                    }
                }
                None => {
                    self.viewport.selection_box_start = ui.input(|input| input.pointer.press_origin());
                }
            }
        }

        if self.viewport.selection_box_start.is_some() {
            self.handle_selection_box(ui, response, projection, resources);
            return;
        }

        // Continue a drag: project the mouse movement onto the arrow's screen direction.
        if let Some(mut drag) = self.viewport.gizmo_drag.take() {
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
                let Some(&(_, lead_start)) = drag.objects.first() else { return };
                let mut lead_position = lead_start + drag.axis.direction() * drag.distance_along_axis;
                if snapping && self.snap.step > 0.0 {
                    let component = &mut lead_position[drag.axis.component()];
                    *component = (*component / self.snap.step).round() * self.snap.step;
                }
                let offset = lead_position - lead_start;
                for &(index, start) in &drag.objects {
                    if let Some(object) = self.scene.objects.get_mut(index) {
                        object.position = start + offset;
                    }
                }
            }
            if response.drag_stopped() {
                let outside = drag
                    .objects
                    .iter()
                    .map(|&(index, _)| self.distance_outside_cell(index, resources.library))
                    .fold(0.0, f32::max);
                if outside > OUTSIDE_CELL_TOLERANCE {
                    self.status_message = super::outside_cell_warning(outside);
                }
            } else {
                self.viewport.gizmo_drag = Some(drag);
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

        let mut picked = match pointer_position {
            Some(pointer) if self.viewport.hovered_axis.is_none() => self.pick_visible_object(
                projection.screen_to_ndc(pointer),
                aspect_ratio(projection.rect),
                resources.library,
            ),
            _ => None,
        };
        // A faded surrounding cell in front of the cell's own objects takes the pointer.
        self.viewport.hovered_context_cell = match pointer_position {
            Some(pointer) if self.viewport.hovered_axis.is_none() => self.context_cell_under(
                projection.screen_to_ndc(pointer),
                aspect_ratio(projection.rect),
                resources.library,
            ),
            _ => None,
        };
        if self.viewport.hovered_context_cell.is_some() {
            picked = None;
        }
        self.viewport.hovered_object = picked;
        if let Some(context) = &mut self.dungeon_context {
            for (index, outline) in context.outlines.iter_mut().enumerate() {
                outline.highlighted = index > 0 && self.viewport.hovered_context_cell == Some(index - 1);
            }
        }
        // Double-clicking a faded cell opens it, keeping this dungeon around it.
        if response.double_clicked_by(egui::PointerButton::Primary) {
            let target = self.viewport.hovered_context_cell.and_then(|cell| {
                let context = self.dungeon_context.as_ref()?;
                Some((context.dungeon.clone(), context.cells.get(cell)?.layout_index))
            });
            if let Some(target) = target {
                self.cell_open_request = Some(target);
                return;
            }
        }

        if clicked && self.viewport.hovered_axis.is_none() {
            let shift_held = ui.input(|input| input.modifiers.shift);
            match picked {
                // Shift+click adds the object to the selection, or takes it out.
                Some(index) if shift_held => {
                    self.toggle_selected(index);
                    self.viewport.selection_changed_in_viewport = true;
                    self.describe_selection(resources);
                }
                Some(_) | None if shift_held => {}
                _ => {
                    self.select_objects(picked.into_iter().collect());
                    self.viewport.selection_changed_in_viewport = picked.is_some();
                    if let Some(description) = picked.and_then(|index| self.describe_object(index, resources))
                    {
                        self.status_message = format!("Selected {description}");
                    }
                }
            }
        }
    }

    /// Puts what is selected in the status bar.
    fn describe_selection(&mut self, resources: &EditorResources<'_>) {
        let selected = self.selected_indices();
        self.status_message = match selected.as_slice() {
            [] => "Nothing selected".into(),
            [index] => match self.describe_object(*index, resources) {
                Some(description) => format!("Selected {description}"),
                None => String::new(),
            },
            several => format!("Selected {} objects; drag the arrows to move them together", several.len()),
        };
    }

    /// While a selection box is dragged: highlights the objects inside it, and on
    /// release selects them (Shift adds them to the selection instead).
    fn handle_selection_box(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        projection: &ViewportProjection,
        resources: &mut EditorResources<'_>,
    ) {
        let (Some(start), Some(pointer)) =
            (self.viewport.selection_box_start, response.interact_pointer_pos())
        else {
            self.viewport.selection_box_start = None;
            self.viewport.selection_box_objects.clear();
            return;
        };
        let selection_box = egui::Rect::from_two_pos(start, pointer);
        self.viewport.hovered_object = None;
        self.viewport.selection_box_objects =
            if selection_box.width().max(selection_box.height()) >= SELECTION_BOX_MIN_SIZE {
                self.objects_in_screen_rect(selection_box, projection, resources.library)
            } else {
                Vec::new()
            };

        if response.drag_stopped() || !response.dragged() {
            let inside = std::mem::take(&mut self.viewport.selection_box_objects);
            self.viewport.selection_box_start = None;
            if ui.input(|input| input.modifiers.shift) {
                let mut indices = self.selected_indices();
                indices.extend(inside);
                self.select_objects(indices);
            } else {
                self.select_objects(inside);
            }
            self.viewport.selection_changed_in_viewport = self.selected_object.is_some();
            self.describe_selection(resources);
        }
    }

    /// The objects the view shows whose centre is inside `rect` on screen.
    fn objects_in_screen_rect(
        &self,
        rect: egui::Rect,
        projection: &ViewportProjection,
        library: &mut Arena2Library,
    ) -> Vec<usize> {
        let settings = &self.view_settings;
        self.scene
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| {
                object.visible
                    && match object.kind {
                        SceneObjectKind::Model { .. } => settings.show_models,
                        SceneObjectKind::Flat { .. } if object.is_marker() => settings.show_markers,
                        SceneObjectKind::Flat { .. } => settings.show_flats,
                        SceneObjectKind::Light { .. } => settings.show_lights,
                    }
            })
            .filter_map(|(index, object)| {
                let center = match object.kind {
                    SceneObjectKind::Model { model_id, .. } => library
                        .model(model_id)
                        .map(|mesh| {
                            object.model_matrix().transform_point3((mesh.bounds_min + mesh.bounds_max) * 0.5)
                        })
                        .unwrap_or(object.position),
                    _ => object.position,
                };
                let screen = projection.world_to_screen(center)?;
                rect.contains(screen).then_some(index)
            })
            .collect()
    }

    /// The faded surrounding cell under `ndc` (index into the context's cells):
    /// the one whose objects the ray hits before any of this cell's, or else the
    /// one whose footprint the ray meets at grid height.
    fn context_cell_under(&self, ndc: Vec2, aspect_ratio: f32, library: &mut Arena2Library) -> Option<usize> {
        let context = self.dungeon_context.as_ref()?;
        let ray = self.camera.ray_through(ndc, aspect_ratio);
        let settings = &self.view_settings;
        let options = PickOptions {
            ignore_back_faces: settings.cull_back_faces,
            include_models: settings.show_models,
            include_flats: settings.show_flats,
            include_lights: false,
            camera_right: self.camera.right(),
        };
        let own_distance = pick_object(&self.scene, library, &ray, &options).map(|hit| hit.distance);
        if let Some(hit) = pick_object(&context.scene, library, &ray, &options) {
            if own_distance.is_none_or(|own| hit.distance < own) {
                return Some(context.scene.objects[hit.object_index].group_index);
            }
            return None;
        }
        if own_distance.is_some() {
            return None;
        }
        let distance = intersect_horizontal_plane(&ray, self.grid_height)?;
        let point = ray.origin + ray.direction * distance;
        context.outlines.iter().skip(1).position(|outline| {
            let local = point - outline.origin;
            (0.0..RDB_BLOCK_SIDE).contains(&local.x) && (0.0..RDB_BLOCK_SIDE).contains(&local.z)
        })
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
        let mut object = object;
        object.id = self.scene.new_object_id();
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
        let active_axis =
            self.viewport.gizmo_drag.as_ref().map(|drag| drag.axis).or(self.viewport.hovered_axis);
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
        if let (Some(cell), Some(pointer)) = (self.viewport.hovered_context_cell, response.hover_pos()) {
            if let Some(cell) = self.dungeon_context.as_ref().and_then(|context| context.cells.get(cell)) {
                painter.text(
                    pointer + egui::vec2(14.0, 14.0),
                    egui::Align2::LEFT_TOP,
                    format!("{}\nDouble-click to open this cell", cell.label),
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );
            }
            return;
        }
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

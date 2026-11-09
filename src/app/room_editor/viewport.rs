//! The room editor's 3D viewport: object picking, the move and rotate gizmo
//! and placing assets from the asset palette. Camera controls are shared with
//! the dungeon view.
//!
//! A model being placed snaps to the open faces of the models already placed
//! (see [`crate::connectors`]): a corridor's end onto another's, a door into a
//! doorway. The best size match wins, then the nearest; faces something is
//! already joined to are skipped, and holding Alt places without snapping.
//!
//! A selected object whose action moves it shows the path it moves along as
//! a purple line, and so does every object its action sets moving, down the
//! chain to its end. The lines are worked out afresh each frame, so they
//! follow any change.

use eframe::egui;
use glam::{Quat, Vec2, Vec3};

use super::history::SceneEdit;
use super::panels::{object_display_name, percent};
use super::RoomEditor;
use crate::app::viewport_tools::{
    aspect_ratio, draw_unclosed_openings, find_unclosed_openings, handle_camera_input, paint_scene,
    size_in_pixels, EditorResources, ViewportProjection, UNCLOSED_CHECK_INTERVAL,
};
use crate::arena2::Arena2Library;
use crate::arena2::RDB_BLOCK_SIDE;
use crate::connectors::{fit, join_offset, OpenFace, Snap};
use crate::picking::{intersect_horizontal_plane, pick_object, PickOptions};
use crate::render::{FrameRequest, RoomOutline};
use crate::scene::OUTSIDE_CELL_TOLERANCE;
use crate::scene::{ObjectId, SceneObject, SceneObjectKind};

/// Length of the gizmo arrows as a fraction of the camera distance to the object.
const GIZMO_SIZE_FACTOR: f32 = 0.15;
/// How close (in points) the pointer must be to an arrow or ring to grab it.
const GIZMO_GRAB_DISTANCE: f32 = 8.0;
/// Radius of the rotation rings as a fraction of the arrows' length.
const GIZMO_RING_FACTOR: f32 = 0.8;
/// Points drawn around each rotation ring.
const GIZMO_RING_SEGMENTS: usize = 48;
/// Step, in degrees, rotation snaps to with grid lock on.
const ROTATION_SNAP_DEGREES: f32 = 15.0;
/// How far (world units, beyond the model's own size) a model being placed
/// may jump to snap onto an open face.
const SNAP_REACH: f32 = 3.0;
/// Faces whose sizes differ by more than this (as a fraction) do not snap.
const MAX_SNAP_MISMATCH: f32 = 0.6;
/// Colour of the face a model being placed snaps to.
const SNAP_FACE_COLOR: egui::Color32 = egui::Color32::from_rgb(90, 255, 140);
/// Colour of the path a selected object's action moves it along.
const TRAJECTORY_COLOR: egui::Color32 = egui::Color32::from_rgb(190, 90, 255);
/// Points along a trajectory line.
const TRAJECTORY_STEPS: usize = 32;
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

/// A part of the gizmo: an arrow moving along an axis, or a ring turning around one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GizmoPart {
    Move(GizmoAxis),
    Rotate(GizmoAxis),
}

impl GizmoPart {
    fn axis(self) -> GizmoAxis {
        match self {
            Self::Move(axis) | Self::Rotate(axis) => axis,
        }
    }
}

/// One gizmo arrow or ring on screen.
#[derive(Debug, Clone)]
struct GizmoHandle {
    part: GizmoPart,
    /// An arrow's start and end, or the points around a ring.
    points: Vec<egui::Pos2>,
    /// Length of the arrows in world units.
    world_length: f32,
}

impl GizmoHandle {
    /// Distance from `point` to the arrow or ring, in points.
    fn distance(&self, point: egui::Pos2) -> f32 {
        let closing =
            matches!(self.part, GizmoPart::Rotate(_)).then(|| (self.points.last(), self.points.first()));
        self.points
            .windows(2)
            .map(|pair| (pair[0], pair[1]))
            .chain(closing.and_then(|(last, first)| Some((*last?, *first?))))
            .map(|(start, end)| distance_to_segment(point, start, end))
            .fold(f32::INFINITY, f32::min)
    }
}

/// An in-progress gizmo drag: moving every selected object together, or turning one.
#[derive(Debug, Clone)]
struct GizmoDrag {
    part: GizmoPart,
    /// Each moved object and where it started. The first one is the last
    /// selected, which lands on the snap grid; the others keep their offsets to it.
    objects: Vec<(usize, Vec3)>,
    /// Total distance dragged along the axis, before snapping.
    distance_along_axis: f32,
    /// The turn being made, when dragging a ring.
    turn: Option<TurnDrag>,
}

/// A ring being dragged: the object turns by the angle the pointer sweeps around the gizmo.
#[derive(Debug, Clone, Copy)]
struct TurnDrag {
    index: usize,
    start_rotation: Quat,
    /// The gizmo's middle on screen, which the pointer sweeps around.
    center: egui::Pos2,
    /// Pointer angle around `center` last frame, in radians.
    last_angle: f32,
    /// Angle swept so far, in radians (before snapping).
    total: f32,
    /// +1 or -1: which way a screen sweep turns the object, given how the ring faces the camera.
    sign: f32,
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
    hovered_part: Option<GizmoPart>,
    /// The open face the model being placed snaps to, to outline it.
    snap_face: Option<OpenFace>,
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

        // When the camera moves, what is under a still pointer changes too.
        if handle_camera_input(&mut self.camera, ui, &response) {
            self.viewport.last_pointer_position = None;
        }

        let projection = ViewportProjection::new(&self.camera, rect);
        let is_placing = self.palette.armed.is_some();
        let tying_from = self.viewport.tying_from;
        let gizmo_handles =
            if is_placing || tying_from.is_some() { Vec::new() } else { self.gizmo_handles(&projection) };
        if let Some(source) = tying_from {
            self.viewport.placement_preview = None;
            self.viewport.snap_face = None;
            self.handle_tying(&response, &projection, source, resources);
        } else if is_placing {
            self.handle_placement(ui, &response, &projection, resources);
        } else {
            self.viewport.placement_preview = None;
            self.viewport.snap_face = None;
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
            highlighted_groups: &[],
            selected_groups: &[],
            placement_preview: self.viewport.placement_preview.as_ref(),
            context_scene: self.dungeon_context.as_ref().map(|context| &context.scene),
            ground_squares: &[],
            water_planes: &water_planes,
            room_outlines: self
                .dungeon_context
                .as_ref()
                .map_or(&ROOM_OUTLINE[..], |context| &context.outlines),
            settings: &self.view_settings,
            grid_height: self.grid_height,
            snap_grid_spacing: self.snap.enabled.then_some(self.snap.step),
            size_in_pixels: size_in_pixels(ui, rect),
            sky: None,
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
        if preview.is_none() {
            self.draw_trajectories(&painter, &projection, resources.library);
        }
        self.update_unclosed_openings(ui, resources);
        if preview.is_none() {
            draw_unclosed_openings(&painter, &projection, &self.unclosed);
        }
        self.draw_gizmo(&painter, &gizmo_handles);
        self.draw_snap_face(&painter, &projection);
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
            self.status_message =
                "Several objects are selected: move them with the arrows, or copy (Ctrl+C), \
                 duplicate (Ctrl+D) or delete (Del) them"
                    .into();
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
        self.viewport.hovered_part = None;
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

    /// Returns the on-screen arrows of the gizmo, at the middle of the visible
    /// selected objects, and its rotation rings when one object is selected
    /// (several objects can only be moved together).
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
        let mut handles = Vec::new();
        if !self.has_multiple_selection() {
            let radius = world_length * GIZMO_RING_FACTOR;
            for axis in GizmoAxis::ALL {
                let (first, second) = ring_basis(axis);
                let points: Option<Vec<egui::Pos2>> = (0..GIZMO_RING_SEGMENTS)
                    .map(|step| {
                        let angle = step as f32 / GIZMO_RING_SEGMENTS as f32 * std::f32::consts::TAU;
                        let point = origin + (first * angle.cos() + second * angle.sin()) * radius;
                        projection.world_to_screen(point)
                    })
                    .collect();
                if let Some(points) = points {
                    handles.push(GizmoHandle { part: GizmoPart::Rotate(axis), points, world_length });
                }
            }
        }
        for axis in GizmoAxis::ALL {
            if let Some(screen_end) = projection.world_to_screen(origin + axis.direction() * world_length) {
                handles.push(GizmoHandle {
                    part: GizmoPart::Move(axis),
                    points: vec![screen_start, screen_end],
                    world_length,
                });
            }
        }
        handles
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

        // Which arrow or ring (if any) is under the pointer; arrows win where they cross a ring.
        if self.viewport.gizmo_drag.is_none() {
            self.viewport.hovered_part =
                pointer_position.and_then(|pointer| gizmo_part_at(gizmo_handles, pointer));
        }

        // Begin a drag on an arrow or ring, or a selection box anywhere else. By the time
        // a drag starts the pointer has moved a little, so what was grabbed is decided by
        // where the button went down.
        if response.drag_started_by(egui::PointerButton::Primary) {
            let press = ui.input(|input| input.pointer.press_origin());
            let grabbed =
                press.and_then(|press| gizmo_part_at(gizmo_handles, press)).or(self.viewport.hovered_part);
            self.viewport.hovered_part = grabbed;
            match grabbed {
                Some(part) => {
                    if self.can_edit() {
                        self.start_gizmo_drag(ui, part, gizmo_handles, projection);
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

        if let Some(mut drag) = self.viewport.gizmo_drag.take() {
            if response.dragged_by(egui::PointerButton::Primary) {
                // With grid lock on (or Ctrl held while it is off) moves land on the
                // snap grid and turns go in whole steps.
                let invert_snap = ui.input(|input| input.modifiers.command);
                let snapping = self.snap.enabled != invert_snap;
                match drag.part {
                    GizmoPart::Move(axis) => {
                        self.continue_move(&mut drag, axis, response, gizmo_handles, snapping)
                    }
                    GizmoPart::Rotate(axis) => self.continue_turn(&mut drag, axis, response, snapping),
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
            Some(pointer) if self.viewport.hovered_part.is_none() => self.pick_visible_object(
                projection.screen_to_ndc(pointer),
                aspect_ratio(projection.rect),
                resources.library,
            ),
            _ => None,
        };
        // A faded surrounding cell in front of the cell's own objects takes the pointer.
        self.viewport.hovered_context_cell = match pointer_position {
            Some(pointer) if self.viewport.hovered_part.is_none() => self.context_cell_under(
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

        if clicked && self.viewport.hovered_part.is_none() {
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

    /// Starts dragging gizmo `part`, recording the objects it changes for undo.
    fn start_gizmo_drag(
        &mut self,
        ui: &egui::Ui,
        part: GizmoPart,
        gizmo_handles: &[GizmoHandle],
        projection: &ViewportProjection,
    ) {
        // The last selected object leads, so it is the one that lands on the grid.
        let mut indices = self.selected_indices();
        indices.reverse();
        let objects: Vec<(usize, Vec3)> = indices
            .iter()
            .filter_map(|&index| self.scene.objects.get(index).map(|object| (index, object.position)))
            .collect();
        let edits = objects
            .iter()
            .map(|&(index, _)| SceneEdit::Replace { index, object: self.scene.objects[index].clone() })
            .collect();
        self.history.record(SceneEdit::Group(edits));

        let turn = match part {
            GizmoPart::Rotate(axis) => objects.first().and_then(|&(index, origin)| {
                let object = self.scene.objects.get(index)?;
                let center = projection.world_to_screen(origin)?;
                let pointer = ui.input(|input| input.pointer.press_origin())?;
                // Which way a sweep on screen turns the object: try a small turn of a ring point.
                let world_length = gizmo_handles.first().map_or(1.0, |handle| handle.world_length);
                let (first, _) = ring_basis(axis);
                let point = origin + first * world_length;
                let turned = origin + Quat::from_axis_angle(axis.direction(), 0.1) * (first * world_length);
                let sign = match (projection.world_to_screen(point), projection.world_to_screen(turned)) {
                    (Some(before), Some(after)) => {
                        let swept = wrap_angle(screen_angle(center, after) - screen_angle(center, before));
                        if swept < 0.0 {
                            -1.0
                        } else {
                            1.0
                        }
                    }
                    _ => 1.0,
                };
                Some(TurnDrag {
                    index,
                    start_rotation: object.rotation(),
                    center,
                    last_angle: screen_angle(center, pointer),
                    total: 0.0,
                    sign,
                })
            }),
            GizmoPart::Move(_) => None,
        };
        self.viewport.gizmo_drag = Some(GizmoDrag { part, objects, distance_along_axis: 0.0, turn });
    }

    /// Moves the dragged objects along `axis`: the mouse movement projected onto the arrow.
    fn continue_move(
        &mut self,
        drag: &mut GizmoDrag,
        axis: GizmoAxis,
        response: &egui::Response,
        gizmo_handles: &[GizmoHandle],
        snapping: bool,
    ) {
        if let Some(handle) = gizmo_handles.iter().find(|handle| handle.part == GizmoPart::Move(axis)) {
            let screen_axis = handle.points[1] - handle.points[0];
            let screen_axis_length_squared = screen_axis.length_sq();
            if screen_axis_length_squared > 1.0 {
                let along = response.drag_delta().dot(screen_axis) / screen_axis_length_squared;
                drag.distance_along_axis += along * handle.world_length;
            }
        }
        let Some(&(_, lead_start)) = drag.objects.first() else { return };
        let mut lead_position = lead_start + axis.direction() * drag.distance_along_axis;
        if snapping && self.snap.step > 0.0 {
            let component = &mut lead_position[axis.component()];
            *component = (*component / self.snap.step).round() * self.snap.step;
        }
        let offset = lead_position - lead_start;
        for &(index, start) in &drag.objects {
            if let Some(object) = self.scene.objects.get_mut(index) {
                object.position = start + offset;
            }
        }
    }

    /// Turns the dragged object around `axis` by the angle the pointer has swept around the gizmo.
    fn continue_turn(
        &mut self,
        drag: &mut GizmoDrag,
        axis: GizmoAxis,
        response: &egui::Response,
        snapping: bool,
    ) {
        let (Some(turn), Some(pointer)) = (drag.turn.as_mut(), response.interact_pointer_pos()) else {
            return;
        };
        let angle = screen_angle(turn.center, pointer);
        turn.total += wrap_angle(angle - turn.last_angle) * turn.sign;
        turn.last_angle = angle;
        let mut degrees = turn.total.to_degrees();
        if snapping {
            degrees = (degrees / ROTATION_SNAP_DEGREES).round() * ROTATION_SNAP_DEGREES;
        }
        let rotation = Quat::from_axis_angle(axis.direction(), degrees.to_radians()) * turn.start_rotation;
        if let Some(object) = self.scene.objects.get_mut(turn.index) {
            object.set_rotation(rotation);
        }
        self.status_message = format!("Turning {degrees:.0}° around {axis:?}");
    }

    /// Finds the best open face near `moving` (a model being placed) to snap it
    /// to: an opening facing one of its own, or for a door a doorway. Faces
    /// closest in size win, then the nearest. `None` if nothing is in reach.
    fn find_snap(
        &mut self,
        moving: &SceneObject,
        door_slots: &crate::connectors::DoorSlots,
        library: &mut Arena2Library,
    ) -> Option<Snap> {
        let SceneObjectKind::Model { model_id, .. } = moving.kind else { return None };
        let local_faces = self.open_faces_of(model_id, library);
        if local_faces.is_empty() {
            return None;
        }
        let moving_rotation = moving.rotation();
        let moving_faces: Vec<OpenFace> =
            local_faces.iter().map(|face| face.transformed(moving_rotation, moving.position)).collect();
        let size =
            library.model(model_id).map_or(0.0, |mesh| (mesh.bounds_max - mesh.bounds_min).max_element());
        let reach = SNAP_REACH + size * 0.5;

        let mut best: Option<Snap> = None;
        let mut consider = |snap: Snap| {
            if snap.distance <= reach
                && snap.mismatch <= MAX_SNAP_MISMATCH
                && best.is_none_or(|best| snap.score(reach) < best.score(reach))
            {
                best = Some(snap);
            }
        };
        let others: Vec<(usize, u32, Vec3, Quat)> = self
            .scene
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.visible)
            .filter_map(|(index, object)| match object.kind {
                SceneObjectKind::Model { model_id, .. } => {
                    Some((index, model_id, object.position, object.rotation()))
                }
                _ => None,
            })
            .filter(|(_, _, position, _)| position.distance(moving.position) < reach + 40.0)
            .collect();
        // Every face nearby, by owner, to tell which faces something is already joined to.
        let mut placed_faces: Vec<(usize, OpenFace)> = Vec::new();
        for &(index, other_id, position, rotation) in &others {
            let faces = self.open_faces_of(other_id, library);
            placed_faces.extend(faces.iter().map(|face| (index, face.transformed(rotation, position))));
        }
        let taken = |owner: usize, face: &OpenFace| {
            placed_faces
                .iter()
                .any(|(other, placed)| *other != owner && crate::connectors::occupies(placed, face))
        };
        for &(owner, other_id, position, rotation) in &others {
            let fixed_faces = self.open_faces_of(other_id, library);
            for fixed in fixed_faces.iter().map(|face| face.transformed(rotation, position)) {
                if taken(owner, &fixed) {
                    continue;
                }
                for face in &moving_faces {
                    let Some(mismatch) = fit(face, &fixed) else { continue };
                    let offset = match face.is_plug {
                        true => crate::connectors::plug_offset(face, &fixed),
                        false => match join_offset(face, &fixed) {
                            Some(offset) => offset,
                            None => continue,
                        },
                    };
                    consider(Snap {
                        position: moving.position + offset,
                        rotation: None,
                        face: fixed,
                        mismatch,
                        distance: offset.length(),
                    });
                }
            }
            // A door can also stand in a doorway inside the piece, where Daggerfall puts doors.
            if let Some(plug) = local_faces.iter().find(|face| face.is_plug) {
                for slot in door_slots.of(other_id) {
                    let slot_rotation = rotation * slot.rotation;
                    let slot_plug = slot.plug.transformed(rotation, position);
                    // A doorway that already has a door in it is taken.
                    if placed_faces
                        .iter()
                        .any(|(_, placed)| placed.is_plug && placed.center.distance(slot_plug.center) < 0.3)
                    {
                        continue;
                    }
                    let mut sizes = [plug.half_size.x, plug.half_size.y];
                    let mut slot_sizes = [slot.plug.half_size.x, slot.plug.half_size.y];
                    sizes.sort_by(f32::total_cmp);
                    slot_sizes.sort_by(f32::total_cmp);
                    let mismatch = ((sizes[0] - slot_sizes[0]).abs() + (sizes[1] - slot_sizes[1]).abs())
                        / (slot_sizes[0] + slot_sizes[1]).max(0.01);
                    let new_position = slot_plug.center - slot_rotation * plug.center;
                    let current_middle = moving.position + moving_rotation * plug.center;
                    consider(Snap {
                        position: new_position,
                        rotation: Some(slot_rotation),
                        face: slot_plug,
                        mismatch,
                        distance: current_middle.distance(slot_plug.center),
                    });
                }
            }
        }
        best
    }

    /// The open faces of model `model_id`, worked out once and remembered.
    fn open_faces_of(&mut self, model_id: u32, library: &mut Arena2Library) -> std::sync::Arc<[OpenFace]> {
        self.open_faces
            .entry(model_id)
            .or_insert_with(|| {
                library
                    .model(model_id)
                    .map_or_else(|| Vec::new().into(), |mesh| crate::connectors::open_faces(&mesh).into())
            })
            .clone()
    }

    /// Draws the path each selected object's action moves it along: where the
    /// middle of the object goes, from where it is to where the action leaves it,
    /// with a dot at the end.
    fn draw_trajectories(
        &self,
        painter: &egui::Painter,
        projection: &ViewportProjection,
        library: &mut Arena2Library,
    ) {
        // Each selected object, and everything down its action chain, once each.
        let mut chain = Vec::new();
        for selected in self.selected_indices() {
            for index in self.scene.action_chain(selected) {
                if !chain.contains(&index) {
                    chain.push(index);
                }
            }
        }
        for index in chain {
            let Some(object) = self.scene.objects.get(index).filter(|object| object.visible) else {
                continue;
            };
            let Some(motion) = self.object_motion(index, library) else { continue };
            // The middle of a model, which moves with it whatever point it turns around.
            let middle = match object.kind {
                SceneObjectKind::Model { model_id, .. } => {
                    library.model(model_id).map_or(Vec3::ZERO, |mesh| mesh.bounds_center())
                }
                _ => Vec3::ZERO,
            };
            let points: Option<Vec<egui::Pos2>> = (0..=TRAJECTORY_STEPS)
                .map(|step| {
                    let moved = object.moved_by(&motion, step as f32 / TRAJECTORY_STEPS as f32);
                    projection.world_to_screen(moved.position + moved.rotation() * middle)
                })
                .collect();
            let Some(points) = points else { continue };
            let (Some(&start), Some(&end)) = (points.first(), points.last()) else { continue };
            if start.distance(end) < 1.0 && points.iter().all(|point| point.distance(start) < 1.0) {
                continue;
            }
            painter.add(egui::Shape::line(points, egui::Stroke::new(2.5_f32, TRAJECTORY_COLOR)));
            painter.circle_stroke(start, 3.5, egui::Stroke::new(1.5_f32, TRAJECTORY_COLOR));
            painter.circle_filled(end, 4.5, TRAJECTORY_COLOR);
        }
    }

    /// Outlines the face a model being placed snaps to.
    /// Finds again the openings that lead to empty space, when the cell (or
    /// the cells shown around it) changed, at most every
    /// [`UNCLOSED_CHECK_INTERVAL`] while it keeps changing. Without the cells
    /// around it, openings on the cell's edge are left out: there they meet the
    /// neighbouring cells.
    fn update_unclosed_openings(&mut self, ui: &egui::Ui, resources: &mut EditorResources<'_>) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let context = self.dungeon_context.as_ref().map(|context| &context.scene.objects);
        for object in self.scene.objects.iter().chain(context.into_iter().flatten()) {
            if let SceneObjectKind::Model { model_id, .. } = object.kind {
                (model_id, object.visible).hash(&mut hasher);
                for value in object.position.to_array().into_iter().chain(object.rotation_degrees.to_array())
                {
                    value.to_bits().hash(&mut hasher);
                }
            }
        }
        (context.is_some(), resources.catalog.openings_left_open.is_some()).hash(&mut hasher);
        let key = hasher.finish();
        let now = ui.input(|input| input.time);
        match self.unclosed_checked {
            Some((checked_key, _)) if checked_key == key => return,
            Some((_, checked_at)) if now - checked_at < UNCLOSED_CHECK_INTERVAL => {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(UNCLOSED_CHECK_INTERVAL));
                return;
            }
            _ => {}
        }
        let mut objects = self.scene.objects.clone();
        let checked = objects.len();
        objects.extend(context.into_iter().flatten().cloned());
        let found = find_unclosed_openings(
            &objects,
            checked,
            context.is_none(),
            resources.library,
            resources.catalog,
            &mut self.open_faces,
        );
        self.unclosed = found.unwrap_or_default();
        self.unclosed_checked = Some((key, now));
    }

    fn draw_snap_face(&self, painter: &egui::Painter, projection: &ViewportProjection) {
        let Some(face) = &self.viewport.snap_face else { return };
        let corners: Option<Vec<egui::Pos2>> =
            face.corners().iter().map(|&corner| projection.world_to_screen(corner)).collect();
        if let Some(mut corners) = corners {
            corners.push(corners[0]);
            painter.add(egui::Shape::line(corners, egui::Stroke::new(2.5_f32, SNAP_FACE_COLOR)));
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
            several => format!(
                "Selected {} objects; drag the arrows to move them together, Ctrl+C to copy them",
                several.len()
            ),
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
        self.viewport.hovered_part = None;
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
            let mut preview =
                surface_point.map(|point| armed.asset.create_object(library, point, armed.yaw_degrees));
            // Snap onto an open face nearby, unless Alt is held.
            self.viewport.snap_face = None;
            if !ui.input(|input| input.modifiers.alt) {
                if let Some(object) = preview.as_mut() {
                    if let Some(snap) = self.find_snap(object, &resources.catalog.door_slots, library) {
                        object.position = snap.position;
                        if let Some(rotation) = snap.rotation {
                            object.set_rotation(rotation);
                        }
                        self.viewport.snap_face = Some(snap.face);
                    }
                }
            }
            self.viewport.placement_preview = preview;
        }

        if !response.clicked_by(egui::PointerButton::Primary) {
            return;
        }
        let Some(object) = self.viewport.placement_preview.clone() else { return };
        if !self.can_edit() {
            self.palette.armed = None;
            self.viewport.placement_preview = None;
            self.viewport.snap_face = None;
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
            self.viewport.snap_face = None;
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

    /// Draws the gizmo's rings and arrows over the viewport image.
    fn draw_gizmo(&self, painter: &egui::Painter, gizmo_handles: &[GizmoHandle]) {
        let active = self.viewport.gizmo_drag.as_ref().map(|drag| drag.part).or(self.viewport.hovered_part);
        for handle in gizmo_handles.iter().filter(|handle| matches!(handle.part, GizmoPart::Rotate(_))) {
            let color = handle.part.axis().color(active == Some(handle.part));
            let mut points = handle.points.clone();
            points.extend(points.first().copied());
            painter.add(egui::Shape::line(points, egui::Stroke::new(2.0_f32, color)));
        }
        for handle in gizmo_handles.iter().filter(|handle| matches!(handle.part, GizmoPart::Move(_))) {
            let color = handle.part.axis().color(active == Some(handle.part));
            painter.line_segment([handle.points[0], handle.points[1]], egui::Stroke::new(3.0_f32, color));
            painter.circle_filled(handle.points[1], 5.0, color);
        }
        if let Some(handle) = gizmo_handles.iter().find(|handle| matches!(handle.part, GizmoPart::Move(_))) {
            painter.circle_filled(handle.points[0], 3.5, egui::Color32::WHITE);
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

/// Two unit vectors spanning the plane a ring around `axis` lies in.
fn ring_basis(axis: GizmoAxis) -> (Vec3, Vec3) {
    match axis {
        GizmoAxis::X => (Vec3::Y, Vec3::Z),
        GizmoAxis::Y => (Vec3::Z, Vec3::X),
        GizmoAxis::Z => (Vec3::X, Vec3::Y),
    }
}

/// Angle of `point` around `center` on screen, in radians.
fn screen_angle(center: egui::Pos2, point: egui::Pos2) -> f32 {
    (point.y - center.y).atan2(point.x - center.x)
}

/// `angle` brought into -π..π.
fn wrap_angle(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// The arrow or ring of the gizmo at `point`, if any; arrows win where they cross a ring.
fn gizmo_part_at(gizmo_handles: &[GizmoHandle], point: egui::Pos2) -> Option<GizmoPart> {
    gizmo_handles
        .iter()
        .map(|handle| {
            let preference = if matches!(handle.part, GizmoPart::Move(_)) { 0.0 } else { 2.0 };
            (handle.part, handle.distance(point) + preference)
        })
        .filter(|(_, distance)| *distance <= GIZMO_GRAB_DISTANCE)
        .min_by(|first, second| first.1.total_cmp(&second.1))
        .map(|(part, _)| part)
}

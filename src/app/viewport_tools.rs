//! Helpers shared by every 3D viewport: screen projection, camera controls
//! and painting the rendered image.

use eframe::{egui, egui_wgpu};
use glam::{Mat4, Vec2, Vec3};

use crate::arena2::{Arena2Library, RDB_BLOCK_SIDE};
use crate::camera::OrbitCamera;
use crate::catalog::AssetCatalog;
use crate::connectors::{is_on_cell_edge, unclosed_faces, OpenFace, UnclosedFace};
use crate::custom_names::CustomNames;
use crate::render::{FrameRequest, SceneRenderer, ViewportTarget};
use crate::scene::SceneObject;

/// Fly speed as a fraction of the orbit distance per second.
const FLY_SPEED_FACTOR: f32 = 0.8;
/// Multiplier applied to fly speed while Shift is held.
const FLY_FAST_MULTIPLIER: f32 = 3.0;
/// Background shown when no GPU renderer is available.
const NO_RENDERER_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(9, 10, 13);

/// Shared, long-lived resources that every editor and view needs while drawing.
pub struct EditorResources<'resources> {
    pub library: &'resources mut Arena2Library,
    pub render_state: Option<&'resources egui_wgpu::RenderState>,
    pub renderer: Option<&'resources mut SceneRenderer>,
    pub catalog: &'resources AssetCatalog,
    /// The user's own names for rooms and models, editable from any editor.
    pub custom_names: &'resources mut CustomNames,
}

/// Maps between world space and a viewport rectangle on screen.
pub struct ViewportProjection {
    pub rect: egui::Rect,
    pub view_projection: Mat4,
}

impl ViewportProjection {
    /// Builds the projection for `camera` drawn into `rect`.
    pub fn new(camera: &OrbitCamera, rect: egui::Rect) -> Self {
        Self { rect, view_projection: camera.view_projection(aspect_ratio(rect)) }
    }

    /// Projects a world position to screen coordinates, or `None` if it is behind the camera.
    pub fn world_to_screen(&self, world_position: Vec3) -> Option<egui::Pos2> {
        let clip = self.view_projection * world_position.extend(1.0);
        if clip.w <= 1e-4 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(egui::pos2(
            self.rect.left() + (ndc.x + 1.0) * 0.5 * self.rect.width(),
            self.rect.top() + (1.0 - ndc.y) * 0.5 * self.rect.height(),
        ))
    }

    /// Converts a screen position to normalised device coordinates.
    pub fn screen_to_ndc(&self, screen_position: egui::Pos2) -> Vec2 {
        Vec2::new(
            (screen_position.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0,
            1.0 - (screen_position.y - self.rect.top()) / self.rect.height() * 2.0,
        )
    }
}

/// Width divided by height of a rectangle, guarding against zero height.
pub fn aspect_ratio(rect: egui::Rect) -> f32 {
    rect.width() / rect.height().max(1.0)
}

/// Orbit (right drag), pan (middle drag or Shift + right drag), zoom (wheel)
/// and movement keys, which work while the pointer is over the view: W A S D
/// move forwards, left, back and right along the ground; Space rises and Ctrl
/// sinks (and E and Q, with the right mouse button held); Shift goes faster. Letters do nothing with Ctrl or Alt held
/// (those are shortcuts such as Ctrl+S), and Ctrl only sinks when held on its
/// own. Pressing a mouse button over the view takes the keyboard away from any
/// text field, so the keys move the camera rather than type.
/// Returns whether the camera moved, so what is under the pointer can be
/// worked out again even though the pointer itself did not move.
pub fn handle_camera_input(camera: &mut OrbitCamera, ui: &egui::Ui, response: &egui::Response) -> bool {
    let before = camera.clone();
    let drag_delta = response.drag_delta();
    let drag_delta = Vec2::new(drag_delta.x, drag_delta.y);
    let shift_held = ui.input(|input| input.modifiers.shift);

    let is_panning = response.dragged_by(egui::PointerButton::Middle)
        || (response.dragged_by(egui::PointerButton::Secondary) && shift_held);
    if is_panning {
        camera.pan(drag_delta, response.rect.height());
    } else if response.dragged_by(egui::PointerButton::Secondary) {
        camera.orbit(drag_delta);
    }

    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            camera.zoom(scroll);
        }
    }

    let over_view = response.hovered() || response.dragged();
    // Clicking the view (with any button) takes the keyboard away from a text field.
    if over_view && ui.input(|input| input.pointer.any_pressed()) {
        ui.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }
    // Keys typed into a text field are not for the camera.
    let typing = ui.ctx().wants_keyboard_input();
    if over_view && !typing {
        // Space would also press whichever button has keyboard focus (the last one
        // clicked), so that focus is let go while Space is used here.
        if ui.input(|input| input.key_down(egui::Key::Space)) {
            ui.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
        }
        let (offset, delta_time) = ui.input(|input| {
            let modifiers = input.modifiers;
            // Letters held with Ctrl or Alt are shortcuts, not movement.
            let letters = !modifiers.command && !modifiers.alt;
            let key = |key| letters && input.key_down(key);
            let axis =
                |positive, negative| f32::from(u8::from(key(positive))) - f32::from(u8::from(key(negative)));
            let ctrl_alone = modifiers.command
                && !modifiers.shift
                && !modifiers.alt
                && input.keys_down.is_empty()
                && !input.pointer.any_down();
            // Without the right mouse button, Q and E turn objects in the cell editor.
            let flying = input.pointer.secondary_down();
            let up = input.key_down(egui::Key::Space) || (flying && key(egui::Key::E));
            let down = ctrl_alone || (flying && key(egui::Key::Q));
            let offset = Vec3::new(
                axis(egui::Key::D, egui::Key::A),
                f32::from(u8::from(up)) - f32::from(u8::from(down)),
                axis(egui::Key::W, egui::Key::S),
            );
            (offset, input.stable_dt.min(0.1))
        });
        if offset != Vec3::ZERO {
            let speed_multiplier = if shift_held { FLY_FAST_MULTIPLIER } else { 1.0 };
            let speed = camera.distance.max(2.0) * FLY_SPEED_FACTOR * speed_multiplier;
            camera.walk(offset.normalize() * speed * delta_time);
            ui.ctx().request_repaint();
        }
    }
    *camera != before
}

/// Renders `request` into `target` and paints the result over `rect`.
///
/// Falls back to a plain background if there is no GPU renderer.
pub fn paint_scene(
    ui: &egui::Ui,
    rect: egui::Rect,
    resources: &mut EditorResources<'_>,
    target: &mut ViewportTarget,
    request: &FrameRequest<'_>,
) {
    let painter = ui.painter_at(rect);
    match (resources.render_state, resources.renderer.as_deref_mut()) {
        (Some(render_state), Some(renderer)) => {
            let texture_id = renderer.render(render_state, resources.library, target, request);
            let full_texture = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture_id, rect, full_texture, egui::Color32::WHITE);
        }
        _ => {
            painter.rect_filled(rect, 0.0, NO_RENDERER_BACKGROUND);
        }
    }
}

/// Size in physical pixels of a rectangle measured in points.
pub fn size_in_pixels(ui: &egui::Ui, rect: egui::Rect) -> [u32; 2] {
    let pixels_per_point = ui.ctx().pixels_per_point();
    [
        (rect.width() * pixels_per_point).round().max(1.0) as u32,
        (rect.height() * pixels_per_point).round().max(1.0) as u32,
    ]
}

/// Fill and outline of the pane over an opening that leads to empty space.
const UNCLOSED_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(90, 75, 10, 90);
const UNCLOSED_STROKE: egui::Color32 = egui::Color32::from_rgb(255, 210, 40);
/// Shortest time between two checks of the openings while things keep changing, in seconds.
pub const UNCLOSED_CHECK_INTERVAL: f64 = 0.3;

/// The openings of the first `checked` objects of `objects` that lead to empty
/// space (see [`unclosed_faces`]), leaving out those Daggerfall's own dungeons
/// leave open too and, with `skip_cell_edges`, those on the cell's edge (where
/// a cell edited on its own meets its neighbours). The rest of `objects` (the
/// cells around) can close openings but are not checked. `None` until the
/// openings Daggerfall leaves open have been learned.
pub fn find_unclosed_openings(
    objects: &[SceneObject],
    checked: usize,
    skip_cell_edges: bool,
    library: &mut Arena2Library,
    catalog: &AssetCatalog,
    faces_cache: &mut std::collections::HashMap<u32, std::sync::Arc<[OpenFace]>>,
) -> Option<Vec<UnclosedFace>> {
    let left_open = catalog.openings_left_open.clone()?;
    Some(
        unclosed_faces(objects, |index| index < checked, library, faces_cache)
            .into_iter()
            .filter(|face| !left_open.contains(face.model_id, face.face_index))
            .filter(|face| !(skip_cell_edges && is_on_cell_edge(&face.face, RDB_BLOCK_SIDE)))
            .collect(),
    )
}

/// Draws a yellow pane over each opening that leads to empty space.
pub fn draw_unclosed_openings(
    painter: &egui::Painter,
    projection: &ViewportProjection,
    faces: &[UnclosedFace],
) {
    for face in faces {
        let corners: Option<Vec<egui::Pos2>> =
            face.face.corners().iter().map(|&corner| projection.world_to_screen(corner)).collect();
        if let Some(corners) = corners {
            painter.add(egui::Shape::convex_polygon(
                corners,
                UNCLOSED_FILL,
                egui::Stroke::new(1.5_f32, UNCLOSED_STROKE),
            ));
        }
    }
}

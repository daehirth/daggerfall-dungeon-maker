//! Helpers shared by every 3D viewport: screen projection, camera controls
//! and painting the rendered image.

use eframe::{egui, egui_wgpu};
use glam::{Mat4, Vec2, Vec3};

use crate::arena2::Arena2Library;
use crate::camera::OrbitCamera;
use crate::catalog::AssetCatalog;
use crate::custom_names::CustomNames;
use crate::render::{FrameRequest, SceneRenderer, ViewportTarget};

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

/// Orbit (right drag), pan (middle drag or Shift + right drag), zoom (wheel) and
/// fly (right mouse + WASD/QE) controls.
pub fn handle_camera_input(camera: &mut OrbitCamera, ui: &egui::Ui, response: &egui::Response) {
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

    let secondary_held = ui.input(|input| input.pointer.secondary_down());
    if secondary_held && (response.hovered() || response.dragged()) {
        let (offset, delta_time) = ui.input(|input| {
            let key_axis = |positive, negative| {
                f32::from(u8::from(input.key_down(positive))) - f32::from(u8::from(input.key_down(negative)))
            };
            let offset = Vec3::new(
                key_axis(egui::Key::D, egui::Key::A),
                key_axis(egui::Key::E, egui::Key::Q),
                key_axis(egui::Key::W, egui::Key::S),
            );
            (offset, input.stable_dt.min(0.1))
        });
        if offset != Vec3::ZERO {
            let speed_multiplier = if shift_held { FLY_FAST_MULTIPLIER } else { 1.0 };
            let speed = camera.distance.max(2.0) * FLY_SPEED_FACTOR * speed_multiplier;
            camera.fly(offset.normalize() * speed * delta_time);
            ui.ctx().request_repaint();
        }
    }
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

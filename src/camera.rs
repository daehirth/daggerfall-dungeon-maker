//! An orbit camera with CAD-style controls.
//!
//! The camera orbits around a target point. The world is left-handed with Y
//! up (matching Daggerfall Unity), so all view and projection matrices use
//! the left-handed variants.

use glam::{Mat4, Vec2, Vec3};

use crate::picking::Ray;

/// Radians of rotation per pixel of mouse movement when orbiting.
const ORBIT_SENSITIVITY: f32 = 0.008;
/// Pitch limit that keeps the camera from flipping over the poles.
const MAX_PITCH: f32 = 1.55;
/// Smallest and largest allowed orbit distances, in world units.
const DISTANCE_RANGE: (f32, f32) = (0.25, 5000.0);
/// Fraction of the distance moved per unit of scroll input.
const ZOOM_SENSITIVITY: f32 = 0.004;

/// A camera orbiting around [`OrbitCamera::target`].
#[derive(Debug, Clone)]
pub struct OrbitCamera {
    /// Point the camera looks at and orbits around.
    pub target: Vec3,
    /// Rotation around the vertical axis, in radians. Zero looks along +Z.
    pub yaw: f32,
    /// Elevation angle in radians. Negative values look downwards.
    pub pitch: f32,
    /// Distance from the target to the eye.
    pub distance: f32,
    /// Vertical field of view in radians.
    pub vertical_fov: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self { target: Vec3::ZERO, yaw: 0.6, pitch: -0.6, distance: 60.0, vertical_fov: 60f32.to_radians() }
    }
}

impl OrbitCamera {
    /// Unit vector pointing from the eye towards the target.
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos())
    }

    /// Unit vector pointing to the right of the view.
    pub fn right(&self) -> Vec3 {
        Vec3::Y.cross(self.forward()).normalize()
    }

    /// Unit vector pointing up in the view.
    pub fn up(&self) -> Vec3 {
        self.forward().cross(self.right())
    }

    /// World position of the eye.
    pub fn eye_position(&self) -> Vec3 {
        self.target - self.forward() * self.distance
    }

    /// Near clip distance, scaled with the orbit distance to keep depth precision.
    fn near_plane(&self) -> f32 {
        (self.distance * 0.002).clamp(0.02, 1.0)
    }

    /// World to view space transform.
    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_lh(self.eye_position(), self.target, Vec3::Y)
    }

    /// View to clip space transform for a viewport with the given aspect ratio.
    pub fn projection_matrix(&self, aspect_ratio: f32) -> Mat4 {
        Mat4::perspective_lh(self.vertical_fov, aspect_ratio.max(0.01), self.near_plane(), 20_000.0)
    }

    /// Combined world to clip space transform.
    pub fn view_projection(&self, aspect_ratio: f32) -> Mat4 {
        self.projection_matrix(aspect_ratio) * self.view_matrix()
    }

    /// Rotates the camera around the target by a mouse movement in pixels.
    pub fn orbit(&mut self, mouse_delta: Vec2) {
        self.yaw += mouse_delta.x * ORBIT_SENSITIVITY;
        self.pitch = (self.pitch - mouse_delta.y * ORBIT_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Slides the camera and target sideways so the scene follows the mouse.
    pub fn pan(&mut self, mouse_delta: Vec2, viewport_height_pixels: f32) {
        let world_units_per_pixel = self.world_units_per_pixel(viewport_height_pixels);
        self.target -= self.right() * mouse_delta.x * world_units_per_pixel;
        self.target += self.up() * mouse_delta.y * world_units_per_pixel;
    }

    /// Moves the camera closer to or further from the target.
    pub fn zoom(&mut self, scroll_amount: f32) {
        let scale = (-scroll_amount * ZOOM_SENSITIVITY).exp();
        self.distance = (self.distance * scale).clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1);
    }

    /// Moves the target relative to the view: `x` right, `y` up, `z` forward.
    pub fn fly(&mut self, local_offset: Vec3) {
        self.target +=
            self.right() * local_offset.x + Vec3::Y * local_offset.y + self.forward() * local_offset.z;
    }

    /// Positions the camera so the box between `minimum` and `maximum` fills the view.
    pub fn frame_bounds(&mut self, minimum: Vec3, maximum: Vec3) {
        let radius = ((maximum - minimum).length() * 0.5).max(1.0);
        self.target = (minimum + maximum) * 0.5;
        self.distance = (radius / (self.vertical_fov * 0.5).sin()).clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1);
    }

    /// Size of one screen pixel at the target distance, in world units.
    pub fn world_units_per_pixel(&self, viewport_height_pixels: f32) -> f32 {
        2.0 * self.distance * (self.vertical_fov * 0.5).tan() / viewport_height_pixels.max(1.0)
    }

    /// Builds a world space ray through a point in normalised device coordinates.
    ///
    /// `ndc` ranges from -1 to 1 on both axes, with +Y at the top of the viewport.
    pub fn ray_through(&self, ndc: Vec2, aspect_ratio: f32) -> Ray {
        let inverse = self.view_projection(aspect_ratio).inverse();
        let near_point = inverse.project_point3(ndc.extend(0.0));
        let far_point = inverse.project_point3(ndc.extend(1.0));
        Ray { origin: near_point, direction: (far_point - near_point).normalize() }
    }
}

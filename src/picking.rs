//! Ray casting used to select objects in the viewport.

use glam::{Vec2, Vec3};

use crate::arena2::{Arena2Library, ModelMesh};
use crate::scene::{Scene, SceneObject, SceneObjectKind};

/// Radius of the clickable sphere around a light, in world units.
pub const LIGHT_PICK_RADIUS: f32 = 0.4;

/// A half-line in world space.
#[derive(Debug, Clone, Copy)]
pub struct Ray {
    /// Start point.
    pub origin: Vec3,
    /// Unit direction.
    pub direction: Vec3,
}

impl Ray {
    /// Point at distance `distance` along the ray.
    pub fn point_at(&self, distance: f32) -> Vec3 {
        self.origin + self.direction * distance
    }
}

/// Which kinds of objects can be picked and how.
#[derive(Debug, Clone, Copy)]
pub struct PickOptions {
    /// Ignore triangles facing away from the camera, matching back-face culling.
    pub ignore_back_faces: bool,
    /// Allow models to be picked.
    pub include_models: bool,
    /// Allow flats and markers to be picked.
    pub include_flats: bool,
    /// Allow lights to be picked.
    pub include_lights: bool,
    /// Camera right vector, used to orient billboards.
    pub camera_right: Vec3,
}

/// The closest object hit by a ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickHit {
    /// Index of the object in the scene.
    pub object_index: usize,
    /// Distance along the ray to the hit point.
    pub distance: f32,
}

/// Returns the closest object hit by `ray`, if any.
pub fn pick_object(
    scene: &Scene,
    library: &mut Arena2Library,
    ray: &Ray,
    options: &PickOptions,
) -> Option<PickHit> {
    let mut closest: Option<(usize, f32)> = None;

    for (index, object) in scene.objects.iter().enumerate() {
        if !object.visible {
            continue;
        }
        let hit_distance = match object.kind {
            SceneObjectKind::Model { model_id, .. } if options.include_models => {
                intersect_model(object, model_id, library, ray, options.ignore_back_faces)
            }
            SceneObjectKind::Flat { texture, .. } if options.include_flats => {
                let size = library.billboard_world_size(texture).unwrap_or(Vec2::ONE);
                intersect_billboard(ray, object.position, size, options.camera_right)
            }
            SceneObjectKind::Light { .. } if options.include_lights => {
                intersect_sphere(ray, object.position, LIGHT_PICK_RADIUS)
            }
            _ => None,
        };

        if let Some(distance) = hit_distance {
            if closest.is_none_or(|(_, closest_distance)| distance < closest_distance) {
                closest = Some((index, distance));
            }
        }
    }
    closest.map(|(object_index, distance)| PickHit { object_index, distance })
}

/// Distance along `ray` to the horizontal plane at `height`, if the ray points towards it.
pub fn intersect_horizontal_plane(ray: &Ray, height: f32) -> Option<f32> {
    if ray.direction.y.abs() < 1e-6 {
        return None;
    }
    let distance = (height - ray.origin.y) / ray.direction.y;
    (distance > 0.0).then_some(distance)
}

/// Intersects `ray` with the triangles of a model instance.
fn intersect_model(
    object: &SceneObject,
    model_id: u32,
    library: &mut Arena2Library,
    ray: &Ray,
    ignore_back_faces: bool,
) -> Option<f32> {
    let mesh = library.model(model_id)?;
    intersect_mesh(object, &mesh, ray, ignore_back_faces)
}

/// Intersects `ray` with the triangles of `mesh` placed as `object`.
pub fn intersect_mesh(
    object: &SceneObject,
    mesh: &ModelMesh,
    ray: &Ray,
    ignore_back_faces: bool,
) -> Option<f32> {
    // Work in model space. The transform is rigid, so distances are preserved.
    let world_to_model = object.model_matrix().inverse();
    let local_ray = Ray {
        origin: world_to_model.transform_point3(ray.origin),
        direction: world_to_model.transform_vector3(ray.direction),
    };

    intersect_sphere(&local_ray, mesh.bounds_center(), mesh.bounding_radius())?;

    let mut closest: Option<f32> = None;
    for submesh in &mesh.submeshes {
        for triangle in submesh.indices.as_chunks::<3>().0 {
            let [first, second, third] = [0, 1, 2].map(|corner| submesh.vertices[triangle[corner] as usize]);
            // Back faces point along the ray; skip them when culling is enabled.
            if ignore_back_faces && first.normal.dot(local_ray.direction) > 0.0 {
                continue;
            }
            if let Some(distance) =
                intersect_triangle(&local_ray, first.position, second.position, third.position)
            {
                closest = Some(closest.map_or(distance, |current: f32| current.min(distance)));
            }
        }
    }
    closest
}

/// Intersects `ray` with an upright billboard rotated to face the camera.
fn intersect_billboard(ray: &Ray, center: Vec3, size: Vec2, camera_right: Vec3) -> Option<f32> {
    let right = Vec3::new(camera_right.x, 0.0, camera_right.z).try_normalize()?;
    let normal = right.cross(Vec3::Y);
    let denominator = normal.dot(ray.direction);
    if denominator.abs() < 1e-6 {
        return None;
    }
    let distance = normal.dot(center - ray.origin) / denominator;
    if distance < 0.0 {
        return None;
    }
    let offset = ray.point_at(distance) - center;
    let inside = offset.dot(right).abs() <= size.x * 0.5 && offset.y.abs() <= size.y * 0.5;
    inside.then_some(distance)
}

/// Returns the distance to the first intersection of `ray` with a sphere.
pub fn intersect_sphere(ray: &Ray, center: Vec3, radius: f32) -> Option<f32> {
    let to_center = center - ray.origin;
    let closest_approach = to_center.dot(ray.direction);
    let distance_squared = to_center.length_squared() - closest_approach * closest_approach;
    let radius_squared = radius * radius;
    if distance_squared > radius_squared {
        return None;
    }
    let half_chord = (radius_squared - distance_squared).sqrt();
    let entry = closest_approach - half_chord;
    let exit = closest_approach + half_chord;
    if exit < 0.0 {
        None
    } else {
        Some(entry.max(0.0))
    }
}

/// Möller–Trumbore ray/triangle intersection (double sided).
pub fn intersect_triangle(ray: &Ray, first: Vec3, second: Vec3, third: Vec3) -> Option<f32> {
    const EPSILON: f32 = 1e-7;
    let edge_1 = second - first;
    let edge_2 = third - first;
    let perpendicular = ray.direction.cross(edge_2);
    let determinant = edge_1.dot(perpendicular);
    if determinant.abs() < EPSILON {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    let to_origin = ray.origin - first;
    let barycentric_u = to_origin.dot(perpendicular) * inverse_determinant;
    if !(0.0..=1.0).contains(&barycentric_u) {
        return None;
    }
    let cross = to_origin.cross(edge_1);
    let barycentric_v = ray.direction.dot(cross) * inverse_determinant;
    if barycentric_v < 0.0 || barycentric_u + barycentric_v > 1.0 {
        return None;
    }
    let distance = edge_2.dot(cross) * inverse_determinant;
    (distance > EPSILON).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_hits_triangle_in_front() {
        let ray = Ray { origin: Vec3::new(0.25, 0.25, -5.0), direction: Vec3::Z };
        let distance = intersect_triangle(&ray, Vec3::ZERO, Vec3::X, Vec3::Y).unwrap();
        assert!((distance - 5.0).abs() < 1e-5);
    }

    #[test]
    fn ray_misses_triangle_outside() {
        let ray = Ray { origin: Vec3::new(2.0, 2.0, -5.0), direction: Vec3::Z };
        assert!(intersect_triangle(&ray, Vec3::ZERO, Vec3::X, Vec3::Y).is_none());
    }

    #[test]
    fn ray_hits_sphere() {
        let ray = Ray { origin: Vec3::new(0.0, 0.0, -10.0), direction: Vec3::Z };
        let distance = intersect_sphere(&ray, Vec3::ZERO, 1.0).unwrap();
        assert!((distance - 9.0).abs() < 1e-5);
    }
}

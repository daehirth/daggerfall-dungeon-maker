//! Editable scene representation of one RDB room block.
//!
//! A [`Scene`] is a flat list of [`SceneObject`]s in world space, built from
//! a single RDB block placed at the world origin. Each object remembers which
//! of the block's object lists it came from so the outliner can group them.

use glam::{EulerRot, Mat4, Quat, Vec3};

use crate::arena2::{
    native_to_world, Arena2Library, RdbBlock, RdbObjectKind, TextureKey, NATIVE_ROTATION_UNITS_PER_DEGREE,
    WORLD_SCALE,
};

/// Texture archive containing editor markers (enemy spawns, start points, quest markers).
pub const MARKER_TEXTURE_ARCHIVE: u16 = 199;

/// Model of the door that leads out of a dungeon.
///
/// Many RDB blocks contain this door, but the game only keeps it in a dungeon's
/// starting block and removes it from every other block (as Daggerfall Unity does).
pub const EXIT_DOOR_MODEL_ID: u32 = 70300;

/// Group index given to objects the user places, so they are listed separately from the room's own lists.
pub const ADDED_OBJECTS_GROUP: usize = usize::MAX;

/// Name given to model objects until the user renames them, for example "Model 70300".
///
/// Objects that still have this name are listed under the model's
/// [display name](crate::custom_names::CustomNames::model_label) instead.
pub fn default_model_object_name(model_id: u32) -> String {
    format!("Model {model_id}")
}

/// Human readable name of an object group.
pub fn group_label(group_index: usize) -> String {
    if group_index == ADDED_OBJECTS_GROUP {
        "Added objects".to_string()
    } else {
        format!("Object list {group_index}")
    }
}

/// What kind of thing a [`SceneObject`] represents.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneObjectKind {
    /// A 3D model from `ARCH3D.BSA`.
    Model {
        /// Model record identifier.
        model_id: u32,
        /// Whether the model is animated by an action (doors, levers, platforms).
        has_action: bool,
    },
    /// A camera facing billboard sprite.
    Flat {
        /// Texture shown on the billboard.
        texture: TextureKey,
    },
    /// A point light.
    Light {
        /// Radius of influence in world units.
        radius: f32,
    },
}

/// A single editable object.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneObject {
    /// Display name used in the outliner.
    pub name: String,
    /// Index of the RDB object list the object was loaded from.
    pub group_index: usize,
    /// Object type and type specific data.
    pub kind: SceneObjectKind,
    /// World space position.
    pub position: Vec3,
    /// Rotation in degrees around the X, Y and Z axes (applied Y, then X, then Z).
    pub rotation_degrees: Vec3,
    /// Hidden objects are not drawn and cannot be picked in the viewport.
    pub visible: bool,
}

impl SceneObject {
    /// Rotation of the object as a quaternion.
    ///
    /// Daggerfall applies yaw first, then pitch, then roll, which matches
    /// the transform order `Rz * Rx * Ry` used by Daggerfall Unity.
    pub fn rotation(&self) -> Quat {
        let radians = self.rotation_degrees * (std::f32::consts::PI / 180.0);
        Quat::from_euler(EulerRot::ZXY, radians.z, radians.x, radians.y)
    }

    /// Local-to-world transform of the object.
    pub fn model_matrix(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation(), self.position)
    }

    /// Returns `true` for the door model that leads out of a dungeon.
    pub fn is_exit_door(&self) -> bool {
        matches!(self.kind, SceneObjectKind::Model { model_id, .. } if model_id == EXIT_DOOR_MODEL_ID)
    }

    /// Returns `true` for model objects the user has not renamed.
    pub fn has_default_model_name(&self) -> bool {
        matches!(self.kind, SceneObjectKind::Model { model_id, .. } if self.name == default_model_object_name(model_id))
    }

    /// Returns `true` for flats that use the editor marker archive.
    pub fn is_marker(&self) -> bool {
        matches!(self.kind, SceneObjectKind::Flat { texture } if texture.archive == MARKER_TEXTURE_ARCHIVE)
    }

    /// Short type label for the user interface.
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            SceneObjectKind::Model { has_action: true, .. } => "Action model",
            SceneObjectKind::Model { .. } => "Model",
            SceneObjectKind::Flat { .. } if self.is_marker() => "Marker",
            SceneObjectKind::Flat { .. } => "Flat",
            SceneObjectKind::Light { .. } => "Light",
        }
    }
}

/// A complete editable room.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// RDB file name the room was loaded from, for example `N0000004.RDB`.
    pub block_name: String,
    /// Every object in the room.
    pub objects: Vec<SceneObject>,
}

impl Scene {
    /// Builds a scene from an RDB block, placing the block's corner at the world origin.
    pub fn from_rdb_block(block: &RdbBlock) -> Self {
        let objects = block
            .objects
            .iter()
            .map(|object| {
                let (name, kind, rotation_degrees) = match &object.kind {
                    RdbObjectKind::Model(model) => (
                        default_model_object_name(model.model_id),
                        SceneObjectKind::Model { model_id: model.model_id, has_action: model.has_action },
                        // Native rotations turn the opposite way to world rotations. Subtracting
                        // from zero (rather than negating) avoids showing "-0" in the UI.
                        Vec3::ZERO - model.rotation.as_vec3() / NATIVE_ROTATION_UNITS_PER_DEGREE,
                    ),
                    RdbObjectKind::Flat(flat) => (
                        format!("Flat {}.{}", flat.texture.archive, flat.texture.record),
                        SceneObjectKind::Flat { texture: flat.texture },
                        Vec3::ZERO,
                    ),
                    RdbObjectKind::Light(light) => (
                        "Light".to_string(),
                        SceneObjectKind::Light { radius: f32::from(light.radius) * WORLD_SCALE },
                        Vec3::ZERO,
                    ),
                };

                SceneObject {
                    name,
                    group_index: object.group_index,
                    kind,
                    position: native_to_world(object.position),
                    rotation_degrees,
                    visible: true,
                }
            })
            .collect();

        Self { block_name: block.name.clone(), objects }
    }

    /// Removes every dungeon exit door, as the game does for blocks that are not
    /// the dungeon's starting block. Returns how many doors were removed.
    pub fn remove_exit_doors(&mut self) -> usize {
        let count_before = self.objects.len();
        self.objects.retain(|object| !object.is_exit_door());
        count_before - self.objects.len()
    }

    /// Sorted, de-duplicated list of the object group indices used in the room.
    pub fn group_indices(&self) -> Vec<usize> {
        let mut groups: Vec<usize> = self.objects.iter().map(|object| object.group_index).collect();
        groups.sort_unstable();
        groups.dedup();
        groups
    }

    /// Computes world space bounds of every visible object.
    ///
    /// Model extents come from their meshes; flats and lights contribute their position.
    pub fn bounds(&self, library: &mut Arena2Library) -> Option<(Vec3, Vec3)> {
        let mut bounds: Option<(Vec3, Vec3)> = None;
        let mut include = |point: Vec3| {
            bounds = Some(match bounds {
                Some((minimum, maximum)) => (minimum.min(point), maximum.max(point)),
                None => (point, point),
            });
        };

        for object in self.objects.iter().filter(|object| object.visible) {
            match object.kind {
                SceneObjectKind::Model { model_id, .. } => match library.model(model_id) {
                    Some(mesh) => {
                        let matrix = object.model_matrix();
                        for corner in box_corners(mesh.bounds_min, mesh.bounds_max) {
                            include(matrix.transform_point3(corner));
                        }
                    }
                    None => include(object.position),
                },
                SceneObjectKind::Flat { .. } | SceneObjectKind::Light { .. } => include(object.position),
            }
        }
        bounds
    }
}

/// Returns the eight corners of an axis aligned box.
pub fn box_corners(minimum: Vec3, maximum: Vec3) -> [Vec3; 8] {
    [
        Vec3::new(minimum.x, minimum.y, minimum.z),
        Vec3::new(maximum.x, minimum.y, minimum.z),
        Vec3::new(minimum.x, maximum.y, minimum.z),
        Vec3::new(maximum.x, maximum.y, minimum.z),
        Vec3::new(minimum.x, minimum.y, maximum.z),
        Vec3::new(maximum.x, minimum.y, maximum.z),
        Vec3::new(minimum.x, maximum.y, maximum.z),
        Vec3::new(maximum.x, maximum.y, maximum.z),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_matches_daggerfall_unity_order() {
        let object = SceneObject {
            name: String::new(),
            group_index: 0,
            kind: SceneObjectKind::Light { radius: 1.0 },
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation_degrees: Vec3::new(30.0, 45.0, 60.0),
            visible: true,
        };
        let radians = object.rotation_degrees * (std::f32::consts::PI / 180.0);
        let expected = Mat4::from_translation(object.position)
            * Mat4::from_rotation_z(radians.z)
            * Mat4::from_rotation_x(radians.x)
            * Mat4::from_rotation_y(radians.y);
        assert!(object.model_matrix().abs_diff_eq(expected, 1e-5));
    }

    #[test]
    fn remove_exit_doors_keeps_other_models() {
        let model = |model_id| SceneObject {
            name: format!("Model {model_id}"),
            group_index: 0,
            kind: SceneObjectKind::Model { model_id, has_action: false },
            position: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            visible: true,
        };
        let mut scene = Scene {
            block_name: "N0000000.RDB".to_string(),
            objects: vec![model(EXIT_DOOR_MODEL_ID), model(1000), model(EXIT_DOOR_MODEL_ID)],
        };

        assert_eq!(scene.remove_exit_doors(), 2);
        assert_eq!(scene.objects, vec![model(1000)]);
    }
}

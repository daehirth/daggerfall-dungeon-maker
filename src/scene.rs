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
use crate::encounters::Enemy;

/// Texture archive containing editor markers (enemy spawns, start points, quest markers).
pub const MARKER_TEXTURE_ARCHIVE: u16 = 199;

/// Record of the start marker. Every cell has one; it also holds the cell's water level.
pub const START_MARKER_RECORD: u8 = 10;
/// Native units of water depth per step of the start marker's water byte.
const WATER_LEVEL_STEP: i32 = 8;

/// Record of the editor marker that the game replaces with a random enemy.
pub const RANDOM_ENEMY_MARKER_RECORD: u8 = 15;

/// Record of the editor marker that the game replaces with the enemy stored in it.
pub const FIXED_ENEMY_MARKER_RECORD: u8 = 16;
/// Enemy id that switches a fixed enemy marker off.
pub const NO_ENEMY_ID: u8 = 99;
/// Action value that makes a fixed enemy passive.
const PASSIVE_ACTION: u8 = 99;
/// Flag bits giving a human enemy's gender.
const FEMALE_FLAG: u16 = 1;
const MALE_FLAG: u16 = 2;

/// Extra data stored with a flat in an RDB block. Kept as read so it is not
/// lost when a cell is copied and saved; for fixed enemy markers it says which
/// enemy appears (see [`FixedEnemy`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlatData {
    /// Raw flag bits; the gender of a human enemy.
    pub flags: u16,
    /// For fixed enemy markers, the enemy id (low byte); for other flats, a faction id.
    pub faction_or_mobile_id: u16,
    /// Action byte; 99 makes a fixed enemy passive.
    pub action: u8,
}

/// The gender of a human enemy, as stored in a fixed enemy marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnemyGender {
    /// The game picks one.
    Either,
    Female,
    Male,
}

impl EnemyGender {
    pub const ALL: [Self; 3] = [Self::Either, Self::Female, Self::Male];

    pub fn name(self) -> &'static str {
        match self {
            Self::Either => "Either",
            Self::Female => "Female",
            Self::Male => "Male",
        }
    }

    /// The gender stored in a flat's flag bits.
    fn from_flags(flags: u16) -> Self {
        if flags & FEMALE_FLAG != 0 {
            Self::Female
        } else if flags & MALE_FLAG != 0 {
            Self::Male
        } else {
            Self::Either
        }
    }

    /// The flag bits for this gender.
    fn flags(self) -> u16 {
        match self {
            Self::Either => 0,
            Self::Female => FEMALE_FLAG,
            Self::Male => MALE_FLAG,
        }
    }
}

/// What a fixed enemy marker spawns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedEnemy {
    /// Enemy id; only the low byte is used, the game ignores the rest. 99 spawns nothing.
    pub mobile_id: u8,
    /// Passive enemies only attack when provoked.
    pub passive: bool,
    /// Gender of a human enemy; ignored for monsters.
    pub gender: EnemyGender,
}

impl FixedEnemy {
    /// The enemy, or `None` if the marker is switched off or names an unknown enemy.
    pub fn enemy(self) -> Option<Enemy> {
        (!self.is_switched_off()).then(|| Enemy::from_mobile_id(u16::from(self.mobile_id))).flatten()
    }

    /// Whether the marker spawns nothing (enemy id 99).
    pub fn is_switched_off(self) -> bool {
        self.mobile_id == NO_ENEMY_ID
    }

    /// What appears, for example "Orc Sergeant".
    pub fn description(self) -> String {
        match self.enemy() {
            Some(enemy) => enemy.name().to_string(),
            None if self.is_switched_off() => "Nothing (marker switched off)".to_string(),
            None => format!("Unknown enemy {}", self.mobile_id),
        }
    }

    /// Gender of a human enemy; `None` for monsters, which have none.
    pub fn human_gender(self) -> Option<EnemyGender> {
        self.enemy().filter(|enemy| enemy.is_human()).map(|_| self.gender)
    }
}

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
        /// Extra data kept from the game data, such as the enemy of a fixed enemy marker.
        data: FlatData,
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
        matches!(self.kind, SceneObjectKind::Flat { texture, .. } if texture.archive == MARKER_TEXTURE_ARCHIVE)
    }

    /// Returns `true` for markers the game replaces with a random enemy chosen by dungeon type.
    pub fn is_random_enemy_marker(&self) -> bool {
        matches!(self.kind, SceneObjectKind::Flat { texture, .. }
            if texture.archive == MARKER_TEXTURE_ARCHIVE && texture.record == RANDOM_ENEMY_MARKER_RECORD)
    }

    /// What a fixed enemy marker spawns; `None` for every other object.
    pub fn fixed_enemy(&self) -> Option<FixedEnemy> {
        match self.kind {
            SceneObjectKind::Flat { texture, data }
                if texture.archive == MARKER_TEXTURE_ARCHIVE
                    && texture.record == FIXED_ENEMY_MARKER_RECORD =>
            {
                Some(FixedEnemy {
                    mobile_id: (data.faction_or_mobile_id & 0xff) as u8,
                    passive: data.action == PASSIVE_ACTION,
                    gender: EnemyGender::from_flags(data.flags),
                })
            }
            _ => None,
        }
    }

    /// Makes a fixed enemy marker spawn `fixed_enemy`. Does nothing for other objects.
    ///
    /// Only the bits that hold the enemy, behaviour and gender change; the rest
    /// of the marker's data is kept as the game data had it.
    pub fn set_fixed_enemy(&mut self, fixed_enemy: FixedEnemy) {
        if self.fixed_enemy().is_none() {
            return;
        }
        let SceneObjectKind::Flat { data, .. } = &mut self.kind else { return };
        data.faction_or_mobile_id = (data.faction_or_mobile_id & 0xff00) | u16::from(fixed_enemy.mobile_id);
        data.action = if fixed_enemy.passive { PASSIVE_ACTION } else { 0 };
        data.flags = (data.flags & !(FEMALE_FLAG | MALE_FLAG)) | fixed_enemy.gender.flags();
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
                        SceneObjectKind::Flat {
                            texture: flat.texture,
                            data: FlatData {
                                flags: flat.flags,
                                faction_or_mobile_id: flat.faction_or_mobile_id,
                                action: flat.action,
                            },
                        },
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

    /// Height of the cell's water surface in world units, or `None` if it has no water.
    ///
    /// As in the game, it comes from the cell's (first) start marker: a non-zero
    /// high data byte `n` puts the water at `8 * n` native units.
    pub fn water_level(&self) -> Option<f32> {
        let data = self.objects.iter().find_map(|object| match object.kind {
            SceneObjectKind::Flat { texture, data }
                if texture.archive == MARKER_TEXTURE_ARCHIVE && texture.record == START_MARKER_RECORD =>
            {
                Some(data)
            }
            _ => None,
        })?;
        let steps = i32::from(data.faction_or_mobile_id >> 8);
        (steps != 0).then(|| native_to_world(glam::IVec3::new(0, -WATER_LEVEL_STEP * steps, 0)).y)
    }

    /// Whether `object` is below the cell's water surface, where random enemy
    /// markers pick from the underwater table.
    pub fn is_under_water(&self, object: &SceneObject) -> bool {
        self.water_level().is_some_and(|water_level| object.position.y < water_level)
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
    fn fixed_enemy_markers_name_their_enemy() {
        let marker = |faction_or_mobile_id, action, flags| SceneObject {
            name: String::new(),
            group_index: 0,
            kind: SceneObjectKind::Flat {
                texture: TextureKey::new(MARKER_TEXTURE_ARCHIVE, FIXED_ENEMY_MARKER_RECORD),
                data: FlatData { flags, faction_or_mobile_id, action },
            },
            position: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            visible: true,
        };
        // The game ignores the high byte.
        let orc = marker(0x3507, 0, 0).fixed_enemy().unwrap();
        assert_eq!((orc.description(), orc.passive, orc.human_gender()), ("Orc".to_string(), false, None));
        let guard = marker(145, PASSIVE_ACTION, MALE_FLAG).fixed_enemy().unwrap();
        assert_eq!(
            (guard.description(), guard.passive, guard.human_gender()),
            ("Knight".to_string(), true, Some(EnemyGender::Male))
        );
        assert!(marker(99, 0, 0).fixed_enemy().unwrap().is_switched_off());
        assert_eq!(marker(80, 0, 0).fixed_enemy().unwrap().description(), "Unknown enemy 80");
    }

    #[test]
    fn setting_a_fixed_enemy_keeps_other_bits() {
        let mut object = SceneObject {
            name: String::new(),
            group_index: 0,
            kind: SceneObjectKind::Flat {
                texture: TextureKey::new(MARKER_TEXTURE_ARCHIVE, FIXED_ENEMY_MARKER_RECORD),
                data: FlatData { flags: 0x0100 | MALE_FLAG, faction_or_mobile_id: 0x3507, action: 0 },
            },
            position: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            visible: true,
        };
        let mage = FixedEnemy { mobile_id: 128, passive: true, gender: EnemyGender::Female };
        object.set_fixed_enemy(mage);
        assert_eq!(object.fixed_enemy(), Some(mage));
        assert_eq!(
            object.kind,
            SceneObjectKind::Flat {
                texture: TextureKey::new(MARKER_TEXTURE_ARCHIVE, FIXED_ENEMY_MARKER_RECORD),
                data: FlatData {
                    flags: 0x0100 | FEMALE_FLAG,
                    faction_or_mobile_id: 0x3580,
                    action: PASSIVE_ACTION
                },
            }
        );
    }

    #[test]
    fn water_level_comes_from_the_start_marker() {
        let flat = |record, faction_or_mobile_id, height| SceneObject {
            name: String::new(),
            group_index: 0,
            kind: SceneObjectKind::Flat {
                texture: TextureKey::new(MARKER_TEXTURE_ARCHIVE, record),
                data: FlatData { faction_or_mobile_id, ..FlatData::default() },
            },
            position: Vec3::new(0.0, height, 0.0),
            rotation_degrees: Vec3::ZERO,
            visible: true,
        };
        // A water byte of 5 puts the water 40 native units (1 m) up.
        let deep = flat(RANDOM_ENEMY_MARKER_RECORD, 0, 0.5);
        let dry = flat(RANDOM_ENEMY_MARKER_RECORD, 0, 1.5);
        let scene = Scene {
            block_name: String::new(),
            objects: vec![flat(START_MARKER_RECORD, 5 << 8, 0.0), deep.clone(), dry.clone()],
        };
        assert_eq!(scene.water_level(), Some(1.0));
        assert!(scene.is_under_water(&deep) && !scene.is_under_water(&dry));

        let dry_cell =
            Scene { block_name: String::new(), objects: vec![flat(START_MARKER_RECORD, 0, 0.0), deep] };
        assert_eq!(dry_cell.water_level(), None);
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

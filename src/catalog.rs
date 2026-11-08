//! The list of assets a user can place in a room, and descriptions of them.
//!
//! The catalog is computed at run time from the user's own game data when
//! the ARENA2 folder is loaded. Nothing is stored with the program: every
//! RDB block is scanned to find the models and flats Daggerfall actually uses
//! in its dungeon rooms, and ARCH3D is listed for the full model set.
//!
//! Daggerfall's models have numbers, not names, so each model also gets a
//! [`ModelDescription`] worked out from its data: whether its faces are drawn
//! with door textures, whether rooms attach actions to it (lifts, levers,
//! doors that swing), how big it is, and the names stored in the headers of
//! the texture archives it is drawn with ("Door Set", "Torture stuff", ...).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use glam::Vec3;

use crate::arena2::{Arena2Library, DoorKind, ModelMesh, RdbObjectKind, TextureKey};
use crate::scene::MARKER_TEXTURE_ARCHIVE;

/// Models at least this wide or deep (in world units) without doors or actions
/// are described as room pieces (walls, floors, corridors) rather than objects.
const ROOM_PIECE_MIN_EXTENT: f32 = 4.0;
/// Number of texture set names included in a model description.
const DESCRIBED_TEXTURE_SET_COUNT: usize = 2;
/// `TEXTURE.000` and `TEXTURE.001` hold plain palette colours rather than pictures.
const SOLID_COLOR_ARCHIVES: [u16; 2] = [0, 1];

/// Texture archive holding light source flats such as torches and braziers.
const LIGHT_SOURCE_FLAT_ARCHIVE: u16 = 210;
/// Texture archive holding animal flats.
const ANIMAL_FLAT_ARCHIVE: u16 = 201;
/// Texture archive holding fixed treasure flats.
const TREASURE_FLAT_ARCHIVE: u16 = 216;
/// Texture archives holding people (NPC) flats.
const PEOPLE_FLAT_ARCHIVES: [u16; 13] = [175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 334, 346, 357];

/// Known editor marker records in archive 199 and what the game uses them for.
const MARKER_NAMES: [(u8, &str); 8] = [
    (8, "Enter marker"),
    (10, "Start marker"),
    (11, "Quest marker"),
    (15, "Random enemy"),
    (16, "Fixed enemy"),
    (17, "Teleport marker"),
    (18, "Quest item marker"),
    (19, "Random treasure"),
];

/// A model that can be placed.
#[derive(Debug, Clone)]
pub struct ModelAsset {
    /// Record identifier in `ARCH3D.BSA`.
    pub model_id: u32,
    /// Number of different rooms that use the model.
    pub room_count: usize,
    /// Total number of times the model is placed across all rooms.
    pub placement_count: usize,
    /// Number of those placements that have an action attached (it moves or triggers something).
    pub action_placement_count: usize,
}

impl ModelAsset {
    /// Whether rooms mostly use the model as a moving or triggering part.
    ///
    /// A majority is required because rooms occasionally attach actions to
    /// ordinary walls and corridors, for example to make a secret passage.
    pub fn is_mostly_used_with_action(&self) -> bool {
        self.action_placement_count * 2 > self.placement_count
    }
}

/// What a model appears to be, worked out from its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCategory {
    /// The door that leads out of a dungeon.
    ExitDoor,
    /// The entrance of a dungeon, as seen from outside.
    DungeonEntrance,
    /// A door, which in dungeons usually swings open through an action.
    Door,
    /// A part that rooms attach actions to, such as a lift, lever or secret wall.
    MovingPart,
    /// A large piece of architecture: wall, floor, corridor or room shell.
    RoomPiece,
    /// Anything smaller: furniture, statues, clutter.
    Object,
}

impl ModelCategory {
    /// Human readable name of the category.
    pub fn label(self) -> &'static str {
        match self {
            Self::ExitDoor => "Exit door",
            Self::DungeonEntrance => "Dungeon entrance",
            Self::Door => "Door",
            Self::MovingPart => "Moving part",
            Self::RoomPiece => "Cell piece",
            Self::Object => "Object",
        }
    }

    /// Works out the category of `mesh`; `used_with_action` tells whether rooms mostly attach an action to it.
    fn of(mesh: &ModelMesh, used_with_action: bool) -> Self {
        let size = mesh.size();
        match mesh.door_kind() {
            Some(DoorKind::DungeonExit) => Self::ExitDoor,
            Some(DoorKind::DungeonEntrance) => Self::DungeonEntrance,
            Some(DoorKind::Building) => Self::Door,
            None if used_with_action => Self::MovingPart,
            None if size.x.max(size.z) >= ROOM_PIECE_MIN_EXTENT => Self::RoomPiece,
            None => Self::Object,
        }
    }
}

impl fmt::Display for ModelCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// A description of a model, worked out from the game data because models have no names.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDescription {
    /// What the model appears to be.
    pub category: ModelCategory,
    /// Names of the texture archives covering most of the model, most used first.
    pub texture_sets: Vec<String>,
    /// Width, height and depth in world units (metres).
    pub size: Vec3,
    /// Number of triangles in the model.
    pub triangle_count: usize,
}

impl ModelDescription {
    /// Describes `mesh`, looking up texture archive names in `library`.
    pub fn new(mesh: &ModelMesh, used_with_action: bool, library: &mut Arena2Library) -> Self {
        let mut texture_sets: Vec<String> = Vec::new();
        for (archive, _) in mesh.triangles_per_texture_archive() {
            if texture_sets.len() == DESCRIBED_TEXTURE_SET_COUNT {
                break;
            }
            // Plain palette colours say nothing about what the model is.
            if SOLID_COLOR_ARCHIVES.contains(&archive) {
                continue;
            }
            // Climate variants share a name ("Door Set" is archives 74, 174 and 374).
            if let Some(name) = library.texture_archive_name(archive) {
                if !texture_sets.iter().any(|known| known.eq_ignore_ascii_case(name)) {
                    texture_sets.push(name.to_string());
                }
            }
        }
        Self {
            category: ModelCategory::of(mesh, used_with_action),
            texture_sets,
            size: mesh.size(),
            triangle_count: mesh.triangle_count(),
        }
    }

    /// One line summary, for example "Door · Door Set · 1.6 × 2.8 × 0.2 m".
    pub fn summary(&self) -> String {
        let mut parts = vec![self.category.label().to_string()];
        if !self.texture_sets.is_empty() {
            parts.push(self.texture_sets.join(", "));
        }
        parts.push(format!("{:.1} × {:.1} × {:.1} m", self.size.x, self.size.y, self.size.z));
        parts.join(" · ")
    }

    /// Whether the category or a texture set name contains `lowercase_search`.
    pub fn matches(&self, lowercase_search: &str) -> bool {
        self.category.label().to_lowercase().contains(lowercase_search)
            || self.texture_sets.iter().any(|name| name.to_lowercase().contains(lowercase_search))
    }
}

/// A flat (billboard sprite) or editor marker that can be placed.
#[derive(Debug, Clone)]
pub struct FlatAsset {
    /// Texture shown by the flat.
    pub texture: TextureKey,
    /// Number of different rooms that use the flat.
    pub room_count: usize,
}

impl FlatAsset {
    /// Short human readable description of the flat.
    pub fn description(&self) -> String {
        flat_description(self.texture)
    }
}

/// Every placeable asset, grouped for the asset palette.
#[derive(Debug, Clone, Default)]
pub struct AssetCatalog {
    /// Models used in at least one dungeon room, most widely used first.
    pub room_models: Vec<ModelAsset>,
    /// Every model in `ARCH3D.BSA`, by id.
    pub all_model_ids: Vec<u32>,
    /// Non-marker flats used in at least one dungeon room, most widely used first.
    pub flats: Vec<FlatAsset>,
    /// Every editor marker record.
    pub markers: Vec<FlatAsset>,
    /// Position of each model in [`Self::room_models`], by model id.
    room_model_positions: HashMap<u32, usize>,
    /// Description of every model that could be decoded, by model id.
    model_descriptions: HashMap<u32, ModelDescription>,
}

/// How often one model is used across all rooms, while the catalog is being built.
#[derive(Default)]
struct ModelUsageCounter {
    rooms: BTreeSet<usize>,
    placement_count: usize,
    action_placement_count: usize,
}

impl AssetCatalog {
    /// Scans every RDB block in the library to build the catalog, and
    /// describes every model in `ARCH3D.BSA`.
    ///
    /// Blocks that fail to parse are skipped and logged.
    pub fn build(library: &mut Arena2Library) -> Self {
        let mut model_usage: BTreeMap<u32, ModelUsageCounter> = BTreeMap::new();
        let mut flat_usage: BTreeMap<TextureKey, BTreeSet<usize>> = BTreeMap::new();

        for (room_index, room_name) in library.rdb_block_names().iter().enumerate() {
            let block = match library.rdb_block(room_name) {
                Ok(block) => block,
                Err(error) => {
                    log::warn!("skipping {room_name} while building the asset catalog: {error}");
                    continue;
                }
            };
            for object in &block.objects {
                match &object.kind {
                    RdbObjectKind::Model(model) => {
                        let usage = model_usage.entry(model.model_id).or_default();
                        usage.rooms.insert(room_index);
                        usage.placement_count += 1;
                        if model.action.is_some() {
                            usage.action_placement_count += 1;
                        }
                    }
                    RdbObjectKind::Flat(flat) => {
                        flat_usage.entry(flat.texture).or_default().insert(room_index);
                    }
                    RdbObjectKind::Light(_) => {}
                }
            }
        }

        let mut room_models: Vec<ModelAsset> = model_usage
            .into_iter()
            .map(|(model_id, usage)| ModelAsset {
                model_id,
                room_count: usage.rooms.len(),
                placement_count: usage.placement_count,
                action_placement_count: usage.action_placement_count,
            })
            .collect();
        room_models.sort_by(|first, second| {
            second.room_count.cmp(&first.room_count).then(first.model_id.cmp(&second.model_id))
        });
        let room_model_positions =
            room_models.iter().enumerate().map(|(position, model)| (model.model_id, position)).collect();

        let mut flats: Vec<FlatAsset> = flat_usage
            .iter()
            .filter(|(texture, _)| texture.archive != MARKER_TEXTURE_ARCHIVE)
            .map(|(&texture, rooms)| FlatAsset { texture, room_count: rooms.len() })
            .collect();
        flats.sort_by(|first, second| {
            second.room_count.cmp(&first.room_count).then(first.texture.cmp(&second.texture))
        });

        let marker_record_count = library.texture_record_count(MARKER_TEXTURE_ARCHIVE);
        let markers = (0..marker_record_count.min(128) as u8)
            .map(|record| {
                let texture = TextureKey::new(MARKER_TEXTURE_ARCHIVE, record);
                let room_count = flat_usage.get(&texture).map_or(0, BTreeSet::len);
                FlatAsset { texture, room_count }
            })
            .collect();

        let mut catalog = Self {
            room_models,
            all_model_ids: library.model_ids(),
            flats,
            markers,
            room_model_positions,
            model_descriptions: HashMap::new(),
        };
        catalog.model_descriptions = catalog
            .all_model_ids
            .iter()
            .filter_map(|&model_id| {
                let mesh = library.read_model_uncached(model_id)?;
                let used_with_action =
                    catalog.room_model(model_id).is_some_and(ModelAsset::is_mostly_used_with_action);
                Some((model_id, ModelDescription::new(&mesh, used_with_action, library)))
            })
            .collect();
        catalog
    }

    /// How dungeon rooms use `model_id`, or `None` if no room uses it.
    pub fn room_model(&self, model_id: u32) -> Option<&ModelAsset> {
        self.room_model_positions.get(&model_id).map(|&position| &self.room_models[position])
    }

    /// The description worked out for `model_id`, or `None` if the model could not be decoded.
    pub fn model_description(&self, model_id: u32) -> Option<&ModelDescription> {
        self.model_descriptions.get(&model_id)
    }
}

/// Short human readable description of a flat texture.
pub fn flat_description(texture: TextureKey) -> String {
    if texture.archive == MARKER_TEXTURE_ARCHIVE {
        return MARKER_NAMES
            .iter()
            .find(|(record, _)| *record == texture.record)
            .map_or_else(|| format!("Editor marker {}", texture.record), |(_, name)| (*name).to_string());
    }
    let category = match texture.archive {
        LIGHT_SOURCE_FLAT_ARCHIVE => "Light source",
        ANIMAL_FLAT_ARCHIVE => "Animal",
        TREASURE_FLAT_ARCHIVE => "Treasure",
        archive if PEOPLE_FLAT_ARCHIVES.contains(&archive) => "Person",
        _ => "Flat",
    };
    format!("{category} {}.{}", texture.archive, texture.record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_known_flats() {
        assert_eq!(flat_description(TextureKey::new(199, 10)), "Start marker");
        assert_eq!(flat_description(TextureKey::new(199, 99)), "Editor marker 99");
        assert_eq!(flat_description(TextureKey::new(210, 3)), "Light source 210.3");
        assert_eq!(flat_description(TextureKey::new(500, 1)), "Flat 500.1");
    }
}

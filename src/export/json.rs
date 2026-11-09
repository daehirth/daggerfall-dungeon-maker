//! The world data JSON Daggerfall Unity reads, as Rust structures.
//!
//! Daggerfall Unity reads these files with FullSerializer into its own
//! `DFBlock`, `DFLocation` and `ModInfo` structures, matching fields by name.
//! Only the fields it uses are written; the rest keep their defaults. Enum
//! values are written by name, as FullSerializer writes them.

use std::collections::HashMap;

use serde::Serialize;

use super::{
    canonical_region_name, model_tag, native_radius, object_groups, object_position, NativePlacement,
};
use crate::arena2::{ClassicLocation, LocationExterior, LocationType, MapPixel};
use crate::scene::{SceneObject, SceneObjectKind};

/// Native units per map pixel, for a location's world position.
const NATIVE_UNITS_PER_MAP_PIXEL: i32 = 32_768;
/// Longitude and latitude units per map pixel.
const LONGITUDE_UNITS_PER_MAP_PIXEL: i32 = 128;
/// `IsExterior` value of a location's own record.
const EXTERIOR_RECORD: u16 = 32_768;
/// `POLITIC.PAK` value of a region: its index plus this.
const POLITIC_REGION_BASE: usize = 128;

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/// A `DFBlock` holding one RDB dungeon block.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Block {
    /// Where the block "is" in `BLOCKS.BSA`; Daggerfall Unity adds it to object
    /// positions to identify saved objects, so each block needs its own.
    pub position: i64,
    /// Left 0 so Daggerfall Unity gives the block the next free index.
    pub index: i32,
    pub name: String,
    #[serde(rename = "Type")]
    pub block_type: &'static str,
    pub rdb_block: RdbBlock,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RdbBlock {
    pub model_reference_list: Vec<ModelReference>,
    pub object_root_list: Vec<ObjectRoot>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ModelReference {
    /// The model id as five digits, for example `"09004"`.
    pub model_id: String,
    pub model_id_num: u32,
    /// Three letter tag, for example `DOR` for an opening door.
    pub description: String,
}

/// One of the block's object lists.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ObjectRoot {
    /// `None` (null) for an empty list, as Daggerfall Unity expects.
    pub rdb_objects: Option<Vec<RdbObject>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RdbObject {
    /// Identifies the object in its block; actions refer to objects by it.
    pub position: i32,
    /// Index in its list.
    pub index: i32,
    pub x_pos: i32,
    pub y_pos: i32,
    pub z_pos: i32,
    #[serde(rename = "Type")]
    pub object_type: &'static str,
    pub resources: Resources,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Resources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_resource: Option<ModelResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flat_resource: Option<FlatResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light_resource: Option<LightResource>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ModelResource {
    pub x_rotation: i32,
    pub y_rotation: i32,
    pub z_rotation: i32,
    /// Index into the block's model reference list.
    pub model_index: u16,
    #[serde(rename = "TriggerFlag_StartingLock")]
    pub trigger_flag_starting_lock: u32,
    pub sound_index: u8,
    pub action_resource: ActionResource,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ActionResource {
    pub position: i64,
    pub axis: u8,
    pub duration: u16,
    pub magnitude: u16,
    /// Position of the object the action sets off next, or -1.
    pub next_object_offset: i32,
    /// Position of the object whose action sets this one off, or -1.
    pub previous_object_offset: i32,
    pub next_object_index: i32,
    /// The action type; 0 means the model has no action.
    pub flags: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct FlatResource {
    pub position: i64,
    pub texture_archive: i32,
    pub texture_record: i32,
    pub flags: u16,
    /// Low byte of `faction_or_mobile_id`.
    pub magnitude: u8,
    /// High byte of `faction_or_mobile_id`.
    pub sound_index: u8,
    pub faction_or_mobile_id: u16,
    /// Position of the object the flat's action sets off, or -1.
    pub next_object_offset: i32,
    pub action: u8,
    pub is_custom_data: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct LightResource {
    pub unknown1: u32,
    pub unknown2: u32,
    pub radius: u16,
}

impl Block {
    /// The block called `name` made from a cell's `objects`.
    ///
    /// Objects get positions in the order they are listed, and action links
    /// (by object id in the editor) become links by those positions. Models
    /// are listed with the tags Daggerfall's cells give them (`tags`).
    pub fn from_cell(
        name: &str,
        position: i64,
        objects: &[SceneObject],
        tags: &HashMap<u32, String>,
    ) -> Self {
        let position_of_id: HashMap<u32, i32> =
            objects.iter().enumerate().map(|(index, object)| (object.id, object_position(index))).collect();
        // Which model's action sets each object off, for its "previous" link.
        let mut previous: HashMap<i32, i32> = HashMap::new();
        for (index, object) in objects.iter().enumerate() {
            if !object.has_action() {
                continue;
            }
            if let Some(target) = object.next.and_then(|id| position_of_id.get(&id)) {
                previous.entry(*target).or_insert(object_position(index));
            }
        }

        let mut model_references: Vec<ModelReference> = Vec::new();
        let mut model_index_of: HashMap<u32, u16> = HashMap::new();
        let mut model_index = |model_id: u32| -> u16 {
            *model_index_of.entry(model_id).or_insert_with(|| {
                model_references.push(ModelReference {
                    model_id: format!("{model_id:05}"),
                    model_id_num: model_id,
                    description: model_tag(model_id, tags),
                });
                (model_references.len() - 1) as u16
            })
        };

        let mut roots = Vec::new();
        for group in object_groups(objects) {
            let mut list = Vec::with_capacity(group.len());
            for (index_in_list, &index) in group.iter().enumerate() {
                let object = &objects[index];
                let own_position = object_position(index);
                let next = object.next.and_then(|id| position_of_id.get(&id)).copied().unwrap_or(-1);
                let placement = NativePlacement::of(object);
                let (object_type, resources) = match &object.kind {
                    SceneObjectKind::Model { model_id, data } => {
                        let action = match data.action {
                            Some(action) => ActionResource {
                                position: i64::from(own_position) + 1,
                                axis: action.axis,
                                duration: action.duration,
                                magnitude: action.magnitude,
                                next_object_offset: next,
                                previous_object_offset: previous.get(&own_position).copied().unwrap_or(-1),
                                next_object_index: -1,
                                flags: i32::from(action.action_type),
                            },
                            None => ActionResource {
                                next_object_offset: -1,
                                previous_object_offset: previous.get(&own_position).copied().unwrap_or(-1),
                                next_object_index: -1,
                                ..ActionResource::default()
                            },
                        };
                        (
                            "Model",
                            Resources {
                                model_resource: Some(ModelResource {
                                    x_rotation: placement.rotation.x,
                                    y_rotation: placement.rotation.y,
                                    z_rotation: placement.rotation.z,
                                    model_index: model_index(*model_id),
                                    trigger_flag_starting_lock: data.trigger_and_lock,
                                    sound_index: data.sound,
                                    action_resource: action,
                                }),
                                flat_resource: None,
                                light_resource: None,
                            },
                        )
                    }
                    SceneObjectKind::Flat { texture, data } => {
                        let [low, high] = data.faction_or_mobile_id.to_le_bytes();
                        (
                            "Flat",
                            Resources {
                                model_resource: None,
                                flat_resource: Some(FlatResource {
                                    position: i64::from(own_position) + 1,
                                    texture_archive: i32::from(texture.archive),
                                    texture_record: i32::from(texture.record),
                                    flags: data.flags,
                                    magnitude: low,
                                    sound_index: high,
                                    faction_or_mobile_id: data.faction_or_mobile_id,
                                    next_object_offset: next,
                                    action: data.action,
                                    is_custom_data: false,
                                }),
                                light_resource: None,
                            },
                        )
                    }
                    SceneObjectKind::Light { radius, data } => (
                        "Light",
                        Resources {
                            model_resource: None,
                            flat_resource: None,
                            light_resource: Some(LightResource {
                                unknown1: data.unknown_1,
                                unknown2: data.unknown_2,
                                radius: native_radius(*radius),
                            }),
                        },
                    ),
                };
                list.push(RdbObject {
                    position: own_position,
                    index: index_in_list as i32,
                    x_pos: placement.position.x,
                    y_pos: placement.position.y,
                    z_pos: placement.position.z,
                    object_type,
                    resources,
                });
            }
            roots.push(ObjectRoot { rdb_objects: (!list.is_empty()).then_some(list) });
        }

        Self {
            position,
            index: 0,
            name: name.to_string(),
            block_type: "Rdb",
            rdb_block: RdbBlock { model_reference_list: model_references, object_root_list: roots },
        }
    }
}

// ---------------------------------------------------------------------------
// Locations
// ---------------------------------------------------------------------------

/// A `DFLocation`: a location with a dungeon.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Location {
    pub loaded: bool,
    pub name: String,
    pub region_name: String,
    pub has_dungeon: bool,
    pub map_table_data: MapTableData,
    pub exterior: Exterior,
    pub dungeon: Dungeon,
    pub climate: Climate,
    pub politic: i32,
    pub region_index: i32,
    /// Daggerfall Unity gives a new location the next free index itself.
    pub location_index: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct MapTableData {
    /// The location's map pixel id (`y × 1000 + x`).
    pub map_id: i32,
    pub latitude: i32,
    pub longitude: i32,
    pub location_type: &'static str,
    pub dungeon_type: &'static str,
    pub discovered: bool,
    pub key: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RecordElement {
    pub header: RecordHeader,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RecordHeader {
    /// World position in native units.
    pub x: i32,
    pub y: i32,
    pub is_exterior: u16,
    pub unknown2: u32,
    pub location_id: u16,
    pub is_interior: u16,
    pub exterior_location_id: u32,
    pub location_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Exterior {
    pub record_element: RecordElement,
    pub building_count: u16,
    pub buildings: Vec<Building>,
    pub exterior_data: ExteriorData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Building {
    pub name_seed: u16,
    pub faction_id: u16,
    pub sector: i16,
    pub location_id: u16,
    pub building_type: BuildingType,
    pub quality: u8,
}

/// A building type, by name where Daggerfall Unity names it.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum BuildingType {
    Named(&'static str),
    Number(u8),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExteriorData {
    pub another_name: String,
    pub map_id: i32,
    pub location_id: u32,
    pub width: u8,
    pub height: u8,
    pub port_town_and_unknown: u8,
    pub block_names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Dungeon {
    pub record_element: RecordElement,
    pub header: DungeonHeader,
    pub blocks: Vec<DungeonBlock>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DungeonHeader {
    pub block_count: u16,
}

/// One block on a dungeon's grid.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DungeonBlock {
    pub x: i8,
    pub z: i8,
    pub is_starting_block: bool,
    pub block_name: String,
    /// Worked out by Daggerfall Unity from the block's start marker.
    pub water_level: i16,
    pub castle_block: bool,
}

/// What decides a location's ground, trees, sky and people.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Climate {
    pub world_climate: i32,
    pub climate_type: &'static str,
    pub nature_set: &'static str,
    pub ground_archive: i32,
    pub nature_archive: i32,
    pub sky_base: i32,
    pub people: &'static str,
    pub names: &'static str,
}

/// What a new dungeon location is made from.
pub struct NewDungeon<'a> {
    pub name: &'a str,
    pub region_index: usize,
    pub pixel: MapPixel,
    pub location_type: LocationType,
    /// Dungeon type number.
    pub dungeon_type: u8,
    pub discovered: bool,
    /// Id of the location; its dungeon gets the next one. Ignored when it
    /// replaces a location, which keeps its own.
    pub location_id: u16,
    /// The game's own location this one replaces, keeping its index, ids and
    /// position; `None` for a new location.
    pub replaces: Option<&'a ClassicLocation>,
    /// The value in the dungeon's record that decides its music (see [`crate::music`]).
    pub music_seed: u32,
    /// `CLIMATE.PAK` value at the location, if known.
    pub climate: Option<u8>,
    /// The entrance above ground, copied from a Daggerfall dungeon.
    pub exterior: &'a LocationExterior,
    pub blocks: Vec<DungeonBlock>,
}

impl Location {
    /// A location holding a dungeon: a new one, or one replacing a location
    /// of the game, which keeps what identifies it.
    pub fn new_dungeon(dungeon: NewDungeon<'_>) -> Self {
        let NewDungeon {
            name,
            region_index,
            pixel,
            location_type,
            dungeon_type,
            discovered,
            location_id,
            replaces,
            music_seed,
            climate,
            exterior,
            blocks,
        } = dungeon;
        let pixel_id = pixel.y * 1000 + pixel.x;
        let (map_id, longitude, latitude, key, x, y) = match replaces {
            Some(classic) => (
                classic.map_id,
                classic.longitude,
                classic.latitude,
                classic.key,
                classic.world_x,
                classic.world_y,
            ),
            None => (
                pixel_id,
                pixel.x * LONGITUDE_UNITS_PER_MAP_PIXEL,
                (crate::arena2::MAP_HEIGHT - 1 - pixel.y) * LONGITUDE_UNITS_PER_MAP_PIXEL,
                0,
                pixel.x * NATIVE_UNITS_PER_MAP_PIXEL,
                (crate::arena2::MAP_HEIGHT - 1 - pixel.y) * NATIVE_UNITS_PER_MAP_PIXEL,
            ),
        };
        let (location_id, dungeon_location_id) = match replaces {
            Some(classic) => (classic.location_id, classic.dungeon_location_id),
            None => (location_id, location_id + 1),
        };
        let location_index = replaces.map_or(0, |classic| classic.location_index);
        let header = |location_id: u16, exterior_location_id: u32, is_exterior: bool| RecordHeader {
            x,
            y,
            is_exterior: if is_exterior { EXTERIOR_RECORD } else { 0 },
            // The location's own record holds its index here, as the game's do;
            // the dungeon's holds the value that picks its music.
            unknown2: if is_exterior { location_index as u32 } else { music_seed },
            location_id,
            is_interior: u16::from(!is_exterior),
            exterior_location_id,
            location_name: name.to_string(),
        };
        let buildings: Vec<Building> = exterior
            .buildings
            .iter()
            .map(|building| Building {
                name_seed: building.name_seed,
                faction_id: building.faction_id,
                sector: building.sector,
                location_id,
                building_type: building_type(building.building_type),
                quality: building.quality,
            })
            .collect();
        Self {
            loaded: true,
            name: name.to_string(),
            region_name: canonical_region_name(region_index).to_string(),
            has_dungeon: true,
            map_table_data: MapTableData {
                map_id,
                latitude,
                longitude,
                location_type: location_type_name(location_type),
                dungeon_type: dungeon_type_name(dungeon_type),
                discovered,
                key,
            },
            exterior: Exterior {
                record_element: RecordElement { header: header(location_id, 0, true) },
                building_count: buildings.len() as u16,
                buildings,
                exterior_data: ExteriorData {
                    another_name: name.to_string(),
                    map_id: pixel_id,
                    location_id: 0,
                    width: exterior.width as u8,
                    height: exterior.height as u8,
                    port_town_and_unknown: 0,
                    block_names: exterior.block_names.clone(),
                },
            },
            dungeon: Dungeon {
                record_element: RecordElement {
                    header: header(dungeon_location_id, u32::from(location_id), false),
                },
                header: DungeonHeader { block_count: blocks.len() as u16 },
                blocks,
            },
            climate: Climate::of(climate),
            politic: (region_index + POLITIC_REGION_BASE) as i32,
            region_index: region_index as i32,
            location_index: location_index as i32,
        }
    }
}

impl Climate {
    /// The climate settings Daggerfall Unity gives a location with `CLIMATE.PAK`
    /// value `world_climate` (its `MapsFile.GetWorldClimateSettings`).
    /// Unknown values, and `None`, get temperate woodland.
    pub fn of(world_climate: Option<u8>) -> Self {
        let value = world_climate.map_or(231, i32::from);
        let (climate_type, ground_archive, nature_set, nature_archive, sky_base, people) = match value {
            // Ocean.
            223 => ("Swamp", 402, "Nature_TemperateWoodland", 504, 24, "Breton"),
            // Desert.
            224 | 225 => ("Desert", 2, "Nature_Desert", 503, 8, "Redguard"),
            226 => ("Mountain", 102, "Nature_Mountains", 510, 0, "Nord"),
            // Rainforest.
            227 => ("Swamp", 402, "Nature_RainForest", 500, 24, "Redguard"),
            228 => ("Swamp", 402, "Nature_Swamp", 502, 24, "Breton"),
            // Subtropical.
            229 => ("Desert", 2, "Nature_SubTropical", 501, 24, "Breton"),
            // Mountain woods.
            230 => ("Temperate", 102, "Nature_WoodlandHills", 506, 16, "Breton"),
            // Haunted woodlands.
            232 => ("Temperate", 302, "Nature_HauntedWoodlands", 508, 16, "Breton"),
            // Woodlands, and anything else.
            _ => ("Temperate", 302, "Nature_TemperateWoodland", 504, 16, "Breton"),
        };
        Self {
            world_climate: value,
            climate_type,
            nature_set,
            ground_archive,
            nature_archive,
            sky_base,
            people,
            // Daggerfall Unity leaves this at its default.
            names: "Nord",
        }
    }
}

/// Daggerfall Unity's name for a location type (`DFRegion.LocationTypes`).
fn location_type_name(location_type: LocationType) -> &'static str {
    const NAMES: [&str; 15] = [
        "TownCity",
        "TownHamlet",
        "TownVillage",
        "HomeFarms",
        "DungeonLabyrinth",
        "ReligionTemple",
        "Tavern",
        "DungeonKeep",
        "HomeWealthy",
        "ReligionCult",
        "DungeonRuin",
        "HomePoor",
        "Graveyard",
        "Coven",
        "HomeYourShips",
    ];
    NAMES.get(location_type.raw() as usize).copied().unwrap_or("DungeonKeep")
}

/// Daggerfall Unity's name for dungeon type `value` (`DFRegion.DungeonTypes`).
fn dungeon_type_name(value: u8) -> &'static str {
    const NAMES: [&str; 19] = [
        "Crypt",
        "OrcStronghold",
        "HumanStronghold",
        "Prison",
        "DesecratedTemple",
        "Mine",
        "NaturalCave",
        "Coven",
        "VampireHaunt",
        "Laboratory",
        "HarpyNest",
        "RuinedCastle",
        "SpiderNest",
        "GiantStronghold",
        "DragonsDen",
        "BarbarianStronghold",
        "VolcanicCaves",
        "ScorpionNest",
        "Cemetery",
    ];
    NAMES.get(usize::from(value)).copied().unwrap_or("NoDungeon")
}

/// Daggerfall Unity's name for building type `value` (`DFLocation.BuildingTypes`).
fn building_type(value: u8) -> BuildingType {
    const NAMES: [&str; 25] = [
        "Alchemist",
        "HouseForSale",
        "Armorer",
        "Bank",
        "Town4",
        "Bookseller",
        "ClothingStore",
        "FurnitureStore",
        "GemStore",
        "GeneralStore",
        "Library",
        "GuildHall",
        "PawnShop",
        "WeaponSmith",
        "Temple",
        "Tavern",
        "Palace",
        "House1",
        "House2",
        "House3",
        "House4",
        "House5",
        "House6",
        "Town23",
        "Ship",
    ];
    match value {
        0x74 => BuildingType::Named("Special1"),
        0xdf => BuildingType::Named("Special2"),
        0xf9 => BuildingType::Named("Special3"),
        0xfa => BuildingType::Named("Special4"),
        _ => NAMES
            .get(usize::from(value))
            .map_or(BuildingType::Number(value), |name| BuildingType::Named(name)),
    }
}

// ---------------------------------------------------------------------------
// Mod manifest
// ---------------------------------------------------------------------------

/// A mod's manifest (`<name>.dfmod.json`): Daggerfall Unity's `ModInfo`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Manifest {
    pub mod_title: String,
    pub mod_version: String,
    pub mod_author: String,
    pub contact_info: String,
    #[serde(rename = "DFUnity_Version")]
    pub dfunity_version: String,
    pub mod_description: String,
    #[serde(rename = "GUID")]
    pub guid: String,
    /// Every file in the mod, by its path in the Unity project that builds it.
    pub files: Vec<String>,
    pub dependencies: Vec<String>,
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::arena2::TextureKey;
    use crate::scene::{FlatData, LightData, ModelData, ObjectAction, ADDED_OBJECTS_GROUP};

    fn object(id: u32, group_index: usize, kind: SceneObjectKind) -> SceneObject {
        SceneObject {
            id,
            name: String::new(),
            group_index,
            kind,
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation_degrees: Vec3::new(0.0, 90.0, 0.0),
            visible: true,
            next: None,
        }
    }

    #[test]
    fn writes_blocks_as_daggerfall_unity_reads_them() {
        let mut lever = object(
            7,
            0,
            SceneObjectKind::Model {
                model_id: 9004,
                data: ModelData {
                    trigger_and_lock: 2,
                    sound: 3,
                    action: Some(ObjectAction { action_type: 1, axis: 4, duration: 20, magnitude: 64 }),
                },
            },
        );
        lever.next = Some(9);
        let door = object(
            9,
            ADDED_OBJECTS_GROUP,
            SceneObjectKind::Model { model_id: 55004, data: ModelData::default() },
        );
        let marker = object(
            3,
            0,
            SceneObjectKind::Flat {
                texture: TextureKey::new(199, 16),
                data: FlatData { flags: 2, faction_or_mobile_id: 0x0105, action: 0 },
            },
        );
        let light = object(
            4,
            0,
            SceneObjectKind::Light { radius: 10.0, data: LightData { unknown_1: 5, unknown_2: 6 } },
        );
        let tags = HashMap::from([(55004, "DOR".to_string())]);
        let block = Block::from_cell("N9000001.RDB", 42, &[lever, door, marker, light], &tags);
        let value = serde_json::to_value(&block).unwrap();

        assert_eq!(value["Name"], "N9000001.RDB");
        assert_eq!(value["Type"], "Rdb");
        let models = &value["RdbBlock"]["ModelReferenceList"];
        assert_eq!(models[0]["ModelId"], "09004");
        assert_eq!(models[0]["Description"], "XXX");
        assert_eq!(models[1]["ModelIdNum"], 55004);
        assert_eq!(models[1]["Description"], "DOR");

        let roots = value["RdbBlock"]["ObjectRootList"].as_array().unwrap();
        assert_eq!(roots.len(), 2, "the cell's list, then the added objects");
        let lever = &roots[0]["RdbObjects"][0];
        assert_eq!(lever["Type"], "Model");
        assert_eq!(
            (lever["XPos"].clone(), lever["YPos"].clone(), lever["ZPos"].clone()),
            (40.into(), (-80).into(), 120.into())
        );
        let resource = &lever["Resources"]["ModelResource"];
        assert_eq!(resource["YRotation"], -512);
        assert_eq!(resource["TriggerFlag_StartingLock"], 2);
        assert_eq!(resource["ActionResource"]["Flags"], 1);
        assert!(lever["Resources"].get("FlatResource").is_none());

        // The lever's action points at the door by the door's position, and back.
        let door = &roots[1]["RdbObjects"][0];
        assert_eq!(resource["ActionResource"]["NextObjectOffset"], door["Position"]);
        assert_eq!(
            door["Resources"]["ModelResource"]["ActionResource"]["PreviousObjectOffset"],
            lever["Position"]
        );
        assert_eq!(door["Resources"]["ModelResource"]["ActionResource"]["Flags"], 0);
        assert_eq!(door["Resources"]["ModelResource"]["ModelIndex"], 1);

        let marker = &roots[0]["RdbObjects"][1]["Resources"]["FlatResource"];
        assert_eq!(
            (marker["TextureArchive"].clone(), marker["TextureRecord"].clone()),
            (199.into(), 16.into())
        );
        assert_eq!((marker["Magnitude"].clone(), marker["SoundIndex"].clone()), (5.into(), 1.into()));
        assert_eq!(marker["NextObjectOffset"], -1);
        let light = &roots[0]["RdbObjects"][2]["Resources"]["LightResource"];
        assert_eq!(light["Radius"], 400);
    }

    #[test]
    fn writes_new_dungeon_locations() {
        let exterior = LocationExterior {
            width: 1,
            height: 1,
            block_names: vec!["RUINAA24.RMB".to_string()],
            buildings: Vec::new(),
        };
        let location = Location::new_dungeon(NewDungeon {
            name: "Test Hold",
            region_index: 16,
            pixel: MapPixel::new(938, 51),
            location_type: LocationType::MediumDungeon,
            dungeon_type: 1,
            discovered: true,
            location_id: 40_000,
            replaces: None,
            music_seed: 7,
            climate: Some(226),
            exterior: &exterior,
            blocks: vec![DungeonBlock {
                x: 0,
                z: 0,
                is_starting_block: true,
                block_name: "N9000001.RDB".to_string(),
                water_level: 0,
                castle_block: false,
            }],
        });
        let value = serde_json::to_value(&location).unwrap();
        // The same numbers a published Daggerfall Unity mod uses for this pixel.
        assert_eq!(value["MapTableData"]["MapId"], 51_938);
        assert_eq!(value["MapTableData"]["Latitude"], 57_344);
        assert_eq!(value["MapTableData"]["Longitude"], 120_064);
        assert_eq!(value["Exterior"]["RecordElement"]["Header"]["X"], 30_736_384);
        assert_eq!(value["Exterior"]["RecordElement"]["Header"]["Y"], 14_680_064);
        assert_eq!(value["MapTableData"]["LocationType"], "DungeonKeep");
        assert_eq!(value["MapTableData"]["DungeonType"], "OrcStronghold");
        assert_eq!(value["RegionName"], "Wrothgarian Mountains");
        assert_eq!(value["Politic"], 144);
        assert_eq!(value["Climate"]["ClimateType"], "Mountain");
        assert_eq!(value["Dungeon"]["RecordElement"]["Header"]["LocationId"], 40_001);
        assert_eq!(value["Dungeon"]["RecordElement"]["Header"]["Unknown2"], 7);
        assert_eq!(value["Dungeon"]["RecordElement"]["Header"]["ExteriorLocationId"], 40_000);
        assert_eq!(value["Dungeon"]["Header"]["BlockCount"], 1);
        assert_eq!(value["Dungeon"]["Blocks"][0]["IsStartingBlock"], true);
    }

    #[test]
    fn replacing_locations_keep_what_identifies_them() {
        let exterior = LocationExterior {
            width: 1,
            height: 1,
            block_names: vec!["RUINAA24.RMB".to_string()],
            buildings: Vec::new(),
        };
        // The numbers of Daggerfall's own "Ruins of The Caorchom Farmstead".
        let classic = ClassicLocation {
            location_index: 2,
            map_id: 2_484_750,
            longitude: 76_584,
            latitude: 14_376,
            discovered: false,
            key: 0,
            location_id: 26_219,
            dungeon_location_id: 26_220,
            world_x: 19_609_600,
            world_y: 3_684_353,
            dungeon_music_seed: 0,
        };
        let location = Location::new_dungeon(NewDungeon {
            name: "Ruins of The Caorchom Farmstead",
            region_index: 0,
            pixel: MapPixel::new(598, 387),
            location_type: LocationType::SmallDungeon,
            dungeon_type: 0,
            discovered: false,
            location_id: 40_000,
            replaces: Some(&classic),
            music_seed: 0,
            climate: Some(224),
            exterior: &exterior,
            blocks: Vec::new(),
        });
        let value = serde_json::to_value(&location).unwrap();
        assert_eq!(value["LocationIndex"], 2);
        assert_eq!(value["MapTableData"]["MapId"], 2_484_750);
        assert_eq!(value["MapTableData"]["Longitude"], 76_584);
        assert_eq!(value["Exterior"]["ExteriorData"]["MapId"], 387_598);
        let exterior_header = &value["Exterior"]["RecordElement"]["Header"];
        assert_eq!(exterior_header["LocationId"], 26_219);
        assert_eq!(exterior_header["Unknown2"], 2);
        assert_eq!(exterior_header["X"], 19_609_600);
        let dungeon_header = &value["Dungeon"]["RecordElement"]["Header"];
        assert_eq!(dungeon_header["LocationId"], 26_220);
        assert_eq!(dungeon_header["ExteriorLocationId"], 26_219);
    }
}

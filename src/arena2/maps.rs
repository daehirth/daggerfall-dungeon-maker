//! Reader for the pre-built dungeon layouts stored in `MAPS.BSA`.
//!
//! `MAPS.BSA` contains four records per region. The `MAPDITEM.###` record
//! lists every dungeon in the region together with the grid of RDB blocks it
//! is assembled from, which is all the viewer needs to rebuild a dungeon.
//! The dungeon's type (crypt, orc stronghold, …), which decides the enemies at
//! its random enemy markers, is stored separately in the region's `MAPTABLE`
//! record, one entry per location. Locations are matched up through the
//! location ids in `MAPPITEM`, as Daggerfall Unity does.

use std::collections::HashMap;

use super::bsa::BsaArchive;
use super::error::{Arena2Error, Arena2Result};
use super::reader::ByteReader;
use crate::encounters::DungeonType;

/// Region names in the order used by the `###` suffix of `MAPS.BSA` records.
pub const REGION_NAMES: [&str; 62] = [
    "Alik'r Desert",
    "Dragontail Mountains",
    "Glenpoint Foothills",
    "Daggerfall Bluffs",
    "Yeorth Burrowland",
    "Dwynnen",
    "Ravennian Forest",
    "Devilrock",
    "Malekna Forest",
    "Isle of Balfiera",
    "Bantha",
    "Dak'fron",
    "Islands in the Western Iliac Bay",
    "Tamarilyn Point",
    "Lainlyn Cliffs",
    "Bjoulsae River",
    "Wrothgarian Mountains",
    "Daggerfall",
    "Glenpoint",
    "Betony",
    "Sentinel",
    "Anticlere",
    "Lainlyn",
    "Wayrest",
    "Gen Tem High Rock village",
    "Gen Rai Hammerfell village",
    "Orsinium Area",
    "Skeffington Wood",
    "Hammerfell bay coast",
    "Hammerfell sea coast",
    "High Rock bay coast",
    "High Rock sea coast",
    "Northmoor",
    "Menevia",
    "Alcaire",
    "Koegria",
    "Bhoriane",
    "Kambria",
    "Phrygias",
    "Urvaius",
    "Ykalon",
    "Daenia",
    "Shalgora",
    "Abibon-Gora",
    "Kairou",
    "Pothago",
    "Myrkwasa",
    "Ayasofya",
    "Tigonus",
    "Kozanset",
    "Satakalaam",
    "Totambu",
    "Mournoth",
    "Ephesus",
    "Santaki",
    "Antiphyllos",
    "Bergama",
    "Gavaudon",
    "Tulune",
    "Glenumbra Moors",
    "Ilessan Hills",
    "Cybiades",
];

/// Leading letter of an RDB block file name, selected by the block type bits.
const RDB_BLOCK_LETTERS: [char; 6] = ['N', 'W', 'L', 'S', 'B', 'M'];
/// Size of one door entry in a location record.
const LOCATION_DOOR_SIZE: usize = 6;
/// Offset of the location id inside the location record header.
const LOCATION_ID_OFFSET: usize = 33;
/// Offset of the location name inside the location record header.
const LOCATION_NAME_OFFSET: usize = 71;
/// Length of the location name field.
const LOCATION_NAME_LENGTH: usize = 32;
/// Total size of the location record header.
const LOCATION_HEADER_SIZE: usize = 112;
/// Size of the dungeon header that follows the location record.
const DUNGEON_HEADER_SIZE: usize = 17;
/// Offset of the block count within the dungeon header.
const DUNGEON_BLOCK_COUNT_OFFSET: usize = 10;
/// Size of one location's entry in a `MAPTABLE` record.
const MAP_TABLE_ENTRY_SIZE: usize = 17;
/// Offset of the dungeon type within a `MAPTABLE` entry.
const MAP_TABLE_DUNGEON_TYPE_OFFSET: usize = 12;
/// Location id of Orsinium, which needs a manual fix for an overlapping block.
const ORSINIUM_LOCATION_ID: u16 = 50015;
/// Index of the Orsinium block that the game moves, and the row it moves it to.
const ORSINIUM_MOVED_BLOCK: usize = 13;
const ORSINIUM_MOVED_BLOCK_ROW: i32 = -2;

/// A dungeon defined in `MAPS.BSA`.
#[derive(Debug, Clone)]
pub struct DungeonDefinition {
    /// Index of the region the dungeon belongs to.
    pub region_index: usize,
    /// Human readable region name.
    pub region_name: &'static str,
    /// Dungeon name as shown in game.
    pub name: String,
    /// The kind of dungeon, which decides the enemies at random enemy markers; `None` if unknown.
    pub dungeon_type: Option<DungeonType>,
    /// Blocks making up the dungeon.
    pub blocks: Vec<DungeonBlockPlacement>,
}

/// One RDB block placed on the dungeon grid.
#[derive(Debug, Clone)]
pub struct DungeonBlockPlacement {
    /// Grid column; each step is one block width.
    pub grid_x: i32,
    /// Grid row; each step is one block width.
    pub grid_z: i32,
    /// Name of the RDB block in `BLOCKS.BSA`.
    pub block_name: String,
    /// Whether the player enters the dungeon in this block.
    pub is_starting_block: bool,
}

/// Reads every dungeon in every region of `MAPS.BSA`, sorted by region then name.
///
/// Regions that fail to parse are logged and skipped so one bad record does
/// not prevent the rest of the game data from being browsed.
pub fn read_all_dungeons(maps_archive: &BsaArchive) -> Vec<DungeonDefinition> {
    let mut dungeons = Vec::new();
    for (region_index, region_name) in REGION_NAMES.iter().enumerate() {
        let record_name = format!("MAPDITEM.{region_index:03}");
        let Some(record) = maps_archive.record_by_name(&record_name) else {
            continue;
        };
        let dungeon_types = read_dungeon_types(maps_archive, region_index).unwrap_or_else(|error| {
            log::warn!("could not read dungeon types of region {region_index}: {error}");
            HashMap::new()
        });
        if let Err(error) =
            read_region_dungeons(region_index, region_name, record, &dungeon_types, &mut dungeons)
        {
            log::warn!("could not read dungeons in {record_name}: {error}");
        }
    }
    dungeons.sort_by(|first, second| {
        (first.region_name, first.name.as_str()).cmp(&(second.region_name, second.name.as_str()))
    });
    dungeons
}

/// Reads all dungeons from one region's `MAPDITEM` record.
fn read_region_dungeons(
    region_index: usize,
    region_name: &'static str,
    record: &[u8],
    dungeon_types: &HashMap<u16, DungeonType>,
    dungeons: &mut Vec<DungeonDefinition>,
) -> Arena2Result<()> {
    if record.is_empty() {
        return Ok(());
    }

    let mut offset_table = ByteReader::new(record);
    let dungeon_count = offset_table.read_u32()? as usize;
    let records_start = 4 + dungeon_count * 8;

    for _ in 0..dungeon_count {
        let record_offset = offset_table.read_u32()? as usize;
        let _is_dungeon = offset_table.read_u16()?;
        let exterior_location_id = offset_table.read_u16()?;
        let mut dungeon = read_dungeon(region_index, region_name, record, records_start + record_offset)?;
        dungeon.dungeon_type = dungeon_types.get(&exterior_location_id).copied();
        dungeons.push(dungeon);
    }
    Ok(())
}

/// Reads a single dungeon record starting at `position`.
fn read_dungeon(
    region_index: usize,
    region_name: &'static str,
    record: &[u8],
    position: usize,
) -> Arena2Result<DungeonDefinition> {
    // Location record: a door list followed by a fixed size header.
    let mut reader = ByteReader::at(record, position);
    let door_count = reader.read_u32()? as usize;
    let header_start = position + 4 + door_count * LOCATION_DOOR_SIZE;

    let location_id = read_location_id(record, position)?;
    let name = ByteReader::at(record, header_start + LOCATION_NAME_OFFSET)
        .read_fixed_string(LOCATION_NAME_LENGTH)?;

    // Dungeon header and block list.
    let dungeon_header_start = header_start + LOCATION_HEADER_SIZE;
    let block_count =
        usize::from(ByteReader::at(record, dungeon_header_start + DUNGEON_BLOCK_COUNT_OFFSET).read_u16()?);
    let mut block_reader = ByteReader::at(record, dungeon_header_start + DUNGEON_HEADER_SIZE);

    let mut blocks = Vec::with_capacity(block_count);
    for _ in 0..block_count {
        // Grid coordinates are signed bytes.
        let grid_x = i32::from(block_reader.read_u8()? as i8);
        let grid_z = i32::from(block_reader.read_u8()? as i8);
        let bitfield = block_reader.read_u16()?;

        let block_number = bitfield & 0x3ff;
        let is_starting_block = bitfield & 0x400 != 0;
        let letter_index = usize::from(bitfield >> 11);
        let letter = RDB_BLOCK_LETTERS.get(letter_index).ok_or_else(|| {
            Arena2Error::format(format!("dungeon '{name}' uses unknown block type {letter_index}"))
        })?;

        blocks.push(DungeonBlockPlacement {
            grid_x,
            grid_z,
            block_name: format!("{letter}{block_number:07}.RDB"),
            is_starting_block,
        });
    }

    // The game moves one of Orsinium's border blocks that would otherwise overlap another.
    if location_id == ORSINIUM_LOCATION_ID {
        if let Some(block) = blocks.get_mut(ORSINIUM_MOVED_BLOCK) {
            block.grid_z = ORSINIUM_MOVED_BLOCK_ROW;
        }
    }

    Ok(DungeonDefinition { region_index, region_name, name, dungeon_type: None, blocks })
}

/// Reads the dungeon type of every location in a region, keyed by location id.
///
/// `MAPNAMES` gives the number of locations, `MAPPITEM` each location's id and
/// `MAPTABLE` each location's dungeon type, all in the same location order.
fn read_dungeon_types(
    maps_archive: &BsaArchive,
    region_index: usize,
) -> Arena2Result<HashMap<u16, DungeonType>> {
    let record = |kind: &str| {
        let name = format!("{kind}.{region_index:03}");
        maps_archive
            .record_by_name(&name)
            .ok_or_else(|| Arena2Error::not_found(format!("{name} in MAPS.BSA")))
    };
    let location_count = ByteReader::new(record("MAPNAMES")?).read_u32()? as usize;
    let locations = record("MAPPITEM")?;
    let map_table = record("MAPTABLE")?;

    let mut dungeon_types = HashMap::new();
    let mut offset_table = ByteReader::new(locations);
    for location_index in 0..location_count {
        let location_start = location_count * 4 + offset_table.read_u32()? as usize;
        let location_id = read_location_id(locations, location_start)?;
        let type_position = location_index * MAP_TABLE_ENTRY_SIZE + MAP_TABLE_DUNGEON_TYPE_OFFSET;
        let raw_type = ByteReader::at(map_table, type_position).read_u8()?;
        if let Some(dungeon_type) = DungeonType::from_raw(raw_type) {
            dungeon_types.insert(location_id, dungeon_type);
        }
    }
    Ok(dungeon_types)
}

/// Reads the id of the location record starting at `position`: past its door list, in its header.
fn read_location_id(record: &[u8], position: usize) -> Arena2Result<u16> {
    let door_count = ByteReader::at(record, position).read_u32()? as usize;
    ByteReader::at(record, position + 4 + door_count * LOCATION_DOOR_SIZE + LOCATION_ID_OFFSET).read_u16()
}

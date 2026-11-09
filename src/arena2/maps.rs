//! Reader for the pre-built dungeon layouts stored in `MAPS.BSA`.
//!
//! `MAPS.BSA` contains four records per region. The `MAPDITEM.###` record
//! lists every dungeon in the region together with the grid of RDB blocks it
//! is assembled from, which is all the viewer needs to rebuild a dungeon.
//! The dungeon's type (crypt, orc stronghold, …), which decides the enemies at
//! its random enemy markers, is stored separately in the region's `MAPTABLE`
//! record, one entry per location. Locations are matched up through the
//! location ids in `MAPPITEM`, as Daggerfall Unity does. `MAPTABLE` also
//! gives the location's position on the world map and its location type.

use std::collections::{HashMap, HashSet};

use super::bsa::BsaArchive;
use super::error::{Arena2Error, Arena2Result};
use super::reader::ByteReader;
use super::world::{LocationType, MapPixel, MapTableEntry};
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
/// Offsets of the world position inside the location record header.
const LOCATION_X_OFFSET: usize = 7;
const LOCATION_Y_OFFSET: usize = 15;
/// Offset, inside a location record header, of the value that decides a dungeon's music.
const LOCATION_MUSIC_SEED_OFFSET: usize = 27;
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
/// Size of the building list header that follows a location record header.
const BUILDING_LIST_HEADER_SIZE: usize = 7;
/// Size of one building entry in a location's building list.
const LOCATION_BUILDING_SIZE: usize = 26;
/// Offsets within a location's exterior data.
const EXTERIOR_WIDTH_OFFSET: usize = 40;
const EXTERIOR_LETTER_OFFSET: usize = 46;
const EXTERIOR_BLOCK_INDEX_OFFSET: usize = 49;
/// Length of each of the exterior's block index, number and character tables.
const EXTERIOR_BLOCK_TABLE_LENGTH: usize = 64;
/// First part of an RMB block name, chosen by the block index.
const RMB_BLOCK_PREFIXES: [&str; 45] = [
    "TVRN", "GENR", "RESI", "WEAP", "ARMR", "ALCH", "BANK", "BOOK", "CLOT", "FURN", "GEMS", "LIBR", "PAWN",
    "TEMP", "TEMP", "PALA", "FARM", "DUNG", "CAST", "MANR", "SHRI", "RUIN", "SHCK", "GRVE", "FILL", "KRAV",
    "KDRA", "KOWL", "KMOO", "KCAN", "KFLA", "KHOR", "KROS", "KWHE", "KSCA", "KHAW", "MAGE", "THIE", "DARK",
    "FIGH", "CUST", "WALL", "MARK", "SHIP", "WITC",
];
/// Second letter of an RMB block name, chosen by two bits of the block character.
const RMB_SECOND_LETTERS: [char; 4] = ['A', 'L', 'M', 'S'];
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
    /// Where the dungeon is on the world map; `None` if unknown.
    pub map_pixel: Option<MapPixel>,
    /// The kind of location (large dungeon, graveyard, …); `None` if unknown.
    pub location_type: Option<LocationType>,
    /// What stands above ground; `None` if unknown.
    pub exterior: Option<LocationExterior>,
    /// The game's own record of the location, needed to replace it; `None` if unknown.
    pub classic: Option<ClassicLocation>,
    /// Blocks making up the dungeon.
    pub blocks: Vec<DungeonBlockPlacement>,
}

/// What identifies a location in the game data, which a replacement of it
/// must keep so the game (and its quests and saved games) still find it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicLocation {
    /// Index of the location in its region.
    pub location_index: usize,
    /// Map id as the game stores it.
    pub map_id: i32,
    pub longitude: i32,
    pub latitude: i32,
    pub discovered: bool,
    pub key: u32,
    /// Id of the location's own record.
    pub location_id: u16,
    /// Id of its dungeon's record.
    pub dungeon_location_id: u16,
    /// World position, in native units, from the location's record.
    pub world_x: i32,
    pub world_y: i32,
    /// The value in the dungeon's record that, with the region, decides the
    /// music played inside (Daggerfall Unity reads it as `Unknown2`).
    pub dungeon_music_seed: u32,
}

/// The above-ground part of a location: a grid of RMB blocks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocationExterior {
    /// Blocks from west to east.
    pub width: usize,
    /// Blocks from south to north.
    pub height: usize,
    /// RMB block names, row by row, as the location names them. A few names
    /// do not exist in `BLOCKS.BSA` and must be resolved as Daggerfall Unity
    /// does (see `Arena2Library::rmb_block`).
    pub block_names: Vec<String>,
    /// The location's buildings (most dungeon entrances have none).
    pub buildings: Vec<LocationBuilding>,
}

/// One building of a location, as its `MAPPITEM` record lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocationBuilding {
    /// Seed for the building's name.
    pub name_seed: u16,
    pub faction_id: u16,
    pub sector: i16,
    /// Building type number (Daggerfall Unity's `BuildingTypes`).
    pub building_type: u8,
    pub quality: u8,
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
        let map_table = read_map_table(maps_archive, region_index).unwrap_or_else(|error| {
            log::warn!("could not read the map table of region {region_index}: {error}");
            HashMap::new()
        });
        if let Err(error) = read_region_dungeons(region_index, region_name, record, &map_table, &mut dungeons)
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
    map_table: &HashMap<u16, LocationInfo>,
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
        let dungeon_position = records_start + record_offset;
        let mut dungeon = read_dungeon(region_index, region_name, record, dungeon_position)?;
        if let Some(info) = map_table.get(&exterior_location_id) {
            dungeon.dungeon_type = DungeonType::from_raw(info.entry.dungeon_type);
            dungeon.map_pixel = Some(info.entry.pixel);
            dungeon.location_type = info.entry.location_type;
            dungeon.exterior = info.exterior.clone();
            dungeon.classic = Some(ClassicLocation {
                location_index: info.location_index,
                map_id: info.entry.map_id,
                longitude: info.entry.longitude,
                latitude: info.entry.latitude,
                discovered: info.entry.discovered,
                key: info.entry.key,
                location_id: exterior_location_id,
                dungeon_location_id: read_location_id(record, dungeon_position)?,
                dungeon_music_seed: read_record_value(record, dungeon_position, LOCATION_MUSIC_SEED_OFFSET)?,
                world_x: info.world_x,
                world_y: info.world_y,
            });
        }
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

    Ok(DungeonDefinition {
        region_index,
        region_name,
        name,
        dungeon_type: None,
        map_pixel: None,
        location_type: None,
        exterior: None,
        classic: None,
        blocks,
    })
}

/// What is known of a location besides its dungeon.
struct LocationInfo {
    entry: MapTableEntry,
    exterior: Option<LocationExterior>,
    /// Index of the location in its region.
    location_index: usize,
    /// World position from the location's record.
    world_x: i32,
    world_y: i32,
}

/// Reads the `MAPTABLE` entry and exterior of every location in a region, keyed by location id.
///
/// `MAPNAMES` gives the number of locations, `MAPPITEM` each location's id and
/// exterior and `MAPTABLE` each location's entry, all in the same location order.
fn read_map_table(
    maps_archive: &BsaArchive,
    region_index: usize,
) -> Arena2Result<HashMap<u16, LocationInfo>> {
    let record = |kind: &str| {
        let name = format!("{kind}.{region_index:03}");
        maps_archive
            .record_by_name(&name)
            .ok_or_else(|| Arena2Error::not_found(format!("{name} in MAPS.BSA")))
    };
    let location_count = ByteReader::new(record("MAPNAMES")?).read_u32()? as usize;
    let locations = record("MAPPITEM")?;
    let map_table = record("MAPTABLE")?;

    let mut entries = HashMap::new();
    let mut offset_table = ByteReader::new(locations);
    for location_index in 0..location_count {
        let location_start = location_count * 4 + offset_table.read_u32()? as usize;
        let location_id = read_location_id(locations, location_start)?;
        let exterior = read_exterior(locations, location_start)
            .map_err(|error| log::warn!("could not read the exterior of location {location_id}: {error}"))
            .ok();
        let door_count = ByteReader::at(locations, location_start).read_u32()? as usize;
        let header = location_start + 4 + door_count * LOCATION_DOOR_SIZE;
        entries.insert(
            location_id,
            LocationInfo {
                entry: MapTableEntry::read(map_table, location_index)?,
                exterior,
                location_index,
                world_x: ByteReader::at(locations, header + LOCATION_X_OFFSET).read_i32()?,
                world_y: ByteReader::at(locations, header + LOCATION_Y_OFFSET).read_i32()?,
            },
        );
    }
    Ok(entries)
}

/// Reads the exterior of the `MAPPITEM` location record starting at `position`.
fn read_exterior(record: &[u8], position: usize) -> Arena2Result<LocationExterior> {
    let door_count = ByteReader::at(record, position).read_u32()? as usize;
    let building_list = position + 4 + door_count * LOCATION_DOOR_SIZE + LOCATION_HEADER_SIZE;
    let building_count = usize::from(ByteReader::at(record, building_list).read_u16()?);
    let buildings_start = building_list + BUILDING_LIST_HEADER_SIZE;
    let exterior = buildings_start + building_count * LOCATION_BUILDING_SIZE;
    let buildings = (0..building_count)
        .map(|building| read_building(record, buildings_start + building * LOCATION_BUILDING_SIZE))
        .collect::<Arena2Result<_>>()?;

    let mut reader = ByteReader::at(record, exterior + EXTERIOR_WIDTH_OFFSET);
    let width = usize::from(reader.read_u8()?);
    let height = usize::from(reader.read_u8()?);
    let first_letter = char::from(ByteReader::at(record, exterior + EXTERIOR_LETTER_OFFSET).read_u8()?);
    let mut tables = ByteReader::at(record, exterior + EXTERIOR_BLOCK_INDEX_OFFSET);
    let indices = tables.read_bytes(EXTERIOR_BLOCK_TABLE_LENGTH)?;
    let numbers = tables.read_bytes(EXTERIOR_BLOCK_TABLE_LENGTH)?;
    let characters = tables.read_bytes(EXTERIOR_BLOCK_TABLE_LENGTH)?;
    if width * height > EXTERIOR_BLOCK_TABLE_LENGTH {
        return Err(Arena2Error::format(format!("exterior of {width} × {height} blocks")));
    }
    let block_names = (0..width * height)
        .map(|block| rmb_block_name(indices[block], numbers[block], characters[block], first_letter))
        .collect::<Arena2Result<_>>()?;
    Ok(LocationExterior { width, height, block_names, buildings })
}

/// Reads one building entry of a location's building list.
fn read_building(record: &[u8], position: usize) -> Arena2Result<LocationBuilding> {
    let mut reader = ByteReader::at(record, position);
    let name_seed = reader.read_u16()?;
    // Service time limit (4 bytes), then four values of unknown meaning (12 bytes).
    reader.read_bytes(16)?;
    let faction_id = reader.read_u16()?;
    let sector = reader.read_u16()? as i16;
    let _location_id = reader.read_u16()?;
    let building_type = reader.read_u8()?;
    let quality = reader.read_u8()?;
    Ok(LocationBuilding { name_seed, faction_id, sector, building_type, quality })
}

/// The id of every location record in `MAPS.BSA`: each location's own and
/// each dungeon's. A new location must use ids none of these use.
pub fn read_all_location_ids(maps_archive: &BsaArchive) -> HashSet<u16> {
    let mut ids = HashSet::new();
    for region_index in 0..REGION_NAMES.len() {
        let record = |kind: &str| maps_archive.record_by_name(&format!("{kind}.{region_index:03}"));
        let result = (|| -> Arena2Result<()> {
            if let (Some(names), Some(locations)) = (record("MAPNAMES"), record("MAPPITEM")) {
                if !names.is_empty() {
                    let location_count = ByteReader::new(names).read_u32()? as usize;
                    let mut offsets = ByteReader::new(locations);
                    for _ in 0..location_count {
                        let start = location_count * 4 + offsets.read_u32()? as usize;
                        ids.insert(read_location_id(locations, start)?);
                    }
                }
            }
            if let Some(dungeons) = record("MAPDITEM").filter(|record| !record.is_empty()) {
                let mut offsets = ByteReader::new(dungeons);
                let dungeon_count = offsets.read_u32()? as usize;
                let records_start = 4 + dungeon_count * 8;
                for _ in 0..dungeon_count {
                    let record_offset = offsets.read_u32()? as usize;
                    let _is_dungeon = offsets.read_u16()?;
                    ids.insert(offsets.read_u16()?);
                    ids.insert(read_location_id(dungeons, records_start + record_offset)?);
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            log::warn!("could not read the location ids of region {region_index}: {error}");
        }
    }
    ids
}

/// Builds an RMB block name from a location's block tables, as Daggerfall Unity does.
fn rmb_block_name(index: u8, number: u8, character: u8, first_letter: char) -> Arena2Result<String> {
    let prefix = RMB_BLOCK_PREFIXES
        .get(usize::from(index))
        .ok_or_else(|| Arena2Error::format(format!("unknown RMB block index {index}")))?;
    let letter1 = if character & 0x10 != 0 { first_letter } else { 'A' };
    let letter2 = RMB_SECOND_LETTERS[usize::from(character.wrapping_mul(2) >> 6)];
    let numbers = match index {
        // Temples.
        13 | 14 => format!("{}{number}", char::from(b'A' + (character & 0xf))),
        _ => format!("{number:02}"),
    };
    Ok(format!("{prefix}{letter1}{letter2}{numbers}.RMB"))
}

/// Reads the `u32` at `offset` in the header of the location record starting at `position`.
fn read_record_value(record: &[u8], position: usize, offset: usize) -> Arena2Result<u32> {
    let door_count = ByteReader::at(record, position).read_u32()? as usize;
    ByteReader::at(record, position + 4 + door_count * LOCATION_DOOR_SIZE + offset).read_u32()
}

/// Reads the id of the location record starting at `position`: past its door list, in its header.
fn read_location_id(record: &[u8], position: usize) -> Arena2Result<u16> {
    let door_count = ByteReader::at(record, position).read_u32()? as usize;
    ByteReader::at(record, position + 4 + door_count * LOCATION_DOOR_SIZE + LOCATION_ID_OFFSET).read_u16()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_rmb_block_names() {
        assert_eq!(rmb_block_name(17, 3, 0, 'B').unwrap(), "DUNGAA03.RMB");
        // Bit 0x10 takes the location's own first letter; bits 5 and 6 the second letter.
        assert_eq!(rmb_block_name(21, 12, 0x10 | 0x40, 'B').unwrap(), "RUINBM12.RMB");
        assert_eq!(rmb_block_name(13, 2, 0x03, 'A').unwrap(), "TEMPAAD2.RMB");
        assert!(rmb_block_name(200, 0, 0, 'A').is_err());
    }
}

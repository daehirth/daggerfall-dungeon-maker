//! Reader for the pre-built dungeon layouts stored in `MAPS.BSA`.
//!
//! `MAPS.BSA` contains four records per region. The `MAPDITEM.###` record
//! lists every dungeon in the region together with the grid of RDB blocks it
//! is assembled from, which is all the viewer needs to rebuild a dungeon.

use super::bsa::BsaArchive;
use super::error::{Arena2Error, Arena2Result};
use super::reader::ByteReader;

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
        if let Err(error) = read_region_dungeons(region_index, region_name, record, &mut dungeons) {
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
        let _exterior_location_id = offset_table.read_u16()?;
        dungeons.push(read_dungeon(region_index, region_name, record, records_start + record_offset)?);
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

    let location_id = ByteReader::at(record, header_start + LOCATION_ID_OFFSET).read_u16()?;
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

    Ok(DungeonDefinition { region_index, region_name, name, blocks })
}

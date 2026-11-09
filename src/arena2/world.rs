//! The world map: which region each map pixel belongs to, and every location on it.
//!
//! The Iliac Bay is 1000 × 500 map pixels. `POLITIC.PAK` gives the region of
//! each pixel (the sea belongs to none). Each region's `MAPNAMES` and
//! `MAPTABLE` records in `MAPS.BSA` list its locations: their names, map
//! positions and location types. Daggerfall Unity reads the same data; a new
//! location must go on a pixel no other location uses.

use std::collections::HashMap;

use super::bsa::BsaArchive;
use super::error::{Arena2Error, Arena2Result};
use super::maps::REGION_NAMES;
use super::reader::ByteReader;

/// Width of the world map, in map pixels.
pub const MAP_WIDTH: i32 = 1000;
/// Height of the world map, in map pixels.
pub const MAP_HEIGHT: i32 = 500;
/// Longitude and latitude units per map pixel.
const UNITS_PER_MAP_PIXEL: i32 = 128;
/// Width of a `POLITIC.PAK` row: one column more than the map, which the game skips.
const PAK_ROW_WIDTH: usize = 1001;
/// `POLITIC.PAK` value of a region's pixels: the region index plus this.
const POLITIC_REGION_BASE: u8 = 128;
/// Length of one name in a `MAPNAMES` record.
const MAP_NAME_LENGTH: usize = 32;
/// Size of one location's entry in a `MAPTABLE` record.
pub(super) const MAP_TABLE_ENTRY_SIZE: usize = 17;

/// A position on the world map, in map pixels: `x` from west to east, `y` from north to south.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MapPixel {
    pub x: i32,
    pub y: i32,
}

impl MapPixel {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// The map pixel of a location at `longitude` and `latitude`, as Daggerfall Unity works it out.
    pub fn from_longitude_latitude(longitude: i32, latitude: i32) -> Self {
        Self { x: longitude / UNITS_PER_MAP_PIXEL, y: MAP_HEIGHT - 1 - latitude / UNITS_PER_MAP_PIXEL }
    }

    /// The longitude and latitude of this pixel, for a location placed on it.
    pub fn longitude_latitude(self) -> (i32, i32) {
        (self.x * UNITS_PER_MAP_PIXEL, (MAP_HEIGHT - 1 - self.y) * UNITS_PER_MAP_PIXEL)
    }

    /// Whether the pixel is on the map.
    pub fn is_on_map(self) -> bool {
        (0..MAP_WIDTH).contains(&self.x) && (0..MAP_HEIGHT).contains(&self.y)
    }
}

impl std::fmt::Display for MapPixel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}, {}", self.x, self.y)
    }
}

/// The kind of a location, which decides its icon on the travel map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocationType {
    City,
    Hamlet,
    Village,
    Farm,
    LargeDungeon,
    Temple,
    Tavern,
    MediumDungeon,
    WealthyHome,
    Cult,
    SmallDungeon,
    PoorHome,
    Graveyard,
    Coven,
    Ship,
}

impl LocationType {
    const ALL: [Self; 15] = [
        Self::City,
        Self::Hamlet,
        Self::Village,
        Self::Farm,
        Self::LargeDungeon,
        Self::Temple,
        Self::Tavern,
        Self::MediumDungeon,
        Self::WealthyHome,
        Self::Cult,
        Self::SmallDungeon,
        Self::PoorHome,
        Self::Graveyard,
        Self::Coven,
        Self::Ship,
    ];

    /// The location types a new dungeon can have: those of Daggerfall's own
    /// dungeons (apart from the few that are cities).
    pub const DUNGEON_TYPES: [Self; 4] =
        [Self::LargeDungeon, Self::MediumDungeon, Self::SmallDungeon, Self::Graveyard];

    /// The type stored in `MAPTABLE` as `value`.
    pub fn from_raw(value: u32) -> Option<Self> {
        Self::ALL.get(value as usize).copied()
    }

    /// The value stored in `MAPTABLE` (and by Daggerfall Unity) for this type.
    pub fn raw(self) -> u32 {
        Self::ALL.iter().position(|&location_type| location_type == self).unwrap_or(0) as u32
    }

    /// Name shown to the user.
    pub fn name(self) -> &'static str {
        match self {
            Self::City => "City",
            Self::Hamlet => "Hamlet",
            Self::Village => "Village",
            Self::Farm => "Farm",
            Self::LargeDungeon => "Large dungeon",
            Self::Temple => "Temple",
            Self::Tavern => "Tavern",
            Self::MediumDungeon => "Medium dungeon",
            Self::WealthyHome => "Wealthy home",
            Self::Cult => "Cult",
            Self::SmallDungeon => "Small dungeon",
            Self::PoorHome => "Poor home",
            Self::Graveyard => "Graveyard",
            Self::Coven => "Coven",
            Self::Ship => "Ship",
        }
    }

    /// The type called `name` (as [`Self::name`] gives it), ignoring case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|location_type| location_type.name().eq_ignore_ascii_case(name))
    }
}

/// One location of the world map.
#[derive(Debug, Clone)]
pub struct WorldLocation {
    pub name: String,
    pub region_index: usize,
    pub pixel: MapPixel,
    /// `None` for a type this program does not know.
    pub location_type: Option<LocationType>,
}

/// One location's entry in its region's `MAPTABLE` record.
#[derive(Debug, Clone, Copy)]
pub(super) struct MapTableEntry {
    pub pixel: MapPixel,
    pub location_type: Option<LocationType>,
    pub dungeon_type: u8,
    /// The location's map id, as stored (the map pixel id plus flags in the high bits).
    pub map_id: i32,
    pub longitude: i32,
    pub latitude: i32,
    /// Whether the travel map shows it from the start.
    pub discovered: bool,
    pub key: u32,
}

impl MapTableEntry {
    /// Reads the entry of location `location_index` from a `MAPTABLE` record.
    pub fn read(map_table: &[u8], location_index: usize) -> Arena2Result<Self> {
        let mut reader = ByteReader::at(map_table, location_index * MAP_TABLE_ENTRY_SIZE);
        let map_id = reader.read_u32()? as i32;
        let bitfield = reader.read_u32()?;
        let latitude_field = reader.read_u32()?;
        let dungeon_type = reader.read_u8()?;
        let key = reader.read_u32()?;
        // As Daggerfall Unity reads them.
        let longitude = ((bitfield & 0x1ff_ffff) >> 8) as i32;
        let location_type = LocationType::from_raw(bitfield.wrapping_mul(4) >> 27);
        let latitude = ((latitude_field & 0xff_ffff) >> 8) as i32;
        Ok(Self {
            pixel: MapPixel::from_longitude_latitude(longitude, latitude),
            location_type,
            dungeon_type,
            map_id,
            longitude,
            latitude,
            discovered: (bitfield >> 24) & 0x40 != 0,
            key,
        })
    }
}

/// The world map: regions and locations.
pub struct WorldMap {
    /// `POLITIC.PAK` value of each map pixel, row by row, or `None` without the file.
    politic: Option<Vec<u8>>,
    /// `CLIMATE.PAK`, every row in full (see [`Self::climate_at`]), or `None` without the file.
    climate: Option<Vec<u8>>,
    /// Every location of every region.
    pub locations: Vec<WorldLocation>,
    /// Indices into `locations` by map pixel.
    locations_by_pixel: HashMap<MapPixel, Vec<usize>>,
}

impl WorldMap {
    /// Reads the locations from `MAPS.BSA`, the regions from `POLITIC.PAK`
    /// and the climates from `CLIMATE.PAK`, where given.
    ///
    /// Regions whose records cannot be read are logged and skipped.
    pub fn read(maps_archive: &BsaArchive, politic_pak: Option<&[u8]>, climate_pak: Option<&[u8]>) -> Self {
        let politic = politic_pak.and_then(|data| match read_politic(data) {
            Ok(values) => Some(values),
            Err(error) => {
                log::warn!("could not read POLITIC.PAK: {error}");
                None
            }
        });
        let climate = climate_pak.and_then(|data| match read_pak_rows(data) {
            Ok(values) => Some(values),
            Err(error) => {
                log::warn!("could not read CLIMATE.PAK: {error}");
                None
            }
        });
        let mut locations = Vec::new();
        for region_index in 0..REGION_NAMES.len() {
            if let Err(error) = read_region_locations(maps_archive, region_index, &mut locations) {
                log::warn!("could not read the locations of region {region_index}: {error}");
            }
        }
        let mut world = Self::new(politic, locations);
        world.climate = climate;
        world
    }

    fn new(politic: Option<Vec<u8>>, locations: Vec<WorldLocation>) -> Self {
        let mut locations_by_pixel: HashMap<MapPixel, Vec<usize>> = HashMap::new();
        for (index, location) in locations.iter().enumerate() {
            locations_by_pixel.entry(location.pixel).or_default().push(index);
        }
        Self { politic, climate: None, locations, locations_by_pixel }
    }

    /// The `CLIMATE.PAK` value Daggerfall Unity gives Daggerfall's own
    /// location at `pixel` (from column `x`, unlike the terrain), so a replaced
    /// location keeps the climate it had.
    pub fn location_climate_at(&self, pixel: MapPixel) -> Option<u8> {
        if !pixel.is_on_map() {
            return None;
        }
        self.climate.as_ref()?.get(pixel.y as usize * PAK_ROW_WIDTH + pixel.x as usize).copied()
    }

    /// The `CLIMATE.PAK` value (223-232) of `pixel`, which decides a
    /// location's ground, trees and sky; `None` off the map or without the file.
    ///
    /// This is the value Daggerfall Unity gives the terrain of the pixel (from
    /// column `x + 1`, like the regions), so a new location matches the ground
    /// around it.
    pub fn climate_at(&self, pixel: MapPixel) -> Option<u8> {
        if !pixel.is_on_map() {
            return None;
        }
        self.climate.as_ref()?.get(pixel.y as usize * PAK_ROW_WIDTH + pixel.x as usize + 1).copied()
    }

    /// Whether the regions of the map are known (`POLITIC.PAK` was read).
    pub fn has_regions(&self) -> bool {
        self.politic.is_some()
    }

    /// The index of the region `pixel` belongs to, or `None` for the sea, off
    /// the map, or when the regions are not known.
    pub fn region_at(&self, pixel: MapPixel) -> Option<usize> {
        if !pixel.is_on_map() {
            return None;
        }
        let value = *self.politic.as_ref()?.get((pixel.y * MAP_WIDTH + pixel.x) as usize)?;
        let region_index = usize::from(value.checked_sub(POLITIC_REGION_BASE)?);
        (region_index < REGION_NAMES.len()).then_some(region_index)
    }

    /// The locations at `pixel`.
    pub fn locations_at(&self, pixel: MapPixel) -> impl Iterator<Item = &WorldLocation> {
        self.locations_by_pixel.get(&pixel).into_iter().flatten().map(|&index| &self.locations[index])
    }
}

/// Name of region `region_index`, or "Unknown region".
pub fn region_name(region_index: usize) -> &'static str {
    REGION_NAMES.get(region_index).copied().unwrap_or("Unknown region")
}

/// Unpacks `POLITIC.PAK`: a row offset table, then each row run-length
/// encoded as (count `u16`, value `u8`) runs. Returns the map's pixels, row by row.
fn read_politic(data: &[u8]) -> Arena2Result<Vec<u8>> {
    let rows = read_pak_rows(data)?;
    let mut values = Vec::with_capacity((MAP_WIDTH * MAP_HEIGHT) as usize);
    for row in rows.as_chunks::<PAK_ROW_WIDTH>().0 {
        // The game reads pixel x from column x + 1.
        values.extend_from_slice(&row[1..]);
    }
    Ok(values)
}

/// Unpacks a `.PAK` map file (`POLITIC.PAK` or `CLIMATE.PAK`): a row offset
/// table, then each row run-length encoded as (count `u16`, value `u8`) runs.
/// Returns every row in full ([`PAK_ROW_WIDTH`] values each), one after another.
fn read_pak_rows(data: &[u8]) -> Arena2Result<Vec<u8>> {
    let mut values = Vec::with_capacity(PAK_ROW_WIDTH * MAP_HEIGHT as usize);
    let mut row = Vec::with_capacity(PAK_ROW_WIDTH);
    let mut offsets = ByteReader::new(data);
    for _ in 0..MAP_HEIGHT {
        let mut runs = ByteReader::at(data, offsets.read_u32()? as usize);
        row.clear();
        while row.len() < PAK_ROW_WIDTH {
            let count = usize::from(runs.read_u16()?);
            let value = runs.read_u8()?;
            if count == 0 {
                return Err(Arena2Error::format("a .PAK file has an empty run"));
            }
            row.extend(std::iter::repeat_n(value, count));
        }
        values.extend_from_slice(&row[..PAK_ROW_WIDTH]);
    }
    Ok(values)
}

/// Adds the locations of region `region_index` to `locations`.
fn read_region_locations(
    maps_archive: &BsaArchive,
    region_index: usize,
    locations: &mut Vec<WorldLocation>,
) -> Arena2Result<()> {
    let (Some(names), Some(map_table)) = (
        maps_archive.record_by_name(&format!("MAPNAMES.{region_index:03}")),
        maps_archive.record_by_name(&format!("MAPTABLE.{region_index:03}")),
    ) else {
        return Ok(());
    };
    if names.is_empty() {
        return Ok(());
    }
    let mut names_reader = ByteReader::new(names);
    let location_count = names_reader.read_u32()? as usize;
    for location_index in 0..location_count {
        let name = names_reader.read_fixed_string(MAP_NAME_LENGTH)?;
        let entry = MapTableEntry::read(map_table, location_index)?;
        locations.push(WorldLocation {
            name,
            region_index,
            pixel: entry.pixel,
            location_type: entry.location_type,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_pixels_round_trip_through_longitude_and_latitude() {
        for pixel in [MapPixel::new(0, 0), MapPixel::new(999, 499), MapPixel::new(204, 312)] {
            let (longitude, latitude) = pixel.longitude_latitude();
            assert_eq!(MapPixel::from_longitude_latitude(longitude, latitude), pixel);
            // Anywhere inside the pixel maps back to it.
            assert_eq!(MapPixel::from_longitude_latitude(longitude + 127, latitude + 127), pixel);
        }
        assert!(!MapPixel::new(1000, 0).is_on_map());
        assert!(!MapPixel::new(0, -1).is_on_map());
    }

    #[test]
    fn location_types_round_trip() {
        for value in 0..15 {
            let location_type = LocationType::from_raw(value).unwrap();
            assert_eq!(location_type.raw(), value);
            assert_eq!(LocationType::from_name(location_type.name()), Some(location_type));
        }
        assert_eq!(LocationType::from_raw(15), None);
    }

    #[test]
    fn reads_map_table_entries() {
        // Longitude 25728 (pixel 201), large dungeon (4), latitude 23936 (pixel 499 - 187 = 312).
        let bitfield: u32 = (25728 << 8) | (4 << 25);
        let latitude_field: u32 = 23936 << 8;
        let mut record = vec![0u8; MAP_TABLE_ENTRY_SIZE];
        record[4..8].copy_from_slice(&bitfield.to_le_bytes());
        record[8..12].copy_from_slice(&latitude_field.to_le_bytes());
        record[12] = 3;
        let entry = MapTableEntry::read(&record, 0).unwrap();
        assert_eq!(entry.pixel, MapPixel::new(201, 312));
        assert_eq!(entry.location_type, Some(LocationType::LargeDungeon));
        assert_eq!(entry.dungeon_type, 3);
    }

    #[test]
    fn unpacks_politic_rows_and_finds_regions() {
        // Every row: the skipped column, 600 pixels of region 17, 400 of sea (64).
        let row_runs: Vec<u8> = [(1u16, 0u8), (600, POLITIC_REGION_BASE + 17), (400, 64)]
            .iter()
            .flat_map(|&(count, value)| {
                let mut run = count.to_le_bytes().to_vec();
                run.push(value);
                run
            })
            .collect();
        let table_size = MAP_HEIGHT as usize * 4;
        let mut data = Vec::new();
        for row in 0..MAP_HEIGHT as usize {
            data.extend_from_slice(&((table_size + row * row_runs.len()) as u32).to_le_bytes());
        }
        for _ in 0..MAP_HEIGHT {
            data.extend_from_slice(&row_runs);
        }
        let politic = read_politic(&data).unwrap();
        assert_eq!(politic.len(), (MAP_WIDTH * MAP_HEIGHT) as usize);

        let location = WorldLocation {
            name: "Somewhere".into(),
            region_index: 17,
            pixel: MapPixel::new(10, 20),
            location_type: Some(LocationType::Village),
        };
        let world = WorldMap::new(Some(politic), vec![location]);
        assert_eq!(world.region_at(MapPixel::new(0, 0)), Some(17));
        assert_eq!(world.region_at(MapPixel::new(599, 499)), Some(17));
        assert_eq!(world.region_at(MapPixel::new(600, 0)), None);
        assert_eq!(world.region_at(MapPixel::new(-1, 0)), None);
        assert_eq!(world.locations_at(MapPixel::new(10, 20)).count(), 1);
        assert_eq!(world.locations_at(MapPixel::new(11, 20)).count(), 0);
    }
}

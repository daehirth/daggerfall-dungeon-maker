//! Readers for the game data stored in a Daggerfall `ARENA2` folder.
//!
//! The submodules each handle one file format. [`Arena2Library`] ties them
//! together, owning the loaded archives and caching decoded assets.
//!
//! # Coordinate system
//!
//! Daggerfall stores positions in integer "native" units with the Y axis
//! pointing down. The viewer uses the same world space as Daggerfall Unity:
//! Y is flipped to point up, the space is left-handed, and one world unit is
//! 40 native units (roughly one metre).

mod blocks;
mod bsa;
mod error;
mod hmi;
mod library;
mod maps;
mod mesh;
mod reader;
mod rmb;
mod texture;
mod world;

pub use blocks::{RdbAction, RdbBlock, RdbObject, RdbObjectKind};
pub use error::{Arena2Error, Arena2Result};
pub use hmi::{parse_hmi, MidiEvent, MidiSong};
pub use library::Arena2Library;
pub use maps::{
    ClassicLocation, DungeonBlockPlacement, DungeonDefinition, LocationBuilding, LocationExterior,
    REGION_NAMES,
};
pub use mesh::{DoorKind, MeshVertex, ModelMesh, SubMesh};
pub use rmb::{RmbBlock, RmbFlat, RmbModel, RMB_BLOCK_SIDE};
pub use texture::RgbaImage;
pub use world::{region_name, LocationType, MapPixel, WorldLocation, WorldMap, MAP_HEIGHT, MAP_WIDTH};

use glam::{IVec3, Vec3};

/// Number of world units per native Daggerfall unit.
pub const WORLD_SCALE: f32 = 1.0 / 40.0;

/// Native rotation units per degree (2048 units make a full turn).
pub const NATIVE_ROTATION_UNITS_PER_DEGREE: f32 = 2048.0 / 360.0;

/// Width and depth of one RDB dungeon block, in native units.
pub const RDB_BLOCK_SIDE_NATIVE: f32 = 2048.0;

/// Width and depth of one RDB dungeon block, in world units.
pub const RDB_BLOCK_SIDE: f32 = RDB_BLOCK_SIDE_NATIVE * WORLD_SCALE;

/// Converts a native position (Y down) into a world position (Y up, scaled).
pub fn native_to_world(native_position: IVec3) -> Vec3 {
    Vec3::new(native_position.x as f32, -native_position.y as f32, native_position.z as f32) * WORLD_SCALE
}

/// Converts a world position back into native units, rounding to the nearest unit.
pub fn world_to_native(world_position: Vec3) -> IVec3 {
    let native = world_position / WORLD_SCALE;
    IVec3::new(native.x.round() as i32, -native.y.round() as i32, native.z.round() as i32)
}

/// Identifies one image: a `TEXTURE.###` archive and a record inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureKey {
    /// Archive number, the `###` in `TEXTURE.###`.
    pub archive: u16,
    /// Record index within the archive.
    pub record: u8,
}

impl TextureKey {
    /// Creates a key from an archive and record number.
    pub fn new(archive: u16, record: u8) -> Self {
        Self { archive, record }
    }

    /// Decodes the packed form used by models and flats: `archive << 7 | record`.
    pub fn from_bitfield(bitfield: u16) -> Self {
        Self { archive: bitfield >> 7, record: (bitfield & 0x7f) as u8 }
    }
}

impl std::fmt::Display for TextureKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "TEXTURE.{:03} #{}", self.archive, self.record)
    }
}

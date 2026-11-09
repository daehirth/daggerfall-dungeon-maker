//! Parser for RMB blocks stored in `BLOCKS.BSA`: the above-ground part of a
//! location (town quarters, castles, ruins, dungeon entrances).
//!
//! An RMB block is a square of 4096 native units. It holds a fixed size header
//! (positions of its buildings, ground tiles, scenery, automap), then one
//! record per building with the building's outside and inside objects, then
//! loose ("misc") models and flats. Only what is seen from outside is read:
//! the buildings' outside models and flats, the loose models and flats and the
//! nature scenery. Positions are worked out as Daggerfall Unity's `RMBLayout`
//! does and returned in world units relative to the block's corner.

use glam::{Quat, Vec3};

use super::error::{Arena2Error, Arena2Result};
use super::reader::ByteReader;
use super::{TextureKey, NATIVE_ROTATION_UNITS_PER_DEGREE, WORLD_SCALE};

/// Width and depth of an RMB block, in native units.
pub const RMB_BLOCK_SIDE_NATIVE: f32 = 4096.0;
/// Width and depth of an RMB block, in world units.
pub const RMB_BLOCK_SIDE: f32 = RMB_BLOCK_SIDE_NATIVE * WORLD_SCALE;

/// Most building records an RMB block can have.
const MAX_BUILDINGS: usize = 32;
/// Size of one building position entry: two unknown values, x, z and rotation.
const BUILDING_POSITION_SIZE: usize = 20;
/// Size of one building data entry.
const BUILDING_DATA_SIZE: usize = 26;
/// Number of ground tiles along each side of a block.
const GROUND_TILES: usize = 16;
/// Side of one ground tile, in native units.
const GROUND_TILE_SIDE: f32 = 256.0;
/// Size of the automap image.
const AUTOMAP_SIZE: usize = 64 * 64;
/// Size of one model record.
const MODEL_RECORD_SIZE: usize = 66;
/// Size of one flat record.
const FLAT_RECORD_SIZE: usize = 17;
/// Size of a building's object header.
const OBJECT_HEADER_SIZE: usize = 17;
/// Flats of this archive only mark things for the original editor.
const EDITOR_FLATS_ARCHIVE: u16 = 199;
/// Nature flats (trees, rocks) come in one archive per climate, 500 to 511.
const NATURE_ARCHIVES: std::ops::RangeInclusive<u16> = 500..=511;
/// The nature archive used in previews: temperate woodland.
pub const PREVIEW_NATURE_ARCHIVE: u16 = 504;
/// Vertical offsets Daggerfall Unity applies, in native units.
const PROPS_OFFSET_Y: f32 = -4.0;
const BLOCK_FLATS_OFFSET_Y: f32 = -6.0;
const NATURE_FLATS_OFFSET_Y: f32 = -2.0;

/// The outside of an RMB block.
#[derive(Debug, Clone, Default)]
pub struct RmbBlock {
    pub name: String,
    pub models: Vec<RmbModel>,
    pub flats: Vec<RmbFlat>,
}

/// A model placed in an RMB block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RmbModel {
    pub model_id: u32,
    /// World position relative to the block's corner.
    pub position: Vec3,
    /// Rotation around the vertical axis, in degrees, as the editor's objects store it.
    pub yaw_degrees: f32,
}

/// A flat placed in an RMB block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RmbFlat {
    pub texture: TextureKey,
    /// World position of the flat's base (it stands on this point), relative to the block's corner.
    pub base_position: Vec3,
}

/// One model record as stored in the file.
struct ModelRecord {
    model_id: u32,
    position: Vec3,
    rotation: f32,
}

/// One flat record as stored in the file.
struct FlatRecord {
    texture: TextureKey,
    position: Vec3,
}

/// Parses the RMB block called `name` from its `BLOCKS.BSA` record.
pub fn parse_rmb_block(name: &str, data: &[u8]) -> Arena2Result<RmbBlock> {
    let mut reader = ByteReader::new(data);
    let building_count = usize::from(reader.read_u8()?);
    let misc_model_count = usize::from(reader.read_u8()?);
    let misc_flat_count = usize::from(reader.read_u8()?);
    if building_count > MAX_BUILDINGS {
        return Err(Arena2Error::format(format!("{name} has {building_count} buildings")));
    }

    let mut building_positions = Vec::with_capacity(building_count);
    for index in 0..MAX_BUILDINGS {
        reader.skip(8);
        let x = reader.read_i32()? as f32;
        let z = reader.read_i32()? as f32;
        let rotation = reader.read_i32()? as f32;
        if index < building_count {
            building_positions.push((x, z, rotation));
        }
    }
    debug_assert_eq!(BUILDING_POSITION_SIZE, 20);
    reader.skip(MAX_BUILDINGS * BUILDING_DATA_SIZE);
    reader.skip(MAX_BUILDINGS * 4); // Unknown section.
    let mut building_sizes = [0usize; MAX_BUILDINGS];
    for size in &mut building_sizes {
        *size = reader.read_i32()?.max(0) as usize;
    }
    reader.skip(8); // Ground header.
    reader.skip(GROUND_TILES * GROUND_TILES); // Ground tiles.
    let scenery = reader.read_bytes(GROUND_TILES * GROUND_TILES)?;
    reader.skip(AUTOMAP_SIZE);
    reader.skip(13 + MAX_BUILDINGS * 13); // Block name and other names.

    let mut block = RmbBlock { name: name.to_string(), ..RmbBlock::default() };
    let to_world = |native: Vec3| native * WORLD_SCALE;

    // Buildings: their outside models turn with the building, their flats do not.
    let mut position = reader.position();
    for (index, &(building_x, building_z, building_rotation)) in building_positions.iter().enumerate() {
        let mut building = ByteReader::at(data, position);
        let header = building.read_bytes(OBJECT_HEADER_SIZE)?;
        let model_count = usize::from(header[0]);
        let flat_count = usize::from(header[1]);
        let building_yaw = -building_rotation / NATIVE_ROTATION_UNITS_PER_DEGREE;
        let turn = Quat::from_rotation_y(building_yaw.to_radians());
        let origin = to_world(Vec3::new(building_x, 0.0, RMB_BLOCK_SIDE_NATIVE - building_z));
        for _ in 0..model_count {
            let record = read_model_record(&mut building)?;
            block.models.push(RmbModel {
                model_id: record.model_id,
                position: origin + turn * to_world(record.position),
                yaw_degrees: building_yaw - record.rotation / NATIVE_ROTATION_UNITS_PER_DEGREE,
            });
        }
        let flats_origin = to_world(Vec3::new(building_x, 0.0, -building_z));
        for _ in 0..flat_count {
            let record = read_flat_record(&mut building)?;
            if record.texture.archive == EDITOR_FLATS_ARCHIVE {
                continue;
            }
            let native = record.position + Vec3::new(0.0, BLOCK_FLATS_OFFSET_Y, RMB_BLOCK_SIDE_NATIVE);
            block.flats.push(RmbFlat {
                texture: preview_texture(record.texture),
                base_position: flats_origin + to_world(native),
            });
        }
        position += building_sizes[index];
    }

    // Loose models and flats.
    let mut misc = ByteReader::at(data, position);
    for _ in 0..misc_model_count {
        let record = read_model_record(&mut misc)?;
        let native = record.position + Vec3::new(0.0, PROPS_OFFSET_Y, RMB_BLOCK_SIDE_NATIVE);
        block.models.push(RmbModel {
            model_id: record.model_id,
            position: to_world(native),
            yaw_degrees: -record.rotation / NATIVE_ROTATION_UNITS_PER_DEGREE,
        });
    }
    for _ in 0..misc_flat_count {
        let record = read_flat_record(&mut misc)?;
        if record.texture.archive == EDITOR_FLATS_ARCHIVE {
            continue;
        }
        let native = record.position + Vec3::new(0.0, BLOCK_FLATS_OFFSET_Y, RMB_BLOCK_SIDE_NATIVE);
        block
            .flats
            .push(RmbFlat { texture: preview_texture(record.texture), base_position: to_world(native) });
    }

    // Nature scenery: one tree or rock per marked ground tile. Rows are stored north first.
    for tile_y in 0..GROUND_TILES {
        for tile_x in 0..GROUND_TILES {
            let value = scenery[(GROUND_TILES - 1 - tile_y) * GROUND_TILES + tile_x];
            if value == 255 {
                continue;
            }
            let record = i32::from(value) / 4 - 1;
            if record < 1 {
                continue;
            }
            let native = Vec3::new(
                tile_x as f32 * GROUND_TILE_SIDE,
                NATURE_FLATS_OFFSET_Y,
                tile_y as f32 * GROUND_TILE_SIDE + GROUND_TILE_SIDE,
            );
            block.flats.push(RmbFlat {
                texture: TextureKey::new(PREVIEW_NATURE_ARCHIVE, record as u8),
                base_position: to_world(native),
            });
        }
    }
    Ok(block)
}

/// Nature flats of any climate are shown with the preview climate's.
fn preview_texture(texture: TextureKey) -> TextureKey {
    match NATURE_ARCHIVES.contains(&texture.archive) {
        true => TextureKey::new(PREVIEW_NATURE_ARCHIVE, texture.record),
        false => texture,
    }
}

/// Reads one model record: its position in native units (Y flipped up) and rotation.
fn read_model_record(reader: &mut ByteReader<'_>) -> Arena2Result<ModelRecord> {
    let start = reader.position();
    let id_high = i32::from(reader.read_i16()?);
    let id_low = i32::from(reader.read_u8()?);
    reader.skip(1 + 12 + 8 + 12); // Object type, unknowns, null, first position.
    let x = reader.read_i32()? as f32;
    let y = reader.read_i32()? as f32;
    let z = reader.read_i32()? as f32;
    reader.skip(4);
    let rotation = f32::from(reader.read_i16()?);
    reader.seek(start + MODEL_RECORD_SIZE);
    let model_id = u32::try_from(id_high * 100 + id_low)
        .map_err(|_| Arena2Error::format(format!("invalid model id {id_high}{id_low:02}")))?;
    Ok(ModelRecord { model_id, position: Vec3::new(x, -y, z), rotation })
}

/// Reads one flat record: its texture and position in native units (Y flipped up).
fn read_flat_record(reader: &mut ByteReader<'_>) -> Arena2Result<FlatRecord> {
    let start = reader.position();
    let x = reader.read_i32()? as f32;
    let y = reader.read_i32()? as f32;
    let z = reader.read_i32()? as f32;
    let texture = TextureKey::from_bitfield(reader.read_u16()?);
    reader.seek(start + FLAT_RECORD_SIZE);
    Ok(FlatRecord { texture, position: Vec3::new(x, -y, z) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an RMB record with one building (one model, one flat) and one loose model.
    fn sample_block() -> Vec<u8> {
        let mut data = vec![1u8, 1, 0];
        for index in 0..MAX_BUILDINGS {
            let (x, z, rotation) = if index == 0 { (1000i32, 1096i32, 512i32) } else { (0, 0, 0) };
            data.extend_from_slice(&[0; 8]);
            data.extend_from_slice(&x.to_le_bytes());
            data.extend_from_slice(&z.to_le_bytes());
            data.extend_from_slice(&rotation.to_le_bytes());
        }
        data.extend(std::iter::repeat_n(0, MAX_BUILDINGS * BUILDING_DATA_SIZE + MAX_BUILDINGS * 4));
        let building_size = OBJECT_HEADER_SIZE + MODEL_RECORD_SIZE + FLAT_RECORD_SIZE;
        for index in 0..MAX_BUILDINGS {
            let size = if index == 0 { building_size as i32 } else { 0 };
            data.extend_from_slice(&size.to_le_bytes());
        }
        data.extend(std::iter::repeat_n(0, 8 + 256));
        let mut scenery = vec![255u8; 256];
        scenery[15 * 16 + 2] = 4 * 3; // Tile (2, 0): record 2.
        data.extend_from_slice(&scenery);
        data.extend(std::iter::repeat_n(0, AUTOMAP_SIZE + 13 + MAX_BUILDINGS * 13));

        let model = |id_high: i16, id_low: u8, x: i32, y: i32, z: i32, rotation: i16| {
            let mut record = Vec::new();
            record.extend_from_slice(&id_high.to_le_bytes());
            record.push(id_low);
            record.extend(std::iter::repeat_n(0, 1 + 12 + 8 + 12));
            for value in [x, y, z] {
                record.extend_from_slice(&value.to_le_bytes());
            }
            record.extend_from_slice(&[0; 4]);
            record.extend_from_slice(&rotation.to_le_bytes());
            record.extend(std::iter::repeat_n(0, MODEL_RECORD_SIZE - record.len()));
            record
        };
        let mut header = vec![0u8; OBJECT_HEADER_SIZE];
        header[0] = 1;
        header[1] = 1;
        data.extend_from_slice(&header);
        data.extend(model(5, 7, 40, -80, 0, 0));
        let mut flat = Vec::new();
        for value in [0i32, 0, 0] {
            flat.extend_from_slice(&value.to_le_bytes());
        }
        flat.extend_from_slice(&((210u16 << 7) | 3).to_le_bytes());
        flat.extend_from_slice(&[0; 3]);
        data.extend(flat);
        data.extend(model(410, 11, 0, 0, 0, 1024));
        data
    }

    #[test]
    fn places_buildings_props_and_scenery_as_daggerfall_unity_does() {
        let block = parse_rmb_block("TEST.RMB", &sample_block()).unwrap();
        assert_eq!(block.models.len(), 2);

        // The building is a quarter turn round (512 units = 90°); its model sits 40 units east of it.
        let building = block.models[0];
        assert_eq!(building.model_id, 507);
        assert!((building.yaw_degrees + 90.0).abs() < 0.01);
        let expected = Vec3::new(1000.0, 0.0, 3000.0) * WORLD_SCALE
            + Quat::from_rotation_y((-90.0f32).to_radians()) * (Vec3::new(40.0, 80.0, 0.0) * WORLD_SCALE);
        assert!(building.position.distance(expected) < 0.001, "{:?}", building.position);

        let prop = block.models[1];
        assert_eq!(prop.model_id, 41011);
        assert!((prop.yaw_degrees + 180.0).abs() < 0.01);
        assert!(prop.position.distance(Vec3::new(0.0, -4.0, 4096.0) * WORLD_SCALE) < 0.001);

        assert_eq!(block.flats.len(), 2);
        assert_eq!(block.flats[0].texture, TextureKey::new(210, 3));
        assert!(block.flats[0].base_position.distance(Vec3::new(1000.0, -6.0, 3000.0) * WORLD_SCALE) < 0.001);
        assert_eq!(block.flats[1].texture, TextureKey::new(PREVIEW_NATURE_ARCHIVE, 2));
        assert!(block.flats[1].base_position.distance(Vec3::new(512.0, -2.0, 256.0) * WORLD_SCALE) < 0.001);
    }
}

//! Parser for RDB dungeon blocks stored in `BLOCKS.BSA`.
//!
//! Each dungeon in Daggerfall is assembled from square blocks of 2048 native
//! units. An RDB block contains a table of model identifiers followed by
//! several linked lists of objects: 3D models, flats (billboard sprites,
//! including editor markers for enemies, treasure and quest locations) and
//! light sources.

use std::collections::HashSet;

use glam::IVec3;

use super::error::{Arena2Error, Arena2Result};
use super::reader::ByteReader;
use super::TextureKey;

/// Number of entries in the model reference table of every RDB block.
const MODEL_REFERENCE_COUNT: usize = 750;
/// Size of one model reference entry: a five character id and a three character tag.
const MODEL_REFERENCE_SIZE: usize = 8;
/// Size of the block header that precedes the model reference table.
const BLOCK_HEADER_SIZE: usize = 20;
/// Safety limit for linked list traversal, far above anything found in game data.
const MAX_OBJECTS_PER_LIST: usize = 100_000;

/// Resource type tag for a 3D model object.
const RESOURCE_TYPE_MODEL: u8 = 0x01;
/// Resource type tag for a light object.
const RESOURCE_TYPE_LIGHT: u8 = 0x02;
/// Resource type tag for a flat (billboard) object.
const RESOURCE_TYPE_FLAT: u8 = 0x03;

/// A fully parsed RDB dungeon block.
#[derive(Debug, Clone)]
pub struct RdbBlock {
    /// Block file name, for example `N0000004.RDB`.
    pub name: String,
    /// Every object found in the block, in file order.
    pub objects: Vec<RdbObject>,
}

/// An object placed inside an RDB block.
#[derive(Debug, Clone)]
pub struct RdbObject {
    /// Offset of the object's record in the block; actions link objects by it.
    pub offset: i32,
    /// Index of the object list this object came from. Lists roughly correspond to rooms.
    pub group_index: usize,
    /// Position in native Daggerfall units relative to the block origin.
    pub position: IVec3,
    /// Type specific data.
    pub kind: RdbObjectKind,
}

/// The type specific part of an [`RdbObject`].
#[derive(Debug, Clone)]
pub enum RdbObjectKind {
    /// A static or animated 3D model.
    Model(RdbModel),
    /// A billboard sprite or editor marker.
    Flat(RdbFlat),
    /// A point light.
    Light(RdbLight),
}

/// Model specific object data.
#[derive(Debug, Clone)]
pub struct RdbModel {
    /// Record identifier of the model in `ARCH3D.BSA`.
    pub model_id: u32,
    /// Rotation around each axis in native units (2048 units per full turn).
    pub rotation: IVec3,
    /// What sets the model's action off (low bits) and, for doors, the starting lock (bits 4 and up).
    pub trigger_and_lock: u32,
    /// Sound played by the action.
    pub sound: u8,
    /// The model's action (door, lever, moving platform and so on), if it has one.
    pub action: Option<RdbAction>,
}

/// An action attached to a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdbAction {
    /// Direction of movement (1-6), or an amount for some action types.
    pub axis: u8,
    pub duration: u16,
    pub magnitude: u16,
    /// Offset of the object the action triggers next, or negative for none.
    pub next_offset: i32,
    /// What the action does (move, rotate, open a door, teleport, hurt, …).
    pub action_type: u8,
}

/// Flat specific object data.
#[derive(Debug, Clone)]
pub struct RdbFlat {
    /// Texture shown by the billboard.
    pub texture: TextureKey,
    /// Raw flag bits (gender for enemies, behaviour for some markers).
    pub flags: u16,
    /// For fixed enemy markers, the enemy (in the low byte); for other flats, a faction id.
    pub faction_or_mobile_id: u16,
    /// Action byte; 99 makes a fixed enemy passive.
    pub action: u8,
    /// Offset of the object this flat's action triggers next, or negative for none.
    pub next_offset: i32,
}

/// Light specific object data.
#[derive(Debug, Clone)]
pub struct RdbLight {
    /// Light radius in native units.
    pub radius: u16,
    /// Two values of unknown meaning, kept as read.
    pub unknown_1: u32,
    pub unknown_2: u32,
}

/// The base fields shared by every object in a linked list.
struct ObjectHeader {
    position: IVec3,
    resource_type: u8,
    resource_offset: usize,
}

/// Parses the RDB block called `name` from its raw bytes.
pub fn parse_rdb_block(name: &str, data: &[u8]) -> Arena2Result<RdbBlock> {
    let mut header = ByteReader::new(data);
    let _unknown = header.read_u32()?;
    let width = header.read_u32()? as usize;
    let height = header.read_u32()? as usize;
    let object_root_offset = header.read_u32()? as usize;

    let model_ids = read_model_reference_table(data)?;

    let mut root_reader = ByteReader::at(data, object_root_offset);
    let mut objects = Vec::new();
    for group_index in 0..width * height {
        let root_offset = root_reader.read_i32()?;
        let Ok(root_offset) = usize::try_from(root_offset) else {
            continue; // A negative root marks an empty group.
        };
        read_object_list(data, root_offset, group_index, &model_ids, &mut objects)?;
    }

    Ok(RdbBlock { name: name.to_string(), objects })
}

/// Reads the model reference table of an RDB block: each `ARCH3D.BSA` model
/// id with the three letter tag the block gives it (for example `DOR` for an
/// opening door). Daggerfall Unity decides some behaviour, such as which
/// models are doors, by these tags.
pub fn read_model_tags(data: &[u8]) -> Arena2Result<Vec<(u32, String)>> {
    let mut reader = ByteReader::at(data, BLOCK_HEADER_SIZE);
    let mut tags = Vec::new();
    for _ in 0..MODEL_REFERENCE_COUNT {
        let entry = reader.read_bytes(MODEL_REFERENCE_SIZE)?;
        let id_text: String =
            entry[..5].iter().take_while(|&&byte| byte != 0).map(|&byte| byte as char).collect();
        if let Ok(model_id) = id_text.trim().parse::<u32>() {
            let tag: String = entry[5..].iter().map(|&byte| byte as char).collect();
            tags.push((model_id, tag));
        }
    }
    Ok(tags)
}

/// Reads the table that maps a model index to an `ARCH3D.BSA` model id.
fn read_model_reference_table(data: &[u8]) -> Arena2Result<Vec<Option<u32>>> {
    let mut reader = ByteReader::at(data, BLOCK_HEADER_SIZE);
    (0..MODEL_REFERENCE_COUNT)
        .map(|_| {
            let entry = reader.read_bytes(MODEL_REFERENCE_SIZE)?;
            let id_text: String =
                entry[..5].iter().take_while(|&&byte| byte != 0).map(|&byte| byte as char).collect();
            Ok(id_text.trim().parse::<u32>().ok())
        })
        .collect()
}

/// Walks one linked list of objects, appending every recognised object to `objects`.
fn read_object_list(
    data: &[u8],
    root_offset: usize,
    group_index: usize,
    model_ids: &[Option<u32>],
    objects: &mut Vec<RdbObject>,
) -> Arena2Result<()> {
    let mut visited_offsets = HashSet::new();
    let mut next_offset = Some(root_offset);

    while let Some(offset) = next_offset {
        if !visited_offsets.insert(offset) || visited_offsets.len() > MAX_OBJECTS_PER_LIST {
            return Err(Arena2Error::format("RDB object list contains a loop"));
        }

        let mut reader = ByteReader::at(data, offset);
        let next = reader.read_i32()?;
        let _previous = reader.read_i32()?;
        let object_header = ObjectHeader {
            position: IVec3::new(reader.read_i32()?, reader.read_i32()?, reader.read_i32()?),
            resource_type: reader.read_u8()?,
            resource_offset: reader.read_u32()? as usize,
        };

        if let Some(kind) = read_object_resource(data, &object_header, model_ids)? {
            objects.push(RdbObject {
                offset: offset as i32,
                group_index,
                position: object_header.position,
                kind,
            });
        }

        next_offset = usize::try_from(next).ok();
    }
    Ok(())
}

/// Reads the type specific resource data of an object.
///
/// Returns `Ok(None)` for objects that should be skipped, such as models that
/// reference an empty slot of the model table.
fn read_object_resource(
    data: &[u8],
    object_header: &ObjectHeader,
    model_ids: &[Option<u32>],
) -> Arena2Result<Option<RdbObjectKind>> {
    let mut reader = ByteReader::at(data, object_header.resource_offset);

    let kind = match object_header.resource_type {
        RESOURCE_TYPE_MODEL => {
            let rotation = IVec3::new(reader.read_i32()?, reader.read_i32()?, reader.read_i32()?);
            let model_index = usize::from(reader.read_u16()?);
            let trigger_and_lock = reader.read_u32()?;
            let sound = reader.read_u8()?;
            let action_offset = reader.read_i32()?;

            let Some(model_id) = model_ids.get(model_index).copied().flatten() else {
                log::warn!("RDB model object references empty model slot {model_index}");
                return Ok(None);
            };
            let action = match usize::try_from(action_offset) {
                Ok(action_offset) if action_offset > 0 => {
                    let mut action_reader = ByteReader::at(data, action_offset);
                    Some(RdbAction {
                        axis: action_reader.read_u8()?,
                        duration: action_reader.read_u16()?,
                        magnitude: action_reader.read_u16()?,
                        next_offset: action_reader.read_i32()?,
                        action_type: action_reader.read_u8()?,
                    })
                }
                _ => None,
            };
            RdbObjectKind::Model(RdbModel { model_id, rotation, trigger_and_lock, sound, action })
        }
        RESOURCE_TYPE_FLAT => {
            let texture = TextureKey::from_bitfield(reader.read_u16()?);
            let flags = reader.read_u16()?;
            let faction_or_mobile_id = reader.read_u16()?;
            let next_offset = reader.read_i32()?;
            let action = reader.read_u8()?;
            RdbObjectKind::Flat(RdbFlat { texture, flags, faction_or_mobile_id, action, next_offset })
        }
        RESOURCE_TYPE_LIGHT => {
            let unknown_1 = reader.read_u32()?;
            let unknown_2 = reader.read_u32()?;
            let radius = reader.read_u16()?;
            RdbObjectKind::Light(RdbLight { radius, unknown_1, unknown_2 })
        }
        other => {
            log::warn!("skipping RDB object with unknown resource type {other}");
            return Ok(None);
        }
    };
    Ok(Some(kind))
}

//! Merging several cells of a dungeon into one cell.
//!
//! The merged cell takes the grid spot of one of them, the *anchor* (the
//! dungeon's start cell if it is among them, otherwise the one furthest
//! towards negative X, then negative Z). Every other cell's objects keep
//! their place in the dungeon: they move by the distance between the cells'
//! spots, so they reach outside the anchor's square, which a cell may do.
//!
//! What a cell holds only once is kept once:
//! * Start markers (which also hold the cell's water level) are kept from the
//!   anchor, or from the first cell that has one; the others are dropped. If
//!   the cells had different water levels the merged cell has that one's, and
//!   [`MergedCell::warnings`] says so.
//! * Exit doors only work in the start cell, so those of the other cells are
//!   dropped, as the game drops them.
//!
//! Object ids are numbered afresh, with the links between a cell's objects
//! kept; each cell's object lists (groups) stay apart.

use std::collections::HashMap;

use glam::Vec3;

use crate::arena2::RDB_BLOCK_SIDE;
use crate::scene::{
    ObjectId, Scene, SceneObject, SceneObjectKind, ADDED_OBJECTS_GROUP, MARKER_TEXTURE_ARCHIVE,
    START_MARKER_RECORD,
};

/// The most different models one cell can use.
pub const MAX_MODELS_PER_CELL: usize = 750;

/// One cell to merge.
#[derive(Debug, Clone, Copy)]
pub struct MergeSource<'cell> {
    /// Its objects, in its own (block) coordinates.
    pub objects: &'cell [SceneObject],
    /// Its grid spot in the dungeon (column, row).
    pub spot: (i32, i32),
    /// Whether it is the dungeon's start cell.
    pub is_start: bool,
}

/// The result of merging cells.
#[derive(Debug, Clone, PartialEq)]
pub struct MergedCell {
    /// Its objects, in the anchor's block coordinates.
    pub objects: Vec<SceneObject>,
    /// Index (in the sources) of the cell whose grid spot it takes.
    pub anchor: usize,
    /// Its grid spot: the anchor's.
    pub spot: (i32, i32),
    /// Whether it is the dungeon's start cell (one of the cells was).
    pub is_start: bool,
    /// What the user should know about the merge, such as water levels lost.
    pub warnings: Vec<String>,
}

/// Merges `sources` into one cell. Fails if there are fewer than two or the
/// result would use more models than a cell can.
pub fn merge_cells(sources: &[MergeSource<'_>]) -> Result<MergedCell, String> {
    if sources.len() < 2 {
        return Err("Select at least two cells to merge".to_string());
    }
    let anchor = anchor_of(sources);
    let anchor_spot = sources[anchor].spot;
    // The anchor's objects come first, so its start marker is the cell's first.
    let order: Vec<usize> =
        std::iter::once(anchor).chain((0..sources.len()).filter(|&index| index != anchor)).collect();
    let marker_source =
        order.iter().copied().find(|&index| sources[index].objects.iter().any(is_start_marker));

    let mut warnings = Vec::new();
    if let Some(kept) = marker_source {
        let level = |index: usize| {
            Scene { block_name: String::new(), objects: sources[index].objects.to_vec() }.water_level()
        };
        let kept_level = level(kept);
        let differs = (0..sources.len()).any(|index| {
            index != kept && sources[index].objects.iter().any(is_start_marker) && level(index) != kept_level
        });
        if differs {
            warnings.push(match kept_level {
                Some(_) => {
                    "The cells had different water levels; the merged cell keeps one of them".to_string()
                }
                None => "Some of the cells had water; the merged cell has none".to_string(),
            });
        }
    }

    let mut objects = Vec::new();
    let mut next_id: ObjectId = 1;
    let mut group_base = 0;
    for &index in &order {
        let source = &sources[index];
        let offset = Vec3::new(
            (source.spot.0 - anchor_spot.0) as f32 * RDB_BLOCK_SIDE,
            0.0,
            (source.spot.1 - anchor_spot.1) as f32 * RDB_BLOCK_SIDE,
        );
        let kept: Vec<&SceneObject> = source
            .objects
            .iter()
            .filter(|object| source.is_start || !object.is_exit_door())
            .filter(|object| !is_start_marker(object) || Some(index) == marker_source)
            .collect();
        let ids: HashMap<ObjectId, ObjectId> = kept
            .iter()
            .map(|object| {
                let id = next_id;
                next_id += 1;
                (object.id, id)
            })
            .collect();
        let groups = kept
            .iter()
            .map(|object| object.group_index)
            .filter(|&group| group != ADDED_OBJECTS_GROUP)
            .max()
            .map_or(0, |group| group + 1);
        objects.extend(kept.into_iter().map(|object| SceneObject {
            id: ids[&object.id],
            next: object.next.and_then(|next| ids.get(&next).copied()),
            position: object.position + offset,
            group_index: if object.group_index == ADDED_OBJECTS_GROUP {
                ADDED_OBJECTS_GROUP
            } else {
                group_base + object.group_index
            },
            ..object.clone()
        }));
        group_base += groups;
    }

    let models = distinct_models(&objects);
    if models > MAX_MODELS_PER_CELL {
        return Err(format!(
            "The merged cell would use {models} different models; a cell can use at most {MAX_MODELS_PER_CELL}"
        ));
    }
    Ok(MergedCell {
        objects,
        anchor,
        spot: anchor_spot,
        is_start: sources.iter().any(|source| source.is_start),
        warnings,
    })
}

/// The cell whose spot the merged cell takes: the start cell, or else the
/// one with the lowest column, then row.
fn anchor_of(sources: &[MergeSource<'_>]) -> usize {
    sources.iter().position(|source| source.is_start).unwrap_or_else(|| {
        (0..sources.len()).min_by_key(|&index| sources[index].spot).expect("there are sources")
    })
}

fn is_start_marker(object: &SceneObject) -> bool {
    matches!(object.kind, SceneObjectKind::Flat { texture, .. }
        if texture.archive == MARKER_TEXTURE_ARCHIVE && texture.record == START_MARKER_RECORD)
}

/// How many different models `objects` use.
pub fn distinct_models(objects: &[SceneObject]) -> usize {
    let mut models: Vec<u32> = objects
        .iter()
        .filter_map(|object| match object.kind {
            SceneObjectKind::Model { model_id, .. } => Some(model_id),
            _ => None,
        })
        .collect();
    models.sort_unstable();
    models.dedup();
    models.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena2::TextureKey;
    use crate::scene::{FlatData, ModelData};

    fn model(id: ObjectId, model_id: u32, group: usize, x: f32, next: Option<ObjectId>) -> SceneObject {
        SceneObject {
            name: format!("Model {model_id}"),
            group_index: group,
            kind: SceneObjectKind::Model { model_id, data: ModelData::default() },
            position: Vec3::new(x, 0.0, 1.0),
            rotation_degrees: Vec3::ZERO,
            visible: true,
            id,
            next,
        }
    }

    fn start_marker(id: ObjectId, water_steps: u16) -> SceneObject {
        SceneObject {
            name: "Start".to_string(),
            group_index: 0,
            kind: SceneObjectKind::Flat {
                texture: TextureKey::new(MARKER_TEXTURE_ARCHIVE, START_MARKER_RECORD),
                data: FlatData { flags: 0, faction_or_mobile_id: water_steps << 8, action: 0 },
            },
            position: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            visible: true,
            id,
            next: None,
        }
    }

    #[test]
    fn merges_cells_keeping_their_places_and_links() {
        let first = vec![start_marker(1, 0), model(2, 55000, 0, 3.0, Some(3)), model(3, 55001, 1, 4.0, None)];
        // The same ids again, and an exit door (70300), in a cell one column further on.
        let second =
            vec![start_marker(1, 0), model(2, 55000, 0, 3.0, Some(3)), model(3, 70300, 0, 5.0, None)];
        let merged = merge_cells(&[
            MergeSource { objects: &second, spot: (1, 0), is_start: false },
            MergeSource { objects: &first, spot: (0, 0), is_start: false },
        ])
        .unwrap();
        // The cell furthest towards negative X leads and keeps its spot.
        assert_eq!((merged.anchor, merged.spot, merged.is_start), (1, (0, 0), false));
        // One start marker; the other cell's exit door is gone (it is not the start).
        let names: Vec<&str> = merged.objects.iter().map(|object| object.name.as_str()).collect();
        assert_eq!(names, ["Start", "Model 55000", "Model 55001", "Model 55000"]);
        // The second cell's objects moved one cell along, keeping their place in the dungeon.
        assert_eq!(merged.objects[3].position, Vec3::new(3.0 + RDB_BLOCK_SIDE, 0.0, 1.0));
        // Ids are new and unique; links stay inside each cell (the second cell's link
        // led to the dropped exit door, so it is gone).
        let ids: Vec<ObjectId> = merged.objects.iter().map(|object| object.id).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        assert_eq!(merged.objects[1].next, Some(3));
        assert_eq!(merged.objects[3].next, None);
        // Object lists stay apart: the first cell used groups 0 and 1, so the second's start at 2.
        assert_eq!(merged.objects[3].group_index, 2);
        assert!(merged.warnings.is_empty());
    }

    #[test]
    fn the_start_cell_leads_and_keeps_its_exit() {
        let start = vec![start_marker(1, 0), model(2, 70300, 0, 0.0, None)];
        let other = vec![start_marker(1, 3)];
        let merged = merge_cells(&[
            MergeSource { objects: &other, spot: (-1, 0), is_start: false },
            MergeSource { objects: &start, spot: (0, 0), is_start: true },
        ])
        .unwrap();
        assert_eq!((merged.anchor, merged.spot, merged.is_start), (1, (0, 0), true));
        assert!(merged.objects.iter().any(SceneObject::is_exit_door));
        assert_eq!(merged.objects.iter().filter(|object| is_start_marker(object)).count(), 1);
        // The other cell had water; the start cell's marker, without, is kept.
        assert_eq!(merged.warnings.len(), 1);
    }

    #[test]
    fn needs_two_cells() {
        let cell = vec![start_marker(1, 0)];
        assert!(merge_cells(&[MergeSource { objects: &cell, spot: (0, 0), is_start: false }]).is_err());
    }
}

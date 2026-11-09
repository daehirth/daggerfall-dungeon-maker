//! Open faces of models: where one dungeon piece can join another.
//!
//! Daggerfall's dungeon pieces (corridors, rooms, stairs, doorways) join where
//! one piece's opening meets another's. An opening is found on the sides of a
//! model's bounding box: polygons lying flat on a side close it off, and the
//! parts of the side they leave uncovered are openings. A corridor's ends are
//! wholly open; a room's wall may have a doorway-sized opening in it.
//!
//! Only the four upright sides are used: the floors and ceilings of pieces
//! rarely reach their box's edges, which would look like openings.
//!
//! Thin models such as doors have no openings of their own; instead their
//! middle plane is a "plug" that fits into an opening of the same size. Doors
//! mostly stand in doorways inside a piece rather than on its sides, so the
//! places doors stand in each piece are also learned from Daggerfall's own
//! cells ([`DoorSlots`]).
//!
//! An opening nothing closes leads out of the dungeon into empty space
//! ([`unclosed_faces`]): no other piece's opening or door is joined to it,
//! and looking out through it meets nothing. Some openings of some pieces
//! (the space under a ramp, the sides of cave chunks) look open by this test
//! but are left so in Daggerfall's own dungeons too; those are learned from
//! the game's dungeons ([`OpeningsLeftOpen`]) and not counted.

use std::collections::{HashMap, HashSet};

use glam::{Quat, Vec2, Vec3};

use crate::arena2::{Arena2Library, ModelMesh, RDB_BLOCK_SIDE};
use crate::scene::{Scene, SceneObject, SceneObjectKind};

/// How far (world units) a polygon may be from a side and still lie on it.
const ON_SIDE_TOLERANCE: f32 = 0.05;
/// Cells along each edge of the grid a side is checked on.
const SIDE_GRID: usize = 32;
/// Smallest opening (world units, both ways), so cracks and slivers are ignored.
const MIN_OPENING_SIZE: f32 = 0.8;
/// A model is a slab (like a door) when its thinnest side is at most this
/// fraction of the next thinnest, and at most [`MAX_SLAB_THICKNESS`].
const SLAB_RATIO: f32 = 0.2;
const MAX_SLAB_THICKNESS: f32 = 0.6;

/// An opening (or a slab's plug) in model space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpenFace {
    /// Middle of the face.
    pub center: Vec3,
    /// Unit normal pointing out of the model (for a plug, either way).
    pub normal: Vec3,
    /// Unit vectors along the face's sides, and half its size along each.
    pub axes: [Vec3; 2],
    pub half_size: Vec2,
    /// Whether this is a slab's plug rather than an opening.
    pub is_plug: bool,
}

impl OpenFace {
    /// The face of an object at `position`, turned by `rotation`.
    pub fn transformed(&self, rotation: Quat, position: Vec3) -> Self {
        Self {
            center: position + rotation * self.center,
            normal: rotation * self.normal,
            axes: [rotation * self.axes[0], rotation * self.axes[1]],
            ..*self
        }
    }

    /// Half the face's size measured along `axes` (for comparing faces that
    /// lie in the same plane but whose own axes may be swapped).
    fn half_size_along(&self, axes: [Vec3; 2]) -> Vec2 {
        let along = |axis: Vec3| {
            self.axes[0].dot(axis).abs() * self.half_size.x + self.axes[1].dot(axis).abs() * self.half_size.y
        };
        Vec2::new(along(axes[0]), along(axes[1]))
    }

    /// The four corners of the face.
    pub fn corners(&self) -> [Vec3; 4] {
        let u = self.axes[0] * self.half_size.x;
        let v = self.axes[1] * self.half_size.y;
        [self.center - u - v, self.center + u - v, self.center + u + v, self.center - u + v]
    }
}

/// Finds the openings of `mesh`, or its plug if it is a slab.
pub fn open_faces(mesh: &ModelMesh) -> Vec<OpenFace> {
    let size = mesh.bounds_max - mesh.bounds_min;
    if let Some(plug) = slab_plug(mesh, size) {
        return vec![plug];
    }
    let mut faces = Vec::new();
    // The upright sides only: X and Z.
    for axis in [0, 2] {
        let (first, second) = tangent_axes(axis);
        for (plane, sign) in [(mesh.bounds_min[axis], -1.0), (mesh.bounds_max[axis], 1.0)] {
            let mut normal = Vec3::ZERO;
            normal[axis] = sign;
            let side_min = Vec2::new(mesh.bounds_min[first], mesh.bounds_min[second]);
            let side_size = Vec2::new(size[first], size[second]);
            if side_size.x < MIN_OPENING_SIZE || side_size.y < MIN_OPENING_SIZE {
                continue;
            }
            let covered = covered_cells(mesh, axis, plane, side_min, side_size);
            for (cell_min, cell_max) in uncovered_regions(&covered) {
                let cell = side_size / SIDE_GRID as f32;
                let low = side_min + Vec2::new(cell_min.0 as f32, cell_min.1 as f32) * cell;
                let high = side_min + Vec2::new(cell_max.0 as f32 + 1.0, cell_max.1 as f32 + 1.0) * cell;
                let opening = high - low;
                if opening.x < MIN_OPENING_SIZE || opening.y < MIN_OPENING_SIZE {
                    continue;
                }
                let mut center = Vec3::ZERO;
                center[axis] = plane;
                center[first] = (low.x + high.x) * 0.5;
                center[second] = (low.y + high.y) * 0.5;
                let mut axis_u = Vec3::ZERO;
                axis_u[first] = 1.0;
                let mut axis_v = Vec3::ZERO;
                axis_v[second] = 1.0;
                faces.push(OpenFace {
                    center,
                    normal,
                    axes: [axis_u, axis_v],
                    half_size: opening * 0.5,
                    is_plug: false,
                });
            }
        }
    }
    faces
}

/// The two axes lying along a side whose normal is along `axis`.
fn tangent_axes(axis: usize) -> (usize, usize) {
    match axis {
        0 => (2, 1),
        1 => (0, 2),
        _ => (0, 1),
    }
}

/// A slab's plug: its middle plane, if the model is thin one way.
fn slab_plug(mesh: &ModelMesh, size: Vec3) -> Option<OpenFace> {
    let mut order = [0usize, 1, 2];
    order.sort_by(|&first, &second| size[first].total_cmp(&size[second]));
    let (thin, middle) = (order[0], order[1]);
    if size[thin] > MAX_SLAB_THICKNESS
        || size[thin] > size[middle] * SLAB_RATIO
        || size[middle] < MIN_OPENING_SIZE
    {
        return None;
    }
    let (first, second) = tangent_axes(thin);
    let mut normal = Vec3::ZERO;
    normal[thin] = 1.0;
    let mut axis_u = Vec3::ZERO;
    axis_u[first] = 1.0;
    let mut axis_v = Vec3::ZERO;
    axis_v[second] = 1.0;
    Some(OpenFace {
        center: mesh.bounds_center(),
        normal,
        axes: [axis_u, axis_v],
        half_size: Vec2::new(size[first], size[second]) * 0.5,
        is_plug: true,
    })
}

/// Marks the cells of a side's grid that polygons lying on the side cover.
fn covered_cells(mesh: &ModelMesh, axis: usize, plane: f32, side_min: Vec2, side_size: Vec2) -> Vec<bool> {
    let (first, second) = tangent_axes(axis);
    let mut covered = vec![false; SIDE_GRID * SIDE_GRID];
    let cell = side_size / SIDE_GRID as f32;
    for submesh in &mesh.submeshes {
        for triangle in submesh.indices.as_chunks::<3>().0 {
            let points = [0, 1, 2].map(|corner| submesh.vertices[triangle[corner] as usize].position);
            if points.iter().any(|point| (point[axis] - plane).abs() > ON_SIDE_TOLERANCE) {
                continue;
            }
            let flat = points.map(|point| Vec2::new(point[first], point[second]));
            for row in 0..SIDE_GRID {
                for column in 0..SIDE_GRID {
                    let center = side_min + Vec2::new(column as f32 + 0.5, row as f32 + 0.5) * cell;
                    if point_in_triangle(center, flat) {
                        covered[row * SIDE_GRID + column] = true;
                    }
                }
            }
        }
    }
    covered
}

fn point_in_triangle(point: Vec2, [a, b, c]: [Vec2; 3]) -> bool {
    let cross = |first: Vec2, second: Vec2, third: Vec2| (second - first).perp_dot(third - first);
    let (d1, d2, d3) = (cross(a, b, point), cross(b, c, point), cross(c, a, point));
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}

/// The bounding cells (min and max column/row) of each connected uncovered region.
fn uncovered_regions(covered: &[bool]) -> Vec<((usize, usize), (usize, usize))> {
    let mut seen = covered.to_vec();
    let mut regions = Vec::new();
    for start in 0..covered.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let (mut low, mut high) = ((SIDE_GRID, SIDE_GRID), (0, 0));
        while let Some(index) = stack.pop() {
            let (column, row) = (index % SIDE_GRID, index / SIDE_GRID);
            low = (low.0.min(column), low.1.min(row));
            high = (high.0.max(column), high.1.max(row));
            let neighbours = [
                (column > 0).then(|| index - 1),
                (column + 1 < SIDE_GRID).then(|| index + 1),
                (row > 0).then(|| index - SIDE_GRID),
                (row + 1 < SIDE_GRID).then(|| index + SIDE_GRID),
            ];
            for neighbour in neighbours.into_iter().flatten() {
                if !seen[neighbour] {
                    seen[neighbour] = true;
                    stack.push(neighbour);
                }
            }
        }
        regions.push((low, high));
    }
    regions
}

/// Where a door stands in a piece, in the piece's own space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorSlot {
    /// Position and rotation a door has there.
    pub position: Vec3,
    pub rotation: Quat,
    /// The door's plug there, in the piece's space, to compare door sizes and draw it.
    pub plug: OpenFace,
}

/// The doorways of every piece, learned from Daggerfall's own cells: wherever
/// a door stands inside a piece, any door of that size can stand.
#[derive(Debug, Clone, Default)]
pub struct DoorSlots {
    by_model: HashMap<u32, Vec<DoorSlot>>,
}

impl DoorSlots {
    /// The doorways learned for model `model_id`.
    pub fn of(&self, model_id: u32) -> &[DoorSlot] {
        self.by_model.get(&model_id).map_or(&[], Vec::as_slice)
    }

    /// Number of pieces with at least one doorway.
    pub fn piece_count(&self) -> usize {
        self.by_model.len()
    }

    /// Learns the doorways of the pieces in `scene`: each door (a slab model)
    /// belongs to the smallest other model whose box holds its middle.
    pub fn learn(&mut self, scene: &Scene, library: &mut Arena2Library) {
        struct Placed {
            index: usize,
            model_id: u32,
            min: Vec3,
            max: Vec3,
            plug: Option<OpenFace>,
        }
        let mut placed = Vec::new();
        for (index, object) in scene.objects.iter().enumerate() {
            let SceneObjectKind::Model { model_id, .. } = object.kind else { continue };
            let Some(mesh) = library.model(model_id) else { continue };
            let size = mesh.bounds_max - mesh.bounds_min;
            let (min, max) = world_box(object, mesh.bounds_min, mesh.bounds_max);
            placed.push(Placed { index, model_id, min, max, plug: slab_plug(&mesh, size) });
        }
        for door in placed.iter().filter(|placed| placed.plug.is_some()) {
            let (Some(plug), door_object) = (door.plug, &scene.objects[door.index]) else { continue };
            let middle = door_object.position + door_object.rotation() * plug.center;
            let container = placed
                .iter()
                .filter(|other| other.plug.is_none())
                .filter(|other| middle.cmpge(other.min - 0.05).all() && middle.cmple(other.max + 0.05).all())
                .min_by(|first, second| {
                    let volume = |placed: &&Placed| (placed.max - placed.min).element_product();
                    volume(first).total_cmp(&volume(second))
                });
            let Some(container) = container else { continue };
            let piece = &scene.objects[container.index];
            let inverse = piece.rotation().inverse();
            let position = inverse * (door_object.position - piece.position);
            let rotation = inverse * door_object.rotation();
            let slot = DoorSlot { position, rotation, plug: plug.transformed(rotation, position) };
            let slots = self.by_model.entry(container.model_id).or_default();
            let known = slots.iter().any(|known| {
                known.plug.center.distance(slot.plug.center) < 0.1
                    && known.plug.normal.dot(slot.plug.normal).abs() > 0.99
            });
            if !known {
                slots.push(slot);
            }
        }
    }
}

/// The box around `object`'s model box (`min` to `max` in model space), in world space.
fn world_box(object: &SceneObject, min: Vec3, max: Vec3) -> (Vec3, Vec3) {
    let rotation = object.rotation();
    let mut low = Vec3::splat(f32::MAX);
    let mut high = Vec3::splat(f32::MIN);
    for corner in 0..8 {
        let point = Vec3::new(
            if corner & 1 == 0 { min.x } else { max.x },
            if corner & 2 == 0 { min.y } else { max.y },
            if corner & 4 == 0 { min.z } else { max.z },
        );
        let world = object.position + rotation * point;
        low = low.min(world);
        high = high.max(world);
    }
    (low, high)
}

/// How far (world units) a look out through an opening may go before what it
/// sees counts as empty space: more than a cell's width, so a look along a
/// long hall still meets its far end.
const LOOK_DISTANCE: f32 = 60.0;
/// How far inside an opening (world units) a wall of the piece's own may
/// stand and still close it: walls often stand a little inside a piece's box.
const OWN_WALL_DEPTH: f32 = 2.0;
/// Smallest piece (world units: height, and width or depth) whose openings
/// are checked: halls and rooms, not furniture.
const MIN_CHECKED_PIECE_SIZE: f32 = 3.0;
/// Smallest opening (world units: width, height) that is checked: one a
/// person could pass through.
const MIN_CHECKED_OPENING: Vec2 = Vec2::new(1.2, 1.6);
/// How close (world units) an opening's plane must be to a cell's edge to be
/// one where the cell meets its neighbour.
const CELL_EDGE_TOLERANCE: f32 = 0.3;

/// An opening of a model that nothing closes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnclosedFace {
    /// Index of the object whose opening it is.
    pub owner: usize,
    /// Its model, and the index of the opening among the model's openings.
    pub model_id: u32,
    pub face_index: usize,
    /// The opening, in world space.
    pub face: OpenFace,
}

/// The openings of the visible models in `objects` for which `check` is true
/// that lead to empty space.
///
/// Only pieces at least [`MIN_CHECKED_PIECE_SIZE`] tall and wide (halls and
/// rooms, not furniture) are checked, and only openings a person could pass
/// through ([`MIN_CHECKED_OPENING`]). An opening is closed when another model's
/// opening is joined to it, or when most looks out through it (from its
/// middle and four points around it) are blocked: by a wall of the piece's own
/// standing just inside it ([`OWN_WALL_DEPTH`]; walls often stand a little
/// inside a piece's box) or by another model within [`LOOK_DISTANCE`] (the
/// piece it opens into, or that piece's far end). Doors close nothing: a door
/// at the dungeon's edge opens onto nothing.
///
/// Every visible model in `objects` can close an opening, so the objects
/// around a cell (its neighbours in a dungeon) can be passed in too and left
/// unchecked. `faces_cache` keeps each model's openings between calls.
pub fn unclosed_faces(
    objects: &[SceneObject],
    check: impl Fn(usize) -> bool,
    library: &mut Arena2Library,
    faces_cache: &mut HashMap<u32, std::sync::Arc<[OpenFace]>>,
) -> Vec<UnclosedFace> {
    struct Placed {
        index: usize,
        /// Whether the model is a slab, such as a door, which closes nothing.
        is_door: bool,
        mesh: std::sync::Arc<ModelMesh>,
        /// Middle and radius of the model, for skipping far objects quickly.
        center: Vec3,
        radius: f32,
    }
    let mut placed = Vec::new();
    let mut faces: Vec<(usize, OpenFace)> = Vec::new();
    // The model and index (among the model's openings) of each face in
    // `faces`, and whether the model is a piece whose openings are checked.
    let mut face_models: Vec<(u32, usize, bool)> = Vec::new();
    for (index, object) in objects.iter().enumerate().filter(|(_, object)| object.visible) {
        let SceneObjectKind::Model { model_id, .. } = object.kind else { continue };
        let Some(mesh) = library.model(model_id) else { continue };
        let model_faces = faces_cache.entry(model_id).or_insert_with(|| open_faces(&mesh).into()).clone();
        let rotation = object.rotation();
        let center = object.position + rotation * mesh.bounds_center();
        faces.extend(model_faces.iter().map(|face| (index, face.transformed(rotation, object.position))));
        let size = mesh.bounds_max - mesh.bounds_min;
        let is_piece = size.y >= MIN_CHECKED_PIECE_SIZE && size.x.max(size.z) >= MIN_CHECKED_PIECE_SIZE;
        face_models.extend((0..model_faces.len()).map(|face_index| (model_id, face_index, is_piece)));
        let is_door = model_faces.iter().any(|face| face.is_plug);
        placed.push(Placed { index, is_door, center, radius: mesh.bounding_radius(), mesh });
    }

    // Faces by the plane they lie in, so joined faces are found without comparing every pair.
    let plane_key = |face: &OpenFace| {
        let normal = face.normal.abs();
        let axis = if normal.x >= normal.y && normal.x >= normal.z {
            0
        } else if normal.y >= normal.z {
            1
        } else {
            2
        };
        (axis, (face.center[axis] * 10.0).round() as i64)
    };
    let mut by_plane: HashMap<(usize, i64), Vec<usize>> = HashMap::new();
    for (position, (_, face)) in faces.iter().enumerate() {
        by_plane.entry(plane_key(face)).or_default().push(position);
    }

    // Whether a look from `origin` along `direction` meets a model other than
    // `owner` (or a door) within `reach`.
    let sees_other = |origin: Vec3, direction: Vec3, owner: usize, reach: f32| {
        let ray = crate::picking::Ray { origin, direction };
        placed.iter().filter(|other| !other.is_door && other.index != owner).any(|other| {
            // Skip models whose sphere the look does not come near.
            let along = (other.center - origin).dot(direction).clamp(0.0, reach);
            if (origin + direction * along).distance(other.center) > other.radius + 0.05 {
                return false;
            }
            crate::picking::intersect_mesh(&objects[other.index], &other.mesh, &ray, false)
                .is_some_and(|distance| distance <= reach)
        })
    };
    // Whether the piece's own model has a wall just inside the opening `face`, at `point`.
    let own_wall = |owner: usize, point: Vec3, face: &OpenFace| {
        let Some(own) = placed.iter().find(|placed| placed.index == owner) else { return false };
        let ray =
            crate::picking::Ray { origin: point - face.normal * OWN_WALL_DEPTH, direction: face.normal };
        crate::picking::intersect_mesh(&objects[owner], &own.mesh, &ray, false)
            .is_some_and(|distance| distance <= OWN_WALL_DEPTH + 0.05)
    };

    let mut unclosed = Vec::new();
    for (position, (owner, face)) in faces.iter().enumerate() {
        if face.is_plug || !check(*owner) || !face_models[position].2 {
            continue;
        }
        // Only openings a person could pass through.
        let size = face.half_size_along([Vec3::new(face.normal.z, 0.0, -face.normal.x), Vec3::Y]) * 2.0;
        if size.x < MIN_CHECKED_OPENING.x || size.y < MIN_CHECKED_OPENING.y {
            continue;
        }
        let (axis, key) = plane_key(face);
        let joined =
            (key - 1..=key + 1).filter_map(|key| by_plane.get(&(axis, key))).flatten().any(|&other| {
                let (other_owner, other_face) = &faces[other];
                other_owner != owner && !other_face.is_plug && occupies(other_face, face)
            });
        if joined {
            continue;
        }
        // Look out through the opening's middle and four points around it:
        // a wall of the piece's own just inside, or another model ahead, closes it.
        let [u, v] = [face.axes[0] * face.half_size.x * 0.5, face.axes[1] * face.half_size.y * 0.5];
        let points = [face.center, face.center + u, face.center - u, face.center + v, face.center - v];
        let blocked = points
            .iter()
            .filter(|&&point| {
                own_wall(*owner, point, face)
                    || sees_other(point - face.normal * 0.05, face.normal, *owner, LOOK_DISTANCE)
            })
            .count();
        let closed = blocked * 2 > points.len();
        if !closed {
            let (model_id, face_index, _) = face_models[position];
            unclosed.push(UnclosedFace { owner: *owner, model_id, face_index, face: *face });
        }
    }
    unclosed
}

/// How often (at least one in this many placements of a model) Daggerfall's
/// own dungeons must leave an opening open for it to count as fine to leave open.
const LEFT_OPEN_RATE: usize = 50;

/// The openings of pieces that Daggerfall's own dungeons often leave open,
/// so leaving them open is not a problem: by model id and index among the
/// model's openings ([`open_faces`]).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpeningsLeftOpen {
    openings: HashSet<(u32, usize)>,
}

impl OpeningsLeftOpen {
    /// Version of the learning; a saved copy made by another version is learned again.
    pub const VERSION: u32 = 1;

    /// Learns them from every dungeon in the game data: an opening counts if
    /// at least one in [`LEFT_OPEN_RATE`] placements of its model leaves it
    /// open. Takes several seconds, so it is meant for a background thread.
    pub fn learn(library: &mut Arena2Library) -> Self {
        let dungeons = library.dungeons();
        let mut cells: HashMap<String, Option<Scene>> = HashMap::new();
        let mut faces_cache = HashMap::new();
        let mut placements: HashMap<u32, usize> = HashMap::new();
        let mut open: HashMap<(u32, usize), usize> = HashMap::new();
        for dungeon in dungeons.iter() {
            let mut objects = Vec::new();
            for block in &dungeon.blocks {
                let cell = cells.entry(block.block_name.clone()).or_insert_with(|| {
                    library.rdb_block(&block.block_name).ok().map(|block| Scene::from_rdb_block(&block))
                });
                let Some(mut scene) = cell.clone() else { continue };
                // As the game shows them: only the start cell keeps its exit doors.
                if !block.is_starting_block {
                    scene.remove_exit_doors();
                }
                let origin = Vec3::new(block.grid_x as f32, 0.0, block.grid_z as f32) * RDB_BLOCK_SIDE;
                objects.extend(
                    scene
                        .objects
                        .into_iter()
                        .map(|object| SceneObject { position: object.position + origin, ..object }),
                );
            }
            for object in &objects {
                if let SceneObjectKind::Model { model_id, .. } = object.kind {
                    *placements.entry(model_id).or_default() += 1;
                }
            }
            for face in unclosed_faces(&objects, |_| true, library, &mut faces_cache) {
                *open.entry((face.model_id, face.face_index)).or_default() += 1;
            }
        }
        let openings = open
            .into_iter()
            .filter(|((model_id, _), count)| {
                count * LEFT_OPEN_RATE >= placements.get(model_id).copied().unwrap_or(0)
            })
            .map(|(key, _)| key)
            .collect();
        Self { openings }
    }

    /// Whether opening `face_index` of model `model_id` may be left open.
    pub fn contains(&self, model_id: u32, face_index: usize) -> bool {
        self.openings.contains(&(model_id, face_index))
    }

    /// How many openings are known to be fine to leave open.
    pub fn len(&self) -> usize {
        self.openings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.openings.is_empty()
    }
}

/// Whether `face` lies on the edge of the cell (the square `side` wide from
/// the origin) and faces out of it: where the cell meets its neighbour.
pub fn is_on_cell_edge(face: &OpenFace, side: f32) -> bool {
    let on = |value: f32, edge: f32| (value - edge).abs() <= CELL_EDGE_TOLERANCE;
    (face.normal.x < -0.99 && on(face.center.x, 0.0))
        || (face.normal.x > 0.99 && on(face.center.x, side))
        || (face.normal.z < -0.99 && on(face.center.z, 0.0))
        || (face.normal.z > 0.99 && on(face.center.z, side))
}

/// A way to snap an object being placed onto one already placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    /// Where the object goes.
    pub position: Vec3,
    /// How it is turned, when the snap decides that (a door in a doorway).
    pub rotation: Option<Quat>,
    /// The face it snapped to, in world space, to show.
    pub face: OpenFace,
    /// How different the two faces' sizes are (0 = the same).
    pub mismatch: f32,
    /// How far the object moved to snap, in world units.
    pub distance: f32,
}

impl Snap {
    /// Lower is better: a closer size match first, then a shorter move.
    pub fn score(&self, snap_distance: f32) -> f32 {
        self.mismatch * 4.0 + self.distance / snap_distance
    }
}

/// How to move an object so its opening `moving` joins opening `fixed`:
/// the two planes are brought together and the bottoms lined up (so floors
/// meet). Openings of the same width are centred on each other; a narrower
/// one may slide along a wider one, staying within it.
pub fn join_offset(moving: &OpenFace, fixed: &OpenFace) -> Option<Vec3> {
    if moving.is_plug || fixed.is_plug || moving.normal.dot(fixed.normal) > -0.99 {
        return None;
    }
    let normal = fixed.normal;
    let along = fixed.axes.into_iter().find(|axis| axis.y.abs() < 0.5)?;
    let moving_half = moving.half_size_along([along, Vec3::Y]);
    let fixed_half = fixed.half_size_along([along, Vec3::Y]);
    let to_plane = normal * (fixed.center - moving.center).dot(normal);
    let offset_along = (moving.center - fixed.center).dot(along);
    let slack = (fixed_half.x - moving_half.x).abs();
    let slide = if slack < 0.05 { 0.0 } else { offset_along.clamp(-slack, slack) };
    let to_along = along * (slide - offset_along);
    let bottom = |center: Vec3, half: Vec2| center.y - half.y;
    let to_floor = Vec3::Y * (bottom(fixed.center, fixed_half) - bottom(moving.center, moving_half));
    Some(to_plane + to_along + to_floor)
}

/// Whether `placed` (a face of an object already placed) is joined to
/// opening `face`: it lies in the face's plane, inside it, and either faces
/// it (another piece's opening) or is a plug (a door standing in it).
pub fn occupies(placed: &OpenFace, face: &OpenFace) -> bool {
    if face.is_plug {
        return false;
    }
    let facing = placed.is_plug || placed.normal.dot(face.normal) < -0.99;
    let offset = placed.center - face.center;
    facing
        && offset.dot(face.normal).abs() < 0.1
        && offset.dot(face.axes[0]).abs() <= face.half_size.x + 0.05
        && offset.dot(face.axes[1]).abs() <= face.half_size.y + 0.05
}

/// How to move a plug (a door) so it stands in opening `fixed`: centred
/// across the opening, in its plane, standing on its bottom edge.
pub fn plug_offset(plug: &OpenFace, fixed: &OpenFace) -> Vec3 {
    let plug_half =
        plug.half_size_along([Vec3::X, Vec3::Y]).y.max(plug.half_size_along([Vec3::Z, Vec3::Y]).y);
    let fixed_half =
        fixed.half_size_along([Vec3::X, Vec3::Y]).y.max(fixed.half_size_along([Vec3::Z, Vec3::Y]).y);
    let mut offset = fixed.center - plug.center;
    // Upright openings: stand on the bottom. (A plug lying flat, like a trapdoor, is centred.)
    if fixed.normal.y.abs() < 0.5 {
        offset.y = (fixed.center.y - fixed_half) - (plug.center.y - plug_half);
    }
    offset
}

/// How well face `moving` (of the object being placed) fits face `fixed` (of
/// one already placed): `None` if they cannot join, otherwise how different
/// their sizes are, from 0 (the same) upwards.
///
/// Two openings join face to face, pointing at each other. A plug joins an
/// opening lying in the same plane, either way round.
pub fn fit(moving: &OpenFace, fixed: &OpenFace) -> Option<f32> {
    if moving.is_plug && fixed.is_plug {
        return None;
    }
    let alignment = moving.normal.dot(fixed.normal);
    let joins = match moving.is_plug || fixed.is_plug {
        true => alignment.abs() > 0.99,
        false => alignment < -0.99,
    };
    if !joins {
        return None;
    }
    let moving_size = moving.half_size_along(fixed.axes);
    let difference = (moving_size - fixed.half_size).abs();
    Some((difference.x + difference.y) / (fixed.half_size.x + fixed.half_size.y).max(0.01))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena2::{MeshVertex, SubMesh, TextureKey};

    /// A box mesh of `size` at the origin with the given sides (by normal) left out.
    fn box_mesh(size: Vec3, open: &[Vec3]) -> ModelMesh {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for axis in 0..3 {
            let (first, second) = tangent_axes(axis);
            for sign in [-1.0f32, 1.0] {
                let mut normal = Vec3::ZERO;
                normal[axis] = sign;
                if open.contains(&normal) {
                    continue;
                }
                let base = vertices.len() as u32;
                for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                    let mut position = Vec3::ZERO;
                    position[axis] = if sign > 0.0 { size[axis] } else { 0.0 };
                    position[first] = u * size[first];
                    position[second] = v * size[second];
                    vertices.push(MeshVertex { position, normal, uv_texels: Vec2::ZERO });
                }
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        ModelMesh {
            model_id: 1,
            submeshes: vec![SubMesh { texture: TextureKey::new(1, 0), vertices, indices }],
            bounds_min: Vec3::ZERO,
            bounds_max: size,
        }
    }

    #[test]
    fn finds_the_open_ends_of_a_corridor() {
        // The open top is ignored: only upright sides count.
        let corridor = box_mesh(Vec3::new(4.0, 3.0, 8.0), &[Vec3::Z, Vec3::NEG_Z, Vec3::Y]);
        let faces = open_faces(&corridor);
        assert_eq!(faces.len(), 2);
        for face in &faces {
            assert!(!face.is_plug);
            assert!(face.normal.z.abs() > 0.99);
            assert!((face.half_size - Vec2::new(2.0, 1.5)).length() < 0.01, "{:?}", face.half_size);
        }
        // Two corridors join end to end, perfectly.
        let other = faces[0].transformed(Quat::IDENTITY, Vec3::new(0.0, 0.0, -8.0));
        let end = faces.iter().find(|face| face.normal.z > 0.0).unwrap();
        assert_eq!(fit(end, &faces.iter().find(|face| face.normal.z < 0.0).unwrap().clone()), Some(0.0));
        assert!(fit(end, end).is_none(), "faces pointing the same way do not join");
        assert!(other.center.z < 0.1);
    }

    #[test]
    fn a_joined_face_is_taken() {
        let corridor = open_faces(&box_mesh(Vec3::new(4.0, 3.0, 8.0), &[Vec3::Z, Vec3::NEG_Z]));
        let front = *corridor.iter().find(|face| face.normal.z > 0.0).unwrap();
        let back = *corridor.iter().find(|face| face.normal.z < 0.0).unwrap();
        // The next corridor's back end sits on this one's front end.
        let next_back = back.transformed(Quat::IDENTITY, Vec3::new(0.0, 0.0, 8.0));
        assert!(occupies(&next_back, &front));
        // Not when it is somewhere else, or facing the same way.
        assert!(!occupies(&back.transformed(Quat::IDENTITY, Vec3::new(0.0, 0.0, 20.0)), &front));
        assert!(!occupies(&front.transformed(Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.0)), &front));
        // A door standing in the opening takes it too.
        let door = open_faces(&box_mesh(Vec3::new(2.0, 3.0, 0.2), &[]))[0];
        assert!(occupies(&door.transformed(Quat::IDENTITY, Vec3::new(1.0, 0.0, 7.9)), &front));
    }

    #[test]
    fn joins_line_up_planes_and_floors() {
        let corridor = open_faces(&box_mesh(Vec3::new(4.0, 3.0, 8.0), &[Vec3::Z, Vec3::NEG_Z]));
        let front = corridor.iter().find(|face| face.normal.z > 0.0).unwrap();
        let back = corridor.iter().find(|face| face.normal.z < 0.0).unwrap();
        // A corridor dropped a little off the end of another, and a bit high, snaps flush.
        let moving = back.transformed(Quat::IDENTITY, Vec3::new(0.7, 0.4, 8.9));
        let offset = join_offset(&moving, front).unwrap();
        assert!((moving.center + offset - front.center).length() < 1e-4, "{offset:?}");

        // A narrow corridor may slide along a wide opening, but not past its edge.
        let wide = open_faces(&box_mesh(Vec3::new(8.0, 3.0, 8.0), &[Vec3::Z]))[0];
        let narrow = back.transformed(Quat::IDENTITY, Vec3::new(1.0, 0.0, 8.5));
        let offset = join_offset(&narrow, &wide).unwrap();
        assert!((offset - Vec3::new(0.0, 0.0, -0.5)).length() < 1e-4, "{offset:?}");
        let far = back.transformed(Quat::IDENTITY, Vec3::new(5.0, 0.0, 8.0));
        let offset = join_offset(&far, &wide).unwrap();
        assert!((offset.x + 1.0).abs() < 1e-4, "{offset:?}");
    }

    #[test]
    fn a_closed_box_has_no_openings_and_a_door_is_a_plug() {
        assert!(open_faces(&box_mesh(Vec3::new(4.0, 3.0, 4.0), &[])).is_empty());
        let door = open_faces(&box_mesh(Vec3::new(2.0, 3.0, 0.2), &[]));
        assert_eq!(door.len(), 1);
        assert!(door[0].is_plug);
        assert!(door[0].normal.z.abs() > 0.99);
        // It fits the end of a corridor of its size, either way round.
        let corridor = open_faces(&box_mesh(Vec3::new(2.0, 3.0, 8.0), &[Vec3::Z, Vec3::NEG_Z]));
        assert_eq!(fit(&door[0], &corridor[0]), Some(0.0));
        assert_eq!(fit(&door[0], &corridor[1]), Some(0.0));
        let wide = open_faces(&box_mesh(Vec3::new(4.0, 3.0, 8.0), &[Vec3::Z]));
        assert!(fit(&door[0], &wide[0]).unwrap() > 0.2);
        // A short door stands on the floor of a tall opening.
        let short = open_faces(&box_mesh(Vec3::new(2.0, 2.0, 0.2), &[]))[0];
        let offset = plug_offset(&short.transformed(Quat::IDENTITY, Vec3::new(0.0, 0.7, 7.0)), &corridor[0]);
        let moved = short.center + Vec3::new(0.0, 0.7, 7.0) + offset;
        assert!((moved.y - 1.0).abs() < 1e-4, "{moved:?}");
    }
}

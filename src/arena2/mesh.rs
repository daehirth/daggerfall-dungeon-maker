//! Parser for 3D models stored in `ARCH3D.BSA`.
//!
//! Each model is a list of planar polygons ("planes"). Every plane references
//! a texture and a set of points from a shared point list. Texture
//! coordinates are stored in an unusual way: the first point holds absolute
//! coordinates, the next two hold deltas, and coordinates for any further
//! points must be reconstructed from an affine mapping solved from the first
//! three points. This reconstruction follows the approach used by
//! Daggerfall Unity's `Arch3dFile` and `FaceUVTool`.
//!
//! Output vertices are converted into the viewer's world space (see
//! [`super::native_to_world`]) so models can be placed directly in a scene.

use std::collections::HashMap;

use glam::{DVec3, IVec3, Vec2, Vec3};

use super::error::{Arena2Error, Arena2Result};
use super::reader::ByteReader;
use super::{TextureKey, WORLD_SCALE};

/// Native point coordinates are stored as fixed point values with 8 fractional bits.
const POINT_DIVISOR: f32 = 256.0;
/// Native texture coordinates are stored in 1/16th of a texel.
const TEXTURE_COORDINATE_DIVISOR: f32 = 16.0;
/// Models with an id below this value use packed texture coordinates.
const PACKED_UV_MODEL_ID_LIMIT: u32 = 905;
/// Minimum angle (radians) between edges for a point to count as a polygon corner.
const CORNER_ANGLE_THRESHOLD: f64 = 0.001;
/// Size in bytes of the fixed model header.
const MODEL_HEADER_SIZE: usize = 64;

/// A single vertex of a model, in world space units relative to the model origin.
#[derive(Debug, Clone, Copy)]
pub struct MeshVertex {
    /// Vertex position.
    pub position: Vec3,
    /// Unit length surface normal.
    pub normal: Vec3,
    /// Texture coordinate measured in texels. Divide by the texture size to normalise.
    pub uv_texels: Vec2,
}

/// The part of a model that uses one texture.
#[derive(Debug, Clone)]
pub struct SubMesh {
    /// Texture applied to every triangle in this sub-mesh.
    pub texture: TextureKey,
    /// Vertex list referenced by [`SubMesh::indices`].
    pub vertices: Vec<MeshVertex>,
    /// Triangle list, three indices per triangle.
    pub indices: Vec<u32>,
}

/// A fully decoded model ready to be uploaded to the GPU or used for picking.
#[derive(Debug, Clone)]
pub struct ModelMesh {
    /// Record identifier of the model within `ARCH3D.BSA`.
    pub model_id: u32,
    /// Geometry grouped by texture.
    pub submeshes: Vec<SubMesh>,
    /// Minimum corner of the axis aligned bounding box.
    pub bounds_min: Vec3,
    /// Maximum corner of the axis aligned bounding box.
    pub bounds_max: Vec3,
}

impl ModelMesh {
    /// Centre of the bounding box.
    pub fn bounds_center(&self) -> Vec3 {
        (self.bounds_min + self.bounds_max) * 0.5
    }

    /// Radius of a sphere centred on [`Self::bounds_center`] enclosing the whole model.
    pub fn bounding_radius(&self) -> f32 {
        (self.bounds_max - self.bounds_min).length() * 0.5
    }

    /// Total number of triangles across all sub-meshes.
    pub fn triangle_count(&self) -> usize {
        self.submeshes.iter().map(|submesh| submesh.indices.len() / 3).sum()
    }

    /// Width, height and depth of the bounding box, in world units.
    pub fn size(&self) -> Vec3 {
        self.bounds_max - self.bounds_min
    }

    /// Number of triangles drawn with each texture archive, most used archive first.
    ///
    /// Archives with equal counts are ordered by archive number.
    pub fn triangles_per_texture_archive(&self) -> Vec<(u16, usize)> {
        let mut counts: HashMap<u16, usize> = HashMap::new();
        for submesh in &self.submeshes {
            *counts.entry(submesh.texture.archive).or_default() += submesh.indices.len() / 3;
        }
        let mut counts: Vec<(u16, usize)> = counts.into_iter().collect();
        counts.sort_by(|first, second| second.1.cmp(&first.1).then(first.0.cmp(&second.0)));
        counts
    }

    /// The most important kind of door this model contains, if any.
    ///
    /// Daggerfall marks door faces by drawing them with one of a few door
    /// texture archives, so a model "is" a door if any face uses one.
    pub fn door_kind(&self) -> Option<DoorKind> {
        self.submeshes.iter().filter_map(|submesh| DoorKind::from_texture(submesh.texture)).max()
    }
}

/// Kinds of door the game recognises from a face's texture, least important first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DoorKind {
    /// A door into a building (texture archive 74 and its climate variants).
    Building,
    /// The entrance of a dungeon, seen from outside (archives 56 and 331).
    DungeonEntrance,
    /// The door that leads out of a dungeon (archive 95).
    DungeonExit,
}

impl DoorKind {
    /// Archive of building door textures.
    const BUILDING_DOOR_ARCHIVE: u16 = 74;
    /// Archive of dungeon entrance textures.
    const DUNGEON_ENTRANCE_ARCHIVE: u16 = 56;
    /// Archive of ruined dungeon entrance textures; its record 0 is plain stone, not a door.
    const RUINED_DUNGEON_ENTRANCE_ARCHIVE: u16 = 331;
    /// Archive of dungeon exit textures.
    const DUNGEON_EXIT_ARCHIVE: u16 = 95;
    /// Scourg Barrow's exterior textures, which are excluded from the climate reduction below.
    const SCOURG_EXTERIOR_ARCHIVE: u16 = 156;

    /// The door kind a face drawn with `texture` represents, following Daggerfall Unity's rules.
    pub fn from_texture(texture: TextureKey) -> Option<Self> {
        // Archives above 100 are climate variants of a base archive (174 and 374 are
        // variants of 74), except for two special sets that keep their own number.
        let base_archive = match texture.archive {
            Self::RUINED_DUNGEON_ENTRANCE_ARCHIVE | Self::SCOURG_EXTERIOR_ARCHIVE => texture.archive,
            archive if archive > 100 => archive % 100,
            archive => archive,
        };
        match base_archive {
            Self::BUILDING_DOOR_ARCHIVE => Some(Self::Building),
            Self::DUNGEON_ENTRANCE_ARCHIVE => Some(Self::DungeonEntrance),
            Self::RUINED_DUNGEON_ENTRANCE_ARCHIVE if texture.record > 0 => Some(Self::DungeonEntrance),
            Self::DUNGEON_EXIT_ARCHIVE => Some(Self::DungeonExit),
            _ => None,
        }
    }
}

/// The layout revision of a model record, which changes how point offsets are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshVersion {
    /// Point offsets are indices that must be multiplied by three.
    Version25,
    /// Point offsets are byte offsets.
    Version26,
    /// Point offsets are byte offsets.
    Version27,
}

/// The parts of the model header needed to decode the geometry.
struct ModelHeader {
    version: MeshVersion,
    plane_count: usize,
    point_list_offset: usize,
    normal_list_offset: usize,
    plane_list_offset: usize,
}

/// A polygon point in native (integer) units, before conversion to world space.
#[derive(Debug, Clone, Copy)]
struct NativePoint {
    position: IVec3,
    u: i32,
    v: i32,
}

/// Decodes the model record `data` with identifier `model_id`.
pub fn parse_model(model_id: u32, data: &[u8]) -> Arena2Result<ModelMesh> {
    let header = read_header(data)?;
    let mut plane_reader = ByteReader::at(data, header.plane_list_offset);
    let mut normal_reader = ByteReader::at(data, header.normal_list_offset);

    let mut submeshes: Vec<SubMesh> = Vec::new();
    let mut submesh_by_texture: HashMap<TextureKey, usize> = HashMap::new();

    for _ in 0..header.plane_count {
        let point_count = usize::from(plane_reader.read_u8()?);
        plane_reader.skip(1);
        let texture = TextureKey::from_bitfield(plane_reader.read_u16()?);
        plane_reader.skip(4);

        let native_normal =
            IVec3::new(normal_reader.read_i32()?, normal_reader.read_i32()?, normal_reader.read_i32()?);

        let mut points = Vec::with_capacity(point_count);
        for point_index in 0..point_count {
            points.push(read_plane_point(data, &header, &mut plane_reader, model_id, point_index)?);
        }

        // Degenerate planes cannot form a triangle.
        if points.len() < 3 {
            continue;
        }

        let polygon = resolve_polygon(&mut points);
        let submesh_index = *submesh_by_texture.entry(texture).or_insert_with(|| {
            submeshes.push(SubMesh { texture, vertices: Vec::new(), indices: Vec::new() });
            submeshes.len() - 1
        });
        append_polygon(&mut submeshes[submesh_index], &polygon, native_normal);
    }

    let (bounds_min, bounds_max) = compute_bounds(&submeshes);
    Ok(ModelMesh { model_id, submeshes, bounds_min, bounds_max })
}

/// Reads and validates the 64 byte model header.
fn read_header(data: &[u8]) -> Arena2Result<ModelHeader> {
    if data.len() < MODEL_HEADER_SIZE {
        return Err(Arena2Error::format("model record is smaller than its header"));
    }

    let mut reader = ByteReader::new(data);
    let version = match reader.read_fixed_string(4)?.as_str() {
        "v2.5" => MeshVersion::Version25,
        "v2.6" => MeshVersion::Version26,
        "v2.7" => MeshVersion::Version27,
        other => return Err(Arena2Error::format(format!("unsupported model version '{other}'"))),
    };

    let _point_count = reader.read_i32()?;
    let plane_count = reader.read_i32()?;
    let _radius = reader.read_u32()?;
    reader.skip(8); // Null padding.
    let _plane_data_offset = reader.read_i32()?;
    let _object_data_offset = reader.read_i32()?;
    let _object_data_count = reader.read_i32()?;
    reader.skip(4 + 8); // Unknown value and null padding.
    let point_list_offset = reader.read_i32()?;
    let normal_list_offset = reader.read_i32()?;
    reader.skip(4); // Unknown value.
    let plane_list_offset = reader.read_i32()?;

    let to_offset = |value: i32, field: &str| {
        usize::try_from(value).map_err(|_| Arena2Error::format(format!("negative {field}")))
    };

    Ok(ModelHeader {
        version,
        plane_count: to_offset(plane_count, "plane count")?,
        point_list_offset: to_offset(point_list_offset, "point list offset")?,
        normal_list_offset: to_offset(normal_list_offset, "normal list offset")?,
        plane_list_offset: to_offset(plane_list_offset, "plane list offset")?,
    })
}

/// Reads one point reference from a plane and resolves its position from the point list.
fn read_plane_point(
    data: &[u8],
    header: &ModelHeader,
    plane_reader: &mut ByteReader<'_>,
    model_id: u32,
    point_index: usize,
) -> Arena2Result<NativePoint> {
    let point_offset = usize::try_from(plane_reader.read_i32()?)
        .map_err(|_| Arena2Error::format("negative point offset"))?;
    let mut u = i32::from(plane_reader.read_i16()?);
    let mut v = i32::from(plane_reader.read_i16()?);

    // Only the first three points carry texture data, and older models pack it.
    if point_index < 3 && model_id < PACKED_UV_MODEL_ID_LIMIT {
        u = unpack_texture_coordinate(u);
        v = unpack_texture_coordinate(v);
    }

    let point_byte_offset = match header.version {
        MeshVersion::Version25 => point_offset * 3,
        MeshVersion::Version26 | MeshVersion::Version27 => point_offset,
    };
    let mut point_reader = ByteReader::at(data, header.point_list_offset + point_byte_offset);
    let position = IVec3::new(point_reader.read_i32()?, point_reader.read_i32()?, point_reader.read_i32()?);

    Ok(NativePoint { position, u, v })
}

/// Removes the packing applied to some texture coordinates in older models.
///
/// Packed values lie outside the range (-14336, 14336) and are offset by a
/// multiple of 8192. The value -7168 is a known exception that is also packed.
fn unpack_texture_coordinate(value: i32) -> i32 {
    const PACKED_RANGE_LIMIT: i32 = 14336;
    const PACKED_EXCEPTION: i32 = -7168;
    const PACKING_STEP_BITS: i32 = 13; // 8192 == 1 << 13

    if value > -PACKED_RANGE_LIMIT && value < PACKED_RANGE_LIMIT && value != PACKED_EXCEPTION {
        return value;
    }

    let next_multiple = (((value - 1) >> PACKING_STEP_BITS) + 1) << PACKING_STEP_BITS;
    let previous_multiple = next_multiple - (1 << PACKING_STEP_BITS);
    let nearest_multiple =
        if value - previous_multiple < next_multiple - value { previous_multiple } else { next_multiple };
    value - nearest_multiple
}

/// Computes final texture coordinates for a plane and returns its corner points in fan order.
fn resolve_polygon(points: &mut [NativePoint]) -> Vec<NativePoint> {
    if points.len() == 3 {
        accumulate_texture_deltas(points);
        return points.to_vec();
    }

    if !compute_face_texture_coordinates(points) {
        // The mapping could not be solved; fall back to the explicit coordinates.
        accumulate_texture_deltas(points);
    }

    // Polygons with more than three points can contain collinear points
    // along their edges. Only the true corners are kept for triangulation.
    let corners = corner_point_indices(points);
    if corners.len() < 3 {
        return points.to_vec();
    }
    corners.into_iter().map(|index| points[index]).collect()
}

/// Turns the delta encoded texture coordinates of points 1 and 2 into absolute values.
fn accumulate_texture_deltas(points: &mut [NativePoint]) {
    points[1].u += points[0].u;
    points[1].v += points[0].v;
    points[2].u += points[1].u;
    points[2].v += points[1].v;
}

/// Solves the affine texture mapping `uv = A·x + B·y + D` from the first three points
/// (projected onto the plane) and applies it to every point.
///
/// Returns `false` if the first three points do not define a valid mapping.
fn compute_face_texture_coordinates(points: &mut [NativePoint]) -> bool {
    let position = |point: &NativePoint| point.position.as_dvec3();
    let origin = position(&points[0]);

    // Build an orthonormal 2D basis lying in the plane of the polygon.
    let first_edge = position(&points[1]) - origin;
    let mut second_edge = position(&points[2]) - origin;
    second_edge -= first_edge * (second_edge.dot(first_edge) / first_edge.dot(first_edge));
    let basis_x = first_edge.normalize();
    let basis_y = second_edge.normalize();

    // Projection truncates to integers to mirror the original algorithm.
    let project = |point: DVec3| -> (f64, f64) { (point.dot(basis_x).trunc(), point.dot(basis_y).trunc()) };
    let (x0, y0) = project(position(&points[0]));
    let (x1, y1) = project(position(&points[1]));
    let (x2, y2) = project(position(&points[2]));

    let u_values = [
        f64::from(points[0].u),
        f64::from(points[0].u + points[1].u),
        f64::from(points[0].u + points[1].u + points[2].u),
    ];
    let v_values = [
        f64::from(points[0].v),
        f64::from(points[0].v + points[1].v),
        f64::from(points[0].v + points[1].v + points[2].v),
    ];

    let determinant = x0 * y1 + y0 * x2 + x1 * y2 - y1 * x2 - y0 * x1 - x0 * y2;
    if determinant == 0.0 || !determinant.is_finite() {
        return false;
    }

    // Rows of the inverted coefficient matrix [x y 1].
    let inverse_row_0 = [(y1 - y2) / determinant, (x2 - x1) / determinant, (x1 * y2 - x2 * y1) / determinant];
    let inverse_row_1 = [(y2 - y0) / determinant, (x0 - x2) / determinant, (x2 * y0 - x0 * y2) / determinant];
    let inverse_row_2 = [(y0 - y1) / determinant, (x1 - x0) / determinant, (x0 * y1 - x1 * y0) / determinant];

    let solve = |values: [f64; 3], column: usize| {
        values[0] * inverse_row_0[column]
            + values[1] * inverse_row_1[column]
            + values[2] * inverse_row_2[column]
    };
    let (u_from_x, u_from_y, u_offset) = (solve(u_values, 0), solve(u_values, 1), solve(u_values, 2));
    let (v_from_x, v_from_y, v_offset) = (solve(v_values, 0), solve(v_values, 1), solve(v_values, 2));

    for (index, point) in points.iter_mut().enumerate() {
        match index {
            0..=2 => {
                point.u = u_values[index] as i32;
                point.v = v_values[index] as i32;
            }
            _ => {
                let (x, y) = project(point.position.as_dvec3());
                point.u = (x * u_from_x + y * u_from_y + u_offset) as i32;
                point.v = (x * v_from_x + y * v_from_y + v_offset) as i32;
            }
        }
    }
    true
}

/// Returns the indices of points where the polygon outline changes direction.
fn corner_point_indices(points: &[NativePoint]) -> Vec<usize> {
    let count = points.len();
    let position = |index: usize| points[index].position.as_dvec3();

    (0..count)
        .filter_map(|index| {
            let (start, middle, end) = if index + 2 < count {
                (index, index + 1, index + 2)
            } else if index + 1 < count {
                (index, index + 1, 0)
            } else {
                (index, 0, 1)
            };
            let to_middle = position(middle) - position(start);
            let to_end = position(end) - position(start);
            let angle = to_middle.angle_between(to_end);
            (angle > CORNER_ANGLE_THRESHOLD).then_some(middle)
        })
        .collect()
}

/// Converts a polygon to world space and appends it to `submesh` as a triangle fan.
fn append_polygon(submesh: &mut SubMesh, polygon: &[NativePoint], native_normal: IVec3) {
    let normal = Vec3::new(native_normal.x as f32, -native_normal.y as f32, native_normal.z as f32)
        .try_normalize()
        .unwrap_or(Vec3::Y);

    let first_vertex = submesh.vertices.len() as u32;
    submesh.vertices.extend(polygon.iter().map(|point| MeshVertex {
        position: Vec3::new(point.position.x as f32, -point.position.y as f32, point.position.z as f32)
            / POINT_DIVISOR
            * WORLD_SCALE,
        normal,
        uv_texels: Vec2::new(point.u as f32, point.v as f32) / TEXTURE_COORDINATE_DIVISOR,
    }));

    // Triangle fan around the first point, using the same winding as the game data.
    for triangle in 0..(polygon.len() as u32 - 2) {
        submesh.indices.extend_from_slice(&[
            first_vertex,
            first_vertex + triangle + 2,
            first_vertex + triangle + 1,
        ]);
    }
}

/// Calculates the axis aligned bounds of every vertex in every sub-mesh.
fn compute_bounds(submeshes: &[SubMesh]) -> (Vec3, Vec3) {
    let mut vertices = submeshes.iter().flat_map(|submesh| submesh.vertices.iter());
    let Some(first) = vertices.next() else {
        return (Vec3::ZERO, Vec3::ZERO);
    };
    vertices.fold((first.position, first.position), |(minimum, maximum), vertex| {
        (minimum.min(vertex.position), maximum.max(vertex.position))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_texture_coordinates() {
        assert_eq!(unpack_texture_coordinate(100), 100);
        assert_eq!(unpack_texture_coordinate(-100), -100);
        assert_eq!(unpack_texture_coordinate(16384 + 64), 64);
        assert_eq!(unpack_texture_coordinate(-16384 - 64), -64);
        assert_eq!(unpack_texture_coordinate(-7168), 1024);
    }

    #[test]
    fn finds_corners_of_square_with_midpoint() {
        let point = |x, z| NativePoint { position: IVec3::new(x, 0, z), u: 0, v: 0 };
        // A square with an extra collinear point on its first edge.
        let points = [point(0, 0), point(5, 0), point(10, 0), point(10, 10), point(0, 10)];
        let corners = corner_point_indices(&points);
        assert_eq!(corners.len(), 4);
        assert!(!corners.contains(&1));
    }
}

//! Readers for `TEXTURE.###` image archives and the `ART_PAL.COL` palette.
//!
//! Texture archives hold one or more paletted (8 bits per pixel) images.
//! Images are stored either uncompressed, as a run-length encoded animation,
//! or with per-row run-length encoding. `TEXTURE.000` and `TEXTURE.001` are
//! special: they contain no pixel data and instead represent solid colours
//! taken straight from the palette.

use super::error::{Arena2Error, Arena2Result};
use super::reader::ByteReader;

/// Number of colours in a Daggerfall palette.
const PALETTE_COLOR_COUNT: usize = 256;
/// Size of a raw `.PAL` palette file.
const RAW_PALETTE_SIZE: usize = PALETTE_COLOR_COUNT * 3;
/// Size of a `.COL` palette file, which has an 8 byte header before the colours.
const HEADERED_PALETTE_SIZE: usize = RAW_PALETTE_SIZE + 8;
/// Offset of the archive's name, which follows the 16-bit record count.
const ARCHIVE_NAME_OFFSET: usize = 2;
/// Length of the archive's name field.
const ARCHIVE_NAME_LENGTH: usize = 24;
/// Size of the archive header, which is also the offset of the first record header.
pub const ARCHIVE_HEADER_SIZE: usize = ARCHIVE_NAME_OFFSET + ARCHIVE_NAME_LENGTH;
/// Offset of the first record header within a texture archive.
const RECORD_HEADER_LIST_OFFSET: usize = ARCHIVE_HEADER_SIZE;
/// Size of one record header.
const RECORD_HEADER_SIZE: usize = 20;
/// Width and height of the images generated for solid colour archives.
const SOLID_TEXTURE_SIZE: u32 = 32;
/// Uncompressed single-frame images are stored with rows padded to this stride.
const UNCOMPRESSED_ROW_STRIDE: usize = 256;
/// Record compression flag: per-row run-length encoding (two variants).
const COMPRESSION_RECORD_RLE: u16 = 0x1108;
const COMPRESSION_IMAGE_RLE: u16 = 0x0108;
/// Row encoding flag indicating the row is run-length encoded.
const ROW_IS_RLE_ENCODED: u16 = 0x8000;
/// Billboard scale values are expressed relative to this divisor.
const BILLBOARD_SCALE_DIVISOR: i32 = 256;

/// A 256 colour RGB palette.
#[derive(Clone)]
pub struct Palette {
    colors: [[u8; 3]; PALETTE_COLOR_COUNT],
}

impl Palette {
    /// Parses a `.PAL` (768 byte) or `.COL` (776 byte) palette file.
    pub fn from_bytes(bytes: &[u8]) -> Arena2Result<Self> {
        let color_data = match bytes.len() {
            RAW_PALETTE_SIZE => bytes,
            HEADERED_PALETTE_SIZE => &bytes[8..],
            other => {
                return Err(Arena2Error::format(format!(
                    "palette files must be {RAW_PALETTE_SIZE} or {HEADERED_PALETTE_SIZE} bytes, found {other}"
                )))
            }
        };

        let mut colors = [[0u8; 3]; PALETTE_COLOR_COUNT];
        let (rgb_triplets, _) = color_data.as_chunks::<3>();
        for (color, rgb) in colors.iter_mut().zip(rgb_triplets) {
            *color = *rgb;
        }
        Ok(Self { colors })
    }

    /// Returns the RGB colour at `index`.
    pub fn color(&self, index: u8) -> [u8; 3] {
        self.colors[usize::from(index)]
    }
}

/// A decoded image with 8-bit RGBA pixels in row-major order, top row first.
#[derive(Clone)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA data.
    pub pixels: Vec<u8>,
}

/// Metadata describing one image record within a texture archive.
#[derive(Debug, Clone)]
pub struct TextureRecordInfo {
    /// Absolute offset of the record header within the archive.
    position: usize,
    /// Image width in pixels.
    pub width: u16,
    /// Image height in pixels.
    pub height: u16,
    /// Compression format flag.
    compression: u16,
    /// Offset of the pixel data relative to [`Self::position`].
    data_offset: usize,
    /// Number of animation frames.
    frame_count: u16,
    /// Horizontal billboard scale adjustment.
    pub scale_x: i16,
    /// Vertical billboard scale adjustment.
    pub scale_y: i16,
}

impl TextureRecordInfo {
    /// Size of the image when shown as a billboard, in native Daggerfall units.
    ///
    /// Billboards are scaled by `size * scale / 256` on top of their pixel size.
    pub fn billboard_size(&self) -> (i32, i32) {
        let scaled = |size: u16, scale: i16| {
            let size = i32::from(size);
            size + size * i32::from(scale) / BILLBOARD_SCALE_DIVISOR
        };
        (scaled(self.width, self.scale_x), scaled(self.height, self.scale_y))
    }
}

/// Distinguishes the two palette-only archives from ordinary image archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SolidColorSet {
    /// `TEXTURE.000`: record N is palette colour N.
    Lower,
    /// `TEXTURE.001`: record N is palette colour 128 + N.
    Upper,
}

/// A texture archive loaded into memory.
pub struct TextureArchive {
    data: Vec<u8>,
    records: Vec<TextureRecordInfo>,
    solid_color_set: Option<SolidColorSet>,
}

impl TextureArchive {
    /// Parses the archive `TEXTURE.{archive_index}` from its raw bytes.
    pub fn parse(archive_index: u16, data: Vec<u8>) -> Arena2Result<Self> {
        let solid_color_set = match archive_index {
            0 => Some(SolidColorSet::Lower),
            1 => Some(SolidColorSet::Upper),
            _ => None,
        };

        let records = if solid_color_set.is_some() { Vec::new() } else { read_record_infos(&data)? };

        Ok(Self { data, records, solid_color_set })
    }

    /// Number of image records in the archive. Solid colour archives report 128.
    pub fn record_count(&self) -> usize {
        if self.solid_color_set.is_some() {
            128
        } else {
            self.records.len()
        }
    }

    /// Returns metadata for `record`, or `None` if it does not exist.
    ///
    /// Solid colour archives report a synthetic 32x32 record for any index below 128.
    pub fn record_info(&self, record: u8) -> Option<TextureRecordInfo> {
        if self.solid_color_set.is_some() {
            return (record < 128).then_some(TextureRecordInfo {
                position: 0,
                width: SOLID_TEXTURE_SIZE as u16,
                height: SOLID_TEXTURE_SIZE as u16,
                compression: 0,
                data_offset: 0,
                frame_count: 1,
                scale_x: 0,
                scale_y: 0,
            });
        }
        self.records.get(usize::from(record)).cloned()
    }

    /// Decodes the first frame of `record` into RGBA pixels.
    ///
    /// Palette index 0 is treated as transparent, except for solid colours.
    pub fn decode(&self, record: u8, palette: &Palette) -> Arena2Result<RgbaImage> {
        if let Some(color_set) = self.solid_color_set {
            let color_index = match color_set {
                SolidColorSet::Lower => record,
                SolidColorSet::Upper => record.saturating_add(128),
            };
            return Ok(solid_color_image(palette.color(color_index)));
        }

        let info = self
            .records
            .get(usize::from(record))
            .ok_or_else(|| Arena2Error::not_found(format!("texture record {record}")))?;
        let indices = match info.compression {
            COMPRESSION_RECORD_RLE | COMPRESSION_IMAGE_RLE => decode_row_rle(&self.data, info)?,
            _ => decode_uncompressed(&self.data, info)?,
        };
        Ok(indexed_to_rgba(u32::from(info.width), u32::from(info.height), &indices, palette))
    }
}

/// Reads the descriptive name stored in a texture archive's header, for example
/// "Door Set" or "Mountain Dungeons A".
///
/// `header` must hold at least the first [`ARCHIVE_HEADER_SIZE`] bytes of the
/// archive. Returns `None` if the name is blank.
pub fn read_archive_name(header: &[u8]) -> Arena2Result<Option<String>> {
    let name = ByteReader::at(header, ARCHIVE_NAME_OFFSET).read_fixed_string(ARCHIVE_NAME_LENGTH)?;
    let name = name.trim();
    Ok((!name.is_empty()).then(|| name.to_string()))
}

/// Reads every record header and record info block in the archive.
fn read_record_infos(data: &[u8]) -> Arena2Result<Vec<TextureRecordInfo>> {
    let mut header = ByteReader::new(data);
    let record_count = usize::try_from(header.read_i16()?)
        .map_err(|_| Arena2Error::format("negative texture record count"))?;

    (0..record_count)
        .map(|record_index| {
            let mut record_header =
                ByteReader::at(data, RECORD_HEADER_LIST_OFFSET + record_index * RECORD_HEADER_SIZE);
            let _type_1 = record_header.read_i16()?;
            let position = usize::try_from(record_header.read_i32()?)
                .map_err(|_| Arena2Error::format("negative texture record offset"))?;

            let mut record = ByteReader::at(data, position);
            let _offset_x = record.read_i16()?;
            let _offset_y = record.read_i16()?;
            let width = u16::try_from(record.read_i16()?).unwrap_or(0);
            let height = u16::try_from(record.read_i16()?).unwrap_or(0);
            let compression = record.read_u16()?;
            let _record_size = record.read_u32()?;
            let data_offset = record.read_u32()? as usize;
            let _is_normal = record.read_i16()?;
            let frame_count = record.read_u16()?;
            let _unknown = record.read_i16()?;
            let scale_x = record.read_i16()?;
            let scale_y = record.read_i16()?;

            Ok(TextureRecordInfo {
                position,
                width,
                height,
                compression,
                data_offset,
                frame_count,
                scale_x,
                scale_y,
            })
        })
        .collect()
}

/// Decodes an uncompressed image, which is either a padded bitmap or a
/// transparent-run encoded animation frame.
fn decode_uncompressed(data: &[u8], info: &TextureRecordInfo) -> Arena2Result<Vec<u8>> {
    let width = usize::from(info.width);
    let height = usize::from(info.height);
    let image_start = info.position + info.data_offset;
    let mut indices = vec![0u8; width * height];

    match info.frame_count {
        0 => return Err(Arena2Error::format("texture record has no frames")),
        1 => {
            // Each row occupies 256 bytes regardless of the image width.
            let mut reader = ByteReader::at(data, image_start);
            for row in indices.chunks_exact_mut(width.max(1)).take(height) {
                row.copy_from_slice(reader.read_bytes(width)?);
                reader.skip(UNCOMPRESSED_ROW_STRIDE.saturating_sub(width));
            }
        }
        _ => {
            // Animated records start with a table of frame offsets; decode frame 0.
            let mut reader = ByteReader::at(data, image_start);
            let first_frame_offset = reader.read_i32()? as usize;
            reader.seek(image_start + first_frame_offset);
            let frame_width = usize::from(reader.read_u16()?);
            let frame_height = usize::from(reader.read_u16()?);

            let mut destination = 0;
            for _ in 0..frame_height.min(height) {
                let mut column = 0;
                while column < frame_width {
                    // Each run is a count of transparent pixels then a count of opaque pixels.
                    let transparent_run = usize::from(reader.read_u8()?);
                    destination += transparent_run;
                    column += transparent_run;
                    let opaque_run = usize::from(reader.read_u8()?);
                    for &pixel in reader.read_bytes(opaque_run)? {
                        if let Some(target) = indices.get_mut(destination) {
                            *target = pixel;
                        }
                        destination += 1;
                    }
                    column += opaque_run;
                }
            }
        }
    }
    Ok(indices)
}

/// Decodes an image whose rows may each be run-length encoded.
fn decode_row_rle(data: &[u8], info: &TextureRecordInfo) -> Arena2Result<Vec<u8>> {
    let width = usize::from(info.width);
    let height = usize::from(info.height);
    let mut indices = Vec::with_capacity(width * height);

    let mut row_headers = ByteReader::at(data, info.position + info.data_offset);
    for _ in 0..height {
        let row_offset = usize::from(row_headers.read_u16()?);
        let row_encoding = row_headers.read_u16()?;
        let mut row_reader = ByteReader::at(data, info.position + row_offset);
        let row_start = indices.len();

        if row_encoding == ROW_IS_RLE_ENCODED {
            let row_width = usize::from(row_reader.read_u16()?);
            while indices.len() - row_start < row_width {
                let probe = row_reader.read_i16()?;
                if probe < 0 {
                    // A negative count repeats the next byte.
                    let pixel = row_reader.read_u8()?;
                    let repeat = usize::from(probe.unsigned_abs());
                    indices.extend(std::iter::repeat_n(pixel, repeat));
                } else if probe > 0 {
                    // A positive count copies that many literal bytes.
                    indices.extend_from_slice(row_reader.read_bytes(probe as usize)?);
                } else {
                    break;
                }
            }
        } else {
            indices.extend_from_slice(row_reader.read_bytes(width)?);
        }

        // Keep every row exactly `width` pixels long even if the data is irregular.
        indices.resize(row_start + width, 0);
    }
    Ok(indices)
}

/// Converts palette indices to RGBA, treating index 0 as fully transparent.
fn indexed_to_rgba(width: u32, height: u32, indices: &[u8], palette: &Palette) -> RgbaImage {
    let pixels = indices
        .iter()
        .flat_map(|&index| {
            let [red, green, blue] = palette.color(index);
            let alpha = if index == 0 { 0 } else { 255 };
            [red, green, blue, alpha]
        })
        .collect();
    RgbaImage { width, height, pixels }
}

/// Builds an opaque square image filled with one colour.
fn solid_color_image([red, green, blue]: [u8; 3]) -> RgbaImage {
    let pixel_count = (SOLID_TEXTURE_SIZE * SOLID_TEXTURE_SIZE) as usize;
    RgbaImage {
        width: SOLID_TEXTURE_SIZE,
        height: SOLID_TEXTURE_SIZE,
        pixels: [red, green, blue, 255].repeat(pixel_count),
    }
}

//! Reader for Daggerfall's BSA archive container.
//!
//! A BSA file is a flat list of records followed by a directory stored at the
//! very end of the file. The directory either names each record with a short
//! string (`BLOCKS.BSA`) or identifies it with a number
//! (`ARCH3D.BSA`). Records are stored back to back starting right after the
//! four byte header, in directory order.

use std::collections::HashMap;
use std::path::Path;

use super::error::{read_file, Arena2Error, Arena2Result};
use super::reader::ByteReader;

/// Directory type marker for archives whose records are identified by name.
const NAME_RECORD_DIRECTORY: u16 = 0x0100;
/// Directory type marker for archives whose records are identified by number.
const NUMBER_RECORD_DIRECTORY: u16 = 0x0200;
/// Size of the archive header (record count and directory type).
const HEADER_SIZE: usize = 4;
/// Size of one entry in a name-record directory.
const NAME_ENTRY_SIZE: usize = 18;
/// Length of the record name field inside a name-record directory entry.
const NAME_FIELD_LENGTH: usize = 14;
/// Size of one entry in a number-record directory.
const NUMBER_ENTRY_SIZE: usize = 8;

/// Describes a single record stored inside a BSA archive.
#[derive(Debug, Clone)]
pub struct BsaEntry {
    /// Record name. For number-record archives this is the number as text.
    pub name: String,
    /// Numeric identifier, only present for number-record archives.
    pub id: Option<u32>,
    /// Absolute offset of the record data within the archive.
    offset: usize,
    /// Length of the record data in bytes.
    size: usize,
}

/// A BSA archive loaded fully into memory.
pub struct BsaArchive {
    data: Vec<u8>,
    entries: Vec<BsaEntry>,
    index_by_name: HashMap<String, usize>,
    index_by_id: HashMap<u32, usize>,
}

impl BsaArchive {
    /// Loads and indexes the BSA archive at `path`.
    pub fn open(path: &Path) -> Arena2Result<Self> {
        let data = read_file(path)?;
        Self::from_bytes(data).map_err(|error| Arena2Error::format(format!("{}: {error}", path.display())))
    }

    /// Indexes a BSA archive that is already in memory.
    pub fn from_bytes(data: Vec<u8>) -> Arena2Result<Self> {
        let mut header = ByteReader::new(&data);
        let record_count = usize::try_from(header.read_i16()?)
            .map_err(|_| Arena2Error::format("negative BSA record count"))?;
        let directory_type = header.read_u16()?;

        let entry_size = match directory_type {
            NAME_RECORD_DIRECTORY => NAME_ENTRY_SIZE,
            NUMBER_RECORD_DIRECTORY => NUMBER_ENTRY_SIZE,
            other => return Err(Arena2Error::format(format!("unknown BSA directory type {other:#06x}"))),
        };

        let directory_size = record_count * entry_size;
        let directory_start = data
            .len()
            .checked_sub(directory_size)
            .ok_or_else(|| Arena2Error::format("BSA directory is larger than the file"))?;

        let mut directory = ByteReader::at(&data, directory_start);
        let mut entries = Vec::with_capacity(record_count);
        let mut next_record_offset = HEADER_SIZE;

        for _ in 0..record_count {
            let (name, id, size) = if directory_type == NAME_RECORD_DIRECTORY {
                let name = directory.read_fixed_string(NAME_FIELD_LENGTH)?;
                let size = directory.read_i32()?;
                (name, None, size)
            } else {
                let id = directory.read_u32()?;
                let size = directory.read_i32()?;
                (id.to_string(), Some(id), size)
            };

            let size = usize::try_from(size)
                .map_err(|_| Arena2Error::format(format!("record '{name}' has a negative size")))?;
            if next_record_offset + size > directory_start {
                return Err(Arena2Error::format(format!(
                    "record '{name}' extends past the end of the record data"
                )));
            }

            entries.push(BsaEntry { name, id, offset: next_record_offset, size });
            next_record_offset += size;
        }

        let index_by_name = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.name.to_ascii_uppercase(), index))
            .collect();
        let index_by_id =
            entries.iter().enumerate().filter_map(|(index, entry)| entry.id.map(|id| (id, index))).collect();

        Ok(Self { data, entries, index_by_name, index_by_id })
    }

    /// Returns every record entry in directory order.
    pub fn entries(&self) -> &[BsaEntry] {
        &self.entries
    }

    /// Returns the raw bytes of the record called `name` (case-insensitive).
    pub fn record_by_name(&self, name: &str) -> Option<&[u8]> {
        let index = *self.index_by_name.get(&name.to_ascii_uppercase())?;
        Some(self.record_data(&self.entries[index]))
    }

    /// Returns the raw bytes of the record with numeric identifier `id`.
    pub fn record_by_id(&self, id: u32) -> Option<&[u8]> {
        let index = *self.index_by_id.get(&id)?;
        Some(self.record_data(&self.entries[index]))
    }

    /// Returns the bytes belonging to `entry`.
    fn record_data(&self, entry: &BsaEntry) -> &[u8] {
        &self.data[entry.offset..entry.offset + entry.size]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a tiny name-record archive containing two records.
    fn build_name_archive() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&2i16.to_le_bytes());
        data.extend_from_slice(&NAME_RECORD_DIRECTORY.to_le_bytes());
        data.extend_from_slice(b"hello");
        data.extend_from_slice(b"abc");
        for (name, size) in [("FIRST.RDB", 5i32), ("SECOND.RDB", 3i32)] {
            let mut field = [0u8; NAME_FIELD_LENGTH];
            field[..name.len()].copy_from_slice(name.as_bytes());
            data.extend_from_slice(&field);
            data.extend_from_slice(&size.to_le_bytes());
        }
        data
    }

    #[test]
    fn indexes_name_records() {
        let archive = BsaArchive::from_bytes(build_name_archive()).unwrap();
        assert_eq!(archive.entries().len(), 2);
        assert_eq!(archive.record_by_name("first.rdb").unwrap(), b"hello");
        assert_eq!(archive.record_by_name("SECOND.RDB").unwrap(), b"abc");
        assert!(archive.record_by_name("MISSING.RDB").is_none());
    }
}

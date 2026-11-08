//! A bounds-checked little-endian cursor over a byte slice.
//!
//! Every Daggerfall file format is little-endian and offset based, so the
//! parsers use this cursor to jump around a file and read primitive values
//! without risking a panic on truncated or corrupt data.

use super::error::{Arena2Error, Arena2Result};

/// A read-only cursor over a borrowed byte slice.
#[derive(Clone)]
pub struct ByteReader<'data> {
    data: &'data [u8],
    position: usize,
}

impl<'data> ByteReader<'data> {
    /// Creates a reader positioned at the start of `data`.
    pub fn new(data: &'data [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// Creates a reader positioned at `position` bytes into `data`.
    pub fn at(data: &'data [u8], position: usize) -> Self {
        Self { data, position }
    }

    /// Moves the cursor to an absolute byte offset.
    pub fn seek(&mut self, position: usize) {
        self.position = position;
    }

    /// Advances the cursor by `count` bytes without reading them.
    pub fn skip(&mut self, count: usize) {
        self.position = self.position.saturating_add(count);
    }

    /// Reads `count` raw bytes and advances the cursor past them.
    pub fn read_bytes(&mut self, count: usize) -> Arena2Result<&'data [u8]> {
        let end =
            self.position.checked_add(count).filter(|&end| end <= self.data.len()).ok_or_else(|| {
                Arena2Error::format(format!(
                    "attempted to read {count} bytes at offset {} of a {}-byte buffer",
                    self.position,
                    self.data.len()
                ))
            })?;
        let bytes = &self.data[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    /// Reads a fixed number of bytes into an array.
    fn read_array<const LENGTH: usize>(&mut self) -> Arena2Result<[u8; LENGTH]> {
        let bytes = self.read_bytes(LENGTH)?;
        // The slice length is guaranteed by `read_bytes`, so this cannot fail.
        Ok(bytes.try_into().expect("slice length matches array length"))
    }

    /// Reads an unsigned 8-bit integer.
    pub fn read_u8(&mut self) -> Arena2Result<u8> {
        Ok(self.read_array::<1>()?[0])
    }

    /// Reads a little-endian unsigned 16-bit integer.
    pub fn read_u16(&mut self) -> Arena2Result<u16> {
        Ok(u16::from_le_bytes(self.read_array()?))
    }

    /// Reads a little-endian signed 16-bit integer.
    pub fn read_i16(&mut self) -> Arena2Result<i16> {
        Ok(i16::from_le_bytes(self.read_array()?))
    }

    /// Reads a little-endian unsigned 32-bit integer.
    pub fn read_u32(&mut self) -> Arena2Result<u32> {
        Ok(u32::from_le_bytes(self.read_array()?))
    }

    /// Reads a little-endian signed 32-bit integer.
    pub fn read_i32(&mut self) -> Arena2Result<i32> {
        Ok(i32::from_le_bytes(self.read_array()?))
    }

    /// Reads a fixed-length text field that may be terminated early by a null byte.
    ///
    /// The cursor always advances by `field_length` bytes. Non-ASCII bytes are
    /// decoded as Latin-1, which matches the character set used by the game.
    pub fn read_fixed_string(&mut self, field_length: usize) -> Arena2Result<String> {
        let field = self.read_bytes(field_length)?;
        let text_length = field.iter().position(|&byte| byte == 0).unwrap_or(field.len());
        Ok(field[..text_length].iter().map(|&byte| byte as char).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_values() {
        let data = [0x01, 0x02, 0xFF, 0xFF, 0x10, 0x00, 0x00, 0x00];
        let mut reader = ByteReader::new(&data);
        assert_eq!(reader.read_u16().unwrap(), 0x0201);
        assert_eq!(reader.read_i16().unwrap(), -1);
        assert_eq!(reader.read_u32().unwrap(), 16);
        assert!(reader.read_u8().is_err());
    }

    #[test]
    fn reads_null_terminated_fixed_strings() {
        let data = *b"ABC\0junkZ";
        let mut reader = ByteReader::new(&data);
        assert_eq!(reader.read_fixed_string(8).unwrap(), "ABC");
        assert_eq!(reader.read_u8().unwrap(), b'Z');
    }
}

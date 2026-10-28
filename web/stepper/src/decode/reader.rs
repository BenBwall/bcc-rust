//! A cursor over the binary format's primitives (WebAssembly 2.0 §5.2):
//! bytes, LEB128 integers, IEEE floats and UTF-8 names. Every read is
//! bounds checked and reports the offset it failed at.

use super::DecodeError;

#[derive(Clone)]
pub struct Reader<'a> {
    bytes:    &'a [u8],
    position: usize,
    /// The module offset of `bytes[0]`, so errors and recorded offsets are
    /// module offsets.
    base:     usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            base: 0,
        }
    }

    /// The module offset of the next byte.
    pub fn offset(&self) -> u32 {
        u32::try_from(self.base + self.position).unwrap_or(u32::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }

    pub fn error(&self, message: impl Into<String>) -> DecodeError {
        DecodeError {
            offset:  self.offset(),
            message: message.into(),
        }
    }

    pub fn byte(&mut self) -> Result<u8, DecodeError> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| self.error("unexpected end of the module"))?;
        self.position += 1;
        Ok(byte)
    }

    pub fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    pub fn take(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .position
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| self.error("a length runs past the end of its section"))?;
        let bytes = &self.bytes[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    /// Everything left.
    pub fn rest(&mut self) -> &'a [u8] {
        let rest = &self.bytes[self.position.min(self.bytes.len())..];
        self.position = self.bytes.len();
        rest
    }

    /// A sub-reader over the next `length` bytes.
    pub fn split(&mut self, length: usize) -> Result<Self, DecodeError> {
        let base = self.base + self.position;
        let bytes = self.take(length)?;
        Ok(Self {
            bytes,
            position: 0,
            base,
        })
    }

    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        let value = self.unsigned(32)?;
        u32::try_from(value).map_err(|_| self.error("an integer is too large"))
    }

    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        let value = self.signed(32)?;
        i32::try_from(value).map_err(|_| self.error("an integer is too large"))
    }

    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        self.signed(64)
    }

    /// A `uN` LEB128 (§5.2.2): at most `ceil(N / 7)` bytes, the unused bits
    /// of the last one zero.
    fn unsigned(&mut self, bits: u32) -> Result<u64, DecodeError> {
        let mut result = 0_u64;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            let payload = u64::from(byte & 0x7F);
            if shift >= bits || (bits - shift < 7 && payload >> (bits - shift) != 0) {
                return Err(self.error("an integer is too large"));
            }
            result |= payload << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
        }
    }

    /// An `sN` LEB128 (§5.2.2), sign extended from its last byte.
    fn signed(&mut self, bits: u32) -> Result<i64, DecodeError> {
        let mut result = 0_i64;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            if shift >= bits {
                return Err(self.error("an integer is too large"));
            }
            let payload = i64::from(byte & 0x7F);
            if bits - shift < 7 {
                // The unused bits must all repeat the sign bit.
                let used = bits - shift;
                let rest = (payload >> (used - 1)) & 0x7F;
                let all = (1 << (8 - used)) - 1;
                if byte & 0x80 != 0 || (rest != 0 && rest != all) {
                    return Err(self.error("an integer is too large"));
                }
            }
            result |= payload << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    result |= -1 << shift;
                }
                return Ok(result);
            }
        }
    }

    pub fn f32_bits(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn f64_bits(&mut self) -> Result<u64, DecodeError> {
        let bytes = self.take(8)?;
        let mut array = [0; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    /// A name (§5.2.4): a length and UTF-8 bytes.
    pub fn name(&mut self) -> Result<String, DecodeError> {
        let length = self.u32()? as usize;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| self.error("a name is not valid UTF-8"))
    }

    /// A vector's length (§5.1.3), bounded by the bytes left so a corrupt
    /// count cannot reserve gigabytes.
    pub fn count(&mut self) -> Result<u32, DecodeError> {
        let count = self.u32()?;
        if count as usize > self.bytes.len() - self.position {
            return Err(self.error("a vector is longer than its section"));
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::Reader;

    #[test]
    fn decodes_leb128_including_padded_forms() {
        assert_eq!(Reader::new(&[0xE5, 0x8E, 0x26]).u32().unwrap(), 624_485);
        // Linkers leave relocatable 5-byte forms in place.
        assert_eq!(
            Reader::new(&[0x81, 0x80, 0x80, 0x80, 0x00]).u32().unwrap(),
            1
        );
        assert!(Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x10]).u32().is_err());
        assert!(
            Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00])
                .u32()
                .is_err()
        );
        assert_eq!(Reader::new(&[0x7F]).i32().unwrap(), -1);
        assert_eq!(Reader::new(&[0xC0, 0xBB, 0x78]).i32().unwrap(), -123_456);
        assert_eq!(
            Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x07]).i32().unwrap(),
            i32::MAX
        );
        assert_eq!(
            Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x78]).i32().unwrap(),
            i32::MIN
        );
        assert!(Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x70]).i32().is_err());
        assert_eq!(
            Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7F])
                .i64()
                .unwrap(),
            i64::MIN
        );
        assert!(Reader::new(&[0x80]).u32().is_err(), "truncated");
    }
}

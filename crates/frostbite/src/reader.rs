//! A bounds-checked cursor over a byte slice.

use crate::{Result, bail};

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// A reader positioned at `pos`, which must be inside the data (or at its end).
    pub fn at(data: &'a [u8], pos: usize) -> Result<Self> {
        if pos > data.len() {
            bail!("offset {pos:#x} is past the end ({:#x})", data.len());
        }
        Ok(Self { data, pos })
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&end| end <= self.data.len());
        let Some(end) = end else {
            bail!(
                "{n} bytes at {:#x} run past the end ({:#x})",
                self.pos,
                self.data.len()
            );
        };
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u32_be(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub fn u64_be(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    /// An unsigned LEB128 number (7 bits per byte, low first), at most 64 bits.
    pub fn leb128(&mut self) -> Result<u64> {
        let start = self.pos;
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(value);
            }
        }
        bail!("LEB128 number at {start:#x} is longer than 64 bits");
    }

    /// A NUL-terminated string at the cursor.
    pub fn cstr(&mut self) -> Result<&'a str> {
        let s = cstr_at(self.data, self.pos)?;
        self.pos += s.len() + 1;
        Ok(s)
    }
}

/// The NUL-terminated UTF-8 string starting at `pos`.
pub fn cstr_at(data: &[u8], pos: usize) -> Result<&str> {
    let Some(rest) = data.get(pos..) else {
        bail!("string offset {pos:#x} is past the end ({:#x})", data.len());
    };
    let Some(len) = rest.iter().position(|&b| b == 0) else {
        bail!("string at {pos:#x} has no terminator");
    };
    match std::str::from_utf8(&rest[..len]) {
        Ok(s) => Ok(s),
        Err(_) => bail!("string at {pos:#x} is not UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_big_endian_numbers_and_stops_at_the_end() {
        let mut r = Reader::new(&[0, 0, 1, 2, 9]);
        assert_eq!(r.u32_be(), Ok(0x102));
        assert_eq!(r.u8(), Ok(9));
        assert!(r.u8().is_err());
        assert!(Reader::at(&[0; 4], 5).is_err());
    }

    #[test]
    fn leb128_reads_multi_byte_numbers() {
        assert_eq!(Reader::new(&[0xcd, 0x10]).leb128(), Ok(0x84d));
        assert_eq!(Reader::new(&[0x7f]).leb128(), Ok(0x7f));
        assert!(Reader::new(&[0x80; 11]).leb128().is_err());
        assert!(Reader::new(&[0x80]).leb128().is_err());
    }

    #[test]
    fn strings_need_a_terminator() {
        let data = b"ab\0cd";
        let mut r = Reader::new(data);
        assert_eq!(r.cstr(), Ok("ab"));
        assert_eq!(r.pos(), 3);
        assert!(r.cstr().is_err());
        assert!(cstr_at(data, 9).is_err());
    }
}

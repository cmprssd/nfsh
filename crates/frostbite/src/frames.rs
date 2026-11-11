//! How nfsh's store keeps an entry: zstd frames of the entry's decompressed bytes, back
//! to back (one, unless a texture's chunk is split by tier; see [`crate::plan`]), at
//! level [`LEVEL`], with the decompressed size and a checksum in each. ebx frames
//! use the store's dictionary, trained on a sample of the install's ebx at import: ebx
//! are small and share their structure, so it makes them about 15% smaller, and does
//! nothing for the rest. A frame names its dictionary by id, so a reader needs no list
//! of which entries use it.
//!
//! A texture's frame may hold its blocks with their fields gathered ([`Layout`]): all the
//! blocks' end points, then all their indices, which compress better than whole blocks.
//! Such a frame is preceded by a 9-byte zstd skippable frame naming the layout, so a
//! reader puts the blocks back without any table: [`FrameDecoder::decode`] does.

use std::io;

use zstd::bulk::{Compressor, Decompressor};
use zstd::dict::EncoderDictionary;
use zstd::zstd_safe;

use crate::{Result, bail};

/// The level every frame is written at: smaller ones lose 4-12% on Heat's data and
/// larger ones gain nothing (measured in `docs/DECISIONS.md`).
pub const LEVEL: i32 = 19;

/// The dictionary's size.
pub const DICTIONARY_SIZE: usize = 256 << 10;

/// The most a frame may decompress to, so a damaged header can't ask for a huge
/// allocation. Entries are far smaller.
pub const MAX_ENTRY: u64 = 1 << 30;

fn zstd_error(e: io::Error) -> crate::Error {
    crate::Error(format!("zstd: {e}"))
}

/// A dictionary trained on `samples`, or none (empty) when there are too few to train
/// on; a store without one is valid, just larger.
pub fn train(samples: &[Vec<u8>]) -> Vec<u8> {
    zstd::dict::from_samples(samples, DICTIONARY_SIZE).unwrap_or_default()
}

/// The dictionary ready for compressing, shared by the import's threads.
pub fn prepare(dictionary: &[u8]) -> Option<EncoderDictionary<'static>> {
    (!dictionary.is_empty()).then(|| EncoderDictionary::copy(dictionary, LEVEL))
}

/// The skippable frame's magic number that says how the next frame's bytes are laid out
/// (zstd reserves 0x184D2A50 to 0x184D2A5F for such frames).
const LAYOUT_MAGIC: u32 = 0x184D_2A5B;

/// How a frame's bytes were arranged before compressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// As the entry holds them.
    #[default]
    Plain,
    /// BC1 blocks of 8 bytes: both colours, then which colour each pixel takes.
    Bc1,
    /// BC2 or BC3 blocks of 16 bytes: alpha end points, alpha indices, colours, colour
    /// indices.
    Bc3,
    /// BC5 blocks of 16 bytes: both channels' end points, then both channels' indices.
    Bc5,
}

impl Layout {
    /// The layout for blocks of the format `name` (`BC1_SRGB`...): BC4, BC6H and BC7 stay
    /// plain, gathering saves little or nothing on them (`docs/SIZES.md`).
    pub fn of_format(name: &str) -> Self {
        match name.split('_').next() {
            Some("BC1" | "BC1A") => Self::Bc1,
            Some("BC2" | "BC3") => Self::Bc3,
            Some("BC5") => Self::Bc5,
            _ => Self::Plain,
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Plain => 0,
            Self::Bc1 => 1,
            Self::Bc3 => 2,
            Self::Bc5 => 3,
        }
    }

    fn of_code(code: u8) -> Option<Self> {
        [Self::Bc1, Self::Bc3, Self::Bc5]
            .into_iter()
            .find(|l| l.code() == code)
    }

    /// The block's size and its fields, each `(offset, length)`; `None` when plain.
    fn fields(self) -> Option<(usize, &'static [(usize, usize)])> {
        Some(match self {
            Self::Plain => return None,
            Self::Bc1 => (8, &[(0, 4), (4, 4)]),
            Self::Bc3 => (16, &[(0, 2), (8, 4), (2, 6), (12, 4)]),
            Self::Bc5 => (16, &[(0, 2), (8, 2), (2, 6), (10, 6)]),
        })
    }

    /// `data` with each whole block's fields gathered: all of the first field, then the
    /// second... Bytes after the last whole block stay at the end, as they are.
    pub fn gather(self, data: &[u8]) -> Vec<u8> {
        let Some((block, fields)) = self.fields() else {
            return data.to_vec();
        };
        let blocks = data.chunks_exact(block);
        let rest = blocks.remainder();
        let mut out = Vec::with_capacity(data.len());
        for &(offset, length) in fields {
            for b in blocks.clone() {
                out.extend_from_slice(&b[offset..offset + length]);
            }
        }
        out.extend_from_slice(rest);
        out
    }

    /// Undoes [`Self::gather`]: `gathered` back into `out`, of the same length.
    pub fn scatter(self, gathered: &[u8], out: &mut [u8]) {
        let Some((block, fields)) = self.fields() else {
            out.copy_from_slice(gathered);
            return;
        };
        let whole = gathered.len() / block * block;
        let mut from = 0;
        for &(offset, length) in fields {
            for b in out[..whole].chunks_exact_mut(block) {
                b[offset..offset + length].copy_from_slice(&gathered[from..from + length]);
                from += length;
            }
        }
        out[whole..].copy_from_slice(&gathered[whole..]);
    }

    /// The skippable frame that says the next frame is in this layout.
    fn marker(self) -> [u8; 9] {
        let mut m = [0; 9];
        m[..4].copy_from_slice(&LAYOUT_MAGIC.to_le_bytes());
        m[4..8].copy_from_slice(&1u32.to_le_bytes());
        m[8] = self.code();
        m
    }
}

/// Writes frames; one per thread.
pub struct FrameEncoder<'d> {
    plain: Compressor<'d>,
    ebx: Option<Compressor<'d>>,
}

impl<'d> FrameEncoder<'d> {
    pub fn new(dictionary: Option<&'d EncoderDictionary<'static>>) -> Result<Self> {
        let mut plain = Compressor::new(LEVEL).map_err(zstd_error)?;
        plain.include_checksum(true).map_err(zstd_error)?;
        let ebx = match dictionary {
            Some(d) => {
                let mut c = Compressor::with_prepared_dictionary(d).map_err(zstd_error)?;
                c.include_checksum(true).map_err(zstd_error)?;
                Some(c)
            }
            None => None,
        };
        Ok(Self { plain, ebx })
    }

    /// A frame of `data`; `ebx` frames use the dictionary, if there is one.
    pub fn encode(&mut self, data: &[u8], ebx: bool) -> Result<Vec<u8>> {
        self.encode_as(data, ebx, Layout::Plain)
    }

    /// A frame of `data` laid out as `layout`, after the marker that says so unless it
    /// is plain.
    pub fn encode_as(&mut self, data: &[u8], ebx: bool, layout: Layout) -> Result<Vec<u8>> {
        let c = match &mut self.ebx {
            Some(c) if ebx => c,
            _ => &mut self.plain,
        };
        if layout == Layout::Plain {
            return c.compress(data).map_err(zstd_error);
        }
        let mut out = layout.marker().to_vec();
        out.extend(c.compress(&layout.gather(data)).map_err(zstd_error)?);
        Ok(out)
    }
}

/// Reads frames back.
pub struct FrameDecoder {
    plain: Decompressor<'static>,
    dictionary: Option<(u32, Decompressor<'static>)>,
}

impl FrameDecoder {
    /// A decoder for a store with `dictionary` (empty for none).
    pub fn new(dictionary: &[u8]) -> Result<Self> {
        let plain = Decompressor::new().map_err(zstd_error)?;
        let dictionary = if dictionary.is_empty() {
            None
        } else {
            let Some(id) = zstd_safe::get_dict_id_from_dict(dictionary) else {
                bail!("the store's dictionary has no id");
            };
            let d = Decompressor::with_dictionary(dictionary).map_err(zstd_error)?;
            Some((id.get(), d))
        };
        Ok(Self { plain, dictionary })
    }

    /// The entry in `stored`, which must be exactly one or more whole frames.
    pub fn decode(&mut self, stored: &[u8]) -> Result<Vec<u8>> {
        let mut frames = Vec::new();
        let mut rest = stored;
        let mut total = 0u64;
        while !rest.is_empty() || frames.is_empty() {
            let layout = match rest.get(..4) {
                Some(m) if m == LAYOUT_MAGIC.to_le_bytes() => {
                    let Some(&[_, _, _, _, 1, 0, 0, 0, code]) = rest.get(..9) else {
                        bail!("a damaged layout marker");
                    };
                    let Some(layout) = Layout::of_code(code) else {
                        bail!("layout {code}; this nfsh reads 1 to 3");
                    };
                    rest = &rest[9..];
                    layout
                }
                _ => Layout::Plain,
            };
            let size = match zstd_safe::get_frame_content_size(rest) {
                Ok(Some(size)) => size,
                _ => bail!("not a zstd frame with its size"),
            };
            total += size;
            if total > MAX_ENTRY {
                bail!("an entry of {total} bytes; at most {MAX_ENTRY}");
            }
            let Ok(len) = zstd_safe::find_frame_compressed_size(rest) else {
                bail!("the {} bytes are not whole frames", stored.len());
            };
            if len == 0 || len > rest.len() {
                bail!("the {} bytes are not whole frames", stored.len());
            }
            let (frame, after) = rest.split_at(len);
            frames.push((frame, size as usize, layout));
            rest = after;
        }
        let mut out = vec![0; total as usize];
        let mut pos = 0;
        let mut gathered = Vec::new();
        for (frame, size, layout) in frames {
            let d = match (
                zstd_safe::get_dict_id_from_frame(frame),
                &mut self.dictionary,
            ) {
                (None, _) => &mut self.plain,
                (Some(id), Some((ours, d))) if id.get() == *ours => d,
                (Some(id), _) => bail!("the frame needs dictionary {id}, not the store's"),
            };
            let to = &mut out[pos..pos + size];
            let done = if layout == Layout::Plain {
                d.decompress_to_buffer(frame, to).map_err(zstd_error)?
            } else {
                gathered.resize(size, 0);
                let done = d
                    .decompress_to_buffer(frame, &mut gathered[..])
                    .map_err(zstd_error)?;
                if done == size {
                    layout.scatter(&gathered, to);
                }
                done
            };
            if done != size {
                bail!("the frame decompressed to {done} of {size} bytes");
            }
            pos += size;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small ebx-like records sharing their layout, enough to train a dictionary.
    fn samples() -> Vec<Vec<u8>> {
        (0..400u32)
            .map(|i| {
                let mut s = b"EBX\x00partition guid; type RaceVehicleBlueprint; ".to_vec();
                s.extend(format!("cars/car_{i:04}/tuning {i} wheel {}", i * 7).bytes());
                s.extend(i.to_le_bytes().repeat(8));
                s
            })
            .collect()
    }

    #[test]
    fn frames_read_back_with_and_without_the_dictionary() {
        let dictionary = train(&samples());
        assert!(!dictionary.is_empty());
        let prepared = prepare(&dictionary);
        let mut enc = FrameEncoder::new(prepared.as_ref()).unwrap();
        let mut dec = FrameDecoder::new(&dictionary).unwrap();
        let entry = &samples()[7];
        let ebx = enc.encode(entry, true).unwrap();
        let other = enc.encode(entry, false).unwrap();
        assert!(ebx.len() < other.len(), "{} vs {}", ebx.len(), other.len());
        assert_eq!(dec.decode(&ebx).unwrap(), *entry);
        assert_eq!(dec.decode(&other).unwrap(), *entry);
        // Empty entries are frames too.
        assert_eq!(dec.decode(&enc.encode(b"", false).unwrap()).unwrap(), b"");
    }

    #[test]
    fn a_store_without_a_dictionary_writes_plain_frames() {
        assert!(train(&[]).is_empty());
        assert!(prepare(&[]).is_none());
        let mut enc = FrameEncoder::new(None).unwrap();
        let frame = enc.encode(b"abcabcabc", true).unwrap();
        assert_eq!(zstd_safe::get_dict_id_from_frame(&frame), None);
        assert_eq!(
            FrameDecoder::new(&[]).unwrap().decode(&frame).unwrap(),
            b"abcabcabc"
        );
    }

    #[test]
    fn damaged_frames_are_errors() {
        let dictionary = train(&samples());
        let prepared = prepare(&dictionary);
        let mut enc = FrameEncoder::new(prepared.as_ref()).unwrap();
        let entry = samples().concat();
        let frame = enc.encode(&entry, false).unwrap();
        let mut dec = FrameDecoder::new(&dictionary).unwrap();
        for len in [0, 4, 10, frame.len() - 1] {
            assert!(dec.decode(&frame[..len]).is_err(), "truncated to {len}");
        }
        // Frames back to back are one entry; anything else after a frame is damage.
        let mut longer = frame.clone();
        longer.extend(&frame);
        assert_eq!(dec.decode(&longer).unwrap(), [&entry[..], &entry].concat());
        let mut trailing = frame.clone();
        trailing.extend(b"junk");
        assert!(dec.decode(&trailing).is_err());
        // A flipped payload byte fails the checksum, if not the decoder.
        let mut flipped = frame.clone();
        let mid = flipped.len() / 2;
        flipped[mid] ^= 0x55;
        assert!(dec.decode(&flipped).is_err());
        // An ebx frame needs the dictionary it was written with.
        let ebx = enc.encode(&samples()[0], true).unwrap();
        let err = FrameDecoder::new(&[]).unwrap().decode(&ebx).unwrap_err();
        assert!(err.0.contains("needs dictionary"), "{err}");
    }

    #[test]
    fn gathered_blocks_scatter_back_byte_for_byte() {
        // 5 BC3 blocks and 3 bytes past the last one.
        let data: Vec<u8> = (0..83u8).collect();
        let gathered = Layout::Bc3.gather(&data);
        assert_eq!(gathered.len(), data.len());
        // The alpha end points of every block come first, then the alpha indices.
        assert_eq!(&gathered[..4], [0, 1, 16, 17]);
        assert_eq!(&gathered[10..14], [8, 9, 10, 11]);
        assert_eq!(&gathered[80..], [80, 81, 82]);
        for layout in [Layout::Plain, Layout::Bc1, Layout::Bc3, Layout::Bc5] {
            for len in [0, 7, 8, 16, 83, 4096] {
                let data: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
                let mut back = vec![0; len];
                layout.scatter(&layout.gather(&data), &mut back);
                assert_eq!(back, data, "{layout:?}, {len} bytes");
            }
        }
        assert_eq!(Layout::of_format("BC1A_SRGB"), Layout::Bc1);
        assert_eq!(Layout::of_format("BC3_UNORM"), Layout::Bc3);
        assert_eq!(Layout::of_format("BC5_UNORM"), Layout::Bc5);
        assert_eq!(Layout::of_format("BC7_SRGB"), Layout::Plain);
        assert_eq!(Layout::of_format("BC4_UNORM"), Layout::Plain);
    }

    #[test]
    fn a_gathered_frame_reads_back_after_its_marker() {
        let mut enc = FrameEncoder::new(None).unwrap();
        let mut dec = FrameDecoder::new(&[]).unwrap();
        let top: Vec<u8> = (0..4096u32).map(|i| (i * 31 % 256) as u8).collect();
        let rest: Vec<u8> = (0..100u32).map(|i| (i * 13 % 256) as u8).collect();
        let first = enc.encode_as(&top, false, Layout::Bc1).unwrap();
        assert_eq!(&first[..9], [0x5B, 0x2A, 0x4D, 0x18, 1, 0, 0, 0, 1]);
        let second = enc.encode_as(&rest, false, Layout::Bc5).unwrap();
        // A frame per tier: each reads alone, and back to back.
        assert_eq!(dec.decode(&first).unwrap(), top);
        assert_eq!(dec.decode(&second).unwrap(), rest);
        let both = [&first[..], &second].concat();
        assert_eq!(dec.decode(&both).unwrap(), [&top[..], &rest].concat());
        // A plain frame has no marker.
        let plain = enc.encode_as(&rest, false, Layout::Plain).unwrap();
        assert_eq!(plain, enc.encode(&rest, false).unwrap());
        // A marker alone, damaged or of an unknown layout is an error.
        assert!(dec.decode(&first[..9]).is_err());
        let mut unknown = first.clone();
        unknown[8] = 9;
        assert!(dec.decode(&unknown).unwrap_err().0.contains("layout 9"));
        let mut long = first.clone();
        long[4] = 2;
        assert!(dec.decode(&long).is_err());
    }
}

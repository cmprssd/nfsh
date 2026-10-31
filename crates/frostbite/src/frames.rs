//! How nfsh's store keeps an entry: one zstd frame of the entry's decompressed bytes, at
//! level [`LEVEL`], with the decompressed size and a checksum in the frame. ebx frames
//! use the store's dictionary, trained on a sample of the install's ebx at import: ebx
//! are small and share their structure, so it makes them about 15% smaller, and does
//! nothing for the rest. A frame names its dictionary by id, so a reader needs no list
//! of which entries use it.

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

    /// One entry's frame; `ebx` frames use the dictionary, if there is one.
    pub fn encode(&mut self, data: &[u8], ebx: bool) -> Result<Vec<u8>> {
        let c = match &mut self.ebx {
            Some(c) if ebx => c,
            _ => &mut self.plain,
        };
        c.compress(data).map_err(zstd_error)
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

    /// The entry in `frame`, which must be exactly one whole frame.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        let size = match zstd_safe::get_frame_content_size(frame) {
            Ok(Some(size)) if size <= MAX_ENTRY => size as usize,
            Ok(Some(size)) => bail!("a frame of {size} bytes; at most {MAX_ENTRY}"),
            _ => bail!("not a zstd frame with its size"),
        };
        if zstd_safe::find_frame_compressed_size(frame) != Ok(frame.len()) {
            bail!("the {} bytes are not exactly one frame", frame.len());
        }
        let d = match (
            zstd_safe::get_dict_id_from_frame(frame),
            &mut self.dictionary,
        ) {
            (None, _) => &mut self.plain,
            (Some(id), Some((ours, d))) if id.get() == *ours => d,
            (Some(id), _) => bail!("the frame needs dictionary {id}, not the store's"),
        };
        let out = d.decompress(frame, size).map_err(zstd_error)?;
        if out.len() != size {
            bail!("the frame decompressed to {} of {size} bytes", out.len());
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
        let mut longer = frame.clone();
        longer.extend(&frame);
        assert!(dec.decode(&longer).is_err());
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
}

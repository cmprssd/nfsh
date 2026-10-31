//! `nfsh texture`: one texture's header from a store, as text; with a path, its pixels
//! written as a .dds file for a look in any image viewer.

use std::fmt::Write;
use std::path::Path;

use frostbite::Result;
use frostbite::store::Store;
use frostbite::texture::{Kind, Texture, Textures};

/// Reads the texture asset `name` from the store in `dir` and describes it; writes it
/// to `dds` when given.
pub fn run(dir: &Path, name: &str, dds: Option<&Path>) -> Result<String> {
    let store = Store::open(dir)?;
    let mut textures = Textures::new(&store)?;
    let t = textures.by_name(name)?;
    let format = match t.format() {
        Some(f) => f.name.to_owned(),
        None => format!("{:#04x} (not worked out)", t.format),
    };
    let chunk: String = t.chunk.iter().map(|b| format!("{b:02x}")).collect();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{name}: {:?} {}×{}, {} layer(s), {format}, group {:?}, flags {:#x}",
        t.kind, t.width, t.height, t.layers, t.group, t.flags
    );
    let _ = writeln!(
        out,
        "chunk {chunk}: {} bytes; {} mips, the first {} only in the whole chunk",
        t.chunk_bytes(),
        t.mips(),
        t.streamed
    );
    for (level, size) in t.mip_sizes.iter().enumerate() {
        let (w, h) = t.mip_extent(level);
        let _ = writeln!(out, "  mip {level}: {w}×{h}, {size} bytes a layer");
    }
    if let Some(path) = dds {
        let pixels = textures.pixels(&t)?;
        std::fs::write(path, dds_file(&t, &pixels)?)
            .map_err(|e| frostbite::Error(format!("{}: {e}", path.display())))?;
        let _ = writeln!(out, "written to {}", path.display());
    }
    Ok(out)
}

/// The texture as a .dds file (with the DX10 header): each layer's mips, largest first.
/// Heat stores each mip's layers together, so the mips are regrouped by layer.
fn dds_file(t: &Texture, pixels: &[u8]) -> Result<Vec<u8>> {
    let Some(format) = t.format() else {
        return Err(frostbite::Error(format!(
            "format {:#04x} is not worked out; no .dds written",
            t.format
        )));
    };
    if t.kind == Kind::Volume {
        return Err(frostbite::Error(
            "volume textures are not written as .dds yet".to_owned(),
        ));
    }
    const CAPS_TEXTURE: u32 = 0x1000;
    const CAPS_COMPLEX: u32 = 0x8;
    const CAPS_MIPMAP: u32 = 0x40_0000;
    const CUBE_ALL_FACES: u32 = 0xFE00;
    const MISC_CUBE: u32 = 0x4;
    let cube = t.kind == Kind::Cube;
    let mut head: Vec<u32> = vec![
        124,                                            // header size
        0x1 | 0x2 | 0x4 | 0x1000 | 0x2_0000 | 0x8_0000, // caps, height, width, format, mip count, linear size
        u32::from(t.height),
        u32::from(t.width),
        t.mip_sizes[0],
        0, // depth
        t.mips() as u32,
    ];
    head.extend([0; 11]);
    head.extend([32, 0x4, u32::from_le_bytes(*b"DX10"), 0, 0, 0, 0, 0]);
    head.extend([
        CAPS_TEXTURE | CAPS_COMPLEX | CAPS_MIPMAP,
        if cube { CUBE_ALL_FACES | 0x200 } else { 0 },
        0,
        0,
        0,
    ]);
    let array_size = if cube { t.layers / 6 } else { t.layers };
    head.extend([
        format.dxgi,
        3, // a 2D texture
        if cube { MISC_CUBE } else { 0 },
        u32::from(array_size),
        0,
    ]);
    let mut out = b"DDS ".to_vec();
    out.extend(head.iter().flat_map(|w| w.to_le_bytes()));
    for layer in 0..t.layers {
        for level in 0..t.mips() {
            let Some(range) = t.mip(level, layer) else {
                return Err(frostbite::Error(format!("no mip {level} of layer {layer}")));
            };
            let Some(bytes) = pixels.get(range) else {
                return Err(frostbite::Error(format!(
                    "mip {level} of layer {layer} is past the chunk"
                )));
            };
            out.extend_from_slice(bytes);
        }
    }
    Ok(out)
}

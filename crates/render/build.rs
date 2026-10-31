//! Compiles every WGSL shader in shaders/ to SPIR-V with naga, so the build needs no
//! external shader compiler. Each `<name>.wgsl` has a `vs_main` and an `fs_main`
//! entry point and becomes `<name>.vert.spv` and `<name>.frag.spv` in OUT_DIR, each
//! keeping its entry point name.

use std::path::Path;

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    println!("cargo:rerun-if-changed=shaders");
    for entry in std::fs::read_dir("shaders").unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("wgsl") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.emit_to_string(&source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.emit_to_string(&source)));
        let stem = path.file_stem().unwrap().to_str().unwrap();
        // Shaders already work in Vulkan's clip space (y down); naga would otherwise flip
        // y, converting from WebGPU's convention.
        let mut spv_options = naga::back::spv::Options::default();
        spv_options
            .flags
            .remove(naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE);
        for (stage, entry, suffix) in [
            (naga::ShaderStage::Vertex, "vs_main", "vert"),
            (naga::ShaderStage::Fragment, "fs_main", "frag"),
        ] {
            let words = naga::back::spv::write_vec(
                &module,
                &info,
                &spv_options,
                Some(&naga::back::spv::PipelineOptions {
                    shader_stage: stage,
                    entry_point: entry.into(),
                }),
            )
            .unwrap_or_else(|e| panic!("{}: {entry}: {e}", path.display()));
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            std::fs::write(Path::new(&out).join(format!("{stem}.{suffix}.spv")), bytes).unwrap();
        }
    }
}

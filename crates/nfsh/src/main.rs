//! nfsh: a new engine for Need for Speed: Heat. For now it has eleven commands: an
//! inventory of the owner's install, a check that every entry of the install or of a
//! store decompresses, the import into nfsh's own store, the copy of a store into an
//! install of the parts and tiers a player keeps, the store's partition GUIDs
//! listed again, an ebx partition printed as text, Palm City's level graph, one
//! mesh's geometry, one texture, what a store's bytes are, and a window to fly through one zone.

mod ebx;
mod fly;
mod import;
mod install;
mod inventory;
mod level;
mod mesh;
mod path;
mod sizes;
mod texture;
mod verify;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: nfsh inventory [INSTALL]
  Lists what a Need for Speed: Heat install holds, as Markdown on stdout.
usage: nfsh verify [INSTALL | STORE]
  Decompresses every entry of an install or an nfsh store and checks its size
  (an install only on Windows: it uses the install's own Oodle library). For a store,
  also checks that the base and any one language find every chunk they name.
usage: nfsh import [INSTALL] STORE
  Recompresses the data the install still uses into nfsh's own store in the folder
  STORE (created if missing; an earlier store there is replaced): identical content
  once, chunks nothing names left out, every frame tagged with its part and tier.
  Checks the copy and prints its size. Windows only, like verifying an install.
usage: nfsh install STORE DEST [--languages L,...] [--cut TIER,...] [--bare-minimum]
  Copies the store in the folder STORE into an install in the folder DEST (created
  if missing; an earlier store or install there is replaced), one file per part and
  tier: the base, the languages L (en, de, es, fr, it, jp, pl, ru; default: every
  one the store has), and every tier but the ones cut: mip0, mip1 and mip2 for a
  texture's top three mips, lod0, lod1, ... for a mesh's LODs, video for the
  cutscenes' movies, voices for the spoken lines and the cinematics' audio, music.
  A texture keeps its smallest mip and a mesh its last LOD. A language the store
  has no Win32/loc table for also keeps the part others: the cinematics' lines in
  the languages without one. --bare-minimum cuts mip0 to mip2, lod0, lod1, video,
  voices and music. Every other command reads an install as it does a store.
usage: nfsh reindex STORE
  Lists every ebx's partition GUID in the index of the store in the folder STORE
  (the import does it; this adds them to a store made before nfsh 0.6.0).
usage: nfsh ebx STORE NAME
  Prints the ebx partition NAME (e.g. levels/mainlevel/staticworld) from the store
  in the folder STORE as text: its imports, then each instance with its fields.
usage: nfsh level STORE [ZONE]
  Palm City's level graph from the store in the folder STORE: without ZONE, the
  bundles the level loads and its streaming zones; with ZONE (a zone number such as
  495, or a bundle name), every mesh the zone places and where.
usage: nfsh mesh STORE NAME [OBJ]
  The mesh asset NAME (a RigidMeshAsset or CompositeMeshAsset ebx, as nfsh level
  lists them) from the store in the folder STORE: its LODs and their sections, with
  the first LOD decoded; written to the file OBJ as a Wavefront .obj when given.
usage: nfsh texture STORE NAME [DDS]
  The texture asset NAME (a TextureAsset ebx) from the store in the folder STORE:
  its kind, size, format and mips; written to the file DDS as a .dds when given.
usage: nfsh sizes STORE
  What the store in the folder STORE holds, by kind, and what streaming it takes,
  as Markdown: reads every entry, once.
usage: nfsh sizes-trial STORE
  Recompresses a sample of the texture and mesh chunks with their fields gathered, and
  the textures split into a frame per tier (top mip, second, third, the rest), and
  compares with what they store now. Nothing is written.
usage: nfsh view STORE ZONE [--shot PNG] [--vsync] [--night] [--camera X,Y,Z,YAW,PITCH]
                 [--path FILE [--headless]]
  Palm City's zone ZONE (as nfsh level takes it), the roads near it and the ground
  under them from the store in the folder STORE, textured and lit, in a window: mouse
  to look, W A S D to move, Space and C up and down, Shift to go faster, N to switch
  between midday and tealnight, Esc to quit;
  the title shows the time of day, the frame rate and frame time. With --shot, one
  frame of the whole zone is drawn without a window and written to the file PNG.
  --vsync waits for the display; --night starts at tealnight; --camera starts at the
  point X,Y,Z turned YAW degrees from +z towards +x and PITCH up, rather than above
  the zone. --path flies the camera path in FILE (lines of T X Y Z YAW PITCH, time
  in seconds) and closes at its end, printing the time to the first frame and the
  frame times along it (mean, median, p99, 1% low, max); --headless draws that path
  without a window, a frame every 1/60 s of path time, one at a time.
INSTALL is the game folder (with Data/ and Patch/); default: $NFSH_INSTALL.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(("view", rest)) = args.split_first().map(|(c, r)| (c.as_str(), r)) {
        let Some(options) = view::options(rest) else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        let start = Instant::now();
        return match view::run(&options) {
            Ok(text) => {
                print!("{text}");
                eprintln!("view took {:.1} s", start.elapsed().as_secs_f64());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("nfsh: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(("install", rest)) = args.split_first().map(|(c, r)| (c.as_str(), r)) {
        let Some(options) = install::options(rest) else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        let start = Instant::now();
        let result = install::run(&options).map(|c| (install::markdown(&c), 0));
        return finish("install", start, result);
    }
    let env_install = || std::env::var_os("NFSH_INSTALL").map(PathBuf::from);
    // The import's last argument is the store; the install may come before it.
    let (cmd, install, store) = match args.as_slice() {
        [cmd, store] if cmd == "import" => (cmd.as_str(), env_install(), Some(store)),
        [cmd, path, store]
            if ["import", "ebx", "level", "mesh", "texture"].contains(&cmd.as_str()) =>
        {
            (cmd.as_str(), Some(path.into()), Some(store))
        }
        [cmd] => (cmd.as_str(), env_install(), None),
        [cmd, path] => (cmd.as_str(), Some(PathBuf::from(path)), None),
        [cmd, path, name, file] if cmd == "mesh" || cmd == "texture" => {
            let start = Instant::now();
            let result = match cmd.as_str() {
                "mesh" => mesh::run(path.as_ref(), name, Some(file.as_ref())),
                _ => texture::run(path.as_ref(), name, Some(file.as_ref())),
            };
            return finish(cmd, start, result.map(|text| (text, 0)));
        }
        _ => ("", None, None),
    };
    let start = Instant::now();
    let result = match (cmd, install, store) {
        ("inventory", Some(install), None) => {
            inventory::scan(&install).map(|report| (report.markdown(), 0))
        }
        ("verify", Some(install), None) => {
            verify::run(&install).map(|report| (report.markdown(), report.failed()))
        }
        ("import", Some(install), Some(store)) => {
            import::run(&install, store.as_ref()).map(|imported| (import::markdown(&imported), 0))
        }
        ("reindex", Some(store), None) => frostbite::store::reindex(&store).map(|size| {
            (
                format!("index rewritten with the partition GUIDs: {size} bytes\n"),
                0,
            )
        }),
        ("sizes", Some(store), None) => sizes::run(&store).map(|text| (text, 0)),
        ("sizes-trial", Some(store), None) => sizes::trial(&store).map(|text| (text, 0)),
        ("ebx", Some(store), Some(name)) => ebx::run(&store, name).map(|text| (text, 0)),
        ("level", Some(store), zone) => {
            level::run(&store, zone.map(String::as_str)).map(|text| (text, 0))
        }
        ("mesh", Some(store), Some(name)) => mesh::run(&store, name, None).map(|text| (text, 0)),
        ("texture", Some(store), Some(name)) => {
            texture::run(&store, name, None).map(|text| (text, 0))
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    finish(cmd, start, result)
}

/// Prints a command's output and how long it took; fails when it failed or reported
/// failures.
fn finish(cmd: &str, start: Instant, result: frostbite::Result<(String, u64)>) -> ExitCode {
    match result {
        Ok((markdown, failed)) => {
            print!("{markdown}");
            eprintln!("{cmd} took {:.1} s", start.elapsed().as_secs_f64());
            if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("nfsh: {e}");
            ExitCode::FAILURE
        }
    }
}

//! nfsh: a new engine for Need for Speed: Heat. For now it has seven commands: an
//! inventory of the owner's install, a check that every entry of the install or of a
//! store decompresses, the import into nfsh's own store, the store's partition GUIDs
//! listed again, an ebx partition printed as text, Palm City's level graph, and one
//! mesh's geometry.

mod ebx;
mod import;
mod inventory;
mod level;
mod mesh;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: nfsh inventory [INSTALL]
  Lists what a Need for Speed: Heat install holds, as Markdown on stdout.
usage: nfsh verify [INSTALL | STORE]
  Decompresses every entry of an install or an nfsh store and checks its size
  (an install only on Windows: it uses the install's own Oodle library).
usage: nfsh import [INSTALL] STORE
  Recompresses the data the install still uses into nfsh's own store in the folder
  STORE (created if missing; an earlier store there is replaced), checks the copy,
  and prints its size. Windows only, like verifying an install.
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
INSTALL is the game folder (with Data/ and Patch/); default: $NFSH_INSTALL.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env_install = || std::env::var_os("NFSH_INSTALL").map(PathBuf::from);
    // The import's last argument is the store; the install may come before it.
    let (cmd, install, store) = match args.as_slice() {
        [cmd, store] if cmd == "import" => (cmd.as_str(), env_install(), Some(store)),
        [cmd, path, store] if ["import", "ebx", "level", "mesh"].contains(&cmd.as_str()) => {
            (cmd.as_str(), Some(path.into()), Some(store))
        }
        [cmd] => (cmd.as_str(), env_install(), None),
        [cmd, path] => (cmd.as_str(), Some(PathBuf::from(path)), None),
        [cmd, path, name, obj] if cmd == "mesh" => {
            let start = Instant::now();
            let result = mesh::run(path.as_ref(), name, Some(obj.as_ref()));
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
        ("ebx", Some(store), Some(name)) => ebx::run(&store, name).map(|text| (text, 0)),
        ("level", Some(store), zone) => {
            level::run(&store, zone.map(String::as_str)).map(|text| (text, 0))
        }
        ("mesh", Some(store), Some(name)) => mesh::run(&store, name, None).map(|text| (text, 0)),
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

//! nfsh: a new engine for Need for Speed: Heat. For now it has three commands: an
//! inventory of the owner's install, a check that every entry of the install or of a
//! store decompresses, and the import into nfsh's own store.

mod import;
mod inventory;
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
INSTALL is the game folder (with Data/ and Patch/); default: $NFSH_INSTALL.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env_install = || std::env::var_os("NFSH_INSTALL").map(PathBuf::from);
    // The import's last argument is the store; the install may come before it.
    let (cmd, install, store) = match args.as_slice() {
        [cmd, store] if cmd == "import" => (cmd.as_str(), env_install(), Some(store)),
        [cmd, path, store] if cmd == "import" => (cmd.as_str(), Some(path.into()), Some(store)),
        [cmd] => (cmd.as_str(), env_install(), None),
        [cmd, path] => (cmd.as_str(), Some(PathBuf::from(path)), None),
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
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
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

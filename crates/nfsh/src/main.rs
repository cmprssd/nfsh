//! nfsh: a new engine for Need for Speed: Heat. For now it has two commands: an
//! inventory of the owner's install, and a check that every entry in it decompresses.

mod inventory;
mod verify;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: nfsh inventory [INSTALL]
  Lists what a Need for Speed: Heat install holds, as Markdown on stdout.
usage: nfsh verify [INSTALL]
  Decompresses every entry the install's tables point at and checks its size
  (Windows only: it uses the install's own Oodle library).
INSTALL is the game folder (with Data/ and Patch/); default: $NFSH_INSTALL.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, install) = match args.as_slice() {
        [cmd] => (cmd, std::env::var_os("NFSH_INSTALL").map(PathBuf::from)),
        [cmd, path] => (cmd, Some(PathBuf::from(path))),
        _ => (&String::new(), None),
    };
    let (Some(install), "inventory" | "verify") = (install, cmd.as_str()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let start = Instant::now();
    let result = if cmd == "inventory" {
        inventory::scan(&install).map(|report| (report.markdown(), 0))
    } else {
        verify::run(&install).map(|report| (report.markdown(), report.failed()))
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

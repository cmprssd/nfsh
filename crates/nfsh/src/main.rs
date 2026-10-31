//! nfsh: a new engine for Need for Speed: Heat. For now it has one command, an
//! inventory of the owner's install.

mod inventory;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: nfsh inventory [INSTALL]
  Lists what a Need for Speed: Heat install holds, as Markdown on stdout.
  INSTALL is the game folder (with Data/ and Patch/); default: $NFSH_INSTALL.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let install = match args.as_slice() {
        [cmd] if cmd == "inventory" => std::env::var_os("NFSH_INSTALL").map(PathBuf::from),
        [cmd, path] if cmd == "inventory" => Some(PathBuf::from(path)),
        _ => None,
    };
    let Some(install) = install else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let start = Instant::now();
    match inventory::scan(&install) {
        Ok(report) => {
            print!("{}", report.markdown());
            eprintln!("scanned in {:.1} s", start.elapsed().as_secs_f64());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nfsh: {e}");
            ExitCode::FAILURE
        }
    }
}

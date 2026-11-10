//! `nfsh install`: copies a store into an install with only the languages and tiers
//! the player keeps (see [`frostbite::copy`]), and reports what it holds.

use std::fmt::Write as _;
use std::path::PathBuf;

use frostbite::Result;
use frostbite::copy::{self, Choice, Copied};
use frostbite::plan::tier;
use frostbite::store::Store;

use crate::inventory::size;

/// What `nfsh install` was asked: the store, the install's folder and the choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub store: PathBuf,
    pub dest: PathBuf,
    pub choice: Choice,
}

/// The cuts `--bare-minimum` stands for: the top three mips, the meshes' LOD 0 and 1,
/// cutscene video, voices and music, as `docs/DECISIONS.md` ("One disc, stripping
/// down") lists them.
pub const BARE_MINIMUM: [&str; 8] = [
    "mip0", "mip1", "mip2", "lod0", "lod1", "video", "voices", "music",
];

/// Reads `STORE DEST [--languages L,…] [--cut TIER,…] [--bare-minimum]`; `None` when
/// they aren't that.
pub fn options(args: &[String]) -> Option<Options> {
    let [store, dest, rest @ ..] = args else {
        return None;
    };
    let mut choice = Choice::default();
    let mut rest = rest.iter();
    let list = |v: &String| v.split(',').map(str::to_owned).collect::<Vec<_>>();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--languages" => choice.languages = Some(list(rest.next()?)),
            "--cut" => {
                for slug in list(rest.next()?) {
                    choice.cut.push(tier::of_slug(&slug)?);
                }
            }
            "--bare-minimum" => {
                choice
                    .cut
                    .extend(BARE_MINIMUM.iter().filter_map(|s| tier::of_slug(s)));
            }
            _ => return None,
        }
    }
    choice.cut.sort_unstable();
    choice.cut.dedup();
    Some(Options {
        store: store.into(),
        dest: dest.into(),
        choice,
    })
}

pub fn run(o: &Options) -> Result<Copied> {
    let store = Store::open(&o.store)?;
    copy::copy(&store, &o.dest, &o.choice, &mut |s| eprintln!("{s}"))
}

pub fn markdown(c: &Copied) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "| file | part | tier | size |\n|------|------|------|-----:|"
    );
    for f in &c.files {
        let part = c.parts.get(usize::from(f.part)).map_or("?", String::as_str);
        let _ = writeln!(
            out,
            "| {} | {part} | {} | {} |",
            f.name,
            tier::name(f.tier),
            size(f.size)
        );
    }
    let total = c.bytes + c.index;
    let _ = writeln!(
        out,
        "\nThe install takes {} ({} bytes, with its index and placement), against the \
         store's {}; {} of the store's {} frames are left out.",
        size(total),
        total,
        size(c.store_bytes),
        c.frames_cut,
        c.frames
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn options_name_the_languages_and_the_tiers_to_cut() {
        let o = options(&args("S D --languages en,de --cut lod0,mip0")).unwrap();
        assert_eq!(o.store, PathBuf::from("S"));
        assert_eq!(o.dest, PathBuf::from("D"));
        assert_eq!(o.choice.languages, Some(vec!["en".into(), "de".into()]));
        assert_eq!(o.choice.cut, [tier::TOP, tier::LOD]);
        let all = options(&args("S D")).unwrap();
        assert_eq!(all.choice, Choice::default());
        let bare = options(&args("S D --bare-minimum --cut mip0")).unwrap();
        assert_eq!(
            bare.choice.cut,
            [
                tier::TOP,
                tier::SECOND,
                tier::THIRD,
                tier::VIDEO,
                tier::VOICES,
                tier::MUSIC,
                tier::LOD,
                tier::LOD + 1
            ]
        );
        assert_eq!(options(&args("S D --cut whole")), None);
        assert_eq!(options(&args("S D --languages")), None);
        assert_eq!(options(&args("S D --other")), None);
        assert_eq!(options(&args("S")), None);
    }
}

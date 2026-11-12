//! Camera paths for the baseline: a recorded flight that nfsh (and, recorded from it,
//! the original) plays the same way every run, and the frame-time statistics taken
//! along it.
//!
//! A path is text, one key per line: `T X Y Z YAW PITCH`, the time in seconds from
//! the start, the position in metres and the angles in degrees, as `nfsh view
//! --camera` takes them. `#` starts a comment. Times rise strictly; between keys the
//! camera moves in a straight line at a steady rate, and every number is interpolated
//! as written (a turn through north is written as yaw 350 to 370, not 350 to 10).

/// One key of a path: the time in seconds, then `[x, y, z, yaw, pitch]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    pub t: f64,
    pub camera: [f32; 5],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    keys: Vec<Key>,
}

impl Path {
    /// Reads a path; the error names the line.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut keys: Vec<Key> = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let error = |why: &str| format!("line {}: {why}", i + 1);
            let numbers: Vec<f64> = line
                .split_whitespace()
                .map(|n| n.parse::<f64>().ok().filter(|n| n.is_finite()))
                .collect::<Option<_>>()
                .ok_or_else(|| error("not a number"))?;
            let [t, x, y, z, yaw, pitch] = numbers[..] else {
                return Err(error("needs 6 numbers: T X Y Z YAW PITCH"));
            };
            if keys.last().is_some_and(|k| t <= k.t) {
                return Err(error("time does not rise"));
            }
            keys.push(Key {
                t,
                camera: [x, y, z, yaw, pitch].map(|v| v as f32),
            });
        }
        if keys.len() < 2 {
            return Err("a path needs at least two keys".into());
        }
        Ok(Self { keys })
    }

    /// How long the path lasts from its first key, in seconds.
    pub fn duration(&self) -> f64 {
        self.keys[self.keys.len() - 1].t - self.keys[0].t
    }

    /// The camera `t` seconds after the first key; held at the ends.
    pub fn at(&self, t: f64) -> [f32; 5] {
        let t = t + self.keys[0].t;
        let next = self.keys.partition_point(|k| k.t <= t);
        if next == 0 {
            return self.keys[0].camera;
        }
        let Some(b) = self.keys.get(next) else {
            return self.keys[self.keys.len() - 1].camera;
        };
        let a = &self.keys[next - 1];
        let f = ((t - a.t) / (b.t - a.t)) as f32;
        std::array::from_fn(|i| a.camera[i] + (b.camera[i] - a.camera[i]) * f)
    }
}

/// Frame times along a path, as the baseline reports them.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub frames: usize,
    pub mean_ms: f64,
    pub median_ms: f64,
    pub p99_ms: f64,
    /// "1% low": the mean of the slowest 1% of frames (at least one), in ms.
    pub low1_ms: f64,
    pub max_ms: f64,
}

impl Stats {
    /// The statistics of `times_ms`; `None` when there are none.
    pub fn of(times_ms: &[f64]) -> Option<Self> {
        let mut t = times_ms.to_vec();
        t.sort_by(f64::total_cmp);
        let n = t.len();
        if n == 0 {
            return None;
        }
        let slow = &t[n - n.div_ceil(100)..];
        Some(Self {
            frames: n,
            mean_ms: t.iter().sum::<f64>() / n as f64,
            median_ms: if n % 2 == 1 {
                t[n / 2]
            } else {
                (t[n / 2 - 1] + t[n / 2]) / 2.0
            },
            p99_ms: t[(n * 99).div_ceil(100) - 1],
            low1_ms: slow.iter().sum::<f64>() / slow.len() as f64,
            max_ms: t[n - 1],
        })
    }

    /// One line, the same in every mode, for bench/baseline.sh to read.
    pub fn line(&self) -> String {
        format!(
            "path: {} frames, mean {:.2} ms ({:.1} FPS), median {:.2} ms, p99 {:.2} ms, 1% low {:.2} ms ({:.1} FPS), max {:.2} ms",
            self.frames,
            self.mean_ms,
            1000.0 / self.mean_ms.max(1e-6),
            self.median_ms,
            self.p99_ms,
            self.low1_ms,
            1000.0 / self.low1_ms.max(1e-6),
            self.max_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_reads_keys_and_comments() {
        let p =
            Path::parse("# t x y z yaw pitch\n\n1 0 0 0 0 0\n3 10 20 -4 90 -10 # end\n").unwrap();
        assert_eq!(p.duration(), 2.0);
        assert_eq!(p.at(-1.0), [0.0; 5]);
        assert_eq!(p.at(0.0), [0.0; 5]);
        assert_eq!(p.at(1.0), [5.0, 10.0, -2.0, 45.0, -5.0]);
        assert_eq!(p.at(2.0), [10.0, 20.0, -4.0, 90.0, -10.0]);
        assert_eq!(p.at(9.0), [10.0, 20.0, -4.0, 90.0, -10.0]);
    }

    #[test]
    fn interpolation_picks_the_keys_around_the_time() {
        let p = Path::parse("0 0 0 0 350 0\n1 10 0 0 370 0\n3 10 0 20 370 0").unwrap();
        assert_eq!(p.at(0.5), [5.0, 0.0, 0.0, 360.0, 0.0]);
        assert_eq!(p.at(1.0), [10.0, 0.0, 0.0, 370.0, 0.0]);
        assert_eq!(p.at(2.5), [10.0, 0.0, 15.0, 370.0, 0.0]);
    }

    #[test]
    fn a_bad_path_says_which_line() {
        let e = |s: &str| Path::parse(s).unwrap_err();
        assert_eq!(
            e("0 0 0 0 0 0\n1 0 0 0 0"),
            "line 2: needs 6 numbers: T X Y Z YAW PITCH"
        );
        assert_eq!(e("0 0 0 0 0 x"), "line 1: not a number");
        assert_eq!(e("0 0 0 0 0 NaN"), "line 1: not a number");
        assert_eq!(e("0 0 0 0 0 0\n0 1 1 1 1 1"), "line 2: time does not rise");
        assert_eq!(e("# only\n0 0 0 0 0 0"), "a path needs at least two keys");
    }

    #[test]
    fn stats_count_the_slowest_one_percent() {
        // 199 frames of 5 ms and one of 25 ms: 1% of 200 is the two slowest.
        let mut t = vec![5.0; 199];
        t.push(25.0);
        assert_eq!(
            Stats::of(&t),
            Some(Stats {
                frames: 200,
                mean_ms: 1020.0 / 200.0,
                median_ms: 5.0,
                p99_ms: 5.0,
                low1_ms: 15.0,
                max_ms: 25.0,
            })
        );
        // Fewer than 100 frames: the slowest one alone; an even count's median.
        let s = Stats::of(&[4.0, 2.0, 8.0, 6.0]).unwrap();
        assert_eq!((s.median_ms, s.p99_ms, s.low1_ms), (5.0, 8.0, 8.0));
        assert_eq!(Stats::of(&[]), None);
        assert_eq!(
            Stats::of(&[10.0, 20.0]).unwrap().line(),
            "path: 2 frames, mean 15.00 ms (66.7 FPS), median 15.00 ms, p99 20.00 ms, 1% low 20.00 ms (50.0 FPS), max 20.00 ms"
        );
    }

    #[test]
    fn every_bench_path_parses() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/paths");
        let mut n = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let file = entry.unwrap().path();
            let path = Path::parse(&std::fs::read_to_string(&file).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            assert!(path.duration() > 0.0, "{}", file.display());
            n += 1;
        }
        assert!(n >= 2);
    }
}

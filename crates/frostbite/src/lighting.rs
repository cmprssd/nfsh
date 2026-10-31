//! A time of day's lighting: the sun, the sky's colours and the exposure, from a
//! visual environment partition (a `VisualEnvironmentBlueprint` and its components).
//! See `docs/FORMATS.md`, "Lighting".

use crate::ebx::Partition;
use crate::level::{field, float, vec3};
use crate::store::Store;
use crate::{Result, bail};

/// Palm City's midday visual environment (bundle `tod_day3`).
pub const MIDDAY: &str =
    "levels/mainlevel/lighting/visualenvironments/lighting_mainlevel_day_02v03_neutral_ve";
/// Palm City's night visual environment (bundle `tod_night3`, with the tealnight
/// radiosity).
pub const TEALNIGHT: &str =
    "levels/mainlevel/lighting/visualenvironments/lighting_mainlevel_twilight_02_neutral_ve";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    /// `SunColor`, linear RGB.
    pub sun_colour: [f32; 3],
    /// `SunIntensity`: the sun's illuminance in lux (170,000 at midday, 6 at night).
    pub sun_intensity: f32,
    /// `SunRotationX` and `SunRotationY`, in degrees: the sun's bearing and its
    /// elevation above the horizon.
    pub sun_rotation: [f32; 2],
    /// `SkyColor` and `GroundColor`: the light from above and below.
    pub sky_colour: [f32; 3],
    pub ground_colour: [f32; 3],
    /// The tonemap's automatic exposure range (`MinEV`, `MaxEV`, EV at ISO 100) and
    /// its `ExposureCompensation`, in stops.
    pub ev: [f32; 2],
    pub exposure_compensation: f32,
}

impl Lighting {
    /// The lighting the viewer uses when a visual environment can't be read: a sun
    /// at midday's angle, exposed for it.
    pub const FALLBACK: Self = Self {
        sun_colour: [1.0, 0.95, 0.85],
        sun_intensity: 100_000.0,
        sun_rotation: [150.0, 35.0],
        sky_colour: [0.32, 0.34, 0.40],
        ground_colour: [0.05; 3],
        ev: [14.0, 15.0],
        exposure_compensation: 0.0,
    };

    /// Reads the `OutdoorLightComponentData` and `TonemapComponentData` of the
    /// visual environment `p`.
    pub fn parse(p: &Partition) -> Result<Self> {
        let component = |ty: &str| {
            let found = p
                .instances
                .iter()
                .map(|i| p.object(i))
                .find(|o| p.type_name(o.type_index) == ty);
            match found {
                Some(o) => Ok(o),
                None => bail!("{}: no {ty}", p.name),
            }
        };
        let light = component("OutdoorLightComponentData")?;
        let tonemap = component("TonemapComponentData")?;
        let colour = |name| vec3(p, field(p, light, name)?);
        Ok(Self {
            sun_colour: colour("SunColor")?,
            sun_intensity: float(p, light, "SunIntensity")?,
            sun_rotation: [
                float(p, light, "SunRotationX")?,
                float(p, light, "SunRotationY")?,
            ],
            sky_colour: colour("SkyColor")?,
            ground_colour: colour("GroundColor")?,
            ev: [float(p, tonemap, "MinEV")?, float(p, tonemap, "MaxEV")?],
            exposure_compensation: float(p, tonemap, "ExposureCompensation")?,
        })
    }

    /// The visual environment in the ebx `name` of `store`.
    pub fn read(store: &Store, name: &str) -> Result<Self> {
        let Some(asset) = store.ebx(name) else {
            bail!("no ebx named {name} in the store");
        };
        let data = store.entries()?.read(asset.location)?;
        let p = Partition::parse(&data).map_err(|e| crate::Error(format!("{name}: {e}")))?;
        Self::parse(&p)
    }

    /// The unit vector towards the sun: the bearing turns from +z towards +x, the
    /// elevation lifts towards +y.
    pub fn sun_direction(&self) -> [f32; 3] {
        let [bearing, elevation] = self.sun_rotation.map(f32::to_radians);
        [
            bearing.sin() * elevation.cos(),
            elevation.sin(),
            bearing.cos() * elevation.cos(),
        ]
    }

    /// The automatic exposure, in EV at ISO 100: what a light meter reads off a grey
    /// card (18%) in the sun (log2 of its luminance × 100 / 12.5), brightened by the
    /// compensation, then kept in the tonemap's range.
    pub fn ev(&self) -> f32 {
        let grey = 0.18 * self.sun_intensity.max(0.0) / std::f32::consts::PI;
        let metered = (grey * 8.0).max(f32::MIN_POSITIVE).log2();
        let [low, high] = self.ev;
        (metered - self.exposure_compensation).clamp(low.min(high), high.max(low))
    }

    /// What a luminance (cd/m²) is multiplied by to give the tonemap's input: a
    /// camera's 1 / (1.2 × 2^EV) at [`Lighting::ev`].
    pub fn exposure(&self) -> f32 {
        1.0 / (1.2 * self.ev().exp2())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebx::tests::{Builder, F, V};

    fn v3(v: [f32; 3]) -> V {
        V::Struct(v.map(V::F32).to_vec())
    }

    fn environment() -> Vec<u8> {
        let colour = F::Struct("Vec3");
        Builder::default()
            .ty(
                "Vec3",
                false,
                &[("x", F::F32), ("y", F::F32), ("z", F::F32)],
            )
            .ty("VisualEnvironmentBlueprint", true, &[])
            .ty(
                "OutdoorLightComponentData",
                true,
                &[
                    ("SunColor", colour),
                    ("GroundColor", colour),
                    ("SkyColor", colour),
                    ("SunIntensity", F::F32),
                    ("SunRotationY", F::F32),
                    ("SunRotationX", F::F32),
                ],
            )
            .ty(
                "TonemapComponentData",
                true,
                &[
                    ("ExposureCompensation", F::F32),
                    ("MaxEV", F::F32),
                    ("MinEV", F::F32),
                ],
            )
            .instance("VisualEnvironmentBlueprint", [1; 16], vec![])
            .instance(
                "TonemapComponentData",
                [2; 16],
                vec![V::F32(1.0), V::F32(15.0), V::F32(14.0)],
            )
            .instance(
                "OutdoorLightComponentData",
                [3; 16],
                vec![
                    v3([1.0, 0.8, 0.6]),
                    v3([0.05; 3]),
                    v3([0.3, 0.3, 0.4]),
                    V::F32(170_000.0),
                    V::F32(30.0),
                    V::F32(90.0),
                ],
            )
            .build("lighting/day_ve", [4; 16])
    }

    #[test]
    fn reads_the_sun_the_sky_and_the_exposure() {
        let data = environment();
        let l = Lighting::parse(&Partition::parse(&data).unwrap()).unwrap();
        assert_eq!(
            l,
            Lighting {
                sun_colour: [1.0, 0.8, 0.6],
                sun_intensity: 170_000.0,
                sun_rotation: [90.0, 30.0],
                sky_colour: [0.3, 0.3, 0.4],
                ground_colour: [0.05; 3],
                ev: [14.0, 15.0],
                exposure_compensation: 1.0,
            }
        );
        // Bearing 90° is +x; 30° up.
        let d = l.sun_direction();
        assert!((d[0] - 0.866).abs() < 1e-3 && (d[1] - 0.5).abs() < 1e-6 && d[2].abs() < 1e-6);
        // The sun meters at EV 16.25, one stop brighter is still above the range.
        assert_eq!(l.ev(), 15.0);
        assert!((l.exposure() - 1.0 / (1.2 * 32768.0)).abs() < 1e-9);
        // A dim sun meters inside the range: 0.18 × 2000π / π × 8 = 2^11.49…, one
        // stop brighter.
        let dim = Lighting {
            sun_intensity: 2000.0 * std::f32::consts::PI,
            ev: [0.0, 15.0],
            ..l
        };
        assert!((dim.ev() - (2880.0f32.log2() - 1.0)).abs() < 1e-4);
        // No sun at all: the bottom of the range.
        let none = Lighting {
            sun_intensity: 0.0,
            ..dim
        };
        assert_eq!(none.ev(), 0.0);
    }

    #[test]
    fn a_partition_without_the_components_is_an_error() {
        // The builder's empty array names type 2, so three types at least.
        let data = Builder::default()
            .ty("Vec2", false, &[("x", F::F32), ("y", F::F32)])
            .ty("Vec3", false, &[("x", F::F32)])
            .ty("VisualEnvironmentBlueprint", true, &[("Name", F::Str)])
            .instance("VisualEnvironmentBlueprint", [1; 16], vec![V::Str("empty")])
            .build("lighting/empty_ve", [4; 16]);
        let e = Lighting::parse(&Partition::parse(&data).unwrap()).unwrap_err();
        assert_eq!(e.0, "lighting/empty_ve: no OutdoorLightComponentData");
    }

    /// Reads midday and tealnight from the store `NFSH_STORE` points at.
    #[test]
    fn reads_midday_and_tealnight() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(std::path::Path::new(&dir)).unwrap();
        let day = Lighting::read(&store, MIDDAY).unwrap();
        let night = Lighting::read(&store, TEALNIGHT).unwrap();
        // Read on the owner's store, 2026-10-31.
        assert_eq!(day.sun_intensity, 170_000.0);
        assert_eq!(day.sun_rotation, [153.071, 34.724]);
        assert_eq!(day.ev, [14.25, 15.0]);
        assert_eq!(night.sun_intensity, 6.0);
        assert_eq!(night.sun_rotation, [229.606, 90.0]);
        assert_eq!(night.ev, [0.75, 2.25]);
        assert_eq!(night.exposure_compensation, -1.1);
        // Each at the top of its range: the sun meters brighter than it allows.
        assert_eq!(day.ev(), 15.0);
        assert_eq!(night.ev(), 2.25);
    }
}

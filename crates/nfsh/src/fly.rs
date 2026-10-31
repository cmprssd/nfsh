//! The viewer's free-flying camera: forward follows the view (pitch included),
//! strafing stays level, up and down are world up and down.

use nfsh_render::Camera;

/// Metres per second, and the multiplier while Shift is held.
pub const SPEED: f32 = 25.0;
pub const FAST: f32 = 6.0;
/// Degrees per mouse count.
pub const DEGREES_PER_COUNT: f32 = 0.1;

/// Which movement keys are held.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Keys {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub fast: bool,
}

/// Moves the camera for `dt` seconds of held keys.
pub fn fly(camera: &mut Camera, keys: Keys, dt: f32) {
    let axis = |plus: bool, minus: bool| f32::from(u8::from(plus)) - f32::from(u8::from(minus));
    let (f, r, u) = (
        axis(keys.forward, keys.back),
        axis(keys.right, keys.left),
        axis(keys.up, keys.down),
    );
    let [forward, right, _] = camera.axes();
    let mut v = [0, 1, 2].map(|k| f * forward[k] + r * right[k]);
    v[1] += u;
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0.0 {
        return;
    }
    let step = SPEED * if keys.fast { FAST } else { 1.0 } * dt / len;
    for (o, v) in camera.origin.iter_mut().zip(v) {
        *o += v * step;
    }
}

/// Turns the camera by a mouse movement in counts (x right, y down): right turns
/// right, down looks down; pitch stops short of straight up and down.
pub fn look(camera: &mut Camera, dx: f64, dy: f64) {
    camera.yaw = (camera.yaw + dx as f32 * DEGREES_PER_COUNT).rem_euclid(360.0);
    camera.pitch = (camera.pitch - dy as f32 * DEGREES_PER_COUNT).clamp(-89.0, 89.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rounded(v: [f32; 3]) -> [f32; 3] {
        v.map(|x| (x * 100.0).round() / 100.0)
    }

    fn held(f: impl FnOnce(&mut Keys)) -> Keys {
        let mut k = Keys::default();
        f(&mut k);
        k
    }

    #[test]
    fn moves_along_the_view() {
        // Yaw 90 faces +x; one second forward is SPEED metres.
        let mut c = Camera::new([0.0; 3], 90.0, 0.0, 1280, 720);
        fly(&mut c, held(|k| k.forward = true), 1.0);
        assert_eq!(rounded(c.origin), [25.0, 0.0, 0.0]);
        // Strafing right from facing +z goes +x.
        let mut c = Camera::new([0.0; 3], 0.0, 0.0, 1280, 720);
        fly(&mut c, held(|k| k.right = true), 1.0);
        assert_eq!(rounded(c.origin), [25.0, 0.0, 0.0]);
        // Looking straight down, forward goes down.
        let mut c = Camera::new([0.0; 3], 0.0, -89.0, 1280, 720);
        fly(&mut c, held(|k| k.forward = true), 0.5);
        assert!(c.origin[1] < -12.4);
    }

    #[test]
    fn diagonals_and_running_keep_the_speed() {
        let mut c = Camera::new([0.0; 3], 0.0, 0.0, 1280, 720);
        fly(
            &mut c,
            held(|k| {
                k.forward = true;
                k.up = true;
                k.fast = true;
            }),
            1.0,
        );
        let d = 150.0 / 2f32.sqrt();
        assert_eq!(rounded(c.origin), rounded([0.0, d, d]));
        // Opposite keys cancel.
        let mut c = Camera::new([0.0; 3], 0.0, 0.0, 1280, 720);
        fly(
            &mut c,
            held(|k| {
                k.forward = true;
                k.back = true;
            }),
            1.0,
        );
        assert_eq!(c.origin, [0.0; 3]);
    }

    #[test]
    fn mouse_turns_and_stops_short_of_vertical() {
        let mut c = Camera::new([0.0; 3], 10.0, 0.0, 1280, 720);
        look(&mut c, -200.0, 100.0);
        assert_eq!((c.yaw.round(), c.pitch.round()), (350.0, -10.0));
        look(&mut c, 0.0, -10_000.0);
        assert_eq!(c.pitch, 89.0);
        look(&mut c, 0.0, 20_000.0);
        assert_eq!(c.pitch, -89.0);
    }
}

//! The view: a position and two angles in Frostbite's world space, mapped to Vulkan
//! clip space with reversed, infinite depth.
//!
//! World space is Frostbite's: y up, and at yaw 0 the camera looks along +z with +x
//! on its right (left-handed, like `Transform`'s right, up, forward).

pub type Vec3 = [f32; 3];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub origin: Vec3,
    /// Turning right, in degrees, from +z towards +x.
    pub yaw: f32,
    /// Looking up, in degrees, between -89 and 89.
    pub pitch: f32,
    /// Vertical field of view in degrees; the horizontal one follows from the aspect.
    pub fov_y: f32,
    pub width: u32,
    pub height: u32,
    /// Metres: closer than this is cut off.
    pub z_near: f32,
}

pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Camera {
    pub fn new(origin: Vec3, yaw: f32, pitch: f32, width: u32, height: u32) -> Self {
        Self {
            origin,
            yaw,
            pitch,
            fov_y: 55.0,
            width,
            height,
            z_near: 0.1,
        }
    }

    /// A camera at `origin` looking at `target`.
    pub fn looking_at(origin: Vec3, target: Vec3, width: u32, height: u32) -> Self {
        let v = [0, 1, 2].map(|i| target[i] - origin[i]);
        let flat = v[0].hypot(v[2]);
        let yaw = v[0].atan2(v[2]).to_degrees();
        let pitch = v[1].atan2(flat).to_degrees().clamp(-89.0, 89.0);
        Self::new(origin, yaw, pitch, width, height)
    }

    /// Forward, right, up: unit vectors.
    pub fn axes(&self) -> [Vec3; 3] {
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let forward = [cp * sy, sp, cp * cy];
        let right = [cy, 0.0, -sy];
        // forward × right, which is up in a left-handed space.
        let up = [
            forward[1] * right[2] - forward[2] * right[1],
            forward[2] * right[0] - forward[0] * right[2],
            forward[0] * right[1] - forward[1] * right[0],
        ];
        [forward, right, up]
    }

    /// The view-projection matrix, column-major (as WGSL `mat4x4` takes it): clip x
    /// right, y down, depth `z_near / distance` (1 at the near plane, 0 at infinity).
    pub fn view_proj(&self) -> [[f32; 4]; 4] {
        let [f, r, u] = self.axes();
        let ty = (self.fov_y.to_radians() / 2.0).tan();
        let tx = ty * self.width.max(1) as f32 / self.height.max(1) as f32;
        let o = self.origin;
        // Rows of the matrix acting on (x, y, z, 1).
        let rows = [
            [r[0] / tx, r[1] / tx, r[2] / tx, -dot(r, o) / tx],
            [-u[0] / ty, -u[1] / ty, -u[2] / ty, dot(u, o) / ty],
            [0.0, 0.0, 0.0, self.z_near],
            [f[0], f[1], f[2], -dot(f, o)],
        ];
        [0, 1, 2, 3].map(|c| [rows[0][c], rows[1][c], rows[2][c], rows[3][c]])
    }

    /// Where a world point lands: (x, y) in pixels from the top-left, and depth.
    pub fn project(&self, p: Vec3) -> Option<(f32, f32, f32)> {
        let m = self.view_proj();
        let clip = [0, 1, 2, 3]
            .map(|row| m[0][row] * p[0] + m[1][row] * p[1] + m[2][row] * p[2] + m[3][row]);
        if clip[3] <= 0.0 {
            return None;
        }
        let (x, y) = (clip[0] / clip[3], clip[1] / clip[3]);
        Some((
            (x + 1.0) * 0.5 * self.width as f32,
            (y + 1.0) * 0.5 * self.height as f32,
            clip[2] / clip[3],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn axes_follow_frostbite() {
        let [f, r, u] = Camera::new([0.0; 3], 0.0, 0.0, 640, 480).axes();
        assert_eq!((f, r), ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]));
        assert!(close(u[1], 1.0));
        // Yaw 90 turns right, to +x; its right is then -z.
        let [f, r, _] = Camera::new([0.0; 3], 90.0, 0.0, 640, 480).axes();
        assert!(close(f[0], 1.0) && close(f[2], 0.0) && close(r[2], -1.0));
        // Positive pitch looks up.
        let [f, _, u] = Camera::new([0.0; 3], 0.0, 30.0, 640, 480).axes();
        assert!(close(f[1], 0.5) && u[2] < 0.0);
    }

    #[test]
    fn projection_puts_points_on_screen() {
        let c = Camera::new([0.0; 3], 0.0, 0.0, 1280, 720);
        // Straight ahead: the centre, depth near / distance.
        let (x, y, z) = c.project([0.0, 0.0, 10.0]).unwrap();
        assert!(close(x, 640.0) && close(y, 360.0) && close(z, 0.01));
        // At the top edge of the vertical field of view: y = 0; right is +x.
        let t = (27.5f32).to_radians().tan();
        let (x, y, _) = c.project([0.0, 10.0 * t, 10.0]).unwrap();
        assert!(close(x, 640.0) && close(y, 0.0));
        let (x, _, _) = c.project([10.0 * t * 1280.0 / 720.0, 0.0, 10.0]).unwrap();
        assert!(close(x, 1280.0));
        // Behind the camera: not on screen.
        assert!(c.project([0.0, 0.0, -1.0]).is_none());
    }

    #[test]
    fn looking_at_a_point_centres_it() {
        let target = [30.0, -5.0, -40.0];
        let c = Camera::looking_at([10.0, 20.0, 5.0], target, 800, 600);
        let (x, y, _) = c.project(target).unwrap();
        assert!(close(x, 400.0) && close(y, 300.0), "{x} {y}");
    }
}

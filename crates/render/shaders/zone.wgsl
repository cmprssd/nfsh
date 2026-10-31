// A zone's meshes, untextured: each placement's transform from the instance, the
// normal from the triangle itself (its screen-space derivatives), lit by a fixed sun
// and a sky above, the ground below.

struct Push {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
}

var<immediate> push: Push;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

@vertex
fn vs_main(
    @location(0) p: vec3<f32>,
    @location(1) right: vec4<f32>,
    @location(2) up: vec4<f32>,
    @location(3) forward: vec4<f32>,
) -> Out {
    let world = p.x * right.xyz + p.y * up.xyz + p.z * forward.xyz
        + vec3(right.w, up.w, forward.w);
    var out: Out;
    out.clip = push.view_proj * vec4(world, 1.0);
    out.world = world;
    return out;
}

const SUN: vec3<f32> = vec3(0.37, 0.84, 0.40);

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    var n = normalize(cross(dpdx(in.world), dpdy(in.world)));
    // Transforms may mirror and triangles are drawn from both sides: face the eye.
    let to_eye = push.eye.xyz - in.world;
    if dot(n, to_eye) < 0.0 {
        n = -n;
    }
    let sun = max(dot(n, normalize(SUN)), 0.0);
    let sky = mix(vec3(0.20, 0.18, 0.16), vec3(0.32, 0.38, 0.46), n.y * 0.5 + 0.5);
    var colour = vec3(0.5) * (sky + vec3(1.0, 0.95, 0.85) * sun * 0.75);
    // Distance haze, so depth reads at a glance.
    let haze = 1.0 - exp(-length(to_eye) / 5000.0);
    colour = mix(colour, vec3(0.62, 0.70, 0.80), haze);
    return vec4(colour, 1.0);
}

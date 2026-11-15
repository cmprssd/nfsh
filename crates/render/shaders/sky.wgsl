// The sky behind everything (sky.rs): one triangle over the screen at infinite depth,
// drawn where nothing else was. Above the horizon, a photographed panorama of the
// upper half of the sky, laid over a gradient of the whole sky where its mask lets it;
// below, the gradient alone. The textures' values times LuminanceScale are cd/m²; the
// sun's disc on top. Exposed and tonemapped as zone.wgsl does.

struct Push {
    // The camera's forward, and its right and up times the tangents of half the
    // view's width and height.
    forward: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // Towards the sun; w: the cosine of the disc's angular radius.
    sun: vec4<f32>,
    // The disc's light, exposed (0: none).
    disc: vec4<f32>,
    // The part of the panorama laid over the dome: u and v at its left and zenith
    // (xy), at its right and horizon (zw).
    uv: vec4<f32>,
    // x: the textures' values to exposed light (LuminanceScale × exposure);
    // y: the bearing of the panorama's left edge, in turns.
    scale: vec4<f32>,
}

var<immediate> push: Push;

@group(0) @binding(0) var panorama: texture_2d<f32>;
@group(0) @binding(1) var gradient: texture_2d<f32>;
@group(0) @binding(2) var mask: texture_2d<f32>;
// Repeats across (around the horizon), clamps up and down.
@group(0) @binding(3) var around: sampler;

const PI: f32 = 3.14159265;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> Out {
    // (-1, -1), (3, -1), (-1, 3): covers the screen.
    let xy = vec2(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: Out;
    // Depth 0 is infinitely far (reversed depth).
    out.position = vec4(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

// A filmic curve (Narkowicz's fit of ACES), as zone.wgsl's.
fn filmic(x: vec3<f32>) -> vec3<f32> {
    return clamp(x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14), vec3(0.0), vec3(1.0));
}

// A change in a coordinate that wraps at 1, across the wrap.
fn wrapped(d: f32) -> f32 {
    return d - round(d);
}

@fragment
fn fs_sky(in: Out) -> @location(0) vec4<f32> {
    // Clip y points down the screen.
    let dir = normalize(push.forward.xyz + in.ndc.x * push.right.xyz - in.ndc.y * push.up.xyz);
    // The bearing turns from +z towards +x, as the sun's does (lighting.rs).
    let bearing = atan2(dir.x, dir.z) / (2.0 * PI);
    let elevation = asin(clamp(dir.y, -1.0, 1.0));
    let u = fract(bearing - push.scale.y);
    // The panorama runs from the zenith (v at its top) down to the horizon; the
    // gradient from the zenith to the nadir.
    let pano = vec2(mix(push.uv.x, push.uv.z, u), mix(push.uv.y, push.uv.w, 1.0 - elevation / (0.5 * PI)));
    let grad = vec2(u, 0.5 - elevation / PI);
    // Explicit gradients, so the seam where u wraps doesn't pick the smallest mip.
    let du = vec2(wrapped(dpdx(u)), wrapped(dpdy(u)));
    let dpano_v = vec2(dpdx(pano.y), dpdy(pano.y));
    let dgrad_v = vec2(dpdx(grad.y), dpdy(grad.y));
    let span = push.uv.z - push.uv.x;
    let pano_dx = vec2(du.x * span, dpano_v.x);
    let pano_dy = vec2(du.y * span, dpano_v.y);
    var sky = textureSampleGrad(gradient, around, grad, vec2(du.x, dgrad_v.x), vec2(du.y, dgrad_v.y)).rgb;
    let photo = textureSampleGrad(panorama, around, pano, pano_dx, pano_dy).rgb;
    let shows = textureSampleGrad(mask, around, pano, pano_dx, pano_dy).r;
    sky = select(sky, mix(sky, photo, shows), elevation >= 0.0);
    var light = sky * push.scale.x;
    if dot(dir, push.sun.xyz) > push.sun.w {
        light += push.disc.rgb;
    }
    return vec4(filmic(light), 1.0);
}

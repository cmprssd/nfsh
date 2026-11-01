// A zone's meshes: each placement's transform from the instance, the colour from the
// section's texture as its look says, the normal from the triangle itself (its
// screen-space derivatives), lit by the time of day's sun and sky, exposed, then
// tonemapped. Decals are blended over what is drawn under them.

struct Push {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // Towards the sun.
    sun: vec4<f32>,
    // The sun's light on a white surface facing it, exposed; w: the ground's.
    sun_light: vec4<f32>,
    // The sky's light from straight above, exposed.
    sky: vec4<f32>,
}

var<immediate> push: Push;

// How a draw's colour comes from its texture (zone.rs, Look).
struct Look {
    // Multiplies the colour; w = 1: the texture's red as grey (sRGB-encoded).
    tint: vec4<f32>,
    // A decal's opacity: the texture's channels weighted by this, times opacity.x.
    select: vec4<f32>,
    opacity: vec4<f32>,
    // A decal's colour when w = 1, else the texture's red (sRGB-encoded) times the
    // vertex colour.
    colour: vec4<f32>,
}

@group(0) @binding(0) var colour: texture_2d<f32>;
@group(0) @binding(1) var filtered: sampler;
@group(1) @binding(0) var<uniform> look: Look;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) vertex_colour: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) p: vec3<f32>,
    @location(1) right: vec4<f32>,
    @location(2) up: vec4<f32>,
    @location(3) forward: vec4<f32>,
    @location(4) uv: vec2<f32>,
    @location(5) vertex_colour: vec4<f32>,
) -> Out {
    let world = p.x * right.xyz + p.y * up.xyz + p.z * forward.xyz
        + vec3(right.w, up.w, forward.w);
    var out: Out;
    out.clip = push.view_proj * vec4(world, 1.0);
    out.world = world;
    out.uv = uv;
    out.vertex_colour = vertex_colour;
    return out;
}

// A filmic curve (Narkowicz's fit of ACES), for the tonemap's FilmicNeutral.
fn filmic(x: vec3<f32>) -> vec3<f32> {
    return clamp(x * (2.51 * x + 0.03) / (x * (2.43 * x + 0.59) + 0.14), vec3(0.0), vec3(1.0));
}

fn shade(in: Out, albedo: vec3<f32>, alpha: f32) -> vec4<f32> {
    var n = normalize(cross(dpdx(in.world), dpdy(in.world)));
    // Transforms may mirror and triangles are drawn from both sides: face the eye.
    let to_eye = push.eye.xyz - in.world;
    if dot(n, to_eye) < 0.0 {
        n = -n;
    }
    let sun = max(dot(n, push.sun.xyz), 0.0);
    let ambient = mix(vec3(push.sun_light.w), push.sky.rgb, n.y * 0.5 + 0.5);
    var light = albedo * (ambient + push.sun_light.rgb * sun);
    // Distance haze: the light falling on the air, so depth reads at a glance.
    let air = (push.sky.rgb + push.sun_light.rgb * max(push.sun.y, 0.0)) * 0.5;
    let haze = 1.0 - exp(-length(to_eye) / 5000.0);
    light = mix(light, air, haze);
    return vec4(filmic(light), alpha);
}

// An sRGB-encoded value, linear: the road textures pack a grey into the red channel
// of a unorm texture, which the sampler doesn't decode.
fn linear(x: f32) -> f32 {
    return select(pow((x + 0.055) / 1.055, 2.4), x / 12.92, x <= 0.04045);
}

fn albedo(c: vec4<f32>) -> vec3<f32> {
    return select(c.rgb, vec3(linear(c.r)), look.tint.w > 0.5) * look.tint.rgb;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4<f32> {
    return shade(in, albedo(textureSample(colour, filtered, in.uv)), 1.0);
}

// Leaves and the like: what the texture's alpha leaves out is not drawn.
@fragment
fn fs_cutout(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(colour, filtered, in.uv);
    if c.a < 0.5 {
        discard;
    }
    return shade(in, albedo(c), 1.0);
}

// Road lines, cracks and tyre marks over the asphalt.
@fragment
fn fs_decal(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(colour, filtered, in.uv);
    let alpha = saturate(dot(c, look.select)) * look.opacity.x * in.vertex_colour.r;
    let paint = select(linear(c.r) * in.vertex_colour.rgb, look.colour.rgb, look.colour.w > 0.5);
    return shade(in, paint, alpha);
}

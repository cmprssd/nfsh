//! A shader program's bytecode: a Direct3D shader container (`DXBC`) with its
//! instructions (shader model 5), read only as far as finding the scale at which a
//! terrain shader samples a texture (see docs/FORMATS.md, "Shader programs").
//! Little-endian.

use crate::reader::Reader;
use crate::{Error, Result, bail};

/// The res's header before the container: two words, the container's size and its
/// offset.
const HEADER: usize = 32;
const MAGIC: &[u8; 4] = b"DXBC";
/// The container's chunk count, then each chunk's offset.
const CHUNK_COUNT_AT: usize = 28;

/// The opcodes nfsh traces; every other one is only stepped over.
const ADD: u32 = 0x00;
const DP2: u32 = 0x0F;
const DP3: u32 = 0x10;
const DP4: u32 = 0x11;
const LT: u32 = 0x31;
const MAD: u32 = 0x32;
const MOVC: u32 = 0x37;
const MUL: u32 = 0x38;
/// `sample` and its variants (`_c`, `_c_lz`, `_l`, `_d`, `_b`).
const SAMPLES: std::ops::RangeInclusive<u32> = 0x45..=0x4A;
const CUSTOM_DATA: u32 = 0x35;
/// Declarations, whose tokens are not all operands.
const DECLARATIONS: [std::ops::RangeInclusive<u32>; 2] = [0x58..=0x6A, 0x85..=0x9C];
/// Instructions whose first operand is read, not written (`breakc`, `callc`, `case`,
/// `continuec`, `discard`, `if`, `retc`, `switch`).
const NO_RESULT: [u32; 8] = [0x03, 0x05, 0x06, 0x08, 0x0D, 0x1F, 0x3F, 0x4C];
/// Instructions writing two registers (`sincos`, `udiv`, `imul`, `umul`, `uaddc`,
/// `usubb`, `swapc`).
const TWO_RESULTS: [u32; 7] = [0x4D, 0x4E, 0x26, 0x51, 0x7A, 0x7B, 0x84];

/// What an operand names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A temporary register, `r<n>`.
    Temp,
    /// A shader input, `v<n>`.
    Input,
    /// Literal values, one per component (or one for all).
    Literal([f32; 4]),
    /// A texture, `t<n>`.
    Texture,
    /// Anything else (outputs, constants, samplers, indexed registers).
    Other,
}

/// An instruction's operand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Operand {
    pub kind: Kind,
    /// Its register number; 0 when it has none or a relative one.
    pub register: u32,
    /// For a destination, the components written (x = 0 … w = 3), in order; for a
    /// source, the component read for each of the destination's x, y, z and w.
    pub components: [u8; 4],
    /// For a destination, how many of `components` are written.
    pub written: usize,
}

impl Operand {
    /// The value read for a destination's component `c`, when it is a literal.
    fn literal(&self, c: u8) -> Option<f32> {
        match self.kind {
            Kind::Literal(v) => Some(v[c as usize]),
            _ => None,
        }
    }

    fn writes(&self, register: u32, c: u8) -> bool {
        self.kind == Kind::Temp
            && self.register == register
            && self.components[..self.written].contains(&c)
    }
}

/// One instruction: its opcode and operands (the destinations first).
#[derive(Debug, Clone, PartialEq)]
pub struct Instruction {
    pub opcode: u32,
    pub operands: Vec<Operand>,
}

impl Instruction {
    /// Whether it writes component `c` of temporary `register`.
    fn writes(&self, register: u32, c: u8) -> bool {
        let results = match self.opcode {
            o if NO_RESULT.contains(&o) => 0,
            o if TWO_RESULTS.contains(&o) => 2,
            _ => 1,
        };
        self.operands
            .iter()
            .take(results)
            .any(|o| o.writes(register, c))
    }
}

/// Reads a bytecode res's instructions, leaving out the declarations.
pub fn parse(data: &[u8]) -> Result<Vec<Instruction>> {
    let container = data
        .get(HEADER..)
        .filter(|c| c.starts_with(MAGIC))
        .ok_or_else(|| Error("no shader container".into()))?;
    let count = Reader::at(container, CHUNK_COUNT_AT)?.u32_le()?;
    let mut offsets = Reader::at(container, CHUNK_COUNT_AT + 4)?;
    for _ in 0..count {
        let at = offsets.u32_le()? as usize;
        let mut chunk = Reader::at(container, at)?;
        let tag = chunk.array::<4>()?;
        let size = chunk.u32_le()? as usize;
        if &tag == b"SHEX" || &tag == b"SHDR" {
            let words = chunk
                .bytes(size)?
                .as_chunks::<4>()
                .0
                .iter()
                .map(|w| u32::from_le_bytes(*w))
                .collect::<Vec<_>>();
            return instructions(&words);
        }
    }
    bail!("no shader code")
}

/// Decodes the instructions after the version and length words.
fn instructions(words: &[u32]) -> Result<Vec<Instruction>> {
    let mut code = Vec::new();
    let mut at = 2;
    while at < words.len() {
        let token = words[at];
        let opcode = token & 0x7FF;
        let length = if opcode == CUSTOM_DATA {
            *words
                .get(at + 1)
                .ok_or_else(|| Error("custom data cut short".into()))? as usize
        } else {
            (token >> 24 & 0x7F) as usize
        };
        let end = at + length;
        if length == 0 || end > words.len() {
            bail!("instruction at word {at}: length {length}");
        }
        if opcode != CUSTOM_DATA && !DECLARATIONS.iter().any(|d| d.contains(&opcode)) {
            let mut w = at + 1;
            let mut extended = token >> 31 != 0;
            while extended {
                extended = words.get(w).is_some_and(|t| t >> 31 != 0);
                w += 1;
            }
            let mut operands = Vec::new();
            while w < end {
                operands.push(operand(&words[..end], &mut w)?);
            }
            code.push(Instruction { opcode, operands });
        }
        at = end;
    }
    Ok(code)
}

/// Decodes the operand at `words[*at]`, moving past it (and any operands its indices
/// hold).
fn operand(words: &[u32], at: &mut usize) -> Result<Operand> {
    let token = word(words, at)?;
    if token >> 31 != 0 {
        word(words, at)?;
    }
    let count = token & 3;
    let selection = token >> 2 & 3;
    let select = token >> 4 & 0xFF;
    let kind = token >> 12 & 0xFF;
    let dimension = token >> 20 & 3;
    let (components, written) = match (count, selection) {
        (2, 0) => {
            let mut c = [0; 4];
            let mut n = 0;
            for i in 0..4u8 {
                if select & 1 << i != 0 {
                    c[n] = i;
                    n += 1;
                }
            }
            (c, n)
        }
        (2, 1) => ([0, 1, 2, 3].map(|i| (select >> (2 * i) & 3) as u8), 4),
        (2, _) => ([(select & 3) as u8; 4], 1),
        (1, _) => ([0; 4], 1),
        _ => ([0; 4], 0),
    };
    if kind == 4 {
        let mut v = [0.0; 4];
        match count {
            1 => v = [f32::from_bits(word(words, at)?); 4],
            2 => {
                for x in &mut v {
                    *x = f32::from_bits(word(words, at)?);
                }
            }
            _ => {}
        }
        return Ok(Operand {
            kind: Kind::Literal(v),
            register: 0,
            components: [0, 1, 2, 3],
            written: 4,
        });
    }
    if kind == 5 {
        bail!("a 64-bit literal");
    }
    let mut register = 0;
    for d in 0..dimension {
        let index = match token >> (22 + 3 * d) & 7 {
            0 => word(words, at)?,
            1 => {
                word(words, at)?;
                word(words, at)?
            }
            2 => {
                operand(words, at)?;
                0
            }
            3 => {
                word(words, at)?;
                operand(words, at)?;
                0
            }
            r => bail!("index representation {r}"),
        };
        if d == 0 {
            register = index;
        }
    }
    let kind = match (kind, dimension) {
        (0, 1) => Kind::Temp,
        (1, 1) => Kind::Input,
        (7, 1) => Kind::Texture,
        _ => Kind::Other,
    };
    Ok(Operand {
        kind,
        register,
        components,
        written,
    })
}

/// The word at `words[*at]`, moving past it.
fn word(words: &[u32], at: &mut usize) -> Result<u32> {
    let w = *words
        .get(*at)
        .ok_or_else(|| Error(format!("operand cut short at word {}", *at)))?;
    *at += 1;
    Ok(w)
}

/// How a shader reads a texture's coordinates from the world position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scale {
    /// The world position times this, the same on both axes.
    Direct(f32),
    /// The world position turned and scaled: this is the scale.
    Rotated(f32),
}

/// The scale of each sample of texture `slot` whose coordinates come from the world
/// position (an input mapped by a `mad`), in instruction order. A sample whose
/// coordinates come from elsewhere is left out.
pub fn sample_scales(code: &[Instruction], slot: u32) -> Vec<Scale> {
    (0..code.len())
        .filter(|&at| sampled_slot(&code[at]) == Some(slot))
        .filter_map(|at| sample_scale(code, at))
        .collect()
}

/// The texture slot instruction `i` samples, if it is a sample.
fn sampled_slot(i: &Instruction) -> Option<u32> {
    SAMPLES.contains(&i.opcode).then_some(())?;
    i.operands
        .get(2)
        .filter(|t| t.kind == Kind::Texture)
        .map(|t| t.register)
}

/// The scale of the sample at `at` when its coordinates come from the world position.
fn sample_scale(code: &[Instruction], at: usize) -> Option<Scale> {
    let uv = code[at].operands.get(1).filter(|o| o.kind == Kind::Temp)?;
    let [u, v] = [0, 1].map(|c| axis_scale(code, at, uv.register, uv.components[c]));
    match (u, v) {
        (Some(Scale::Direct(a)), Some(Scale::Direct(b))) if a == b => Some(Scale::Direct(a)),
        (Some(Scale::Rotated(s)), _) | (_, Some(Scale::Rotated(s))) => Some(Scale::Rotated(s)),
        _ => None,
    }
}

/// One texture's sample overlaid on a value, then mixed in:
/// `base + strength × (overlay(base, blend) − base)`, where `overlay(b, x)` is `2bx`
/// below a half and `1 − 2(1 − b)(1 − x)` from it.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    /// The texture slots the base is computed from.
    pub base: std::collections::BTreeSet<u32>,
    /// The blend's texture slot…
    pub slot: u32,
    /// …the scale it is sampled at…
    pub scale: Option<Scale>,
    /// …and the channel of it each written component takes, in order.
    pub channels: Vec<u8>,
    pub strength: f32,
}

/// Every overlay of a sampled texture onto a value in `code`, in instruction order:
/// a `movc` choosing between its two halves by `lt base, 0.5`, the first the `mul`
/// of the base with a sample (doubled by an `add`); mixed in by a `mad` with a
/// literal (`base + k × (movc − base)`), or taken whole (strength 1).
pub fn overlays(code: &[Instruction]) -> Vec<Overlay> {
    let mixed: Vec<(usize, Overlay)> = (0..code.len())
        .filter(|&at| code[at].opcode == MAD)
        .filter_map(|at| {
            let mut movc = None;
            let o = overlay(code, at, |c| {
                let [_, diff, k, b] = code[at].operands.get(..4)? else {
                    return None;
                };
                let strength = k.literal(c)?;
                let base = value(code, at, b, c)?;
                // diff = movc − base
                let d = value(code, at, diff, c)?;
                let add = &code[d.0];
                (add.opcode == ADD).then_some(())?;
                let sources: Vec<Value> = add.operands[1..]
                    .iter()
                    .map(|o| value(code, d.0, o, d.2))
                    .collect::<Option<_>>()?;
                sources.contains(&base).then_some(())?;
                let m = *sources.iter().find(|v| **v != base)?;
                let (core, b) = halves(code, m)?;
                (b == base).then_some(())?;
                movc = Some(m.0);
                Some((core, strength))
            })?;
            Some((movc?, o))
        })
        .collect();
    let taken: Vec<usize> = mixed.iter().map(|(m, _)| *m).collect();
    let mut all: Vec<(usize, Overlay)> = (0..code.len())
        .filter(|&at| code[at].opcode == MOVC && !taken.contains(&at))
        .filter_map(|at| {
            let dest = code[at].operands.first()?;
            let o = overlay(code, at, |c| {
                let m = (at, dest.register, c);
                Some((halves(code, m)?.0, 1.0))
            })?;
            Some((at, o))
        })
        .chain(mixed)
        .collect();
    all.sort_by_key(|(at, _)| *at);
    all.into_iter().map(|(_, o)| o).collect()
}

/// A value: the instruction defining it, the register and the component.
type Value = (usize, u32, u8);

/// An overlay's blend: its slot, scale and channel, and its base.
type Core = (u32, Option<Scale>, u8, Value);

/// The overlay the instruction at `at` ends, from `each` destination component's core
/// and strength; none unless every component has one, all of one slot and strength.
fn overlay(
    code: &[Instruction],
    at: usize,
    mut each: impl FnMut(u8) -> Option<(Core, f32)>,
) -> Option<Overlay> {
    let dest = code[at].operands.first()?;
    let mut channels = Vec::new();
    let mut found: Option<(u32, Option<Scale>, f32)> = None;
    let mut base = std::collections::BTreeSet::new();
    for &c in &dest.components[..dest.written] {
        let ((slot, scale, channel, from), strength) = each(c)?;
        if found.is_some_and(|f| f.0 != slot || f.2 != strength) {
            return None;
        }
        found = Some((slot, scale, strength));
        channels.push(channel);
        base.extend(slots(code, from.0 + 1, from.1, from.2));
    }
    let (slot, scale, strength) = found?;
    Some(Overlay {
        base,
        slot,
        scale,
        channels,
        strength,
    })
}

/// The value source operand `o` of the instruction at `at` reads for its
/// destination's component `c`.
fn value(code: &[Instruction], at: usize, o: &Operand, c: u8) -> Option<Value> {
    (o.kind == Kind::Temp).then_some(())?;
    let k = o.components[c as usize];
    Some((definition(code, at, o.register, k)?, o.register, k))
}

/// The `movc` value `m` as an overlay's two halves: its blend's core and its base.
fn halves(code: &[Instruction], m: Value) -> Option<(Core, Value)> {
    let movc = &code[m.0];
    (movc.opcode == MOVC).then_some(())?;
    let cond = value(code, m.0, movc.operands.get(1)?, m.2)?;
    let lt = &code[cond.0];
    (lt.opcode == LT && lt.operands.get(2)?.literal(cond.2) == Some(0.5)).then_some(())?;
    let base = value(code, cond.0, lt.operands.get(1)?, cond.2)?;
    // twice the product: add p, p
    let doubled = value(code, m.0, movc.operands.get(2)?, m.2)?;
    let twice = &code[doubled.0];
    (twice.opcode == ADD).then_some(())?;
    let product = value(code, doubled.0, twice.operands.get(1)?, doubled.2)?;
    (value(code, doubled.0, twice.operands.get(2)?, doubled.2)? == product).then_some(())?;
    let mul = &code[product.0];
    (mul.opcode == MUL).then_some(())?;
    let factors: Vec<Value> = mul.operands[1..]
        .iter()
        .map(|o| value(code, product.0, o, product.2))
        .collect::<Option<_>>()?;
    factors.contains(&base).then_some(())?;
    let blend = *factors.iter().find(|v| **v != base)?;
    let sample = &code[blend.0];
    let slot = sampled_slot(sample)?;
    // The resource's swizzle says which of its channels each component receives.
    let channel = sample.operands.get(2)?.components[blend.2 as usize];
    Some(((slot, sample_scale(code, blend.0), channel, base), base))
}

/// The texture slots whose samples component `c` of `register` at `at` is computed
/// from, through every instruction before it (ignoring branches).
fn slots(code: &[Instruction], at: usize, register: u32, c: u8) -> std::collections::BTreeSet<u32> {
    let mut out = std::collections::BTreeSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut todo: Vec<(usize, u32, u8)> = vec![(at, register, c)];
    while let Some((at, register, c)) = todo.pop() {
        let Some(d) = definition(code, at, register, c) else {
            continue;
        };
        if !seen.insert((d, c)) {
            continue;
        }
        let i = &code[d];
        if let Some(slot) = sampled_slot(i) {
            out.insert(slot);
            continue;
        }
        let read: &[u8] = match i.opcode {
            DP2 => &[0, 1],
            DP3 => &[0, 1, 2],
            DP4 => &[0, 1, 2, 3],
            _ => &[c],
        };
        for o in i.operands.iter().skip(1).filter(|o| o.kind == Kind::Temp) {
            for &k in read {
                todo.push((d, o.register, o.components[k as usize]));
            }
        }
    }
    out
}

/// The last instruction before `at` writing component `c` of temporary `register`.
fn definition(code: &[Instruction], at: usize, register: u32, c: u8) -> Option<usize> {
    code[..at].iter().rposition(|i| i.writes(register, c))
}

/// Whether component `c` of `register` at `at` is the world position: written from
/// an input.
fn is_world(code: &[Instruction], at: usize, register: u32, c: u8) -> bool {
    definition(code, at, register, c)
        .is_some_and(|d| code[d].operands[1..].iter().any(|o| o.kind == Kind::Input))
}

/// The scale component `c` of `register` at `at` has as a coordinate: the world
/// position times a literal (`mul`, or `mad` adding a literal), or a literal pair's
/// length in a `dp2` with it (a turned axis); a `mad` adding a register is the other
/// half of a turn, and says nothing on its own.
fn axis_scale(code: &[Instruction], at: usize, register: u32, c: u8) -> Option<Scale> {
    let d = definition(code, at, register, c)?;
    let i = &code[d];
    let sources = i.operands.get(1..)?;
    let world = |o: &Operand, k: u8| o.kind == Kind::Temp && is_world(code, d, o.register, k);
    match i.opcode {
        MUL | MAD => {
            if i.opcode == MAD && sources.get(2)?.literal(0).is_none() {
                return None;
            }
            let (a, b) = (sources.first()?, sources.get(1)?);
            let (r, l) = if a.literal(0).is_some() {
                (b, a)
            } else {
                (a, b)
            };
            let s = l.literal(c)?.abs();
            world(r, r.components[c as usize]).then_some(Scale::Direct(s))
        }
        DP2 => {
            let (a, b) = (sources.first()?, sources.get(1)?);
            let (r, l) = if a.literal(0).is_some() {
                (b, a)
            } else {
                (a, b)
            };
            let (x, y) = (l.literal(0)?, l.literal(1)?);
            (world(r, r.components[0]) && world(r, r.components[1]))
                .then(|| Scale::Rotated(x.hypot(y)))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes operand tokens for the shapes the tests use.
    pub(crate) mod op {
        /// `r<n>` written with `mask` (bits x = 1 … w = 8).
        pub fn dest(n: u32, mask: u32) -> Vec<u32> {
            vec![2 | mask << 4 | 1 << 20, n]
        }
        /// `<kind><n>` read with `swizzle` (four component numbers).
        fn read(kind: u32, n: u32, swizzle: [u32; 4]) -> Vec<u32> {
            let s = swizzle
                .iter()
                .enumerate()
                .map(|(i, c)| c << (2 * i))
                .sum::<u32>();
            vec![2 | 1 << 2 | s << 4 | kind << 12 | 1 << 20, n]
        }
        pub fn temp(n: u32, swizzle: [u32; 4]) -> Vec<u32> {
            read(0, n, swizzle)
        }
        pub fn input(n: u32, swizzle: [u32; 4]) -> Vec<u32> {
            read(1, n, swizzle)
        }
        pub fn texture(n: u32) -> Vec<u32> {
            read(7, n, [0, 1, 2, 3])
        }
        /// `t<n>` read with `swizzle`: the channel each component receives.
        pub fn texture_as(n: u32, swizzle: [u32; 4]) -> Vec<u32> {
            read(7, n, swizzle)
        }
        pub fn sampler(n: u32) -> Vec<u32> {
            vec![6 << 12 | 1 << 20, n]
        }
        /// `cb<n>[i]`, two-dimensional.
        pub fn constant(n: u32, i: u32, swizzle: [u32; 4]) -> Vec<u32> {
            let s = swizzle
                .iter()
                .enumerate()
                .map(|(i, c)| c << (2 * i))
                .sum::<u32>();
            vec![2 | 1 << 2 | s << 4 | 8 << 12 | 2 << 20, n, i]
        }
        pub fn literal(v: [f32; 4]) -> Vec<u32> {
            let mut w = vec![2 | 4 << 12];
            w.extend(v.map(f32::to_bits));
            w
        }
        pub fn scalar(v: f32) -> Vec<u32> {
            vec![1 | 4 << 12, v.to_bits()]
        }
    }

    /// An instruction of `opcode` with `operands`.
    pub(crate) fn ins(opcode: u32, operands: &[Vec<u32>]) -> Vec<u32> {
        let body: Vec<u32> = operands.concat();
        let mut w = vec![opcode | ((body.len() as u32 + 1) << 24)];
        w.extend(body);
        w
    }

    /// A bytecode res: the header, then a container with an input signature chunk and
    /// a code chunk holding `code` after a declaration.
    pub(crate) fn res(code: &[Vec<u32>]) -> Vec<u8> {
        let mut words = vec![0x50, 0];
        // dcl_temps 16
        words.extend([0x68 | 2 << 24, 16]);
        for i in code {
            words.extend(i);
        }
        words[1] = words.len() as u32;
        let shex: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let isgn = [b"ISGN".as_slice(), &8u32.to_le_bytes(), &[0; 8]].concat();
        let first = CHUNK_COUNT_AT + 12;
        let mut c = vec![0; first];
        c[..4].copy_from_slice(MAGIC);
        c[CHUNK_COUNT_AT..][..4].copy_from_slice(&2u32.to_le_bytes());
        c[CHUNK_COUNT_AT + 4..][..4].copy_from_slice(&(first as u32).to_le_bytes());
        let second = (first + isgn.len()) as u32;
        c[CHUNK_COUNT_AT + 8..][..4].copy_from_slice(&second.to_le_bytes());
        c.extend(isgn);
        c.extend(b"SHEX");
        c.extend((shex.len() as u32).to_le_bytes());
        c.extend(shex);
        let mut d = vec![0; HEADER];
        d.extend(c);
        d
    }

    /// The world position into r0.xy as (z, x), the way the terrain shaders start.
    pub(crate) fn world() -> Vec<u32> {
        ins(
            MAD,
            &[
                op::dest(0, 3),
                op::input(1, [1, 0, 1, 1]),
                op::constant(1, 2, [1, 0, 1, 1]),
                op::constant(1, 2, [3, 2, 3, 3]),
            ],
        )
    }

    /// Samples texture `t` at r<n>.xy.
    pub(crate) fn sample(n: u32, t: u32) -> Vec<u32> {
        ins(
            0x45,
            &[
                op::dest(15, 7),
                op::temp(n, [0, 1, 0, 0]),
                op::texture(t),
                op::sampler(3),
            ],
        )
    }

    #[test]
    fn finds_direct_and_turned_scales() {
        let code = [
            world(),
            // r9 = r0.yxyx * (0.02, 0.02, 0.2, 0.2): t5 at r9.xy, t3 at r9.zw
            ins(
                MUL,
                &[
                    op::dest(9, 15),
                    op::temp(0, [1, 0, 1, 0]),
                    op::literal([0.02, 0.02, 0.2, 0.2]),
                ],
            ),
            sample(9, 5),
            ins(
                0x45,
                &[
                    op::dest(12, 7),
                    op::temp(9, [2, 3, 2, 2]),
                    op::texture(3),
                    op::sampler(3),
                ],
            ),
            // A turn: r12.x = r0.y * 0.08 + r11.x, r12.y = dot(r0.xy, (0.08, 0.06)).
            ins(
                MUL,
                &[op::dest(11, 1), op::temp(0, [0; 4]), op::scalar(0.06)],
            ),
            ins(
                MAD,
                &[
                    op::dest(12, 1),
                    op::temp(0, [1; 4]),
                    op::scalar(0.08),
                    op::temp(11, [0; 4]),
                ],
            ),
            ins(
                DP2,
                &[
                    op::dest(12, 2),
                    op::temp(0, [0, 1, 0, 0]),
                    op::literal([0.08, 0.06, 0.0, 0.0]),
                ],
            ),
            sample(12, 8),
            // mad with a literal offset is still direct.
            ins(
                MAD,
                &[
                    op::dest(2, 3),
                    op::temp(0, [1, 0, 1, 1]),
                    op::literal([0.1, 0.1, 0.0, 0.0]),
                    op::literal([0.5, 0.5, 0.0, 0.0]),
                ],
            ),
            sample(2, 8),
            // A coordinate not from the world position: a texture's own result.
            sample(15, 8),
            // r0 overwritten by something else: its later samples are not world-based.
            ins(
                MUL,
                &[op::dest(0, 3), op::temp(15, [0, 1, 0, 0]), op::scalar(2.0)],
            ),
            ins(
                MUL,
                &[op::dest(4, 3), op::temp(0, [0, 1, 0, 0]), op::scalar(0.5)],
            ),
            sample(4, 8),
        ];
        let code = parse(&res(&code)).unwrap();
        assert_eq!(code.len(), 14, "the declaration is left out");
        assert_eq!(sample_scales(&code, 5), [Scale::Direct(0.02)]);
        assert_eq!(sample_scales(&code, 3), [Scale::Direct(0.2)]);
        let s8 = sample_scales(&code, 8);
        assert_eq!(s8.len(), 2);
        match s8[0] {
            Scale::Rotated(s) => assert!((s - 0.1).abs() < 1e-6, "{s}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s8[1], Scale::Direct(0.1));
        assert!(sample_scales(&code, 6).is_empty());
    }

    #[test]
    fn finds_overlays_mixed_in_and_whole() {
        let xyz = [0, 1, 2, 2];
        let at = |op: u32, d: u32, sources: &[Vec<u32>]| {
            let mut o = vec![op::dest(d, 7)];
            o.extend_from_slice(sources);
            ins(op, &o)
        };
        let sample = |d: u32, mask: u32, uv: [u32; 4], t: Vec<u32>| {
            ins(
                0x45,
                &[op::dest(d, mask), op::temp(9, uv), t, op::sampler(3)],
            )
        };
        // The overlay of r1 with r2's `channel` (from a sample at r2's `components`):
        // lt, the doubled product, a screen half, the movc into r`out`.
        let overlay = |out: u32, r2: [u32; 4]| {
            vec![
                at(LT, 3, &[op::temp(1, xyz), op::literal([0.5; 4])]),
                at(MUL, 4, &[op::temp(2, r2), op::temp(1, xyz)]),
                at(ADD, 4, &[op::temp(4, xyz), op::temp(4, xyz)]),
                at(ADD, 5, &[op::temp(1, xyz), op::temp(2, r2)]),
                at(
                    MOVC,
                    out,
                    &[op::temp(3, xyz), op::temp(4, xyz), op::temp(5, xyz)],
                ),
            ]
        };
        let mut code = vec![
            world(),
            // r9 = r0.yxyx * (0.02, 0.02, 0.2, 0.2)
            ins(
                MUL,
                &[
                    op::dest(9, 15),
                    op::temp(0, [1, 0, 1, 0]),
                    op::literal([0.02, 0.02, 0.2, 0.2]),
                ],
            ),
            // The base, t3 at 0.2; the blend, t5 at 0.02.
            sample(1, 7, [2, 3, 2, 2], op::texture(3)),
            sample(2, 7, [0, 1, 0, 0], op::texture(5)),
        ];
        // Mixed in by 0.9: r8 = r1 + 0.9 (r6 − r1), with r2's blue.
        code.extend(overlay(6, [2; 4]));
        code.push(at(ADD, 7, &[op::temp(1, xyz), op::temp(6, xyz)]));
        code.push(at(
            MAD,
            8,
            &[op::temp(7, xyz), op::literal([0.9; 4]), op::temp(1, xyz)],
        ));
        // Taken whole, with r2.z from a sample swizzled `.zwyx`: the texture's y.
        code.push(sample(2, 12, [0, 1, 0, 0], op::texture_as(5, [2, 3, 1, 0])));
        code.extend(overlay(10, [2; 4]));
        let code = parse(&res(&code)).unwrap();
        let o = overlays(&code);
        let base = std::collections::BTreeSet::from([3]);
        assert_eq!(
            o,
            [
                Overlay {
                    base: base.clone(),
                    slot: 5,
                    scale: Some(Scale::Direct(0.02)),
                    channels: vec![2; 3],
                    strength: 0.9,
                },
                Overlay {
                    base,
                    slot: 5,
                    scale: Some(Scale::Direct(0.02)),
                    channels: vec![1; 3],
                    strength: 1.0,
                },
            ]
        );
    }

    #[test]
    fn damaged_bytecode_is_an_error() {
        let d = res(&[world(), sample(0, 1)]);
        assert!(parse(&d).is_ok());
        assert!(parse(&d[..d.len() - 4]).is_err(), "cut short");
        assert!(parse(&d[HEADER..]).is_err(), "no header");
        let mut long = d.clone();
        let last = long.len() - 4 * 9;
        long[last + 3] = 0x7F;
        assert!(parse(&long).is_err(), "an instruction past the end");
        let mut none = d;
        none[HEADER + CHUNK_COUNT_AT] = 1;
        assert!(parse(&none).is_err(), "no code chunk");
    }
}

//! A shader program's bytecode: a Direct3D shader container (`DXBC`) with its
//! instructions (shader model 5), read only as far as finding the scale at which a
//! terrain shader samples a texture, and which textures' channels and inputs an
//! output is computed from (see docs/FORMATS.md, "Shader programs").
//! Little-endian.

use std::collections::BTreeSet;

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
const ELSE: u32 = 0x12;
const ENDIF: u32 = 0x15;
const IF: u32 = 0x1F;
const LD: u32 = 0x2D;
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
    /// An output, `o<n>`.
    Output,
    /// A constant, `cb<buffer>[<register>]`.
    Constant { buffer: u32 },
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
    /// Whether a source is read negated (its extended token's `neg` modifier).
    pub negated: bool,
}

impl Operand {
    /// The value read for a destination's component `c`, when it is a literal.
    fn literal(&self, c: u8) -> Option<f32> {
        match self.kind {
            Kind::Literal(v) => Some(v[c as usize]),
            _ => None,
        }
    }

    fn writes_as(&self, kind: Kind, register: u32, c: u8) -> bool {
        self.kind == kind
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
    /// How many of its operands it writes.
    fn results(&self) -> usize {
        match self.opcode {
            o if NO_RESULT.contains(&o) => 0,
            o if TWO_RESULTS.contains(&o) => 2,
            _ => 1,
        }
    }

    /// Whether it writes component `c` of temporary `register`.
    fn writes(&self, register: u32, c: u8) -> bool {
        self.writes_as(Kind::Temp, register, c)
    }

    fn writes_as(&self, kind: Kind, register: u32, c: u8) -> bool {
        self.operands
            .iter()
            .take(self.results())
            .any(|o| o.writes_as(kind, register, c))
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
    // The extended token's type 1 is a modifier: 1 negates, 3 negates the magnitude.
    let negated = token >> 31 != 0 && {
        let e = word(words, at)?;
        e & 0x3F == 1 && matches!(e >> 6 & 0xFF, 1 | 3)
    };
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
            negated: false,
        });
    }
    if kind == 5 {
        bail!("a 64-bit literal");
    }
    let mut register = 0;
    let mut second = 0;
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
        match d {
            0 => register = index,
            1 => second = index,
            _ => {}
        }
    }
    let kind = match (kind, dimension) {
        (0, 1) => Kind::Temp,
        (1, 1) => Kind::Input,
        (2, 1) => Kind::Output,
        (7, 1) => Kind::Texture,
        (8, 2) => {
            let buffer = register;
            register = second;
            Kind::Constant { buffer }
        }
        _ => Kind::Other,
    };
    Ok(Operand {
        kind,
        register,
        components,
        written,
        negated,
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

/// The scale of the sample at `at` when its coordinates come from the world position:
/// direct when each coordinate is its own axis times the same literal, turned when the
/// two are a rotation of the position (a right angle included, which the compiler
/// writes as a `mad` by 0), the scale being its rows' length.
fn sample_scale(code: &[Instruction], at: usize) -> Option<Scale> {
    let uv = code[at].operands.get(1).filter(|o| o.kind == Kind::Temp)?;
    let [u, v] = [0, 1].map(|c| linear(code, at, uv.register, uv.components[c], DEPTH));
    let ([ux, uz], [vx, vz]) = (u?, v?);
    if ux != 0.0 && uz == 0.0 && vx == 0.0 && vz.abs() == ux.abs() {
        return Some(Scale::Direct(ux.abs()));
    }
    let (lu, lv) = (ux.hypot(uz), vx.hypot(vz));
    let turned = lu > 0.0
        && (lu - lv).abs() <= 1e-5 * lu
        && (ux * vx + uz * vz).abs() <= 1e-5 * lu * lv
        && ux * vz - uz * vx > 0.0;
    turned.then_some(Scale::Rotated(lu.max(lv)))
}

/// One texture's sample overlaid on a value, then mixed in:
/// `base + strength × (overlay(base, blend) − base)`, where `overlay(b, x)` is `2bx`
/// below a half and `1 − 2(1 − b)(1 − x)` from it.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    /// The texture slots the base is computed from.
    pub base: BTreeSet<u32>,
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
    let mut base = BTreeSet::new();
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
fn slots(code: &[Instruction], at: usize, register: u32, c: u8) -> BTreeSet<u32> {
    let Some(d) = definition(code, at, register, c) else {
        return BTreeSet::new();
    };
    trace(code, d, c, sampled_slot, false)
        .into_iter()
        .filter_map(|s| match s {
            Source::Texture { slot, .. } => Some(slot),
            Source::Input { .. } => None,
        })
        .collect()
}

/// What a value is computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// A channel (0 red … 3 alpha) of the texture in `slot`, sampled or fetched.
    Texture { slot: u32, channel: u8 },
    /// A component of an input, `v<register>`.
    Input { register: u32, component: u8 },
}

/// The textures' channels and the inputs component `c` (x = 0 … w = 3) of output
/// `o<register>` is computed from, as the program leaves it: through every
/// instruction before its last write, both arms of an `if` (a layer's work is
/// skipped where its weight is small, zeros written instead), not through a texture
/// read's coordinates. Constants and literals are left out; empty when the program
/// never writes it.
pub fn output_sources(code: &[Instruction], register: u32, c: u8) -> BTreeSet<Source> {
    code.iter()
        .rposition(|i| i.writes_as(Kind::Output, register, c))
        .map(|d| trace(code, d, c, read_slot, true))
        .unwrap_or_default()
}

/// The texture slots `code` samples or fetches from.
pub fn read_slots(code: &[Instruction]) -> BTreeSet<u32> {
    code.iter().filter_map(read_slot).collect()
}

/// The constants (`(buffer, register)`) the coordinates of every read of texture
/// `slot` are computed from, through the temporaries (not through other reads).
pub fn read_constants(code: &[Instruction], slot: u32) -> BTreeSet<(u32, u32)> {
    let mut out = BTreeSet::new();
    let mut todo = Vec::new();
    for (at, i) in code.iter().enumerate() {
        if read_slot(i) != Some(slot) {
            continue;
        }
        let Some(coords) = i.operands.get(1) else {
            continue;
        };
        for &k in &coords.components[..2] {
            match coords.kind {
                Kind::Temp => todo.extend(definition(code, at, coords.register, k).map(|d| (d, k))),
                Kind::Constant { buffer } => {
                    out.insert((buffer, coords.register));
                }
                _ => {}
            }
        }
    }
    out.extend(constants(code, todo));
    out
}

/// The constants (`(buffer, register)`) component `c` of output `o<register>` is
/// computed from, as the program leaves it (ignoring branches), not through a
/// texture read's coordinates.
pub fn output_constants(code: &[Instruction], register: u32, c: u8) -> BTreeSet<(u32, u32)> {
    code.iter()
        .rposition(|i| i.writes_as(Kind::Output, register, c))
        .map(|d| constants(code, vec![(d, c)]))
        .unwrap_or_default()
}

/// The constants the values the instructions in `todo` write (each an instruction
/// and a component) are computed from, through the temporaries; a texture read ends
/// the walk.
fn constants(code: &[Instruction], mut todo: Vec<(usize, u8)>) -> BTreeSet<(u32, u32)> {
    let mut out = BTreeSet::new();
    let mut seen = std::collections::HashSet::new();
    while let Some((d, c)) = todo.pop() {
        if !seen.insert((d, c)) {
            continue;
        }
        let i = &code[d];
        if read_slot(i).is_some() {
            continue;
        }
        let read: &[u8] = match i.opcode {
            DP2 => &[0, 1],
            DP3 => &[0, 1, 2],
            DP4 => &[0, 1, 2, 3],
            _ => &[c],
        };
        for o in i.operands.iter().skip(i.results()) {
            for &k in read {
                let k = o.components[k as usize];
                match o.kind {
                    Kind::Temp => todo.extend(definition(code, d, o.register, k).map(|e| (e, k))),
                    Kind::Constant { buffer } => {
                        out.insert((buffer, o.register));
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// The output registers `code` writes.
pub fn outputs(code: &[Instruction]) -> BTreeSet<u32> {
    code.iter()
        .flat_map(|i| i.operands.iter().take(i.results()))
        .filter(|o| o.kind == Kind::Output)
        .map(|o| o.register)
        .collect()
}

/// The texture slot instruction `i` samples or fetches (`ld`) from.
fn read_slot(i: &Instruction) -> Option<u32> {
    if i.opcode == LD {
        return i
            .operands
            .get(2)
            .filter(|t| t.kind == Kind::Texture)
            .map(|t| t.register);
    }
    sampled_slot(i)
}

/// What component `c` of the value the instruction at `at` writes is computed from,
/// a texture read being one `reads` names a slot for; through every write that can
/// reach a read when `arms`, else only the last one before it.
fn trace(
    code: &[Instruction],
    at: usize,
    c: u8,
    reads: fn(&Instruction) -> Option<u32>,
    arms: bool,
) -> BTreeSet<Source> {
    let mut out = BTreeSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut todo: Vec<(usize, u8)> = vec![(at, c)];
    while let Some((d, c)) = todo.pop() {
        if !seen.insert((d, c)) {
            continue;
        }
        let i = &code[d];
        if let Some(slot) = reads(i) {
            // The resource's swizzle says which of its channels each component takes.
            let channel = i.operands[2].components[c as usize];
            out.insert(Source::Texture { slot, channel });
            continue;
        }
        let read: &[u8] = match i.opcode {
            DP2 => &[0, 1],
            DP3 => &[0, 1, 2],
            DP4 => &[0, 1, 2, 3],
            _ => &[c],
        };
        for o in i.operands.iter().skip(i.results()) {
            for &k in read {
                let k = o.components[k as usize];
                match o.kind {
                    Kind::Temp if arms => todo.extend(
                        definitions(code, d, o.register, k)
                            .into_iter()
                            .map(|e| (e, k)),
                    ),
                    Kind::Temp => todo.extend(definition(code, d, o.register, k).map(|e| (e, k))),
                    Kind::Input => {
                        out.insert(Source::Input {
                            register: o.register,
                            component: k,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// The last instruction before `at` writing component `c` of temporary `register`.
fn definition(code: &[Instruction], at: usize, register: u32, c: u8) -> Option<usize> {
    code[..at].iter().rposition(|i| i.writes(register, c))
}

/// Every instruction before `at` whose write of component `c` of temporary
/// `register` can reach it: the last one on each way through the `if`/`else`/`endif`
/// blocks before it (loops are stepped over, as if run once).
fn definitions(code: &[Instruction], at: usize, register: u32, c: u8) -> BTreeSet<usize> {
    let mut reaching = BTreeSet::new();
    // For each open `if`: what reached it, and what reached the end of its first arm.
    let mut open: Vec<(BTreeSet<usize>, Option<BTreeSet<usize>>)> = Vec::new();
    for (k, i) in code[..at].iter().enumerate() {
        match i.opcode {
            IF => open.push((reaching.clone(), None)),
            ELSE => {
                if let Some((before, first)) = open.last_mut() {
                    *first = Some(std::mem::replace(&mut reaching, before.clone()));
                }
            }
            ENDIF => {
                if let Some((before, first)) = open.pop() {
                    reaching.extend(first.unwrap_or(before));
                }
            }
            _ if i.writes(register, c) => reaching = BTreeSet::from([k]),
            _ => {}
        }
    }
    reaching
}

/// How many instructions back [`linear`] follows a coordinate.
const DEPTH: u8 = 8;

/// Component `c` of `register` at `at` as a linear form of the world position (an
/// input mapped by a `mad` with constants): its coefficients on the input's x and y,
/// constant terms left out. Followed through `mul`, `mad` and `add` by literals and
/// `dp2` with a literal pair; anything else (a constant buffer's factor, a texture's
/// result, another operation) is none.
fn linear(code: &[Instruction], at: usize, register: u32, c: u8, depth: u8) -> Option<[f32; 2]> {
    let d = definition(code, at, register, c)?;
    let i = &code[d];
    let sources = i.operands.get(1..)?;
    if let Some(input) = sources.iter().find(|o| o.kind == Kind::Input) {
        // The world position itself: an input times and plus constants.
        let k = input.components[c as usize];
        return (i.opcode == MAD && sources[0].kind == Kind::Input && k < 2).then(|| {
            let mut e = [0.0; 2];
            e[k as usize] = if input.negated { -1.0 } else { 1.0 };
            e
        });
    }
    // A source's form at component `k`: a literal adds nothing to it.
    let form = |o: &Operand, k: u8| -> Option<[f32; 2]> {
        let f = match o.kind {
            Kind::Literal(_) => [0.0; 2],
            Kind::Temp if depth > 0 => {
                linear(code, d, o.register, o.components[k as usize], depth - 1)?
            }
            _ => return None,
        };
        Some(if o.negated { f.map(|x| -x) } else { f })
    };
    let scaled = |f: [f32; 2], l: f32| f.map(|x| x * l);
    let add = |a: [f32; 2], b: [f32; 2]| [a[0] + b[0], a[1] + b[1]];
    // A product of a register and a literal at component `k`.
    let product = |a: &Operand, b: &Operand, k: u8| -> Option<[f32; 2]> {
        let (r, l) = if a.literal(0).is_some() {
            (b, a)
        } else {
            (a, b)
        };
        let sign = if l.negated { -1.0 } else { 1.0 };
        Some(scaled(form(r, k)?, sign * l.literal(k)?))
    };
    match i.opcode {
        ADD => Some(add(form(sources.first()?, c)?, form(sources.get(1)?, c)?)),
        MUL => product(sources.first()?, sources.get(1)?, c),
        MAD => Some(add(
            product(sources.first()?, sources.get(1)?, c)?,
            form(sources.get(2)?, c)?,
        )),
        DP2 => {
            let (a, b) = (sources.first()?, sources.get(1)?);
            let (r, l) = if a.literal(0).is_some() {
                (b, a)
            } else {
                (a, b)
            };
            let sign = if l.negated { -1.0 } else { 1.0 };
            let x = scaled(form(r, 0)?, sign * l.literal(0)?);
            let y = scaled(form(r, 1)?, sign * l.literal(1)?);
            Some(add(x, y))
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
        /// `o<n>` written with `mask`.
        pub fn output(n: u32, mask: u32) -> Vec<u32> {
            vec![2 | mask << 4 | 2 << 12 | 1 << 20, n]
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
        /// An operand read negated: its token extended by a `neg` modifier.
        pub fn neg(mut o: Vec<u32>) -> Vec<u32> {
            o[0] |= 1 << 31;
            o.insert(1, 1 | 1 << 6);
            o
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
            // A turn: r12.x = r0.y * 0.08 - r11.x, r12.y = dot(r0.xy, (0.08, 0.06)).
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
                    op::neg(op::temp(11, [0; 4])),
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
            // A right angle, as the sand waves are: r14.xyz = r0.yxy * (0.08, 0.08, 0),
            // r13.x = r0.y * 0 - r14.y, r13.y = r14.x + r14.z.
            ins(
                MUL,
                &[
                    op::dest(14, 7),
                    op::temp(0, [1, 0, 1, 1]),
                    op::literal([0.08, 0.08, 0.0, 0.0]),
                ],
            ),
            ins(
                MAD,
                &[
                    op::dest(13, 1),
                    op::temp(0, [1; 4]),
                    op::scalar(0.0),
                    op::neg(op::temp(14, [1; 4])),
                ],
            ),
            ins(
                ADD,
                &[op::dest(13, 2), op::temp(14, [0; 4]), op::temp(14, [2; 4])],
            ),
            sample(13, 10),
            // A mirror is no turn: r16 = r0.xy * 0.08 swaps the axes.
            ins(
                MUL,
                &[op::dest(16, 3), op::temp(0, [0, 1, 0, 0]), op::scalar(0.08)],
            ),
            sample(16, 11),
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
        assert_eq!(code.len(), 20, "the declaration is left out");
        assert_eq!(sample_scales(&code, 5), [Scale::Direct(0.02)]);
        assert_eq!(sample_scales(&code, 3), [Scale::Direct(0.2)]);
        let s8 = sample_scales(&code, 8);
        assert_eq!(s8.len(), 2);
        match s8[0] {
            Scale::Rotated(s) => assert!((s - 0.1).abs() < 1e-6, "{s}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(s8[1], Scale::Direct(0.1));
        assert_eq!(sample_scales(&code, 10), [Scale::Rotated(0.08)]);
        assert!(sample_scales(&code, 11).is_empty(), "a mirror");
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
        let base = BTreeSet::from([3]);
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

    /// An output's sources follow its value back through temporaries to the texture
    /// channels and inputs it is made from, not to a read's coordinates.
    #[test]
    fn traces_an_output_through_both_arms_of_a_branch() {
        let sample = |r, slot| {
            ins(
                0x45,
                &[
                    op::dest(r, 1),
                    op::input(1, [0, 1, 0, 0]),
                    op::texture(slot),
                    op::sampler(0),
                ],
            )
        };
        let zero = ins(0x36, &[op::dest(1, 1), op::literal([0.0; 4])]);
        let code = parse(&res(&[
            // r1.x = t9.r, overwritten on both ways below: not a source
            sample(1, 9),
            // if v1.x: r1.x = t3.r, else r1.x = 0 (a layer skipped)
            ins(IF, &[op::input(1, [0, 0, 0, 0])]),
            sample(1, 3),
            ins(ELSE, &[]),
            zero,
            ins(ENDIF, &[]),
            // if v1.y: r1.x += t4.r, no else: the sum or what came before
            ins(IF, &[op::input(1, [1, 1, 1, 1])]),
            sample(2, 4),
            ins(
                ADD,
                &[
                    op::dest(1, 1),
                    op::temp(1, [0, 0, 0, 0]),
                    op::temp(2, [0, 0, 0, 0]),
                ],
            ),
            ins(ENDIF, &[]),
            ins(0x36, &[op::output(2, 2), op::temp(1, [0, 0, 0, 0])]),
        ]))
        .unwrap();
        let texture = |slot| Source::Texture { slot, channel: 0 };
        assert_eq!(
            output_sources(&code, 2, 1),
            BTreeSet::from([texture(3), texture(4)])
        );
    }

    #[test]
    fn traces_an_output_to_texture_channels_and_inputs() {
        let code = parse(&res(&[
            // r0.xy = v2.xy: coordinates, which the outputs must not pick up
            ins(0x36, &[op::dest(0, 3), op::input(2, [0, 1, 0, 0])]),
            // r15.xyz = t2.zyx: x takes blue
            ins(
                0x45,
                &[
                    op::dest(15, 7),
                    op::temp(0, [0, 1, 0, 0]),
                    op::texture_as(2, [2, 1, 0, 3]),
                    op::sampler(0),
                ],
            ),
            // o0.w = r15.x × v1.w
            ins(
                MUL,
                &[
                    op::output(0, 8),
                    op::temp(15, [0, 0, 0, 0]),
                    op::input(1, [3, 3, 3, 3]),
                ],
            ),
            // r14.x = ld t5 at r0, then o1.x = r14.x + r15.y (green)
            ins(
                LD,
                &[op::dest(14, 1), op::temp(0, [0, 1, 0, 0]), op::texture(5)],
            ),
            ins(
                ADD,
                &[
                    op::output(1, 1),
                    op::temp(14, [0, 0, 0, 0]),
                    op::temp(15, [1, 1, 1, 1]),
                ],
            ),
        ]))
        .unwrap();
        let texture = |slot, channel| Source::Texture { slot, channel };
        assert_eq!(
            output_sources(&code, 0, 3),
            BTreeSet::from([
                texture(2, 2),
                Source::Input {
                    register: 1,
                    component: 3
                }
            ])
        );
        assert_eq!(
            output_sources(&code, 1, 0),
            BTreeSet::from([texture(5, 0), texture(2, 1)])
        );
        assert!(output_sources(&code, 2, 0).is_empty());
        assert_eq!(outputs(&code), BTreeSet::from([0, 1]));
        assert_eq!(read_slots(&code), BTreeSet::from([2, 5]));
    }

    #[test]
    fn finds_the_constants_coordinates_and_outputs_come_from() {
        let code = parse(&res(&[
            // r0.xy = v2.xy × cb1[1].xy + cb1[1].zw, then t5 fetched at r0 × cb1[2]
            ins(
                MAD,
                &[
                    op::dest(0, 3),
                    op::input(2, [0, 1, 0, 0]),
                    op::constant(1, 1, [0, 1, 0, 0]),
                    op::constant(1, 1, [2, 3, 2, 2]),
                ],
            ),
            ins(
                MUL,
                &[
                    op::dest(1, 3),
                    op::temp(0, [0, 1, 0, 0]),
                    op::constant(1, 2, [0, 1, 0, 0]),
                ],
            ),
            ins(
                LD,
                &[op::dest(14, 1), op::temp(1, [0, 1, 0, 0]), op::texture(5)],
            ),
            // t6 sampled at r0 directly
            sample(0, 6),
            // o2.y = r14.x × cb1[0].x; the read's coordinates' constants stay out
            ins(
                MUL,
                &[
                    op::output(2, 2),
                    op::temp(14, [0, 0, 0, 0]),
                    op::constant(1, 0, [0, 0, 0, 0]),
                ],
            ),
        ]))
        .unwrap();
        assert_eq!(read_constants(&code, 5), BTreeSet::from([(1, 1), (1, 2)]));
        assert_eq!(read_constants(&code, 6), BTreeSet::from([(1, 1)]));
        assert!(read_constants(&code, 7).is_empty());
        assert_eq!(output_constants(&code, 2, 1), BTreeSet::from([(1, 0)]));
        assert!(output_constants(&code, 0, 0).is_empty());
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

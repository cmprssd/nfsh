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
const DP2: u32 = 0x0F;
const MAD: u32 = 0x32;
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
    code.iter()
        .enumerate()
        .filter(|(_, i)| SAMPLES.contains(&i.opcode))
        .filter(|(_, i)| {
            i.operands
                .get(2)
                .is_some_and(|t| t.kind == Kind::Texture && t.register == slot)
        })
        .filter_map(|(at, i)| {
            let uv = i.operands.get(1).filter(|o| o.kind == Kind::Temp)?;
            let [u, v] = [0, 1].map(|c| axis_scale(code, at, uv.register, uv.components[c]));
            match (u, v) {
                (Some(Scale::Direct(a)), Some(Scale::Direct(b))) if a == b => {
                    Some(Scale::Direct(a))
                }
                (Some(Scale::Rotated(s)), _) | (_, Some(Scale::Rotated(s))) => {
                    Some(Scale::Rotated(s))
                }
                _ => None,
            }
        })
        .collect()
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

    /// The world position into r0.xy, the way the terrain shaders start.
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

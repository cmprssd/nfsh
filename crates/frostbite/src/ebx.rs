//! Ebx partitions: typed objects (*instances*) with the descriptions of the types they
//! use, little-endian (see `docs/FORMATS.md`, "Ebx partitions").
//!
//! [`Partition::parse`] checks the whole layout up front: the sections, every type's
//! field range, every instance's place in the instance data and every array's place
//! after it. Reading a value afterwards fails only on a value that points outside the
//! partition (a string, array or instance index out of range).

use std::collections::HashMap;

use crate::reader::{Reader, cstr_at};
use crate::{Result, bail};

pub const MAGIC: u32 = 0x0FB4_D1CE;

/// How deep base types and structs are followed; real partitions nest far less, and
/// the limit keeps a type that contains itself from recursing forever.
const MAX_DEPTH: usize = 32;

/// A name's hash, as types and fields store it: `h = h * 33 ^ byte` from 5381, over
/// the name as written.
pub fn name_hash(name: &str) -> u32 {
    name.bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b))
}

/// The 0x40-byte header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Every section before the strings; also where the strings start.
    pub meta_size: u32,
    /// The rest of the partition: strings, instances, arrays and boxed values.
    pub rest_size: u32,
    pub import_count: u32,
    pub repeater_count: u16,
    /// The first this many repeaters hold exported instances, each with a GUID.
    pub exported_repeaters: u16,
    /// How many distinct types the instances have.
    pub instance_types: u16,
    pub type_count: u16,
    pub field_count: u16,
    pub names_size: u16,
    pub strings_size: u32,
    pub array_count: u32,
    pub instance_data_size: u32,
    pub guid: [u8; 16],
    pub boxed_count: u32,
    /// Where the boxed values start, counted from the start of the instance data.
    pub boxed_offset: u32,
}

/// What a field holds, or what a type is: bits 5-9 of its flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A type's link to its base type; its fields come first.
    Base,
    /// A value type, stored in place.
    Struct,
    /// A reference to an instance, here or in another partition.
    Class,
    Array,
    String,
    Enum,
    Bool,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float32,
    Float64,
    Guid,
    /// Names a res by its id.
    ResourceRef,
    TypeRef,
    BoxedValue,
    /// A kind not worked out yet.
    Other(u8),
}

impl Kind {
    pub fn from_flags(flags: u16) -> Self {
        match (flags >> 5) & 0x1f {
            0x00 => Self::Base,
            0x02 => Self::Struct,
            0x03 => Self::Class,
            0x04 => Self::Array,
            0x07 => Self::String,
            0x08 => Self::Enum,
            0x0a => Self::Bool,
            0x0b => Self::Int8,
            0x0c => Self::UInt8,
            0x0d => Self::Int16,
            0x0e => Self::UInt16,
            0x0f => Self::Int32,
            0x10 => Self::UInt32,
            0x11 => Self::Int64,
            0x12 => Self::UInt64,
            0x13 => Self::Float32,
            0x14 => Self::Float64,
            0x15 => Self::Guid,
            0x17 => Self::ResourceRef,
            0x19 => Self::TypeRef,
            0x1a => Self::BoxedValue,
            other => Self::Other(other as u8),
        }
    }

    /// Bytes a value of this kind takes in place; `None` for a struct (its type's size).
    /// Of the kinds not worked out, 0x09 takes 8 bytes and 0x16 takes 20 (measured over
    /// every type in the install).
    pub fn width(self) -> Option<usize> {
        Some(match self {
            Self::Bool | Self::Int8 | Self::UInt8 => 1,
            Self::Int16 | Self::UInt16 => 2,
            Self::Class | Self::Array | Self::String | Self::Enum => 4,
            Self::Int32 | Self::UInt32 | Self::Float32 => 4,
            Self::Int64 | Self::UInt64 | Self::Float64 => 8,
            Self::ResourceRef | Self::TypeRef => 8,
            Self::Guid | Self::BoxedValue => 16,
            Self::Other(0x09) => 8,
            Self::Other(0x16) => 20,
            Self::Base | Self::Struct | Self::Other(_) => return None,
        })
    }
}

/// A partition this one points at, and the instance in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Import {
    pub partition: [u8; 16],
    pub instance: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub name_hash: u32,
    pub flags: u16,
    /// For a base, struct, class, array or enum field: the type it names.
    pub type_index: u16,
    /// From the start of the containing instance or struct.
    pub offset: u32,
}

impl Field {
    pub fn kind(&self) -> Kind {
        Kind::from_flags(self.flags)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Type {
    pub name_hash: u32,
    pub first_field: u32,
    pub field_count: u8,
    pub alignment: u8,
    pub flags: u16,
    pub size: u16,
}

impl Type {
    pub fn kind(&self) -> Kind {
        Kind::from_flags(self.flags)
    }

    /// Bytes at the start of an instance of this type that the file leaves out: a
    /// type aligned to fewer than 8 bytes doesn't store the first 8 (its fields start
    /// at offset 8 or later).
    pub fn hidden(&self) -> usize {
        if self.alignment < 8 { 8 } else { 0 }
    }
}

/// An object in the partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instance {
    pub type_index: u16,
    /// Set for an exported instance, which other partitions can point at.
    pub guid: Option<[u8; 16]>,
    /// Where its field offsets count from (up to 8 bytes before its stored bytes; see
    /// [`Type::hidden`]).
    pub base: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Array {
    /// Where the elements start; the count is in the 4 bytes before.
    pub offset: usize,
    pub count: u32,
    /// The array type; its one field gives the element kind.
    pub type_index: u16,
}

/// A class reference's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ref {
    Null,
    /// An instance of this partition, by index.
    Internal(usize),
    /// An entry of the import table, by index.
    Import(usize),
}

/// A struct value, or the fields of an instance: where its type's fields are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Object {
    pub type_index: u16,
    pub base: usize,
}

/// One field's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float32(f32),
    Float64(f64),
    /// `None` for no string (offset 0xFFFFFFFF).
    String(Option<&'a str>),
    /// The value and the enum type.
    Enum(u32, u16),
    Guid([u8; 16]),
    ResourceRef(u64),
    TypeRef(u64),
    BoxedValue([u8; 16]),
    Class(Ref),
    /// Index into [`Partition::arrays`], and the field's array type (record 0, the
    /// shared empty array, has some other array's type).
    Array(usize, u16),
    Struct(Object),
    /// A kind not worked out, with its bytes when its width is known (else none).
    Other(u8, &'a [u8]),
}

#[derive(Debug, Clone)]
pub struct Partition<'a> {
    data: &'a [u8],
    pub header: Header,
    /// The partition's own name, the first string.
    pub name: &'a str,
    pub imports: Vec<Import>,
    pub fields: Vec<Field>,
    pub types: Vec<Type>,
    pub instances: Vec<Instance>,
    pub arrays: Vec<Array>,
    /// The boxed value records (8 bytes each; meaning not worked out).
    pub boxed: Vec<[u8; 8]>,
    names: HashMap<u32, &'a str>,
    strings: &'a [u8],
}

impl<'a> Partition<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let magic = r.u32_le()?;
        if magic != MAGIC {
            bail!("ebx magic {magic:08x}, expected {MAGIC:08x}");
        }
        let header = Header {
            meta_size: r.u32_le()?,
            rest_size: r.u32_le()?,
            import_count: r.u32_le()?,
            repeater_count: r.u16_le()?,
            exported_repeaters: r.u16_le()?,
            instance_types: r.u16_le()?,
            type_count: r.u16_le()?,
            field_count: r.u16_le()?,
            names_size: r.u16_le()?,
            strings_size: r.u32_le()?,
            array_count: r.u32_le()?,
            instance_data_size: r.u32_le()?,
            guid: r.array()?,
            boxed_count: r.u32_le()?,
            boxed_offset: r.u32_le()?,
        };
        let size = header.meta_size as u64 + header.rest_size as u64;
        if size != data.len() as u64 {
            bail!(
                "ebx sizes add up to {size} bytes, the entry has {}",
                data.len()
            );
        }

        let imports = (0..header.import_count)
            .map(|_| {
                Ok(Import {
                    partition: r.array()?,
                    instance: r.array()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut names = HashMap::new();
        for name in r.bytes(header.names_size.into())?.split(|&b| b == 0) {
            if let Ok(name) = std::str::from_utf8(name)
                && !name.is_empty()
            {
                names.insert(name_hash(name), name);
            }
        }
        let fields = (0..header.field_count)
            .map(|_| {
                let field = Field {
                    name_hash: r.u32_le()?,
                    flags: r.u16_le()?,
                    type_index: r.u16_le()?,
                    offset: r.u32_le()?,
                };
                r.u32_le()?; // a second offset, not worked out
                Ok(field)
            })
            .collect::<Result<Vec<_>>>()?;
        let types = (0..header.type_count)
            .map(|_| {
                let ty = Type {
                    name_hash: r.u32_le()?,
                    first_field: r.u32_le()?,
                    field_count: r.u8()?,
                    alignment: r.u8()?,
                    flags: r.u16_le()?,
                    size: r.u16_le()?,
                };
                r.u16_le()?; // a second size, not worked out
                Ok(ty)
            })
            .collect::<Result<Vec<_>>>()?;
        let repeaters = (0..header.repeater_count)
            .map(|_| Ok((r.u16_le()?, r.u16_le()?)))
            .collect::<Result<Vec<_>>>()?;
        r.align(16)?;
        let array_records = (0..header.array_count)
            .map(|_| Ok((r.u32_le()?, r.u32_le()?, r.u32_le()?)))
            .collect::<Result<Vec<_>>>()?;
        r.align(16)?;
        let boxed = (0..header.boxed_count)
            .map(|_| r.array())
            .collect::<Result<Vec<_>>>()?;
        r.align(16)?;
        if r.pos() != header.meta_size as usize {
            bail!(
                "ebx sections end at {:#x}, the header says {:#x}",
                r.pos(),
                header.meta_size
            );
        }
        let strings = r.bytes(header.strings_size as usize)?;
        let name = cstr_at(strings, 0)?;
        r.align(16)?;

        for (i, ty) in types.iter().enumerate() {
            let end = ty.first_field as u64 + u64::from(ty.field_count);
            if end > fields.len() as u64 {
                bail!("ebx type {i} has fields up to {end}, of {}", fields.len());
            }
        }
        for (i, f) in fields.iter().enumerate() {
            let typed = matches!(
                f.kind(),
                Kind::Base | Kind::Struct | Kind::Array | Kind::Enum
            );
            if typed && usize::from(f.type_index) >= types.len() {
                bail!(
                    "ebx field {i} names type {}, of {}",
                    f.type_index,
                    types.len()
                );
            }
        }
        let mut partition = Self {
            data,
            header,
            name,
            imports,
            fields,
            types,
            instances: Vec::new(),
            arrays: Vec::new(),
            boxed,
            names,
            strings,
        };
        partition.instances = partition.place_instances(r.pos(), &repeaters)?;
        partition.arrays = partition.place_arrays(&array_records)?;
        Ok(partition)
    }

    /// Lays the instances out in repeater order from `start`: each at its type's
    /// alignment, an exported one right after its GUID.
    fn place_instances(&self, start: usize, repeaters: &[(u16, u16)]) -> Result<Vec<Instance>> {
        let end = start + self.header.instance_data_size as usize;
        let mut pos = start;
        let mut instances = Vec::new();
        for (i, &(type_index, count)) in repeaters.iter().enumerate() {
            let ty = self.ty(type_index)?;
            let exported = i < usize::from(self.header.exported_repeaters);
            for _ in 0..count {
                let guid_size = if exported { 16 } else { 0 };
                let stored = pos + guid_size;
                let stored = stored.next_multiple_of(usize::from(ty.alignment.max(1)));
                let guid = match exported {
                    true => Some(Reader::at(self.data, stored - 16)?.array()?),
                    false => None,
                };
                let Some(base) = stored.checked_sub(ty.hidden()) else {
                    bail!("ebx instance {} starts before the data", instances.len());
                };
                pos = base + usize::from(ty.size);
                if pos > end {
                    bail!(
                        "ebx instance {} ends at {pos:#x}, past the instance data ({end:#x})",
                        instances.len()
                    );
                }
                instances.push(Instance {
                    type_index,
                    guid,
                    base,
                });
            }
        }
        Ok(instances)
    }

    /// Where each array is, checking its count and that its elements fit.
    fn place_arrays(&self, records: &[(u32, u32, u32)]) -> Result<Vec<Array>> {
        let start = self.arrays_start();
        let mut arrays = Vec::with_capacity(records.len());
        for (i, &(offset, count, type_index)) in records.iter().enumerate() {
            let Ok(type_index) = u16::try_from(type_index) else {
                bail!("ebx array {i} names type {type_index}");
            };
            let array = Array {
                offset: start + offset as usize,
                count,
                type_index,
            };
            let stored = Reader::at(self.data, array.offset - 4)?.u32_le()?;
            if stored != count {
                bail!("ebx array {i} holds {stored} elements, its record says {count}");
            }
            let width = self.element_width(&array)?;
            let fits = (count as u64)
                .checked_mul(width as u64)
                .is_some_and(|n| array.offset as u64 + n <= self.data.len() as u64);
            if !fits {
                bail!("ebx array {i} ({count} × {width} bytes) runs past the end");
            }
            arrays.push(array);
        }
        Ok(arrays)
    }

    /// Where the array elements start: right after the instance data.
    pub fn arrays_start(&self) -> usize {
        self.instances_start() + self.header.instance_data_size as usize
    }

    /// Where the instance data starts: after the strings, at a multiple of 16.
    pub fn instances_start(&self) -> usize {
        (self.header.meta_size as usize + self.strings.len()).next_multiple_of(16)
    }

    pub fn ty(&self, index: u16) -> Result<&Type> {
        match self.types.get(usize::from(index)) {
            Some(ty) => Ok(ty),
            None => bail!("ebx type {index} is out of range ({})", self.types.len()),
        }
    }

    /// The name a hash stands for, if the partition lists it.
    pub fn name(&self, hash: u32) -> Option<&'a str> {
        self.names.get(&hash).copied()
    }

    /// A type's name, or its hash when the names table lacks it.
    pub fn type_name(&self, index: u16) -> String {
        match self.types.get(usize::from(index)) {
            Some(ty) => self
                .name(ty.name_hash)
                .map_or_else(|| format!("{:08x}", ty.name_hash), str::to_owned),
            None => format!("type {index}"),
        }
    }

    /// A type's own fields, as listed (its base field first, if any).
    pub fn own_fields(&self, ty: &Type) -> &[Field] {
        let start = ty.first_field as usize;
        let end = start + usize::from(ty.field_count);
        self.fields.get(start..end).unwrap_or_default()
    }

    /// Every field of a type, its base types' first, without the base links.
    pub fn fields_of(&self, type_index: u16) -> Result<Vec<&Field>> {
        let mut chain = vec![self.ty(type_index)?];
        while let Some(base) = self
            .own_fields(chain[chain.len() - 1])
            .iter()
            .find(|f| f.kind() == Kind::Base)
        {
            if chain.len() >= MAX_DEPTH {
                bail!("ebx type {type_index} has more than {MAX_DEPTH} base types");
            }
            chain.push(self.ty(base.type_index)?);
        }
        Ok(chain
            .iter()
            .rev()
            .flat_map(|ty| self.own_fields(ty))
            .filter(|f| f.kind() != Kind::Base)
            .collect())
    }

    /// Whether `type_index` is `ancestor` or derives from it.
    pub fn derives_from(&self, type_index: u16, ancestor: u16) -> bool {
        let mut t = type_index;
        for _ in 0..MAX_DEPTH {
            if t == ancestor {
                return true;
            }
            let Ok(ty) = self.ty(t) else { return false };
            match self.own_fields(ty).iter().find(|f| f.kind() == Kind::Base) {
                Some(base) => t = base.type_index,
                None => return false,
            }
        }
        false
    }

    /// An instance's fields.
    pub fn object(&self, instance: &Instance) -> Object {
        Object {
            type_index: instance.type_index,
            base: instance.base,
        }
    }

    /// The value of `field` in the object whose fields start at `base`.
    pub fn value(&self, field: &Field, base: usize) -> Result<Value<'a>> {
        self.read(field.kind(), field.type_index, base + field.offset as usize)
    }

    /// The array's element `index`.
    pub fn element(&self, array: &Array, index: u32) -> Result<Value<'a>> {
        if index >= array.count {
            bail!(
                "ebx array element {index} is out of range ({})",
                array.count
            );
        }
        let member = self.member(array)?;
        let width = self.element_width(array)?;
        self.read(
            member.kind(),
            member.type_index,
            array.offset + index as usize * width,
        )
    }

    /// The field describing an array's elements.
    fn member(&self, array: &Array) -> Result<&Field> {
        let ty = self.ty(array.type_index)?;
        match self.own_fields(ty).first() {
            Some(member) => Ok(member),
            None => bail!("ebx array type {} has no element field", array.type_index),
        }
    }

    fn element_width(&self, array: &Array) -> Result<usize> {
        let member = self.member(array)?;
        match member.kind() {
            Kind::Struct => Ok(usize::from(self.ty(member.type_index)?.size)),
            kind => match kind.width() {
                Some(width) => Ok(width),
                None => bail!("ebx array of {kind:?}, whose width is not known"),
            },
        }
    }

    fn read(&self, kind: Kind, type_index: u16, at: usize) -> Result<Value<'a>> {
        let mut r = Reader::at(self.data, at)?;
        Ok(match kind {
            Kind::Bool => Value::Bool(r.u8()? != 0),
            Kind::Int8 => Value::Int(r.u8()? as i8 as i64),
            Kind::UInt8 => Value::UInt(r.u8()?.into()),
            Kind::Int16 => Value::Int(r.u16_le()? as i16 as i64),
            Kind::UInt16 => Value::UInt(r.u16_le()?.into()),
            Kind::Int32 => Value::Int(r.u32_le()? as i32 as i64),
            Kind::UInt32 => Value::UInt(r.u32_le()?.into()),
            Kind::Int64 => Value::Int(r.u64_le()? as i64),
            Kind::UInt64 => Value::UInt(r.u64_le()?),
            Kind::Float32 => Value::Float32(f32::from_bits(r.u32_le()?)),
            Kind::Float64 => Value::Float64(f64::from_bits(r.u64_le()?)),
            Kind::String => Value::String(match r.u32_le()? {
                u32::MAX => None,
                offset => Some(cstr_at(self.strings, offset as usize)?),
            }),
            Kind::Enum => Value::Enum(r.u32_le()?, type_index),
            Kind::Guid => Value::Guid(r.array()?),
            Kind::ResourceRef => Value::ResourceRef(r.u64_le()?),
            Kind::TypeRef => Value::TypeRef(r.u64_le()?),
            Kind::BoxedValue => Value::BoxedValue(r.array()?),
            Kind::Class => Value::Class(self.class_ref(r.u32_le()?)?),
            Kind::Array => {
                let index = r.u32_le()? as usize;
                if index >= self.arrays.len() {
                    bail!("ebx array {index} is out of range ({})", self.arrays.len());
                }
                Value::Array(index, type_index)
            }
            Kind::Struct => Value::Struct(Object {
                type_index,
                base: at,
            }),
            Kind::Base => bail!("ebx base field read as a value"),
            Kind::Other(k) => Value::Other(k, r.bytes(kind.width().unwrap_or(0))?),
        })
    }

    /// A class reference: 0 for none, an import index with the top bit set, otherwise
    /// an instance index plus one.
    fn class_ref(&self, value: u32) -> Result<Ref> {
        let import = value & 0x8000_0000 != 0;
        let (index, count) = match import {
            true => ((value & 0x7fff_ffff) as usize, self.imports.len()),
            false => ((value as usize).wrapping_sub(1), self.instances.len()),
        };
        match (value, import) {
            (0, _) => Ok(Ref::Null),
            _ if index >= count => bail!("ebx class reference {value:#x} is out of range"),
            (_, true) => Ok(Ref::Import(index)),
            (_, false) => Ok(Ref::Internal(index)),
        }
    }

    /// The enum member named for `value`, if the enum type lists one.
    pub fn enum_member(&self, type_index: u16, value: u32) -> Option<&'a str> {
        let ty = self.types.get(usize::from(type_index))?;
        self.own_fields(ty)
            .iter()
            .find(|f| f.offset == value)
            .and_then(|f| self.name(f.name_hash))
    }

    /// The res id in the resource field `name` of the first instance that has one (a
    /// mesh asset's `MeshSetResource`, a texture asset's `Resource`).
    pub fn resource(&self, name: &str) -> Result<u64> {
        for instance in &self.instances {
            let o = self.object(instance);
            for f in self.fields_of(o.type_index)? {
                if self.name(f.name_hash) == Some(name) {
                    match self.value(f, o.base)? {
                        Value::ResourceRef(id) if id != 0 => return Ok(id),
                        _ => bail!("{}: {name} names no res", self.name),
                    }
                }
            }
        }
        bail!("{}: no instance has a {name}", self.name)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::Path;

    use crate::Location;
    use crate::store::{Store, in_parallel};

    const CLASS: u16 = 0x0063;
    const ARRAY_TYPE: u16 = 0x0089;
    const fn kind(k: u16) -> u16 {
        k << 5
    }

    fn put16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn put32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn pad(out: &mut Vec<u8>, align: usize) {
        out.resize(out.len().next_multiple_of(align), 0);
    }

    /// A small partition laid out by the rules in FORMATS.md: two `Thing` instances
    /// (align 4, so 8 hidden bytes each), the first exported; a `Thing` has a name, a
    /// float, an array of floats and a reference. The first points at the second, the
    /// second at the one import. The partition's GUID is 0x44 repeated.
    pub(crate) fn sample() -> Vec<u8> {
        let mut d = vec![0; 0x40];
        d.extend_from_slice(&[0x11; 16]); // import: partition GUID
        d.extend_from_slice(&[0x22; 16]); // import: instance GUID
        let names = b"$\0Name\0Weight\0Items\0Other\0member\0DataContainer\0Thing\0float[]\0";
        d.extend_from_slice(names);
        let h = |s: &str| name_hash(s);
        // fields: name hash, flags, type, offset, second offset
        let fields: [(u32, u16, u16, u32); 6] = [
            (h("$"), kind(0x00), 0, 8),
            (h("Name"), kind(0x07), 0, 8),
            (h("Weight"), kind(0x13), 0, 12),
            (h("Items"), kind(0x04), 2, 16),
            (h("Other"), kind(0x03), 0, 20),
            (h("member"), kind(0x13), 0, 0),
        ];
        for (hash, flags, ty, offset) in fields {
            put32(&mut d, hash);
            put16(&mut d, flags);
            put16(&mut d, ty);
            put32(&mut d, offset);
            put32(&mut d, 0);
        }
        // types: name hash, first field, field count, alignment, flags, size
        let types: [(u32, u32, u8, u8, u16, u16); 3] = [
            (h("DataContainer"), 0, 0, 4, CLASS, 8),
            (h("Thing"), 0, 5, 4, CLASS, 24),
            (h("float[]"), 5, 1, 4, ARRAY_TYPE, 4),
        ];
        for (hash, first, count, align, flags, size) in types {
            put32(&mut d, hash);
            put32(&mut d, first);
            d.extend_from_slice(&[count, align]);
            put16(&mut d, flags);
            put16(&mut d, size);
            put16(&mut d, 0);
        }
        // repeaters: one exported Thing, then one more
        for (ty, count) in [(1u16, 1u16), (1, 1)] {
            put16(&mut d, ty);
            put16(&mut d, count);
        }
        pad(&mut d, 16);
        // arrays: the empty one, then three floats at offset 16
        for (offset, count, ty) in [(0u32, 0u32, 2u32), (16, 3, 2)] {
            put32(&mut d, offset);
            put32(&mut d, count);
            put32(&mut d, ty);
        }
        pad(&mut d, 16);
        let meta = d.len();
        d.extend_from_slice(b"test/sample\0hello\0");
        let strings = d.len() - meta;
        pad(&mut d, 16);
        let instances = d.len();
        // instance 0: GUID, then its fields from offset 8
        d.extend_from_slice(&[0x33; 16]);
        put32(&mut d, 12); // Name: "hello"
        put32(&mut d, 1.5f32.to_bits());
        put32(&mut d, 1); // Items: array 1
        put32(&mut d, 2); // Other: instance 1
        // instance 1: no GUID
        put32(&mut d, u32::MAX); // Name: none
        put32(&mut d, (-2.0f32).to_bits());
        put32(&mut d, 0); // Items: the empty array
        put32(&mut d, 0x8000_0000); // Other: import 0
        let end = d.len();
        d.resize((end + 4).next_multiple_of(16), 0);
        let instance_data = d.len() - instances;
        let arrays = d.len();
        d.resize(arrays + 12, 0);
        put32(&mut d, 3);
        for x in [0.25f32, 0.5, 0.75] {
            put32(&mut d, x.to_bits());
        }
        let boxed = d.len() - instances;

        let mut header = Vec::new();
        put32(&mut header, MAGIC);
        put32(&mut header, meta as u32);
        put32(&mut header, (d.len() - meta) as u32);
        put32(&mut header, 1); // imports
        put16(&mut header, 2); // repeaters
        put16(&mut header, 1); // exported repeaters
        put16(&mut header, 1); // instance types
        put16(&mut header, 3); // types
        put16(&mut header, 6); // fields
        put16(&mut header, names.len() as u16);
        put32(&mut header, strings as u32);
        put32(&mut header, 2); // arrays
        put32(&mut header, instance_data as u32);
        header.extend_from_slice(&[0x44; 16]);
        put32(&mut header, 0); // boxed values
        put32(&mut header, boxed as u32);
        d[..0x40].copy_from_slice(&header);
        d
    }

    /// A value for [`Builder`]; a field's kind comes from its type's declaration.
    #[derive(Debug, Clone)]
    pub(crate) enum V {
        F32(f32),
        U32(u32),
        Str(&'static str),
        /// A class reference as stored: 0, instance + 1, or 0x80000000 | import.
        Ref(u32),
        /// A struct's field values, in its type's order.
        Struct(Vec<V>),
        Array(Vec<V>),
    }

    /// The kind of a field [`Builder::ty`] declares, with the type it names.
    #[derive(Debug, Clone, Copy)]
    pub(crate) enum F {
        F32,
        U32,
        Str,
        Ref,
        Struct(&'static str),
        Array(&'static str),
    }

    /// Lays out partitions by the rules in FORMATS.md, for tests that need more than
    /// [`sample`]: types aligned to 4 (class instances hide their first 8 bytes),
    /// every instance exported, one per repeater.
    /// A declared type: name, type flags, fields (name and kind; an array type has
    /// one).
    type Declared = (&'static str, u16, Vec<(&'static str, F)>);

    #[derive(Debug, Clone, Default)]
    pub(crate) struct Builder {
        types: Vec<Declared>,
        imports: Vec<([u8; 16], [u8; 16])>,
        instances: Vec<(&'static str, [u8; 16], Vec<V>)>,
    }

    impl Builder {
        /// A class type when `class`, else a struct.
        pub(crate) fn ty(
            mut self,
            name: &'static str,
            class: bool,
            fields: &[(&'static str, F)],
        ) -> Self {
            let flags = if class { CLASS } else { 0x0045 };
            self.types.push((name, flags, fields.to_vec()));
            self
        }

        pub(crate) fn array(mut self, name: &'static str, member: F) -> Self {
            self.types
                .push((name, ARRAY_TYPE, vec![("member", member)]));
            self
        }

        pub(crate) fn import(mut self, partition: [u8; 16], instance: [u8; 16]) -> Self {
            self.imports.push((partition, instance));
            self
        }

        pub(crate) fn instance(mut self, ty: &'static str, guid: [u8; 16], values: Vec<V>) -> Self {
            self.instances.push((ty, guid, values));
            self
        }

        fn type_index(&self, name: &str) -> u16 {
            match self.types.iter().position(|t| t.0 == name) {
                Some(i) => i as u16,
                None => panic!("type {name} not declared"),
            }
        }

        fn width(&self, f: F) -> usize {
            match f {
                F::Struct(name) => self.size(name),
                _ => 4,
            }
        }

        /// A type's size: its fields back to back, after 8 hidden bytes for a class.
        fn size(&self, name: &str) -> usize {
            let (_, flags, fields) = &self.types[usize::from(self.type_index(name))];
            let start = if *flags == CLASS { 8 } else { 0 };
            start + fields.iter().map(|&(_, f)| self.width(f)).sum::<usize>()
        }

        /// Writes `v` as a value of kind `f`. Strings go to `strings`; an array's
        /// elements are written to `arrays` (nested arrays first) and `v` stores its
        /// record index (0, the shared empty one, for none).
        fn value(
            &self,
            f: F,
            v: &V,
            out: &mut Vec<u8>,
            strings: &mut Vec<u8>,
            arrays: &mut Vec<(u16, u32, Vec<u8>)>,
        ) {
            match (f, v) {
                (F::F32, V::F32(x)) => put32(out, x.to_bits()),
                (F::U32, V::U32(n)) | (F::Ref, V::Ref(n)) => put32(out, *n),
                (F::Str, V::Str(s)) => {
                    put32(out, strings.len() as u32);
                    strings.extend_from_slice(s.as_bytes());
                    strings.push(0);
                }
                (F::Struct(name), V::Struct(values)) => {
                    self.fields(name, values, out, strings, arrays)
                }
                (F::Array(name), V::Array(elements)) if elements.is_empty() => {
                    let _ = self.type_index(name);
                    put32(out, 0);
                }
                (F::Array(name), V::Array(elements)) => {
                    let ty = self.type_index(name);
                    let member = self.types[usize::from(ty)].2[0].1;
                    let mut bytes = Vec::new();
                    for e in elements {
                        self.value(member, e, &mut bytes, strings, arrays);
                    }
                    arrays.push((ty, elements.len() as u32, bytes));
                    put32(out, arrays.len() as u32); // record 0 is the empty array
                }
                _ => panic!("{v:?} is not a {f:?}"),
            }
        }

        fn fields(
            &self,
            ty: &str,
            values: &[V],
            out: &mut Vec<u8>,
            strings: &mut Vec<u8>,
            arrays: &mut Vec<(u16, u32, Vec<u8>)>,
        ) {
            let fields = &self.types[usize::from(self.type_index(ty))].2;
            assert_eq!(fields.len(), values.len(), "values for {ty}");
            for (&(_, f), v) in fields.iter().zip(values) {
                self.value(f, v, out, strings, arrays);
            }
        }

        /// The partition `name` with the GUID `guid`.
        pub(crate) fn build(&self, name: &str, guid: [u8; 16]) -> Vec<u8> {
            let mut d = vec![0; 0x40];
            for (p, i) in &self.imports {
                d.extend_from_slice(p);
                d.extend_from_slice(i);
            }
            let mut names = Vec::new();
            for (ty, _, fields) in &self.types {
                for n in std::iter::once(ty).chain(fields.iter().map(|f| &f.0)) {
                    names.extend_from_slice(n.as_bytes());
                    names.push(0);
                }
            }
            d.extend_from_slice(&names);
            let mut field_count = 0;
            for (_, flags, fields) in &self.types {
                let mut offset = if *flags == CLASS { 8 } else { 0 };
                for &(n, f) in fields {
                    let (k, ty) = match f {
                        F::F32 => (0x13, 0),
                        F::U32 => (0x10, 0),
                        F::Str => (0x07, 0),
                        F::Ref => (0x03, 0),
                        F::Struct(t) => (0x02, self.type_index(t)),
                        F::Array(t) => (0x04, self.type_index(t)),
                    };
                    put32(&mut d, name_hash(n));
                    put16(&mut d, kind(k));
                    put16(&mut d, ty);
                    // An array type's member sits at offset 0.
                    put32(
                        &mut d,
                        if *flags == ARRAY_TYPE {
                            0
                        } else {
                            offset as u32
                        },
                    );
                    put32(&mut d, 0);
                    offset += self.width(f);
                }
            }
            for (ty, flags, fields) in &self.types {
                let size = match *flags {
                    ARRAY_TYPE => self.width(fields[0].1),
                    _ => self.size(ty),
                };
                put32(&mut d, name_hash(ty));
                put32(&mut d, field_count);
                d.extend_from_slice(&[fields.len() as u8, 4]);
                put16(&mut d, *flags);
                put16(&mut d, size as u16);
                put16(&mut d, 0);
                field_count += fields.len() as u32;
            }
            for (ty, _, _) in &self.instances {
                put16(&mut d, self.type_index(ty));
                put16(&mut d, 1);
            }
            pad(&mut d, 16);

            let mut strings = name.as_bytes().to_vec();
            strings.push(0);
            let mut arrays = Vec::new();
            let mut data = Vec::new();
            for (ty, guid, values) in &self.instances {
                pad(&mut data, 4);
                data.extend_from_slice(guid);
                self.fields(ty, values, &mut data, &mut strings, &mut arrays);
            }
            let mut records = vec![(0u32, 0u32, 2u32)];
            let mut elements = vec![0; 12];
            for (ty, count, bytes) in &arrays {
                put32(&mut elements, *count);
                records.push((elements.len() as u32, *count, u32::from(*ty)));
                elements.extend_from_slice(bytes);
            }
            for &(offset, count, ty) in &records {
                put32(&mut d, offset);
                put32(&mut d, count);
                put32(&mut d, ty);
            }
            pad(&mut d, 16);
            let meta = d.len();
            d.extend_from_slice(&strings);
            pad(&mut d, 16);
            let instances = d.len();
            d.extend_from_slice(&data);
            let end = d.len();
            d.resize((end + 4).next_multiple_of(16), 0);
            let instance_data = d.len() - instances;
            d.extend_from_slice(&elements);
            let boxed = d.len() - instances;

            let mut header = Vec::new();
            put32(&mut header, MAGIC);
            put32(&mut header, meta as u32);
            put32(&mut header, (d.len() - meta) as u32);
            put32(&mut header, self.imports.len() as u32);
            let repeaters = self.instances.len() as u16;
            put16(&mut header, repeaters);
            put16(&mut header, repeaters); // all exported
            put16(&mut header, repeaters);
            put16(&mut header, self.types.len() as u16);
            put16(&mut header, field_count as u16);
            put16(&mut header, names.len() as u16);
            put32(&mut header, strings.len() as u32);
            put32(&mut header, records.len() as u32);
            put32(&mut header, instance_data as u32);
            header.extend_from_slice(&guid);
            put32(&mut header, 0);
            put32(&mut header, boxed as u32);
            d[..0x40].copy_from_slice(&header);
            d
        }
    }

    #[test]
    fn the_builder_writes_partitions_that_read_back() {
        let b = Builder::default()
            .ty("Vec2", false, &[("x", F::F32), ("y", F::F32)])
            .array("Vec2[]", F::Struct("Vec2"))
            .array("Uint32[]", F::U32)
            .ty(
                "Thing",
                true,
                &[
                    ("Name", F::Str),
                    ("At", F::Struct("Vec2")),
                    ("Path", F::Array("Vec2[]")),
                    ("Next", F::Ref),
                ],
            )
            .ty(
                "Holder",
                true,
                &[
                    ("Lists", F::Array("Uint32[]")),
                    ("Empty", F::Array("Uint32[]")),
                ],
            )
            .import([1; 16], [2; 16])
            .instance(
                "Thing",
                [3; 16],
                vec![
                    V::Str("one"),
                    V::Struct(vec![V::F32(1.0), V::F32(2.0)]),
                    V::Array(vec![V::Struct(vec![V::F32(3.0), V::F32(4.0)])]),
                    V::Ref(2),
                ],
            )
            .instance(
                "Holder",
                [4; 16],
                vec![V::Array(vec![V::U32(7), V::U32(8)]), V::Array(vec![])],
            )
            .instance(
                "Thing",
                [5; 16],
                vec![
                    V::Str("two"),
                    V::Struct(vec![V::F32(0.0), V::F32(0.0)]),
                    V::Array(vec![]),
                    V::Ref(0x8000_0000),
                ],
            );
        let data = b.build("test/built", [9; 16]);
        let p = Partition::parse(&data).unwrap();
        assert_eq!(p.name, "test/built");
        assert_eq!(p.header.guid, [9; 16]);
        assert_eq!(
            p.instances.iter().map(|i| i.guid).collect::<Vec<_>>(),
            [Some([3; 16]), Some([4; 16]), Some([5; 16])]
        );
        let v = values(&p).unwrap();
        let ok = |i: usize, want: &str| assert_eq!(v[i], want, "{v:#?}");
        ok(0, r#"Name = String(Some("one"))"#);
        ok(2, "Path = Array(1, 1)");
        ok(4, "Next = Class(Internal(1))");
        ok(5, "Lists = Array(2, 2)");
        ok(6, "  UInt(7)");
        ok(7, "  UInt(8)");
        ok(8, "Empty = Array(0, 2)");
        ok(9, r#"Name = String(Some("two"))"#);
        ok(12, "Next = Class(Import(0))");
        // Structs hold their fields in place, in arrays too.
        let xy = |o: Object| -> Vec<Value> {
            p.fields_of(o.type_index)
                .unwrap()
                .iter()
                .map(|f| p.value(f, o.base).unwrap())
                .collect()
        };
        let fields = p.fields_of(p.instances[0].type_index).unwrap();
        let Value::Struct(at) = p.value(fields[1], p.instances[0].base).unwrap() else {
            panic!("At is not a struct");
        };
        assert_eq!(xy(at), [Value::Float32(1.0), Value::Float32(2.0)]);
        let Value::Struct(first) = p.element(&p.arrays[1], 0).unwrap() else {
            panic!("Path[0] is not a struct");
        };
        assert_eq!(xy(first), [Value::Float32(3.0), Value::Float32(4.0)]);
    }

    /// Every value of every instance, as text, so one comparison checks them all.
    fn values(p: &Partition) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for instance in &p.instances {
            for field in p.fields_of(instance.type_index)? {
                let v = p.value(field, instance.base)?;
                out.push(format!(
                    "{} = {v:?}",
                    p.name(field.name_hash).unwrap_or("?")
                ));
                if let Value::Array(i, _) = v {
                    let array = p.arrays[i];
                    for e in 0..array.count {
                        out.push(format!("  {:?}", p.element(&array, e)?));
                    }
                }
            }
        }
        Ok(out)
    }

    #[test]
    fn name_hash_matches_the_base_link() {
        assert_eq!(name_hash("$"), 0x0002_b581);
    }

    #[test]
    fn reads_the_sample_partition() {
        let data = sample();
        let p = Partition::parse(&data).unwrap();
        assert_eq!(p.name, "test/sample");
        assert_eq!(p.header.guid, [0x44; 16]);
        assert_eq!(p.type_name(1), "Thing");
        assert_eq!(p.instances.len(), 2);
        assert_eq!(p.instances[0].guid, Some([0x33; 16]));
        assert_eq!(p.instances[1].guid, None);
        // The first instance's fields start right after its GUID, at offset 8.
        assert_eq!(p.instances[0].base, p.instances_start() + 16 - 8);
        // Each stores 24 - 8 bytes, so the second's base is 16 bytes on.
        assert_eq!(p.instances[1].base, p.instances[0].base + 16);
        assert_eq!(
            values(&p).unwrap(),
            [
                "Name = String(Some(\"hello\"))",
                "Weight = Float32(1.5)",
                "Items = Array(1, 2)",
                "  Float32(0.25)",
                "  Float32(0.5)",
                "  Float32(0.75)",
                "Other = Class(Internal(1))",
                "Name = String(None)",
                "Weight = Float32(-2.0)",
                "Items = Array(0, 2)",
                "Other = Class(Import(0))",
            ]
        );
    }

    #[test]
    fn rejects_bad_headers_and_counts() {
        let data = sample();
        let mut bad = data.clone();
        bad[0] ^= 1;
        assert!(Partition::parse(&bad).unwrap_err().0.contains("magic"));
        let mut bad = data.clone();
        bad[0x24] += 16; // instance data longer than it is: the arrays move
        assert!(Partition::parse(&bad).is_err());
        let mut bad = data.clone();
        let count_at = bad.len() - 16;
        bad[count_at] = 4; // the array's stored count disagrees with its record
        assert!(Partition::parse(&bad).unwrap_err().0.contains("array 1"));
    }

    #[test]
    fn out_of_range_references_are_errors_not_panics() {
        let mut data = sample();
        let p = Partition::parse(&data).unwrap();
        let other = p.instances[0].base + 20;
        data[other..other + 4].copy_from_slice(&3u32.to_le_bytes()); // instance 2 of 2
        let p = Partition::parse(&data).unwrap();
        assert!(values(&p).unwrap_err().0.contains("out of range"));
    }

    /// Every truncation and every single-byte change parses or fails cleanly, and
    /// reading what parses never panics.
    #[test]
    fn damaged_partitions_never_panic() {
        let data = sample();
        for len in 0..data.len() {
            assert!(Partition::parse(&data[..len]).is_err());
        }
        for i in 0..data.len() {
            for x in [0x01, 0x80, 0xff] {
                let mut bad = data.clone();
                bad[i] ^= x;
                if let Ok(p) = Partition::parse(&bad) {
                    let _ = values(&p);
                }
            }
        }
    }

    /// What reading every value of a store's partitions found.
    #[derive(Debug, Default)]
    struct Census {
        partitions: u64,
        instances: u64,
        values: u64,
        /// Values of kinds not worked out, by kind.
        others: BTreeMap<u8, u64>,
        /// Rules that did not hold, with an example each.
        broken: BTreeMap<&'static str, (u64, String)>,
    }

    impl Census {
        fn broke(&mut self, rule: &'static str, example: impl FnOnce() -> String) {
            let entry = self.broken.entry(rule).or_insert_with(|| (0, example()));
            entry.0 += 1;
        }

        fn add(&mut self, other: Census) {
            self.partitions += other.partitions;
            self.instances += other.instances;
            self.values += other.values;
            for (k, n) in other.others {
                *self.others.entry(k).or_default() += n;
            }
            for (rule, (n, example)) in other.broken {
                self.broken.entry(rule).or_insert((0, example)).0 += n;
            }
        }
    }

    /// Reads every value of an object, following structs and arrays, and checks the
    /// rules a reader relies on but `parse` does not enforce.
    fn census_object(p: &Partition, o: Object, depth: usize, c: &mut Census) -> Result<()> {
        if depth > MAX_DEPTH {
            bail!("structs nested deeper than {MAX_DEPTH}");
        }
        for field in p.fields_of(o.type_index)? {
            let v = p.value(field, o.base)?;
            census_value(p, field, v, depth, c)?;
        }
        Ok(())
    }

    fn census_value(
        p: &Partition,
        field: &Field,
        v: Value,
        depth: usize,
        c: &mut Census,
    ) -> Result<()> {
        c.values += 1;
        match v {
            Value::Other(k, _) => *c.others.entry(k).or_default() += 1,
            Value::Struct(s) => census_object(p, s, depth + 1, c)?,
            Value::Class(Ref::Internal(i)) => {
                let target = p.instances[i].type_index;
                if !p.derives_from(target, field.type_index) {
                    c.broke(
                        "an internal reference's target has the field's class",
                        || {
                            format!(
                                "{}: {} -> {}",
                                p.name,
                                p.type_name(field.type_index),
                                p.type_name(target)
                            )
                        },
                    );
                }
            }
            Value::Array(i, ty) => {
                let array = p.arrays[i];
                if array.count > 0 && array.type_index != ty {
                    c.broke("a non-empty array's record has the field's type", || {
                        format!(
                            "{}: {} vs {}",
                            p.name,
                            p.type_name(array.type_index),
                            p.type_name(ty)
                        )
                    });
                }
                let member = *p.member(&array)?;
                for e in 0..array.count {
                    census_value(p, &member, p.element(&array, e)?, depth + 1, c)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn census(p: &Partition) -> Result<Census> {
        let mut c = Census {
            partitions: 1,
            instances: p.instances.len() as u64,
            ..Census::default()
        };
        let distinct: std::collections::BTreeSet<u16> =
            p.instances.iter().map(|i| i.type_index).collect();
        if distinct.len() != usize::from(p.header.instance_types) {
            c.broke("header 0x14 counts the instances' distinct types", || {
                p.name.to_owned()
            });
        }
        if let Some(last) = p.instances.last() {
            let end = last.base + usize::from(p.ty(last.type_index)?.size);
            if (end + 4).next_multiple_of(16) != p.arrays_start() {
                c.broke(
                    "the arrays start at the instances' end + 4, aligned to 16",
                    || format!("{}: end {end:#x}, arrays {:#x}", p.name, p.arrays_start()),
                );
            }
        }
        let boxed_start = p.instances_start() + p.header.boxed_offset as usize;
        let arrays_end = p
            .arrays
            .iter()
            .map(|a| a.offset + a.count as usize * p.element_width(a).unwrap_or(0))
            .max();
        if boxed_start > p.data.len() || arrays_end.is_some_and(|end| end > boxed_start) {
            c.broke("the boxed values start after the arrays", || {
                p.name.to_owned()
            });
        }
        if p.boxed.is_empty() && boxed_start != p.data.len() {
            c.broke("without boxed values, 0x3C is the end", || {
                p.name.to_owned()
            });
        }
        // Fields of known width fit in their type without overlapping.
        for (t, ty) in p.types.iter().enumerate() {
            if !matches!(ty.kind(), Kind::Struct | Kind::Class) {
                continue;
            }
            let mut spans: Vec<(usize, usize)> = p
                .own_fields(ty)
                .iter()
                .filter_map(|f| {
                    let width = match f.kind() {
                        Kind::Struct => p.ty(f.type_index).ok().map(|s| usize::from(s.size)),
                        k => k.width(),
                    }?;
                    Some((f.offset as usize, f.offset as usize + width))
                })
                .collect();
            spans.sort();
            let fits = spans.last().is_none_or(|s| s.1 <= usize::from(ty.size));
            let apart = spans.windows(2).all(|w| w[0].1 <= w[1].0);
            if !fits || !apart {
                c.broke("fields fit their type without overlapping", || {
                    format!("{}: {}", p.name, p.type_name(t as u16))
                });
            }
        }
        for instance in &p.instances {
            census_object(p, p.object(instance), 0, &mut c)?;
        }
        Ok(c)
    }

    /// Reads every value of every ebx in nfsh's store when `NFSH_STORE` points at one
    /// (local-ci sets it on the Windows side), checking the rules in FORMATS.md.
    #[test]
    fn reads_every_ebx_in_the_store() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let mut locations: Vec<Location> = store
            .superbundles
            .iter()
            .flat_map(|sb| &sb.bundles)
            .flat_map(|b| &b.bundle.ebx)
            .map(|e| e.location)
            .collect();
        locations.sort_unstable_by_key(|l| (l.cas, l.offset));
        locations.dedup();
        let censuses = in_parallel(
            &locations,
            || store.entries(),
            |entries, &l| {
                let data = entries.read(l)?;
                let p = Partition::parse(&data).map_err(|e| crate::Error(format!("{l:?}: {e}")))?;
                census(&p).map_err(|e| crate::Error(format!("{}: {e}", p.name)))
            },
        )
        .unwrap();
        let mut total = Census::default();
        for c in censuses {
            total.add(c);
        }
        eprintln!("{total:#?}");
        assert_eq!(total.partitions, locations.len() as u64);
        assert!(
            total.partitions > 700_000,
            "only {} partitions",
            total.partitions
        );
        assert!(total.broken.is_empty(), "rules broken: {:#?}", total.broken);
    }
}

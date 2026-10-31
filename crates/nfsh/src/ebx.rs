//! `nfsh ebx`: one ebx partition from a store, printed as text.

use std::fmt::Write;
use std::path::Path;

use frostbite::Result;
use frostbite::ebx::{Object, Partition, Ref, Value};
use frostbite::store::Store;

/// How deep structs and arrays are printed; deeper ones are elided.
const MAX_DEPTH: usize = 24;

/// Reads the ebx `name` from the store in `dir` and prints it.
pub fn run(dir: &Path, name: &str) -> Result<String> {
    let store = Store::open(dir)?;
    let Some(asset) = store.ebx(name) else {
        return Err(frostbite::Error(format!(
            "no ebx named {name} in the store"
        )));
    };
    let data = store.entries()?.read(asset.location)?;
    text(&Partition::parse(&data)?)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn guid(g: &[u8; 16]) -> String {
    hex(g)
}

/// The partition as text: its imports, then each instance with its fields.
pub fn text(p: &Partition) -> Result<String> {
    let mut out = String::new();
    let _ = writeln!(out, "partition {} {}", p.name, guid(&p.header.guid));
    let _ = writeln!(
        out,
        "{} imports, {} types, {} instances, {} arrays",
        p.imports.len(),
        p.types.len(),
        p.instances.len(),
        p.arrays.len()
    );
    for (i, import) in p.imports.iter().enumerate() {
        let _ = writeln!(
            out,
            "import {i}: {}/{}",
            guid(&import.partition),
            guid(&import.instance)
        );
    }
    for (i, instance) in p.instances.iter().enumerate() {
        let id = instance.guid.as_ref().map(guid).unwrap_or_default();
        let _ = writeln!(
            out,
            "\ninstance {i} {} {id}",
            p.type_name(instance.type_index)
        );
        object(p, &mut out, p.object(instance), 1)?;
    }
    Ok(out)
}

fn object(p: &Partition, out: &mut String, o: Object, depth: usize) -> Result<()> {
    for field in p.fields_of(o.type_index)? {
        let name = p.name(field.name_hash).unwrap_or("?");
        let _ = write!(out, "{:indent$}{name} = ", "", indent = 2 * depth);
        value(p, out, p.value(field, o.base)?, depth)?;
    }
    Ok(())
}

fn value(p: &Partition, out: &mut String, v: Value, depth: usize) -> Result<()> {
    let _ = match v {
        Value::Bool(b) => writeln!(out, "{b}"),
        Value::Int(n) => writeln!(out, "{n}"),
        Value::UInt(n) => writeln!(out, "{n}"),
        Value::Float32(x) => writeln!(out, "{x}"),
        Value::Float64(x) => writeln!(out, "{x}"),
        Value::String(Some(s)) => writeln!(out, "{s:?}"),
        Value::String(None) => writeln!(out, "null"),
        Value::Enum(n, ty) => match p.enum_member(ty, n) {
            Some(member) => writeln!(out, "{member}"),
            None => writeln!(out, "{n} ({})", p.type_name(ty)),
        },
        Value::Guid(g) => writeln!(out, "{}", guid(&g)),
        Value::ResourceRef(id) => writeln!(out, "res {id:016x}"),
        Value::TypeRef(t) => writeln!(out, "type {t:016x}"),
        Value::BoxedValue(b) => writeln!(out, "boxed {}", guid(&b)),
        Value::Class(Ref::Null) => writeln!(out, "null"),
        Value::Class(Ref::Internal(i)) => writeln!(out, "-> instance {i}"),
        Value::Class(Ref::Import(i)) => writeln!(out, "-> import {i}"),
        Value::Other(kind, bytes) => writeln!(out, "(kind {kind:#04x}) {}", hex(bytes)),
        Value::Struct(s) if depth >= MAX_DEPTH => {
            writeln!(out, "{} {{…}}", p.type_name(s.type_index))
        }
        Value::Struct(s) => {
            let _ = writeln!(out, "{}", p.type_name(s.type_index));
            return object(p, out, s, depth + 1);
        }
        Value::Array(i, ty) => {
            let Some(&array) = p.arrays.get(i) else {
                return Err(frostbite::Error(format!("ebx array {i} is out of range")));
            };
            let _ = writeln!(out, "{} [{}]", p.type_name(ty), array.count);
            if depth >= MAX_DEPTH {
                return Ok(());
            }
            for e in 0..array.count {
                let _ = write!(out, "{:indent$}[{e}] ", "", indent = 2 * (depth + 1));
                value(p, out, p.element(&array, e)?, depth + 1)?;
            }
            Ok(())
        }
    };
    Ok(())
}

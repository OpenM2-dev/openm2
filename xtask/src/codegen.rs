//! `just gen`: turns `crates/m2-protocol/packets/<profile>.toml` into
//! `crates/m2-protocol/src/profile/<profile>/generated.rs` and
//! `docs/specs/protocol/<profile>/messages.md`. `just drift` checks they
//! are current.
//!
//! A profile declares openm2's own messages: a message is
//! its id as a varint, then its fields; fields may be strings, blobs, lists
//! and options of their own length. The output is the message structs,
//! `ClientMessage`/`ServerMessage` and the message tables.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::util::{display_path, repo_root};

/// Names the generated code already uses.
const RESERVED_NAMES: &[&str] = &[
    "ClientMessage",
    "ServerMessage",
    "Message",
    "MessageSpec",
    "Direction",
    "Reader",
    "Writer",
    "Decode",
    "Encode",
];

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe",
    "use", "where", "while", "abstract", "become", "box", "do", "final", "macro", "override",
    "priv", "try", "typeof", "unsized", "virtual", "yield",
];

// ---------------------------------------------------------------------------
// TOML model

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    profile: RawProfile,
    #[serde(default, rename = "struct")]
    structs: Vec<RawStruct>,
    #[serde(default, rename = "message")]
    messages: Vec<RawMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    name: String,
    doc: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStruct {
    name: String,
    doc: String,
    fields: Vec<RawField>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMessage {
    name: String,
    dir: Dir,
    id: u16,
    doc: String,
    #[serde(default)]
    fields: Vec<RawField>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    doc: String,
    #[serde(default)]
    secret: bool,
}

/// Which way a message goes: a message goes one way only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Dir {
    C2s,
    S2c,
}

impl Dir {
    fn label(self) -> &'static str {
        match self {
            Dir::C2s => "client → server",
            Dir::S2c => "server → client",
        }
    }
}

// ---------------------------------------------------------------------------
// Field types

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prim {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
}

impl Prim {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "u8" => Prim::U8,
            "u16" => Prim::U16,
            "u32" => Prim::U32,
            "u64" => Prim::U64,
            "i8" => Prim::I8,
            "i16" => Prim::I16,
            "i32" => Prim::I32,
            "i64" => Prim::I64,
            "f32" => Prim::F32,
            _ => return None,
        })
    }

    fn rust(self) -> &'static str {
        match self {
            Prim::U8 => "u8",
            Prim::U16 => "u16",
            Prim::U32 => "u32",
            Prim::U64 => "u64",
            Prim::I8 => "i8",
            Prim::I16 => "i16",
            Prim::I32 => "i32",
            Prim::I64 => "i64",
            Prim::F32 => "f32",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Prim(Prim),
    Array(Box<Ty>, usize),
    Struct(String),
    /// 0 or 1.
    Bool,
    /// UTF-8 of at most N bytes after a u16 length.
    Utf8(usize),
    /// At most N bytes after a u16 length.
    Blob(usize),
    /// At most N items after a u16 count.
    List(Box<Ty>, usize),
    /// 0, or 1 and the value.
    Opt(Box<Ty>),
}

fn parse_len(s: &str) -> Result<usize> {
    let n: usize = s
        .trim()
        .parse()
        .with_context(|| format!("`{s}` is not a length"))?;
    ensure!(n > 0, "lengths must be at least 1");
    Ok(n)
}

/// The most of a `string`, `blob` or `list`: its length or count is a u16.
fn parse_max(s: &str) -> Result<usize> {
    let n = parse_len(s)?;
    ensure!(
        n <= usize::from(u16::MAX),
        "at most {} fits a u16",
        u16::MAX
    );
    Ok(n)
}

fn parse_ty(s: &str) -> Result<Ty> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let (elem, len) = inner
            .rsplit_once(';')
            .with_context(|| format!("array type `{s}` must look like [T; N]"))?;
        return Ok(Ty::Array(Box::new(parse_ty(elem)?), parse_len(len)?));
    }
    if let Some(len) = s.strip_prefix("string<").and_then(|x| x.strip_suffix('>')) {
        return Ok(Ty::Utf8(parse_max(len)?));
    }
    if let Some(len) = s.strip_prefix("blob<").and_then(|x| x.strip_suffix('>')) {
        return Ok(Ty::Blob(parse_max(len)?));
    }
    if let Some(inner) = s.strip_prefix("list<").and_then(|x| x.strip_suffix('>')) {
        let (elem, len) = inner
            .rsplit_once(';')
            .with_context(|| format!("list type `{s}` must look like list<T; N>"))?;
        return Ok(Ty::List(Box::new(parse_ty(elem)?), parse_max(len)?));
    }
    if let Some(inner) = s.strip_prefix("opt<").and_then(|x| x.strip_suffix('>')) {
        return Ok(Ty::Opt(Box::new(parse_ty(inner)?)));
    }
    if s == "bool" {
        return Ok(Ty::Bool);
    }
    if let Some(prim) = Prim::parse(s) {
        return Ok(Ty::Prim(prim));
    }
    if is_pascal(s) {
        return Ok(Ty::Struct(s.to_owned()));
    }
    bail!("unknown type `{s}`")
}

impl Ty {
    fn rust(&self) -> String {
        match self {
            Ty::Prim(p) => p.rust().to_owned(),
            Ty::Array(elem, n) => format!("[{}; {n}]", elem.rust()),
            Ty::Struct(name) => name.clone(),
            Ty::Bool => "bool".to_owned(),
            Ty::Utf8(_) => "String".to_owned(),
            Ty::Blob(_) => "Vec<u8>".to_owned(),
            Ty::List(elem, _) => format!("Vec<{}>", elem.rust()),
            Ty::Opt(elem) => format!("Option<{}>", elem.rust()),
        }
    }

    /// The type as written in the TOML, normalised.
    fn spec(&self) -> String {
        match self {
            Ty::Prim(p) => p.rust().to_owned(),
            Ty::Array(elem, n) => format!("[{}; {n}]", elem.spec()),
            Ty::Struct(name) => name.clone(),
            Ty::Bool => "bool".to_owned(),
            Ty::Utf8(n) => format!("string<{n}>"),
            Ty::Blob(n) => format!("blob<{n}>"),
            Ty::List(elem, n) => format!("list<{}; {n}>", elem.spec()),
            Ty::Opt(elem) => format!("opt<{}>", elem.spec()),
        }
    }

    /// False if the type contains a float (no `Eq`).
    fn has_eq(&self, structs: &BTreeMap<String, Struct>) -> bool {
        match self {
            Ty::Prim(p) => *p != Prim::F32,
            Ty::Bool | Ty::Utf8(_) | Ty::Blob(_) => true,
            Ty::Array(elem, _) | Ty::List(elem, _) | Ty::Opt(elem) => elem.has_eq(structs),
            Ty::Struct(name) => structs.get(name).is_some_and(|s| s.eq),
        }
    }

    /// True for a string, blob, list or option: read with a length, a count
    /// or a tag, not through `Decode`.
    fn is_variable(&self) -> bool {
        matches!(self, Ty::Utf8(_) | Ty::Blob(_) | Ty::List(..) | Ty::Opt(_))
    }

    /// True if its length varies: it is variable or holds something that is.
    fn varies(&self, structs: &BTreeMap<String, Struct>) -> bool {
        match self {
            Ty::Prim(_) | Ty::Bool => false,
            Ty::Utf8(_) | Ty::Blob(_) | Ty::List(..) | Ty::Opt(_) => true,
            Ty::Array(elem, _) => elem.varies(structs),
            Ty::Struct(name) => structs.get(name).is_some_and(|s| s.varies),
        }
    }

    fn struct_name(&self) -> Option<&str> {
        match self {
            Ty::Struct(name) => Some(name),
            Ty::Array(elem, _) | Ty::List(elem, _) | Ty::Opt(elem) => elem.struct_name(),
            _ => None,
        }
    }
}

/// An expression reading one `ty` from the reader `r`, giving a `Result`.
fn decode_expr(ty: &Ty) -> String {
    match ty {
        Ty::Utf8(n) => format!("r.string({n})"),
        Ty::Blob(n) => format!("r.blob({n})"),
        Ty::List(elem, n) => format!("r.list({n}, {})", decode_fn(elem)),
        Ty::Opt(elem) => format!("r.opt({})", decode_fn(elem)),
        _ => "Decode::decode(r)".to_owned(),
    }
}

/// A function reading one `ty` from a reader, for lists and options.
fn decode_fn(ty: &Ty) -> String {
    if ty.is_variable() {
        format!("|r| {}", decode_expr(ty))
    } else {
        "Decode::decode".to_owned()
    }
}

/// An expression writing `value` (a place, or a reference with `by_ref`)
/// of type `ty` to the writer `w`, giving a `Result`.
fn encode_expr(ty: &Ty, value: &str, by_ref: bool) -> String {
    let borrow = if by_ref { "" } else { "&" };
    match ty {
        Ty::Utf8(n) => format!("w.string({borrow}{value}, {n})"),
        Ty::Blob(n) => format!("w.blob({borrow}{value}, {n})"),
        Ty::List(elem, n) => format!(
            "w.list({borrow}{value}, {n}, |w, x| {})",
            encode_expr(elem, "x", true)
        ),
        Ty::Opt(elem) => format!(
            "w.opt({borrow}{value}, |w, x| {})",
            encode_expr(elem, "x", true)
        ),
        _ => format!("{value}.encode(w)"),
    }
}

// ---------------------------------------------------------------------------
// Validated model

#[derive(Debug)]
struct Field {
    name: String,
    ty: Ty,
    doc: String,
    secret: bool,
}

#[derive(Debug)]
struct Struct {
    name: String,
    doc: String,
    fields: Vec<Field>,
    /// True when a field's length varies.
    varies: bool,
    eq: bool,
}

#[derive(Debug)]
struct MessageDef {
    name: String,
    dir: Dir,
    id: u16,
    doc: String,
    fields: Vec<Field>,
    eq: bool,
}

impl MessageDef {
    fn secrets(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| f.secret)
    }
}

#[derive(Debug)]
struct Model {
    name: String,
    doc: String,
    source: String,
    structs: BTreeMap<String, Struct>,
    /// Struct names in declaration order.
    struct_order: Vec<String>,
    messages: Vec<MessageDef>,
}

fn is_pascal(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase()) && chars.all(|c| c.is_ascii_alphanumeric())
}

fn is_snake(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn check_doc(doc: &str, what: &str) -> Result<String> {
    let doc = doc.trim();
    ensure!(!doc.is_empty(), "{what} needs a non-empty `doc`");
    ensure!(!doc.contains('\n'), "{what}: keep `doc` on one line");
    Ok(doc.to_owned())
}

/// What a list of fields belongs to; decides which field options are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Struct,
    Message,
}

fn resolve_fields(
    raw: &[RawField],
    owner_name: &str,
    structs: &BTreeMap<String, Struct>,
    owner: Owner,
) -> Result<Vec<Field>> {
    let mut seen = BTreeSet::new();
    let mut fields = Vec::new();
    for f in raw {
        let what = format!("{owner_name}.{}", f.name);
        ensure!(is_snake(&f.name), "{what}: field names are snake_case");
        ensure!(
            !RUST_KEYWORDS.contains(&f.name.as_str()),
            "{what}: `{}` is a Rust keyword, pick another name (e.g. `kind` for `type`)",
            f.name
        );
        ensure!(seen.insert(f.name.clone()), "{what}: duplicate field");
        let ty = parse_ty(&f.ty).with_context(|| what.clone())?;
        if let Some(name) = ty.struct_name() {
            ensure!(
                structs.contains_key(name),
                "{what}: struct `{name}` is not declared above it"
            );
        }
        if let Ty::Array(elem, _) = &ty {
            ensure!(
                !elem.varies(structs),
                "{what}: array items need a fixed size; use list<T; N>"
            );
        }
        if f.secret {
            ensure!(
                owner == Owner::Message,
                "{what}: `secret` is only allowed on message fields, not inside a [[struct]]"
            );
            ensure!(
                matches!(ty, Ty::Prim(_) | Ty::Bool | Ty::Utf8(_) | Ty::Blob(_)),
                "{what}: `secret` is for numbers, strings and blobs"
            );
        }
        fields.push(Field {
            name: f.name.clone(),
            ty,
            doc: check_doc(&f.doc, &what)?,
            secret: f.secret,
        });
    }
    Ok(fields)
}

fn read_raw(path: &Path) -> Result<RawFile> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", display_path(path)))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", display_path(path)))
}

fn load(path: &Path) -> Result<Model> {
    let raw = read_raw(path)?;
    let profile_name = raw.profile.name.clone();
    ensure!(
        is_snake(&profile_name),
        "profile name must be lowercase letters, digits and `_`"
    );
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    ensure!(
        file_stem == profile_name,
        "{}: profile name `{profile_name}` must match the file name",
        display_path(path)
    );

    let mut names = BTreeSet::new();
    let mut structs = BTreeMap::new();
    let mut struct_order = Vec::new();
    for s in &raw.structs {
        ensure!(is_pascal(&s.name), "struct `{}` must be PascalCase", s.name);
        ensure!(
            !RESERVED_NAMES.contains(&s.name.as_str()),
            "`{}` is reserved",
            s.name
        );
        ensure!(names.insert(s.name.clone()), "duplicate name `{}`", s.name);
        let fields = resolve_fields(&s.fields, &s.name, &structs, Owner::Struct)?;
        ensure!(!fields.is_empty(), "struct `{}` has no fields", s.name);
        let varies = fields.iter().any(|f| f.ty.varies(&structs));
        let eq = fields.iter().all(|f| f.ty.has_eq(&structs));
        struct_order.push(s.name.clone());
        structs.insert(
            s.name.clone(),
            Struct {
                name: s.name.clone(),
                doc: check_doc(&s.doc, &s.name)?,
                fields,
                varies,
                eq,
            },
        );
    }

    let mut ids = BTreeSet::new();
    let mut messages = Vec::new();
    for m in &raw.messages {
        let what = format!("message `{}`", m.name);
        ensure!(is_pascal(&m.name), "{what}: names are PascalCase");
        ensure!(
            !RESERVED_NAMES.contains(&m.name.as_str()),
            "{what}: the name is reserved"
        );
        ensure!(names.insert(m.name.clone()), "{what}: duplicate name");
        ensure!(
            ids.insert((m.dir == Dir::C2s, m.id)),
            "{what}: id {} is already used for {}",
            m.id,
            m.dir.label()
        );
        let fields = resolve_fields(&m.fields, &m.name, &structs, Owner::Message)?;
        let eq = fields.iter().all(|f| f.ty.has_eq(&structs));
        messages.push(MessageDef {
            name: m.name.clone(),
            dir: m.dir,
            id: m.id,
            doc: check_doc(&m.doc, &what)?,
            fields,
            eq,
        });
    }

    Ok(Model {
        name: profile_name,
        doc: check_doc(&raw.profile.doc, "profile")?,
        source: display_path(path),
        structs,
        struct_order,
        messages,
    })
}

/// The messages of one direction, by id.
fn of_direction(model: &Model, dir: Dir) -> Vec<&MessageDef> {
    let mut rows: Vec<&MessageDef> = model.messages.iter().filter(|m| m.dir == dir).collect();
    rows.sort_by_key(|m| m.id);
    rows
}

// ---------------------------------------------------------------------------
// Rust output

fn render_type(out: &mut String, name: &str, doc: &str, fields: &[Field], eq: bool) -> Result<()> {
    writeln!(out, "/// {doc}")?;
    // A struct with secret fields gets a hand-written Debug that hides them.
    let has_secret = fields.iter().any(|f| f.secret);
    let derives = match (has_secret, eq) {
        (false, true) => "Debug, Clone, PartialEq, Eq",
        (false, false) => "Debug, Clone, PartialEq",
        (true, true) => "Clone, PartialEq, Eq",
        (true, false) => "Clone, PartialEq",
    };
    writeln!(out, "#[derive({derives})]")?;
    if fields.is_empty() {
        writeln!(out, "pub struct {name};\n")?;
        writeln!(
            out,
            "impl Decode for {name} {{ fn decode(_r: &mut Reader<'_>) -> Result<Self, DecodeError> {{ Ok(Self) }} }}\n"
        )?;
        writeln!(
            out,
            "impl Encode for {name} {{ fn encode(&self, _w: &mut Writer) -> Result<(), EncodeError> {{ Ok(()) }} }}\n"
        )?;
        return Ok(());
    }
    writeln!(out, "pub struct {name} {{")?;
    for f in fields {
        writeln!(out, "    /// {}", f.doc)?;
        writeln!(out, "    pub {}: {},", f.name, f.ty.rust())?;
    }
    writeln!(out, "}}\n")?;

    writeln!(out, "impl Decode for {name} {{")?;
    writeln!(
        out,
        "    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {{"
    )?;
    writeln!(out, "        Ok(Self {{")?;
    for f in fields {
        writeln!(out, "            {}: {}?,", f.name, decode_expr(&f.ty))?;
    }
    writeln!(out, "        }})\n    }}\n}}\n")?;

    writeln!(out, "impl Encode for {name} {{")?;
    writeln!(
        out,
        "    fn encode(&self, w: &mut Writer) -> Result<(), EncodeError> {{"
    )?;
    for f in fields {
        let value = format!("self.{}", f.name);
        writeln!(out, "        {}?;", encode_expr(&f.ty, &value, false))?;
    }
    writeln!(out, "        Ok(())\n    }}\n}}\n")?;

    if has_secret {
        writeln!(out, "/// Secret fields are shown as `<redacted>`.")?;
        writeln!(out, "impl std::fmt::Debug for {name} {{")?;
        writeln!(
            out,
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{"
        )?;
        writeln!(out, "        f.debug_struct(\"{name}\")")?;
        for field in fields {
            if field.secret {
                writeln!(
                    out,
                    "            .field(\"{}\", &format_args!(\"<redacted>\"))",
                    field.name
                )?;
            } else {
                writeln!(out, "            .field(\"{0}\", &self.{0})", field.name)?;
            }
        }
        writeln!(out, "            .finish()\n    }}\n}}\n")?;
    }
    Ok(())
}

fn render_rust(model: &Model) -> Result<String> {
    let mut out = String::new();
    writeln!(
        out,
        "// @generated by `just gen` from {}. Do not edit by hand.\n",
        model.source
    )?;
    writeln!(
        out,
        "use crate::codec::{{Decode, DecodeError, Encode, EncodeError, Reader, Writer}};"
    )?;
    writeln!(
        out,
        "use crate::message::{{Message, MessageSpec, message_id}};\n"
    )?;

    for (constant, dir, who) in [
        ("CLIENT_MESSAGES", Dir::C2s, "client"),
        ("SERVER_MESSAGES", Dir::S2c, "server"),
    ] {
        writeln!(
            out,
            "/// The messages the {who} sends in the `{}` profile, by id.",
            model.name
        )?;
        writeln!(out, "pub const {constant}: &[MessageSpec] = &[")?;
        for m in of_direction(model, dir) {
            let secrets: Vec<String> = m.secrets().map(|f| format!("\"{}\"", f.name)).collect();
            writeln!(
                out,
                "MessageSpec {{ name: \"{}\", id: {}, secrets: &[{}] }},",
                m.name,
                m.id,
                secrets.join(", ")
            )?;
        }
        writeln!(out, "];\n")?;
    }

    for name in &model.struct_order {
        if let Some(s) = model.structs.get(name) {
            render_type(&mut out, &s.name, &s.doc, &s.fields, s.eq)?;
        }
    }

    for m in &model.messages {
        render_type(&mut out, &m.name, &m.doc, &m.fields, m.eq)?;
        writeln!(out, "impl Message for {} {{", m.name)?;
        writeln!(out, "    const ID: u16 = {};", m.id)?;
        writeln!(out, "    const NAME: &'static str = \"{}\";", m.name)?;
        writeln!(out, "}}\n")?;
        render_redacted(&mut out, m)?;
    }

    render_enum(&mut out, model, "ClientMessage", Dir::C2s, "client")?;
    render_enum(&mut out, model, "ServerMessage", Dir::S2c, "server")?;
    Ok(out)
}

/// `redacted()` for a message with secret fields: a copy with them blanked.
fn render_redacted(out: &mut String, m: &MessageDef) -> Result<()> {
    if m.secrets().next().is_none() {
        return Ok(());
    }
    writeln!(out, "impl {} {{", m.name)?;
    writeln!(
        out,
        "    /// A copy with the secret fields blanked, for captures and logs."
    )?;
    writeln!(out, "    pub fn redacted(&self) -> Self {{")?;
    writeln!(out, "        Self {{")?;
    for f in m.secrets() {
        writeln!(out, "            {}: Default::default(),", f.name)?;
    }
    if m.fields.iter().any(|f| !f.secret) {
        writeln!(out, "            ..self.clone()")?;
    }
    writeln!(out, "        }}\n    }}\n}}\n")?;
    Ok(())
}

fn render_enum(
    out: &mut String,
    model: &Model,
    enum_name: &str,
    dir: Dir,
    who: &str,
) -> Result<()> {
    let rows = of_direction(model, dir);
    let eq = rows.iter().all(|m| m.eq);
    writeln!(
        out,
        "/// Every message the {who} sends in the `{}` profile.",
        model.name
    )?;
    writeln!(
        out,
        "#[derive({})]",
        if eq {
            "Debug, Clone, PartialEq, Eq"
        } else {
            "Debug, Clone, PartialEq"
        }
    )?;
    writeln!(
        out,
        "// Messages live only while they are handled; boxing the big ones would make every match awkward."
    )?;
    writeln!(out, "#[allow(clippy::large_enum_variant)]")?;
    writeln!(out, "pub enum {enum_name} {{")?;
    for m in &rows {
        writeln!(out, "    /// {}", m.doc)?;
        writeln!(out, "    {0}({0}),", m.name)?;
    }
    writeln!(out, "}}\n")?;

    writeln!(out, "impl {enum_name} {{")?;
    writeln!(
        out,
        "    /// Decodes a whole message: its id, then its fields. An id this side\n    /// does not know is [`DecodeError::UnknownId`]."
    )?;
    writeln!(
        out,
        "    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {{"
    )?;
    writeln!(out, "        match message_id(bytes)? {{")?;
    for m in &rows {
        writeln!(
            out,
            "            {} => {}::from_bytes(bytes).map(Self::{}),",
            m.id, m.name, m.name
        )?;
    }
    writeln!(
        out,
        "            id => Err(DecodeError::UnknownId {{ id }}),\n        }}\n    }}\n"
    )?;

    writeln!(out, "    /// The whole message: its id, then its fields.")?;
    writeln!(
        out,
        "    pub fn to_bytes(&self) -> Result<Vec<u8>, EncodeError> {{\n        match self {{"
    )?;
    for m in &rows {
        writeln!(out, "            Self::{}(m) => m.to_bytes(),", m.name)?;
    }
    writeln!(out, "        }}\n    }}\n")?;

    writeln!(
        out,
        "    /// Its id.\n    pub fn id(&self) -> u16 {{\n        match self {{"
    )?;
    for m in &rows {
        writeln!(out, "            Self::{0}(_) => {0}::ID,", m.name)?;
    }
    writeln!(out, "        }}\n    }}\n")?;

    writeln!(
        out,
        "    /// Its name in the TOML.\n    pub fn name(&self) -> &'static str {{\n        match self {{"
    )?;
    for m in &rows {
        writeln!(out, "            Self::{0}(_) => {0}::NAME,", m.name)?;
    }
    writeln!(out, "        }}\n    }}\n")?;

    writeln!(
        out,
        "    /// A copy with the secret fields blanked, for captures and logs.\n    pub fn redacted(&self) -> Self {{\n        match self {{"
    )?;
    for m in &rows {
        if m.secrets().next().is_some() {
            writeln!(
                out,
                "            Self::{0}(m) => Self::{0}(m.redacted()),",
                m.name
            )?;
        } else {
            writeln!(
                out,
                "            Self::{0}(m) => Self::{0}(m.clone()),",
                m.name
            )?;
        }
    }
    writeln!(out, "        }}\n    }}\n}}\n")?;
    Ok(())
}

fn rustfmt(code: &str) -> Result<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start rustfmt")?;
    child
        .stdin
        .take()
        .context("rustfmt stdin")?
        .write_all(code.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "rustfmt rejected the generated code (a generator bug):\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

// ---------------------------------------------------------------------------
// Markdown output

fn render_fields_md(out: &mut String, fields: &[Field]) -> Result<()> {
    if fields.is_empty() {
        writeln!(out, "No fields.")?;
        return Ok(());
    }
    writeln!(out, "| Field | Type | Description |")?;
    writeln!(out, "|---|---|---|")?;
    for f in fields {
        let secret = if f.secret {
            " **Secret:** blanked in logs and captures."
        } else {
            ""
        };
        writeln!(
            out,
            "| `{}` | `{}` | {}{secret} |",
            f.name,
            f.ty.spec(),
            f.doc
        )?;
    }
    Ok(())
}

fn render_markdown(model: &Model) -> Result<String> {
    let mut out = String::new();
    writeln!(
        out,
        "<!-- @generated by `just gen` from {}. Do not edit by hand. -->\n",
        model.source
    )?;
    writeln!(out, "# `{}` messages\n", model.name)?;
    writeln!(out, "{}\n", model.doc)?;
    writeln!(
        out,
        "A message is its id, a LEB128 varint, then its fields in order; on the wire each message follows its length, also a varint ([`README.md`](README.md)). Integers are little-endian. `string<N>` is a u16 byte length, then at most N bytes of UTF-8; `blob<N>` a u16 length, then at most N bytes; `list<T; N>` a u16 count, then at most N items; `opt<T>` a 0, or a 1 and then the value; `bool` a 0 or a 1. Ids are per direction. Behaviour (when each message is sent and what the other side does) is described in the other files of this folder.\n"
    )?;

    for (title, dir) in [("Client → server", Dir::C2s), ("Server → client", Dir::S2c)] {
        writeln!(out, "## {title}\n")?;
        let rows = of_direction(model, dir);
        if rows.is_empty() {
            writeln!(out, "None yet.\n")?;
            continue;
        }
        writeln!(out, "| Id | Name | Description |")?;
        writeln!(out, "|---|---|---|")?;
        for m in rows {
            writeln!(out, "| {} | {} | {} |", m.id, m.name, m.doc)?;
        }
        writeln!(out)?;
    }

    if !model.messages.is_empty() {
        writeln!(out, "## Message layouts\n")?;
        for dir in [Dir::C2s, Dir::S2c] {
            for m in of_direction(model, dir) {
                writeln!(out, "### {} (id {}, {})\n", m.name, m.id, m.dir.label())?;
                writeln!(out, "{}\n", m.doc)?;
                render_fields_md(&mut out, &m.fields)?;
                writeln!(out)?;
            }
        }
    }

    if !model.struct_order.is_empty() {
        writeln!(out, "## Structs\n")?;
        for name in &model.struct_order {
            if let Some(s) = model.structs.get(name) {
                writeln!(out, "### {}\n", s.name)?;
                writeln!(out, "{}\n", s.doc)?;
                render_fields_md(&mut out, &s.fields)?;
                writeln!(out)?;
            }
        }
    }
    Ok(out.trim_end().to_owned() + "\n")
}

// ---------------------------------------------------------------------------
// Commands

fn profile_sources() -> Result<Vec<PathBuf>> {
    let dir = repo_root().join("crates/m2-protocol/packets");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", display_path(&dir)))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    Ok(paths)
}

/// Every generated file and its expected content.
fn outputs() -> Result<Vec<(PathBuf, String)>> {
    let root = repo_root();
    let mut files = Vec::new();
    for source in profile_sources()? {
        let model = load(&source)?;
        let name = &model.name;
        files.push((
            root.join(format!(
                "crates/m2-protocol/src/profile/{name}/generated.rs"
            )),
            rustfmt(&render_rust(&model)?)?,
        ));
        files.push((
            root.join(format!("docs/specs/protocol/{name}/messages.md")),
            render_markdown(&model)?,
        ));
    }
    Ok(files)
}

/// `just gen`
pub fn write_all() -> Result<()> {
    for (path, content) in outputs()? {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        println!("wrote {}", display_path(&path));
    }
    Ok(())
}

/// `just drift`
pub fn drift() -> Result<()> {
    let mut stale = Vec::new();
    for (path, expected) in outputs()? {
        let current = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if current != expected {
            stale.push(display_path(&path));
        }
    }
    if !stale.is_empty() {
        bail!(
            "generated files are out of date: {}\nrun `just gen` and stage the result",
            stale.join(", ")
        );
    }
    println!("generated files are up to date");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types() {
        assert_eq!(parse_ty("u32").unwrap(), Ty::Prim(Prim::U32));
        assert_eq!(
            parse_ty("[u32; 4]").unwrap(),
            Ty::Array(Box::new(Ty::Prim(Prim::U32)), 4)
        );
        assert_eq!(parse_ty("bool").unwrap(), Ty::Bool);
        assert_eq!(parse_ty("string<64>").unwrap(), Ty::Utf8(64));
        assert_eq!(parse_ty("blob<32>").unwrap(), Ty::Blob(32));
        assert_eq!(
            parse_ty("list<Character; 4>").unwrap(),
            Ty::List(Box::new(Ty::Struct("Character".into())), 4)
        );
        assert_eq!(
            parse_ty("opt<list<u8; 2>>").unwrap(),
            Ty::Opt(Box::new(Ty::List(Box::new(Ty::Prim(Prim::U8)), 2)))
        );
        assert!(parse_ty("string<0>").is_err());
        assert!(parse_ty("u24").is_err());
        // No fixed-width or rest-of-frame types.
        for refused in ["str<25>", "bytes<4>", "tail", "text"] {
            assert!(parse_ty(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn names() {
        assert!(is_pascal("LoginRequest"));
        assert!(!is_pascal("loginRequest"));
        assert!(is_snake("handshake_id"));
        assert!(!is_snake("HandshakeId"));
    }

    fn load_str(toml: &str) -> Result<Model> {
        // Tests run in parallel: give each call its own folder.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("openm2-gen-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("demo.toml");
        std::fs::write(&path, toml)?;
        let model = load(&path);
        std::fs::remove_dir_all(&dir).ok();
        model
    }

    const HEAD: &str = "[profile]\nname = \"demo\"\ndoc = \"Demo.\"\n";

    fn one_field(ty: &str, secret: bool) -> String {
        format!(
            "{HEAD}[[message]]\nname = \"A\"\ndir = \"c2s\"\nid = 1\ndoc = \"A.\"\nfields = [{{ name = \"x\", type = \"{ty}\", doc = \"x\", secret = {secret} }}]\n"
        )
    }

    #[test]
    fn keyword_field_names_are_rejected() {
        let toml = format!(
            "{HEAD}[[message]]\nname = \"Chat\"\ndir = \"s2c\"\nid = 4\ndoc = \"Chat.\"\nfields = [{{ name = \"type\", type = \"u8\", doc = \"t\" }}]\n"
        );
        let err = load_str(&toml).unwrap_err().to_string();
        assert!(err.contains("keyword"), "{err}");
    }

    #[test]
    fn ids_are_unique_per_direction() {
        let toml = format!(
            "{HEAD}[[message]]\nname = \"A\"\ndir = \"c2s\"\nid = 1\ndoc = \"A.\"\n\n[[message]]\nname = \"B\"\ndir = \"s2c\"\nid = 1\ndoc = \"B.\"\n\n[[message]]\nname = \"C\"\ndir = \"c2s\"\nid = 1\ndoc = \"C.\"\n"
        );
        let err = load_str(&toml).unwrap_err().to_string();
        assert!(err.contains("id 1 is already used"), "{err}");
    }

    #[test]
    fn messages_go_one_way() {
        let toml =
            format!("{HEAD}[[message]]\nname = \"A\"\ndir = \"both\"\nid = 1\ndoc = \"A.\"\n");
        let err = format!("{:#}", load_str(&toml).unwrap_err());
        assert!(err.contains("unknown variant `both`"), "{err}");
    }

    #[test]
    fn arrays_need_fixed_items_and_maxima_fit_a_u16() {
        let err = load_str(&one_field("[string<8>; 2]", false))
            .unwrap_err()
            .to_string();
        assert!(err.contains("list<T; N>"), "{err}");
        let err = format!(
            "{:#}",
            load_str(&one_field("string<70000>", false)).unwrap_err()
        );
        assert!(err.contains("fits a u16"), "{err}");
        assert!(load_str(&one_field("list<opt<string<8>>; 3>", false)).is_ok());
    }

    #[test]
    fn arrays_of_structs_that_vary_are_refused() {
        let toml = format!(
            "{HEAD}[[struct]]\nname = \"Named\"\ndoc = \"N.\"\nfields = [{{ name = \"name\", type = \"string<8>\", doc = \"n\" }}]\n\n[[message]]\nname = \"A\"\ndir = \"c2s\"\nid = 1\ndoc = \"A.\"\nfields = [{{ name = \"x\", type = \"[Named; 2]\", doc = \"x\" }}]\n"
        );
        let err = load_str(&toml).unwrap_err().to_string();
        assert!(err.contains("fixed size"), "{err}");
    }

    #[test]
    fn structs_are_declared_before_use() {
        let err = load_str(&one_field("Later", false))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not declared above it"), "{err}");
    }

    #[test]
    fn secrets_are_redacted_and_listed() {
        let toml = format!(
            "{HEAD}[[message]]\nname = \"Login\"\ndir = \"c2s\"\nid = 3\ndoc = \"L.\"\nfields = [{{ name = \"user\", type = \"string<8>\", doc = \"u\" }}, {{ name = \"password\", type = \"string<8>\", doc = \"p\", secret = true }}]\n"
        );
        let model = load_str(&toml).unwrap();
        let rust = render_rust(&model).unwrap();
        assert!(rust.contains("<redacted>"), "{rust}");
        assert!(rust.contains("password: Default::default()"), "{rust}");
        assert!(rust.contains("secrets: &[\"password\"]"), "{rust}");
        assert!(rust.contains("w.string(&self.password, 8)"), "{rust}");
        assert!(rust.contains("r.string(8)?"), "{rust}");
        assert!(!rust.contains("#[derive(Debug, Clone, PartialEq, Eq)]\npub struct Login"));
    }

    #[test]
    fn secret_lists_are_refused() {
        let err = load_str(&one_field("list<u8; 2>", true))
            .unwrap_err()
            .to_string();
        assert!(err.contains("numbers, strings and blobs"), "{err}");
    }

    #[test]
    fn secret_is_not_allowed_in_structs() {
        let toml = format!(
            "{HEAD}[[struct]]\nname = \"Pair\"\ndoc = \"P.\"\nfields = [{{ name = \"key\", type = \"u32\", doc = \"k\", secret = true }}]\n"
        );
        let err = load_str(&toml).unwrap_err().to_string();
        assert!(err.contains("only allowed on message fields"), "{err}");
    }
}

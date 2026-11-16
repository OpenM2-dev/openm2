//! Game file commands: `pack ls|cat|extract`, `vfs ls|cat|check`, `proto dump`.
//!
//! Names print escaped (`\xc7` for bytes outside printable ASCII), and the
//! commands that take a name accept the same escapes, so what `ls` prints can
//! be pasted into `cat`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use m2_assets::{Archive, Source, Vfs, extract, load_keys};
use m2_formats::crc32::crc32;
use m2_formats::{Entry, EntryKind, Keys, Proto};

/// Environment variable read for the keys file when `--keys` is not given.
pub const KEYS_ENV: &str = "OPENM2_KEYS";

/// Where the keys come from.
#[derive(Debug, Args)]
pub struct KeysArg {
    /// keys.toml with your client's keys (default: $OPENM2_KEYS; without
    /// keys only unencrypted data can be read)
    #[arg(long, value_name = "FILE")]
    keys: Option<PathBuf>,
}

impl KeysArg {
    pub(crate) fn load(&self) -> Result<Keys> {
        let path = self
            .keys
            .clone()
            .or_else(|| std::env::var_os(KEYS_ENV).map(PathBuf::from));
        match path {
            Some(path) => Ok(load_keys(&path)?),
            None => Ok(Keys::none()),
        }
    }
}

/// Where binary output goes.
#[derive(Debug, Args)]
pub struct OutArg {
    /// Write to this file instead of stdout
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
}

impl OutArg {
    fn write(&self, data: &[u8]) -> Result<()> {
        match &self.out {
            Some(path) => {
                std::fs::write(path, data).with_context(|| format!("writing {}", path.display()))
            }
            None => {
                let mut stdout = std::io::stdout().lock();
                if stdout.is_terminal() {
                    bail!("refusing to print binary data to a terminal; use --out <file>");
                }
                stdout.write_all(data)?;
                Ok(stdout.flush()?)
            }
        }
    }
}

/// `openm2 pack …`
#[derive(Debug, Subcommand)]
pub enum PackCommand {
    /// List the entries of an archive: type, real size, stored size, name
    Ls {
        /// The archive's .eix file
        index: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Write one decoded entry to --out or stdout
    Cat {
        /// The archive's .eix file
        index: PathBuf,
        /// Entry name as `pack ls` prints it
        name: String,
        #[command(flatten)]
        out: OutArg,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Write the decoded entries under a folder (drive prefixes dropped, unsafe
    /// names skipped, nothing outside the folder)
    Extract {
        /// The archive's .eix file
        index: PathBuf,
        /// Folder to write to; files that already exist stop the extraction
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Only names starting with this (after normalizing)
        #[arg(long)]
        prefix: Option<String>,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// `openm2 vfs …`
#[derive(Debug, Subcommand)]
pub enum VfsCommand {
    /// List every file in the client's archives, sorted by name
    Ls {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        /// Only names starting with this (after normalizing)
        prefix: Option<String>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Write one file, found by game path, to --out or stdout
    Cat {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        /// Game path, e.g. `locale/en/item_proto`
        path: String,
        /// Loose folder searched before the archives (repeatable)
        #[arg(long, value_name = "DIR")]
        loose: Vec<PathBuf>,
        #[command(flatten)]
        out: OutArg,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode every entry of every archive, check the CRCs and parse the proto
    /// tables. Exit code 1 if anything fails.
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// `openm2 proto …`
#[derive(Debug, Subcommand)]
pub enum ProtoCommand {
    /// Print item_proto or mob_proto as JSON Lines, one record per line
    Dump {
        /// A file, or with --client a game path (e.g. `locale/en/item_proto`)
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// Bytes from a name typed on the command line: `\xNN` and `\\` are
/// unescaped, as `ls` prints them.
pub(crate) fn name_bytes(name: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = name.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        rest = tail;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        match rest {
            [b'x', high, low, tail @ ..] => {
                let hex = [*high, *low];
                let text = std::str::from_utf8(&hex).ok();
                let value = text.and_then(|t| u8::from_str_radix(t, 16).ok());
                out.push(value.with_context(|| format!("bad escape in `{name}`"))?);
                rest = tail;
            }
            [b'\\', tail @ ..] => {
                out.push(b'\\');
                rest = tail;
            }
            // A lone backslash is a path separator.
            _ => out.push(b'\\'),
        }
    }
    Ok(out)
}

fn shown(bytes: &[u8]) -> String {
    bytes.escape_ascii().to_string()
}

fn kind_counts<'a>(entries: impl Iterator<Item = &'a Entry>) -> String {
    let mut counts: BTreeMap<EntryKind, usize> = BTreeMap::new();
    for entry in entries {
        *counts.entry(entry.kind).or_default() += 1;
    }
    counts
        .iter()
        .map(|(kind, n)| match kind {
            EntryKind::Unknown(byte) => format!("unknown type {byte}: {n}"),
            kind => format!("{} {n}", kind.name()),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Runs a `pack` command.
pub fn pack(command: PackCommand) -> Result<ExitCode> {
    match command {
        PackCommand::Ls { index, keys } => {
            let archive = Archive::open(&index, &keys.load()?)?;
            let mut out = String::new();
            for entry in archive.entries() {
                let _ = writeln!(
                    out,
                    "{:<10} {:>10}  {}",
                    entry.kind.name(),
                    entry.listed_len(),
                    entry.name
                );
            }
            let _ = writeln!(
                out,
                "{} entries ({})",
                archive.entries().len(),
                kind_counts(archive.entries().iter())
            );
            print!("{out}");
        }
        PackCommand::Cat {
            index,
            name,
            out,
            keys,
        } => {
            let archive = Archive::open(&index, &keys.load()?)?;
            out.write(&archive.read_name(&name_bytes(&name)?)?)?;
        }
        PackCommand::Extract {
            index,
            out,
            prefix,
            keys,
        } => {
            let archive = Archive::open(&index, &keys.load()?)?;
            let prefix = m2_formats::normalize_name(&name_bytes(prefix.as_deref().unwrap_or(""))?);
            let report = extract(&archive, &out, |entry| {
                m2_formats::normalize_name(entry.name.as_bytes()).starts_with(&prefix)
            })?;
            for (name, why) in &report.skipped {
                eprintln!("skipped {name}: {why}");
            }
            println!(
                "wrote {} files ({} bytes) to {}{}",
                report.written,
                report.bytes,
                out.display(),
                match report.skipped.len() {
                    0 => String::new(),
                    n => format!("; skipped {n}"),
                }
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn open_vfs(client: &Path, keys: &Keys) -> Result<Vfs> {
    let vfs = Vfs::open_client(client, keys)?;
    if vfs.archives().is_empty() {
        bail!("no .eix archives in {}", client.join("pack").display());
    }
    Ok(vfs)
}

/// Runs a `vfs` command.
/// Reads a file from disk, or with `client` a game path from its archives.
pub(crate) fn read_input(path: &str, client: Option<&Path>, keys: &KeysArg) -> Result<Vec<u8>> {
    match client {
        Some(client) => {
            let vfs = open_vfs(client, &keys.load()?)?;
            Ok(vfs.read(&name_bytes(path)?)?)
        }
        None => std::fs::read(path).with_context(|| format!("reading {path}")),
    }
}

pub fn vfs(command: VfsCommand) -> Result<ExitCode> {
    match command {
        VfsCommand::Ls {
            client,
            prefix,
            keys,
        } => {
            let vfs = open_vfs(&client, &keys.load()?)?;
            let prefix = m2_formats::normalize_name(&name_bytes(prefix.as_deref().unwrap_or(""))?);
            let mut out = String::new();
            for (name, archive, entry) in
                vfs.files().filter(|(name, _, _)| name.starts_with(&prefix))
            {
                let _ = writeln!(
                    out,
                    "{:<10} {:>10}  {}  {}",
                    entry.kind.name(),
                    entry.listed_len(),
                    shown(name),
                    archive_label(archive)
                );
            }
            print!("{out}");
            if !vfs.shadowed().is_empty() {
                eprintln!(
                    "note: {} names are in more than one archive; the first archive by file name wins",
                    vfs.shadowed().len()
                );
            }
        }
        VfsCommand::Cat {
            client,
            path,
            loose,
            out,
            keys,
        } => {
            let mut vfs = open_vfs(&client, &keys.load()?)?;
            for dir in loose {
                vfs.add_loose_dir(dir);
            }
            let name = name_bytes(&path)?;
            if let Some(Source::Loose(file)) = vfs.source(&name) {
                eprintln!("from loose file {}", file.display());
            }
            out.write(&vfs.read(&name)?)?;
        }
        VfsCommand::Check { client, keys } => return check(&client, &keys.load()?),
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn archive_label(archive: &Archive) -> String {
    archive
        .path()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Problems listed per archive before the rest are only counted.
const SHOWN_PROBLEMS: usize = 10;

/// `vfs check`: decode everything, check CRCs, parse the proto tables.
fn check(client: &Path, keys: &Keys) -> Result<ExitCode> {
    let vfs = open_vfs(client, keys)?;
    let mut problems = 0usize;
    for archive in vfs.archives() {
        let entries = archive.entries();
        println!(
            "{}: {} entries ({})",
            archive_label(archive),
            entries.len(),
            kind_counts(entries.iter())
        );
        let (mut decoded, mut name_ok, mut server_keyed, mut data_crc_ok) = (0, 0, 0, 0);
        let mut failures = Vec::new();
        for entry in entries.iter().filter(|entry| !entry.name.is_empty()) {
            if crc32(entry.name.as_bytes()) == entry.name_crc {
                name_ok += 1;
            }
            if !entry.kind.is_supported() {
                server_keyed += 1;
                continue;
            }
            let stored = match archive.read_stored(entry) {
                Ok(stored) => stored,
                Err(error) => {
                    failures.push(error.to_string());
                    continue;
                }
            };
            if let Err(error) = entry.decode(&stored, keys) {
                failures.push(format!("`{}`: {error}", entry.name));
                continue;
            }
            decoded += 1;
            // The data CRC covers the listed length. Real clients have entries
            // where it matches nothing, so a mismatch is counted, not failed.
            let listed = archive.read_raw(entry, u64::from(entry.listed_len()));
            if listed.is_ok_and(|bytes| crc32(&bytes) == entry.data_crc) {
                data_crc_ok += 1;
            }
        }
        let named = entries
            .iter()
            .filter(|entry| !entry.name.is_empty())
            .count();
        println!(
            "  decoded {decoded}/{}{}; name CRC matches {name_ok}/{named}; data CRC matches {data_crc_ok}/{decoded}",
            named - server_keyed,
            if server_keyed > 0 {
                format!(" ({server_keyed} need server keys, skipped)")
            } else {
                String::new()
            }
        );
        if archive.duplicate_names() > 0 {
            println!(
                "  {} names repeated within the index",
                archive.duplicate_names()
            );
        }
        for failure in failures.iter().take(SHOWN_PROBLEMS) {
            println!("  FAILED {failure}");
        }
        if failures.len() > SHOWN_PROBLEMS {
            println!("  … and {} more", failures.len() - SHOWN_PROBLEMS);
        }
        // CRC mismatches are counted above, not failed: the client reads its
        // files without checking them.
        problems += failures.len();
    }
    if let Some(first) = vfs.shadowed().first() {
        println!(
            "{} names are in more than one archive (e.g. `{}`: used from {}, ignored in {})",
            vfs.shadowed().len(),
            first.name,
            first.used.file_name().unwrap_or_default().to_string_lossy(),
            first
                .ignored
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
        );
    }

    let tables: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| is_proto_name(name))
        .collect();
    if tables.is_empty() {
        println!("no item_proto or mob_proto found");
    }
    for name in tables {
        match vfs
            .read(&name)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(Proto::parse(&bytes, keys)?))
        {
            Ok(Proto::Item(table)) => println!("{}: {} items", shown(&name), table.records.len()),
            Ok(Proto::Mob(table)) => {
                println!("{}: {} monsters", shown(&name), table.records.len());
            }
            Err(error) => {
                problems += 1;
                println!("{}: FAILED {error:#}", shown(&name));
            }
        }
    }

    if problems == 0 {
        println!("ok");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("{problems} problems");
        Ok(ExitCode::FAILURE)
    }
}

fn is_proto_name(name: &[u8]) -> bool {
    let file = name.rsplit(|b| *b == b'/').next().unwrap_or(name);
    file == b"item_proto" || file == b"mob_proto"
}

/// Runs a `proto` command.
pub fn proto(command: ProtoCommand) -> Result<ExitCode> {
    let ProtoCommand::Dump { path, client, keys } = command;
    let keys = keys.load()?;
    let bytes = match &client {
        Some(client) => open_vfs(client, &keys)?.read(&name_bytes(&path)?)?,
        None => std::fs::read(&path).with_context(|| format!("reading {path}"))?,
    };
    let table = Proto::parse(&bytes, &keys).with_context(|| format!("parsing {path}"))?;
    let mut out = std::io::stdout().lock();
    let count = match &table {
        Proto::Item(table) => {
            for record in &table.records {
                serde_json::to_writer(&mut out, record)?;
                writeln!(out)?;
            }
            table.records.len()
        }
        Proto::Mob(table) => {
            for record in &table.records {
                serde_json::to_writer(&mut out, record)?;
                writeln!(out)?;
            }
            table.records.len()
        }
    };
    out.flush()?;
    eprintln!(
        "{count} {}",
        match table {
            Proto::Item(_) => "items",
            Proto::Mob(_) => "monsters",
        }
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_unescape_like_ls_prints_them() {
        assert_eq!(
            name_bytes("a\\xc7\\xD1b").unwrap(),
            [b'a', 0xC7, 0xD1, b'b']
        );
        assert_eq!(name_bytes("d:\\ymir work").unwrap(), b"d:\\ymir work");
        assert_eq!(name_bytes("a\\\\b").unwrap(), b"a\\b");
        assert!(name_bytes("a\\xZZ").is_err());
        let raw = [b'x', 0xB0, b'\\', b'y'];
        assert_eq!(name_bytes(&shown(&raw)).unwrap(), raw);
    }

    #[test]
    fn proto_names() {
        assert!(is_proto_name(b"locale/en/item_proto"));
        assert!(is_proto_name(b"mob_proto"));
        assert!(!is_proto_name(b"locale/en/item_proto.txt"));
    }
}

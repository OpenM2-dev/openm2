//! Writing archive entries to a folder, never outside it.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path};

use m2_formats::{Entry, RawStr};

use crate::archive::Archive;
use crate::error::AssetError;
use crate::path::safe_relative_path;

/// What [`extract`] did.
#[derive(Debug, Default)]
pub struct ExtractReport {
    /// Files written.
    pub written: usize,
    /// Bytes written.
    pub bytes: u64,
    /// Entries not written, and why.
    pub skipped: Vec<(RawStr, String)>,
}

/// Writes every entry `wanted` accepts under `out`.
///
/// - Names go through [`safe_relative_path`]; refused names are skipped and
///   reported, as are entries that cannot be decoded.
/// - Folders are created one level at a time, and an existing link or file
///   where a folder should be stops the extraction: nothing is written
///   through a link.
/// - Files are created new; an existing file stops the extraction. Extract
///   into an empty folder.
pub fn extract(
    archive: &Archive,
    out: &Path,
    wanted: impl Fn(&Entry) -> bool,
) -> Result<ExtractReport, AssetError> {
    fs::create_dir_all(out).map_err(AssetError::io(out))?;
    let mut report = ExtractReport::default();
    for entry in archive.entries() {
        if entry.name.is_empty() || !wanted(entry) {
            continue;
        }
        let relative = match safe_relative_path(entry.name.as_bytes()) {
            Ok(relative) => relative,
            Err(reason) => {
                report
                    .skipped
                    .push((entry.name.clone(), format!("not a safe path: {reason}")));
                continue;
            }
        };
        let data = match archive.read(entry) {
            Ok(data) => data,
            Err(error) => {
                report.skipped.push((entry.name.clone(), error.to_string()));
                continue;
            }
        };
        write_inside(out, &relative, &data)?;
        report.written += 1;
        report.bytes += data.len() as u64;
    }
    Ok(report)
}

/// Creates `root/relative` with `data`. `relative` comes from
/// [`safe_relative_path`]: only normal components.
fn write_inside(root: &Path, relative: &Path, data: &[u8]) -> Result<(), AssetError> {
    let parts: Vec<_> = relative.components().collect();
    let Some((file, folders)) = parts.split_last() else {
        return Err(unsafe_path(relative, "it is empty"));
    };
    let mut path = root.to_path_buf();
    for folder in folders {
        let Component::Normal(name) = folder else {
            return Err(unsafe_path(
                relative,
                "it has a component that is not a name",
            ));
        };
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(unsafe_path(
                    relative,
                    "a folder on the way is a file or a link",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(AssetError::io(&path))?;
            }
            Err(error) => {
                return Err(AssetError::Io {
                    path: path.clone(),
                    cause: error,
                });
            }
        }
    }
    let Component::Normal(name) = file else {
        return Err(unsafe_path(
            relative,
            "it has a component that is not a name",
        ));
    };
    path.push(name);
    let mut handle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(AssetError::io(&path))?;
    handle.write_all(data).map_err(AssetError::io(&path))
}

fn unsafe_path(relative: &Path, reason: &'static str) -> AssetError {
    AssetError::UnsafePath {
        name: RawStr::from(relative.to_string_lossy().as_ref()),
        reason,
    }
}

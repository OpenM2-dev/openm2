//! Where a file goes in a version's folder: relative paths that can't leave
//! it on any system.

use std::fmt;
use std::path::PathBuf;

use crate::UpdateError;

/// The longest path, in bytes.
pub const MAX_PATH: usize = 240;
/// The most parts a path has.
pub const MAX_PARTS: usize = 32;
/// The longest part of a path, in bytes.
pub const MAX_PART: usize = 120;

/// Names Windows keeps for devices, whatever their extension.
const DEVICES: [&str; 24] = [
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A checked path inside a version's folder, its parts split by `/`
/// (`docs/specs/update.md`, "The signed manifest").
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct InstallPath(String);

impl InstallPath {
    /// Checks `path`.
    pub fn new(path: &str) -> Result<Self, UpdateError> {
        check(path).map_err(|why| UpdateError::Path {
            path: crate::shown(path),
            why,
        })?;
        Ok(Self(path.to_owned()))
    }

    /// As written, with `/`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Its parts.
    pub fn parts(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// As a relative path of this system, to join to a folder.
    pub fn to_relative(&self) -> PathBuf {
        self.parts().collect()
    }

    /// The same path in lower case, to compare as Windows and macOS do.
    pub(crate) fn folded(&self) -> String {
        self.0.to_ascii_lowercase()
    }
}

impl fmt::Display for InstallPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which rule `path` breaks, if any.
fn check(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("empty");
    }
    if path.len() > MAX_PATH {
        return Err("longer than 240 bytes");
    }
    if path.split('/').count() > MAX_PARTS {
        return Err("more than 32 parts");
    }
    for part in path.split('/') {
        if part.is_empty() {
            return Err("an empty part (`/` at the start or the end, or twice)");
        }
        if part.len() > MAX_PART {
            return Err("a part longer than 120 bytes");
        }
        if !part.bytes().all(allowed) {
            return Err("a character other than a-z, A-Z, 0-9, space and . _ - + ( )");
        }
        if part.starts_with('.') {
            return Err("a part starting with `.`");
        }
        if part.starts_with(' ') || part.ends_with(' ') || part.ends_with('.') {
            return Err("a part starting or ending with a space, or ending with `.`");
        }
        // Windows ignores spaces before an extension: `con .txt` is `con`.
        let stem = part.split('.').next().unwrap_or(part).trim_end();
        if DEVICES
            .iter()
            .any(|device| stem.eq_ignore_ascii_case(device))
        {
            return Err("a Windows device name");
        }
    }
    Ok(())
}

fn allowed(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'.' | b'_' | b'-' | b'+' | b'(' | b')')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_paths() {
        for path in [
            "openm2.exe",
            "LICENSE-MIT",
            "packs/Forest Trees (v2)/bark_01.dds",
            "a+b/c_d-e.f.g",
            "conifer.txt",
            "com10",
            "lpt",
        ] {
            assert_eq!(InstallPath::new(path).unwrap().as_str(), path);
        }
        let deep = vec!["a"; MAX_PARTS].join("/");
        assert!(InstallPath::new(&deep).is_ok());
        assert!(InstallPath::new(&"a".repeat(MAX_PART)).is_ok());
    }

    #[test]
    fn paths_that_could_leave_are_refused() {
        for (path, says) in [
            ("", "empty"),
            ("/etc/passwd", "empty part"),
            ("a/", "empty part"),
            ("a//b", "empty part"),
            ("../x", "starting with `.`"),
            ("a/../../x", "starting with `.`"),
            ("a/./b", "starting with `.`"),
            (".hidden", "starting with `.`"),
            ("C:/Windows/x", "character"),
            ("C:x", "character"),
            ("a\\b", "character"),
            ("\\\\server\\share", "character"),
            ("file.txt:stream", "character"),
            ("a\0b", "character"),
            ("tab\there", "character"),
            ("é", "character"),
            ("~user", "character"),
            ("a*", "character"),
            ("x.", "ending"),
            ("x ", "ending"),
            (" x", "starting"),
            ("con", "device"),
            ("CON", "device"),
            ("a/Nul.txt", "device"),
            ("aux.tar.gz", "device"),
            ("con .txt", "device"),
            ("COM1", "device"),
            ("lpt9.log", "device"),
            ("com0", "device"),
        ] {
            let error = InstallPath::new(path).unwrap_err().to_string();
            assert!(error.contains(says), "{path:?}: {error}");
        }
        assert!(InstallPath::new(&"a".repeat(MAX_PART + 1)).is_err());
        assert!(InstallPath::new(&vec!["a"; MAX_PARTS + 1].join("/")).is_err());
        assert!(InstallPath::new(&vec!["abcdefghij"; 22].join("/")).is_err());
    }

    #[test]
    fn errors_print_safely() {
        let error = InstallPath::new("a\u{1b}[31mb/..").unwrap_err().to_string();
        assert!(!error.contains('\u{1b}'), "{error}");
    }

    #[test]
    fn joined_for_this_system() {
        let path = InstallPath::new("packs/trees/bark.dds").unwrap();
        assert_eq!(
            path.to_relative(),
            PathBuf::from("packs").join("trees").join("bark.dds")
        );
    }
}

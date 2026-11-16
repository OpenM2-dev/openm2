//! Game paths and the files they map to on disk.
//!
//! Archive names and paths in game data are untrusted bytes. Before one
//! becomes a path on disk (extraction, loose files) it goes through
//! [`safe_relative_path`], which either gives a relative path that stays
//! inside the folder it is joined to, or refuses. The rules are in
//! `docs/specs/formats/eterpack.md` (section "Names on disk").

use std::path::PathBuf;

use m2_formats::normalize_name;

/// Longest name of one path component we create (common file system limit).
const MAX_COMPONENT: usize = 255;

/// Maps a game path to a relative path that is safe to join to a folder.
///
/// - The name is normalized first (lowercase ASCII, `\` → `/`).
/// - A drive prefix is dropped: `d:/ymir work/a.gr2` → `ymir work/a.gr2`.
/// - Refused: absolute paths, `.` and `..`, empty paths, control characters,
///   names ending in `.` or a space, and Windows device names (`con`, `nul`,
///   `com1`, …).
/// - Bytes that are not portable in file names (`%`, `<>:"|?*` and anything
///   outside ASCII) are written as `%XX`, so the mapping stays one-to-one and
///   is the same on every OS.
pub fn safe_relative_path(name: &[u8]) -> Result<PathBuf, &'static str> {
    let name = normalize_name(name);
    let rest: &[u8] = match name.as_slice() {
        [drive, b':', rest @ ..] if drive.is_ascii_alphabetic() => {
            rest.strip_prefix(b"/").unwrap_or(rest)
        }
        other => other,
    };
    if rest.starts_with(b"/") {
        return Err("it is an absolute path");
    }
    let mut path = PathBuf::new();
    for part in rest.split(|b| *b == b'/').filter(|part| !part.is_empty()) {
        if part == b"." || part == b".." {
            return Err("it contains `.` or `..`");
        }
        if part.iter().any(|b| *b < 0x20 || *b == 0x7F) {
            return Err("it contains a control character");
        }
        if part.ends_with(b".") || part.ends_with(b" ") {
            return Err("a name ends in `.` or a space");
        }
        if is_device_name(part) {
            return Err("a name is a Windows device name");
        }
        let escaped = escape(part);
        if escaped.len() > MAX_COMPONENT {
            return Err("a name is longer than 255 bytes");
        }
        path.push(escaped);
    }
    if path.as_os_str().is_empty() {
        return Err("it is empty");
    }
    Ok(path)
}

fn escape(part: &[u8]) -> String {
    part.iter()
        .map(|&b| match b {
            b'%' | b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*' | 0x80..=0xFF => {
                format!("%{b:02X}")
            }
            _ => char::from(b).to_string(),
        })
        .collect()
}

/// Names Windows treats as devices, with or without an extension.
fn is_device_name(part: &[u8]) -> bool {
    let stem = part.split(|b| *b == b'.').next().unwrap_or(part);
    let stem = stem.trim_ascii_end();
    match stem {
        b"con" | b"prn" | b"aux" | b"nul" | b"conin$" | b"conout$" => true,
        [b'c', b'o', b'm', digit] | [b'l', b'p', b't', digit] => digit.is_ascii_digit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(name: &str) -> String {
        safe_relative_path(name.as_bytes())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    }

    #[test]
    fn drive_prefix_is_dropped_and_names_normalized() {
        assert_eq!(ok("d:/ymir work/pc/a.gr2"), "ymir work/pc/a.gr2");
        assert_eq!(ok("D:\\Ymir Work\\PC\\A.GR2"), "ymir work/pc/a.gr2");
        assert_eq!(ok("locale/en/item_proto"), "locale/en/item_proto");
        assert_eq!(ok("a//b"), "a/b");
    }

    #[test]
    fn odd_bytes_are_escaped() {
        assert_eq!(ok("a%b"), "a%25b");
        assert_eq!(ok("x/what?.txt"), "x/what%3F.txt");
        assert_eq!(
            safe_relative_path(&[b'a', 0xC7, 0xD1])
                .unwrap()
                .to_string_lossy(),
            "a%C7%D1"
        );
    }

    #[test]
    fn escapes_out_of_the_folder_are_refused() {
        for name in [
            "../x",
            "a/../../x",
            "./x",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "d:/../x",
            "c:\\..\\x",
            "",
            "d:",
            "d:/",
            "a/\u{1b}[31m",
            "a/b\0c",
            "con",
            "a/NUL.txt",
            "lpt1.log",
            "a/x.",
            "a/x ",
        ] {
            assert!(
                safe_relative_path(name.as_bytes()).is_err(),
                "`{}` should be refused",
                name.escape_debug()
            );
        }
    }

    #[test]
    fn device_like_names_that_are_not_devices_are_fine() {
        assert_eq!(ok("console.txt"), "console.txt");
        assert_eq!(ok("com10"), "com10");
        assert_eq!(ok("nulls/a"), "nulls/a");
    }

    #[test]
    fn long_names_are_refused() {
        let long = "a".repeat(256);
        assert!(safe_relative_path(long.as_bytes()).is_err());
        assert!(safe_relative_path("a".repeat(255).as_bytes()).is_ok());
    }
}

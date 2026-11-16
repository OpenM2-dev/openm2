//! Test support: temporary folders and synthetic clients made with our own
//! writers and test keys.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::{ArchiveWriter, EntryKind, Keys, XteaKey};

/// Our test keys (never a real client's).
pub fn test_keys() -> Keys {
    Keys {
        eterpack_index: Some(XteaKey([
            0x0123_4567,
            0x89AB_CDEF,
            0xFEDC_BA98,
            0x7654_3210,
        ])),
        eterpack_entries: Some(XteaKey([
            0x1111_1111,
            0x2222_2222,
            0x3333_3333,
            0x4444_4444,
        ])),
        item_proto: Some(XteaKey([
            0xA5A5_A5A5,
            0x5A5A_5A5A,
            0x0F0F_0F0F,
            0xF0F0_F0F0,
        ])),
        mob_proto: Some(XteaKey([1, 2, 3, 4])),
    }
}

/// A folder under Cargo's test temp dir, removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "m2-assets-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes `<dir>/<stem>.eix` and `.epk` holding `files`, index encrypted if
/// `encrypt_index`. Returns the `.eix` path.
pub fn write_archive(
    dir: &Path,
    stem: &str,
    files: &[(&str, &[u8], EntryKind)],
    encrypt_index: bool,
) -> PathBuf {
    let keys = test_keys();
    let mut writer = ArchiveWriter::new();
    for (name, data, kind) in files {
        writer.add(name.as_bytes(), data, *kind, &keys).unwrap();
    }
    let (eix, epk) = writer
        .finish(encrypt_index.then_some(keys.eterpack_index.as_ref().unwrap()))
        .unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let index = dir.join(format!("{stem}.eix"));
    std::fs::write(&index, eix).unwrap();
    std::fs::write(dir.join(format!("{stem}.epk")), epk).unwrap();
    index
}

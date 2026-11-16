//! Bundles: how `openm2 release manifest --bundles` cuts a
//! version's files into them, and the requests that fetch the files a
//! launcher needs.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Bundle, FileEntry, Manifest, Place, Sha256};

/// The largest bundle `release manifest --bundles` cuts: 16 MiB. Files of
/// this size or more stay on their own.
pub const BUNDLE_SIZE: u64 = 16 << 20;

/// The largest gap between two files a launcher needs from one bundle that
/// it fetches along with them, rather than asking for each apart: 256 KiB.
pub const RANGE_GAP: u64 = 256 << 10;

/// How `release manifest --bundles` cuts `files`, in the order of their
/// paths, into bundles: for each bundle, the files of its contents by their
/// index in `files`, in the order of their bytes. Each content goes in
/// once, at its first file; empty files and files of `BUNDLE_SIZE` or more
/// stay on their own; a bundle ends when the next content wouldn't fit.
pub fn cut_bundles(files: &[FileEntry]) -> Vec<Vec<usize>> {
    let mut seen = BTreeSet::new();
    let mut bundles: Vec<Vec<usize>> = Vec::new();
    let mut filled = 0u64;
    for (index, file) in files.iter().enumerate() {
        if file.size == 0 || file.size >= BUNDLE_SIZE || !seen.insert((file.size, file.sha256)) {
            continue;
        }
        match bundles.last_mut() {
            Some(last) if filled + file.size <= BUNDLE_SIZE => {
                last.push(index);
                filled += file.size;
            }
            _ => {
                bundles.push(vec![index]);
                filled = file.size;
            }
        }
    }
    bundles
}

/// Gives each of `files` whose content `cut` (from [`cut_bundles`]) put in
/// a bundle its place there, the contents of each bundle one after another
/// from 0; files with the same content get the same place, and the others
/// none.
pub fn place_bundled(files: &mut [FileEntry], cut: &[Vec<usize>]) {
    let mut places = BTreeMap::new();
    for (bundle, indices) in cut.iter().enumerate() {
        let mut offset = 0u64;
        for file in indices.iter().filter_map(|&index| files.get(index)) {
            places.insert((file.size, file.sha256), Place { bundle, offset });
            offset += file.size;
        }
    }
    for file in files {
        file.place = places.get(&(file.size, file.sha256)).copied();
    }
}

/// One request of a launcher's: an object of the channel (`files/<sha256>`,
/// a file's or a bundle's), whole or a range of its bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Request<'a> {
    /// The object's SHA-256.
    pub object: Sha256,
    /// The bytes asked for: the first, and the one after the last. `None`
    /// asks for the whole object.
    pub range: Option<(u64, u64)>,
    /// How many bytes come back.
    pub size: u64,
    /// The contents it brings, in the order of their bytes.
    pub parts: Vec<Part<'a>>,
}

/// One content a request brings.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Part<'a> {
    /// Where its bytes start in what comes back.
    pub at: u64,
    /// The files with this content, at least one: the first is written, and
    /// the others are linked to it.
    pub files: Vec<&'a FileEntry>,
}

/// The requests that fetch `needed`, files of `manifest`. Each content is
/// fetched once, for every needed file with it. A file on its own is a
/// request of its own. Bundled files come by bundle: neighbours, and files
/// less than [`RANGE_GAP`] apart, in one request for a range, or for the
/// whole bundle when that range is all of it. Bundles first, in their
/// order, then the files on their own, in the order `needed` names them.
pub fn requests<'a>(manifest: &'a Manifest, needed: &[&'a FileEntry]) -> Vec<Request<'a>> {
    // Each content's needed files, in the order they first come.
    let mut contents: Vec<Vec<&FileEntry>> = Vec::new();
    let mut by_content: BTreeMap<(u64, Sha256), usize> = BTreeMap::new();
    for &file in needed {
        let key = (file.size, file.sha256);
        if let Some(files) = by_content
            .get(&key)
            .and_then(|&index| contents.get_mut(index))
        {
            files.push(file);
        } else {
            by_content.insert(key, contents.len());
            contents.push(vec![file]);
        }
    }
    let mut in_bundles: BTreeMap<usize, Vec<(u64, Vec<&FileEntry>)>> = BTreeMap::new();
    let mut alone = Vec::new();
    for files in contents {
        let place = files
            .iter()
            .find_map(|file| file.place)
            .filter(|place| manifest.bundles().get(place.bundle).is_some());
        match place {
            Some(place) => in_bundles
                .entry(place.bundle)
                .or_default()
                .push((place.offset, files)),
            None => alone.push(files),
        }
    }
    let mut requests = Vec::new();
    for (index, mut list) in in_bundles {
        let Some(bundle) = manifest.bundles().get(index) else {
            continue;
        };
        list.sort_by_key(|&(offset, _)| offset);
        let mut run = Vec::new();
        let (mut start, mut end) = (0, 0);
        for (offset, files) in list {
            if !run.is_empty() && offset.saturating_sub(end) >= RANGE_GAP {
                requests.push(ranged(bundle, start, end, std::mem::take(&mut run)));
            }
            if run.is_empty() {
                (start, end) = (offset, offset);
            }
            let size = files.first().map_or(0, |file| file.size);
            end = end.max(offset.saturating_add(size));
            run.push((offset, files));
        }
        if !run.is_empty() {
            requests.push(ranged(bundle, start, end, run));
        }
    }
    for files in alone {
        let Some(first) = files.first() else { continue };
        requests.push(Request {
            object: first.sha256,
            range: None,
            size: first.size,
            parts: vec![Part { at: 0, files }],
        });
    }
    requests
}

/// The request for the bytes `start..end` of `bundle`, which bring `run`'s
/// contents, from their offsets.
fn ranged<'a>(
    bundle: &Bundle,
    start: u64,
    end: u64,
    run: Vec<(u64, Vec<&'a FileEntry>)>,
) -> Request<'a> {
    Request {
        object: bundle.sha256,
        range: (start != 0 || end != bundle.size).then_some((start, end)),
        size: end.saturating_sub(start),
        parts: run
            .into_iter()
            .map(|(offset, files)| Part {
                at: offset.saturating_sub(start),
                files,
            })
            .collect(),
    }
}

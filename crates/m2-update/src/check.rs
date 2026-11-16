//! What a launcher makes of a manifest it has opened: whether it counts, and
//! where each of its files can come from.

use std::collections::BTreeMap;

use crate::{FileEntry, InstallPath, Manifest, Sha256, Timestamp, UpdateError};

/// What a manifest that counts means for the installed version.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// The installed version's own sequence: nothing to do.
    Current,
    /// Newer, with the same files (wherever their bytes would be fetched
    /// from): only the installed manifest is replaced (a manifest signed
    /// again to last longer, or bundled another way).
    Renewed,
    /// Newer, or nothing is installed: an update.
    Update,
}

/// Checks a manifest that has been opened (its signature checked) against
/// the channel asked for, the time `now` and the installed version's
/// manifest. An installed manifest of another channel counts as nothing
/// installed, so a launcher can move to another channel.
pub fn check(
    manifest: &Manifest,
    channel: &str,
    now: Timestamp,
    installed: Option<&Manifest>,
) -> Result<Verdict, UpdateError> {
    if manifest.channel() != channel {
        return Err(UpdateError::OtherChannel {
            expected: crate::shown(channel),
            found: manifest.channel().to_owned(),
        });
    }
    if now >= manifest.expires() {
        return Err(UpdateError::Expired {
            at: manifest.expires(),
        });
    }
    let Some(installed) = installed.filter(|installed| installed.channel() == channel) else {
        return Ok(Verdict::Update);
    };
    Ok(match manifest.sequence().cmp(&installed.sequence()) {
        std::cmp::Ordering::Less => {
            return Err(UpdateError::Older {
                found: manifest.sequence(),
                installed: installed.sequence(),
            });
        }
        std::cmp::Ordering::Equal => Verdict::Current,
        std::cmp::Ordering::Greater if manifest.same_files(installed) => Verdict::Renewed,
        std::cmp::Ordering::Greater => Verdict::Update,
    })
}

/// Where a file of a new version can come from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// The installed version has a file with the same size and SHA-256 here.
    Installed(InstallPath),
    /// The version before it has one here.
    Previous(InstallPath),
    /// Nowhere: it's downloaded.
    Download,
}

/// One file of a new version and where to take it from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step<'a> {
    /// The file.
    pub file: &'a FileEntry,
    /// Where it can come from, by the manifests: whoever takes it checks its
    /// size and SHA-256 again.
    pub source: Source,
}

/// Where each file of `new` can come from: a file with the same size and
/// SHA-256 in the installed version first, then in the one before it,
/// whatever its path, else a download. In `new`'s order.
pub fn plan<'a>(
    new: &'a Manifest,
    installed: Option<&Manifest>,
    previous: Option<&Manifest>,
) -> Vec<Step<'a>> {
    let index = |manifest: Option<&Manifest>| {
        let mut by_content: BTreeMap<(u64, Sha256), InstallPath> = BTreeMap::new();
        for file in manifest.map(Manifest::files).unwrap_or_default() {
            by_content
                .entry((file.size, file.sha256))
                .or_insert_with(|| file.path.clone());
        }
        by_content
    };
    let installed = index(installed);
    let previous = index(previous);
    new.files()
        .iter()
        .map(|file| {
            let key = (file.size, file.sha256);
            let source = if let Some(path) = installed.get(&key) {
                Source::Installed(path.clone())
            } else if let Some(path) = previous.get(&key) {
                Source::Previous(path.clone())
            } else {
                Source::Download
            };
            Step { file, source }
        })
        .collect()
}

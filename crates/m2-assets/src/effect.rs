//! Effects in the client: where the files an effect script names are, and
//! which item script a weapon's hit effects come from
//! (`docs/specs/formats/effect.md`).
//!
//! Effect scripts name their textures and mesh files by a bare file name
//! beside the script (`yeongi.dds`), by a full game path (`D:\YMIR
//! WORK\effect\hit\blow_1\yeongi.dds`), or by a path that lost its `ymir
//! work` (`D:\pc\assassin\effect\energyup.mde`). Mesh files name their
//! textures as the modelling tool saved them, often a `.jpg` or `.tga` with a
//! `.dds` of the same name beside it in the client.

use m2_formats::RawStr;
use m2_formats::normalize_name;
use m2_formats::script::effect::EffectData;
use m2_formats::script::fly::FlyingData;
use m2_formats::script::item_list::ItemLine;

use crate::{AssetError, Vfs, texture_paths};

/// The game paths to try for `name`, a texture that the effect script or
/// mesh file at game path `owner` names, best first: the name as written,
/// then its file name in the owner's folder (as for model textures,
/// [`texture_paths`]); after each, the same name ending in `.dds` if it ends
/// otherwise.
pub fn effect_texture_paths(owner: &[u8], name: &[u8]) -> Vec<Vec<u8>> {
    let mut paths = Vec::new();
    for path in texture_paths(owner, name) {
        let dds = as_dds(&path);
        paths.push(path);
        if let Some(dds) = dds
            && !paths.contains(&dds)
        {
            paths.push(dds);
        }
    }
    paths
}

/// `path` with its extension replaced by `.dds`, unless it is one already
/// or has no extension.
fn as_dds(path: &[u8]) -> Option<Vec<u8>> {
    let dot = path.iter().rposition(|b| *b == b'.')?;
    if path.iter().skip(dot).any(|b| *b == b'/') {
        return None;
    }
    let stem = path.get(..dot)?;
    let extension = path.get(dot..)?;
    if extension.eq_ignore_ascii_case(b".dds") {
        return None;
    }
    Some([stem, b".dds"].concat())
}

/// The first of [`effect_texture_paths`] that the VFS holds.
pub fn find_effect_texture(vfs: &Vfs, owner: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    effect_texture_paths(owner, name)
        .into_iter()
        .find(|path| vfs.source(path).is_some())
}

/// Where the mesh file (`.mde`) that the effect script at game path
/// `script` names is: the name as written, else its file name beside the
/// script.
pub fn find_effect_mesh(vfs: &Vfs, script: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    texture_paths(script, name)
        .into_iter()
        .find(|path| vfs.source(path).is_some())
}

/// Where an effect script (`.mse`) that the flying effect at game path `fly`
/// names is (`effect.md`, "Flying effects"): the name as written, else its
/// file name beside the flying effect, as most write only a file name.
pub fn find_fly_effect(vfs: &Vfs, fly: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    find_effect_mesh(vfs, fly, name)
}

/// Reads and parses the flying effect script at game path `path`.
pub fn read_flying(vfs: &Vfs, path: &[u8]) -> Result<FlyingData, AssetError> {
    let bytes = vfs.read(path)?;
    FlyingData::parse(&bytes).map_err(|cause| AssetError::Parse {
        name: RawStr::from(path),
        cause,
    })
}

/// Reads and parses the effect script at game path `path`.
pub fn read_effect(vfs: &Vfs, path: &[u8]) -> Result<EffectData, AssetError> {
    let bytes = vfs.read(path)?;
    EffectData::parse(&bytes).map_err(|cause| AssetError::Parse {
        name: RawStr::from(path),
        cause,
    })
}

/// The game path of the item script of an item drawn with `entry`'s model
/// (openm2's reading, `effect.md`, "Hit effects"): the model's file name with
/// `.msm` in the folder above the model's, so
/// `d:/ymir work/item/weapon/00010.gr2` has `d:/ymir work/item/00010.msm`.
/// None without a model.
pub fn item_script_path(entry: &ItemLine) -> Option<Vec<u8>> {
    let model = normalize_name(entry.model.as_ref()?.as_bytes());
    let file_start = model.iter().rposition(|b| *b == b'/')?;
    let folder = model.get(..file_start)?;
    let parent_end = folder.iter().rposition(|b| *b == b'/')?;
    let parent = model.get(..=parent_end)?;
    let file = model.get(file_start + 1..)?;
    let dot = file.iter().rposition(|b| *b == b'.').unwrap_or(file.len());
    let stem = file.get(..dot)?;
    Some([parent, stem, b".msm"].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_textures_are_looked_for_as_written_then_beside_the_owner() {
        let script = b"d:/ymir work/effect/hit/blow_1/blow_1.mse";
        assert_eq!(
            effect_texture_paths(script, b"yeongi.dds"),
            [
                b"yeongi.dds".to_vec(),
                b"d:/ymir work/effect/hit/blow_1/yeongi.dds".to_vec()
            ]
        );
        assert_eq!(
            effect_texture_paths(script, b"D:\\pc\\assassin\\effect\\energyup.mde"),
            [
                b"d:/pc/assassin/effect/energyup.mde".to_vec(),
                b"d:/pc/assassin/effect/energyup.dds".to_vec(),
                b"d:/ymir work/effect/hit/blow_1/energyup.mde".to_vec(),
                b"d:/ymir work/effect/hit/blow_1/energyup.dds".to_vec(),
            ]
        );
        let mesh = b"d:/ymir work/effect/etc/click/click_select.mde";
        assert_eq!(
            effect_texture_paths(
                mesh,
                b"D:\\Ymir Work\\effect\\etc\\click\\click_select copy.jpg"
            ),
            [
                b"d:/ymir work/effect/etc/click/click_select copy.jpg".to_vec(),
                b"d:/ymir work/effect/etc/click/click_select copy.dds".to_vec(),
            ]
        );
    }

    #[test]
    fn a_weapons_item_script_is_named_after_its_model() {
        let entry = |model: Option<&str>| ItemLine {
            model: model.map(Into::into),
            ..ItemLine::default()
        };
        assert_eq!(
            item_script_path(&entry(Some("d:/ymir work/item/weapon/00010.gr2"))),
            Some(b"d:/ymir work/item/00010.msm".to_vec())
        );
        assert_eq!(item_script_path(&entry(Some("00010.gr2"))), None);
        assert_eq!(item_script_path(&entry(None)), None);
    }
}

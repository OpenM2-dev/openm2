//! Where a model's textures are, and the image a sub-image cuts from.
//!
//! GR2 files name their textures by the path on the artist's machine, such as
//! `D:\ymir work\pc\warrior\warrior_4-1.dds`. Most of those paths are game
//! paths as they stand. The others (`E:\metin2data\…`) are found by their
//! file name in the model's folder.

use m2_formats::normalize_name;
use m2_formats::script::sub::SubImage;

use crate::Vfs;

/// The game paths to try for `texture`, a texture a model at game path
/// `model` names, best first: the name as written (normalized), then its file
/// name in the model's folder.
pub fn texture_paths(model: &[u8], texture: &[u8]) -> Vec<Vec<u8>> {
    let written = normalize_name(texture);
    let file_name = written.rsplit(|b| *b == b'/').next().unwrap_or(&written);
    let model = normalize_name(model);
    let folder = model
        .iter()
        .rposition(|b| *b == b'/')
        .and_then(|end| model.get(..=end))
        .unwrap_or_default();
    let beside = [folder, file_name].concat();
    let mut paths = vec![written.clone()];
    if beside != written {
        paths.push(beside);
    }
    paths
}

/// The folder version-1.0 sub-images name their image in.
pub const UI_FOLDER: &[u8] = b"d:/ymir work/ui/";

/// The game path of the image a sub-image at game path `sub_path` cuts from
/// (`docs/specs/formats/script.md`, "Sub-images"): version 1.0 names a file
/// in [`UI_FOLDER`], version 2.0 a file in the sub-image's own folder.
pub fn sub_image_path(sub_path: &[u8], sub: &SubImage) -> Vec<u8> {
    let image = normalize_name(sub.image.as_bytes());
    let folder = if sub.version.as_bytes() == b"1.0" {
        UI_FOLDER.to_vec()
    } else {
        let path = normalize_name(sub_path);
        let end = path.iter().rposition(|b| *b == b'/').map_or(0, |at| at + 1);
        path.get(..end).unwrap_or_default().to_vec()
    };
    [folder, image].concat()
}

/// The first of [`texture_paths`] that the VFS holds.
pub fn find_texture(vfs: &Vfs, model: &[u8], texture: &[u8]) -> Option<Vec<u8>> {
    texture_paths(model, texture)
        .into_iter()
        .find(|path| vfs.source(path).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_written_path_comes_first_then_the_models_folder() {
        assert_eq!(
            texture_paths(
                b"d:/ymir work/pc/warrior/warrior_4-1.gr2",
                b"D:\\Ymir Work\\pc\\warrior\\warrior_face.DDS"
            ),
            [b"d:/ymir work/pc/warrior/warrior_face.dds".to_vec()]
        );
        assert_eq!(
            texture_paths(
                b"D:/Ymir Work/monster/wolf/wolf.gr2",
                b"E:\\metin2data\\monster\\wolf.dds"
            ),
            [
                b"e:/metin2data/monster/wolf.dds".to_vec(),
                b"d:/ymir work/monster/wolf/wolf.dds".to_vec()
            ]
        );
    }

    #[test]
    fn names_without_folders_work() {
        assert_eq!(
            texture_paths(b"model.gr2", b"skin.dds"),
            [b"skin.dds".to_vec()]
        );
        assert_eq!(
            texture_paths(b"pc/model.gr2", b"skin.dds"),
            [b"skin.dds".to_vec(), b"pc/skin.dds".to_vec()]
        );
    }

    #[test]
    fn version_1_sub_images_cut_from_the_ui_folder_version_2_from_their_own() {
        let sub = |version: &str, image: &str| SubImage {
            version: version.into(),
            image: image.into(),
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        assert_eq!(
            sub_image_path(
                b"d:/ymir work/ui/public/close_button_01.sub",
                &sub("1.0", "Public.dds")
            ),
            b"d:/ymir work/ui/public.dds"
        );
        assert_eq!(
            sub_image_path(
                b"metin2_map_milgyo/atlas.sub",
                &sub("1.0", "metin2_map_milgyo_atlas.dds")
            ),
            b"d:/ymir work/ui/metin2_map_milgyo_atlas.dds"
        );
        assert_eq!(
            sub_image_path(
                b"D:/Ymir Work/UILoading/catacomb/loading0.sub",
                &sub("2.0", "Loading0.JPG")
            ),
            b"d:/ymir work/uiloading/catacomb/loading0.jpg"
        );
    }
}

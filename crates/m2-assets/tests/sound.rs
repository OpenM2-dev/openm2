//! Sounds in a synthetic client: sound scripts beside motions, music in the
//! `BGM` folder and the archives, and the volumes of `metin2.cfg`.

mod support;

use m2_assets::sound::{
    SoundSettings, music_names, read_music, read_sound_settings, read_sounds_of, sound_script_path,
};
use m2_assets::{AssetError, Vfs};
use m2_formats::EntryKind;
use m2_formats::script::sound::{SoundScript, TimedSound};
use support::{TempDir, test_keys, write_archive};

fn client() -> (TempDir, Vfs) {
    let tmp = TempDir::new("sound");
    let run = SoundScript {
        sounds: vec![TimedSound {
            time: 0.5,
            file: "sound/common/walk_dirt_n.wav".into(),
        }],
    }
    .write()
    .unwrap();
    let files: Vec<(&str, &[u8], EntryKind)> = vec![
        ("sound/pc/warrior/general/run.mss", &run, EntryKind::Lzo),
        (
            "sound/pc/warrior/general/wait.mss",
            b"ScriptType Other\r\n",
            EntryKind::Lzo,
        ),
        ("bgm/login_window.mp3", b"archive music", EntryKind::Stored),
        ("bgm/m2bg.mp3", b"archive m2bg", EntryKind::Stored),
    ];
    write_archive(&tmp.path().join("pack"), "sound", &files, true);
    let music = tmp.path().join("BGM");
    std::fs::create_dir_all(&music).unwrap();
    std::fs::write(music.join("M2BG.mp3"), b"loose m2bg").unwrap();
    std::fs::write(music.join("desert.mp3"), b"loose desert").unwrap();
    std::fs::write(music.join(".hidden.mp3"), b"skipped").unwrap();
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    (tmp, vfs)
}

#[test]
fn motions_find_their_sound_scripts() {
    let (_tmp, vfs) = client();
    assert_eq!(
        sound_script_path(b"d:/ymir work/pc/warrior/general/run.msa").unwrap(),
        b"sound/pc/warrior/general/run.mss"
    );
    let script = read_sounds_of(&vfs, b"d:/ymir work/pc/warrior/general/run.msa")
        .unwrap()
        .unwrap();
    assert_eq!(script.sounds[0].time, 0.5);
    // No script: nothing. A broken one: an error naming it.
    assert!(read_sounds_of(&vfs, b"d:/ymir work/pc/warrior/general/walk.msa").is_none());
    let error = read_sounds_of(&vfs, b"d:/ymir work/pc/warrior/general/wait.msa")
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("wait.mss"), "{error}");
}

#[test]
fn music_comes_from_the_bgm_folder_then_the_archives() {
    let (tmp, vfs) = client();
    // The loose file wins, whatever its case.
    assert_eq!(
        read_music(tmp.path(), &vfs, "m2bg.mp3").unwrap(),
        b"loose m2bg"
    );
    assert_eq!(
        read_music(tmp.path(), &vfs, "desert.mp3").unwrap(),
        b"loose desert"
    );
    assert_eq!(
        read_music(tmp.path(), &vfs, "login_window.mp3").unwrap(),
        b"archive music"
    );
    for name in ["missing.mp3", "../pack/sound.eix", "", ".hidden.mp3"] {
        assert!(
            matches!(
                read_music(tmp.path(), &vfs, name),
                Err(AssetError::NotFound(_))
            ),
            "{name}"
        );
    }
    assert_eq!(
        music_names(tmp.path(), &vfs),
        ["desert.mp3", "login_window.mp3", "m2bg.mp3"]
    );
}

#[test]
fn the_original_clients_volumes_are_read() {
    let settings = SoundSettings::parse(
        b"WIDTH\t\t\t1920\r\nMUSIC_VOLUME\t\t0.205\r\nVOICE_VOLUME\t1\r\n\r\n",
    )
    .unwrap();
    assert_eq!(settings.music, Some(0.205));
    assert_eq!(settings.sounds, Some(1.0));
    // Out of range is clamped; not a number is refused; missing is none.
    assert_eq!(
        SoundSettings::parse(b"MUSIC_VOLUME 4\n").unwrap().music,
        Some(1.0)
    );
    assert!(SoundSettings::parse(b"VOICE_VOLUME loud\n").is_err());
    assert!(SoundSettings::parse(b"MUSIC_VOLUME\n").is_err());
    assert_eq!(SoundSettings::parse(b"").unwrap(), SoundSettings::default());

    let tmp = TempDir::new("settings");
    assert_eq!(read_sound_settings(tmp.path()).unwrap(), None);
    std::fs::write(tmp.path().join("metin2.cfg"), b"MUSIC_VOLUME\t0.5\r\n").unwrap();
    assert_eq!(
        read_sound_settings(tmp.path()).unwrap().unwrap().music,
        Some(0.5)
    );
}

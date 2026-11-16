//! `openm2 sound info|wav|check` and the sound lines of `script check` on a
//! synthetic client: WAV sounds and a sound script in an archive, music in
//! the archive and in the `BGM` folder (silent MP3 frames written by hand),
//! a map's sound spot with its ambience property, and `metin2.cfg`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::map::ambience::{AmbienceSpot, AreaAmbience};
use m2_formats::map::property::Property;
use m2_formats::script::sound::{SoundScript, TimedSound};
use m2_formats::wav::{self, Wave};
use m2_formats::{ArchiveWriter, EntryKind, Keys, RawStr};

/// A temp folder, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-sounds-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn openm2(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openm2"))
        .args(args)
        .env_remove("RUST_LOG")
        .env_remove("OPENM2_KEYS")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// Half a second of a 22,050 Hz mono WAV sound.
fn wav_sound() -> Vec<u8> {
    let samples: Vec<i16> = (0..11_025)
        .map(|n| if n % 50 < 25 { 8_000 } else { -8_000 })
        .collect();
    wav::write(&Wave {
        channels: 1,
        sample_rate: 22_050,
        bits: 16,
        data: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
    })
    .unwrap()
}

/// `count` silent MPEG-1 layer III frames: 128 kbit/s, 44.1 kHz, mono, 417
/// bytes each (`FF FB 90 C0`, then zeros).
fn silent_mp3(count: usize) -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
    frame.repeat(count)
}

fn client(dir: &Dir, broken: bool) -> String {
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    let script = SoundScript {
        sounds: vec![
            TimedSound {
                time: 0.25,
                file: "sound/common/step.wav".into(),
            },
            TimedSound {
                time: 0.75,
                file: "sound/common/gone.wav".into(),
            },
        ],
    };
    let property = Property {
        id: 42,
        values: vec![
            (
                RawStr::from("ambiencesoundvector"),
                RawStr::from("sound/ambience/water.wav"),
            ),
            (
                RawStr::from("maxvolumeareapercentage"),
                RawStr::from("0.300000"),
            ),
            (RawStr::from("playtype"), RawStr::from("LOOP")),
            (RawStr::from("propertytype"), RawStr::from("Ambience")),
        ],
    };
    let spots = AreaAmbience {
        spots: vec![AmbienceSpot {
            position: [1_000.0, -2_000.0, 0.0],
            property: 42,
            range: 5_000.0,
            full_volume: 0.25,
        }],
    };
    let mut files = vec![
        ("sound/pc/test/run.mss", script.write().unwrap()),
        ("sound/common/step.wav", wav_sound()),
        ("sound/ambience/water.wav", wav_sound()),
        ("bgm/m2bg.mp3", silent_mp3(40)),
        ("property/test/water.pra", property.write().unwrap()),
        (
            "metin2_map_test/000000/areaambiencedata.txt",
            spots.write().unwrap(),
        ),
    ];
    if broken {
        files.push(("sound/common/broken.wav", b"RIFF\x04\0\0\0WAVE".to_vec()));
    }
    for (name, bytes) in files {
        pack.add(name.as_bytes(), &bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/sound.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/sound.epk"), epk).unwrap();
    std::fs::create_dir_all(dir.0.join("client/BGM")).unwrap();
    std::fs::write(dir.0.join("client/BGM/Desert.mp3"), silent_mp3(60)).unwrap();
    std::fs::write(
        dir.0.join("client/metin2.cfg"),
        b"WIDTH\t\t1280\r\nMUSIC_VOLUME\t\t0.25\r\nVOICE_VOLUME\t\t0.8\r\n",
    )
    .unwrap();
    dir.path("client")
}

#[test]
fn sound_check_decodes_every_sound_and_finds_the_spots_sounds() {
    let dir = Dir::new("check");
    let client = client(&dir, false);
    let output = openm2(&["sound", "check", "--client", &client]);
    let out = stdout(&output);
    assert!(output.status.success(), "{out}{}", stderr(&output));
    assert!(
        out.contains("sounds: 3 files (MP3 1, WAV 2), decoded 3; longest "),
        "{out}"
    );
    assert!(
        out.contains("  22050 Hz mono 2, 44100 Hz mono 1\n"),
        "{out}"
    );
    assert!(out.contains("music: 2 files, "), "{out}");
    assert!(
        out.contains("volumes in metin2.cfg: music 0.25, sounds 0.8\n"),
        "{out}"
    );
    assert!(
        out.contains(
            "sound spots: 1 in 1 files; 1 with an ambience property, played LOOP 1; sounds all found\n"
        ),
        "{out}"
    );
}

#[test]
fn sound_check_fails_on_a_sound_that_does_not_decode() {
    let dir = Dir::new("broken");
    let client = client(&dir, true);
    let output = openm2(&["sound", "check", "--client", &client]);
    let out = stdout(&output);
    assert!(!output.status.success(), "{out}");
    assert!(
        out.contains("1 failed:\n  sound/common/broken.wav: "),
        "{out}"
    );
}

#[test]
fn script_check_reads_sound_scripts_and_follows_their_files() {
    let dir = Dir::new("scripts");
    let client = client(&dir, false);
    let output = openm2(&["script", "check", "--client", &client]);
    let out = stdout(&output);
    assert!(output.status.success(), "{out}{}", stderr(&output));
    assert!(
        out.contains(
            "sound scripts: 1 scripts, 2 sounds (at most 2 in one, the latest 0.750 s in); 0 beside a motion script, 0 beside an effect script\n"
        ),
        "{out}"
    );
    assert!(
        out.contains(
            "  sound files named: 2 (\"wav\" 2); 1 missing (e.g. sound/common/gone.wav in sound/pc/test/run.mss)\n"
        ),
        "{out}"
    );
}

#[test]
fn sound_info_and_wav_describe_and_convert_sounds() {
    let dir = Dir::new("info");
    let client = client(&dir, false);
    let output = openm2(&[
        "sound",
        "info",
        "--client",
        &client,
        "sound/common/step.wav",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "WAV (PCM), 22050 Hz, mono, 11025 frames, 0.500 s, peak 0.244\n"
    );

    // Music from the archive, decoded whole into a WAV file.
    let out = dir.path("music.wav");
    let output = openm2(&[
        "sound",
        "wav",
        "--client",
        &client,
        "bgm/m2bg.mp3",
        "--out",
        &out,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let wave = wav::parse(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(
        (wave.channels, wave.sample_rate, wave.bits),
        (1, 44_100, 16)
    );
    assert!(wave.frames() >= 30 * 1152, "{}", wave.frames());
    assert!(wave.samples().all(|s| s == 0.0));

    // A file on disk; something that is not a sound.
    let output = openm2(&["sound", "info", &dir.path("music.wav")]);
    assert!(stdout(&output).starts_with("WAV (PCM), 44100 Hz, mono, "));
    std::fs::write(dir.path("text.wav"), b"not a sound").unwrap();
    let output = openm2(&["sound", "info", &dir.path("text.wav")]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("not a sound openm2 reads"),
        "{}",
        stderr(&output)
    );
}

//! Sounds: WAV files byte for byte, sound scripts (`.mss`), the areas' sound
//! spots (`areaambiencedata.txt`) and `Ambience` properties. Files are built
//! by our writers; the bytes are written out by hand.

mod common;

use common::hex;
use m2_formats::FormatError;
use m2_formats::map::ambience::{AmbienceSpot, AreaAmbience};
use m2_formats::map::property::{Ambience, PlayType, Property};
use m2_formats::script::sound::{self, SoundScript, TimedSound};
use m2_formats::wav::{self, Wave};
use proptest::prelude::*;

fn mono(samples: &[i16]) -> Wave {
    Wave {
        channels: 1,
        sample_rate: 22_050,
        bits: 16,
        data: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
    }
}

#[test]
fn a_wav_file_byte_for_byte() {
    let wave = mono(&[0, 16_384, -32_768]);
    let mut expected = b"RIFF".to_vec();
    // 4 ("WAVE") + 24 (fmt) + 8 + 6 (data).
    expected.extend(hex("2a000000"));
    expected.extend(b"WAVE");
    expected.extend(b"fmt ");
    // 16 bytes: PCM, one channel, 22,050 Hz, 44,100 bytes a second, two
    // bytes a frame, 16 bits.
    expected.extend(hex("10000000 0100 0100 22560000 44ac0000 0200 1000"));
    expected.extend(b"data");
    expected.extend(hex("06000000 0000 0040 0080"));
    assert_eq!(expected.len(), 50);
    let bytes = wav::write(&wave).unwrap();
    assert_eq!(bytes, expected);
    let read = wav::parse(&bytes).unwrap();
    assert_eq!(read, wave);
    assert_eq!(read.frames(), 3);
    assert_eq!(read.samples().collect::<Vec<_>>(), [0.0, 0.5, -1.0]);
    assert!((read.duration() - 3.0 / 22_050.0).abs() < 1e-12);

    // Chunks are padded to an even length: three 8-bit samples.
    let odd = Wave {
        bits: 8,
        data: vec![128, 255, 0],
        ..wave
    };
    let bytes = wav::write(&odd).unwrap();
    assert_eq!(&bytes[4..8], hex("28000000"));
    assert_eq!(&bytes[32..36], hex("0100 0800"));
    assert_eq!(&bytes[40..], hex("03000000 80ff00 00"));
    assert_eq!(wav::parse(&bytes).unwrap(), odd);
}

#[test]
fn other_chunks_are_skipped_and_8_bit_stereo_reads() {
    let mut body = b"WAVE".to_vec();
    // A chunk before `fmt ` and an odd one between `fmt ` and `data`, padded.
    body.extend(b"JUNK");
    body.extend(hex("04000000 01020304"));
    body.extend(b"fmt ");
    body.extend(hex("12000000 0100 0200 44ac0000 88580100 0200 0800 0000"));
    body.extend(b"cue ");
    body.extend(hex("03000000 aabbcc 00"));
    body.extend(b"data");
    body.extend(hex("04000000 80ff0040"));
    body.extend(b"LIST");
    body.extend(hex("00000000"));
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((body.len() as u32).to_le_bytes());
    bytes.extend(body);
    let wave = wav::parse(&bytes).unwrap();
    assert_eq!((wave.channels, wave.sample_rate, wave.bits), (2, 44_100, 8));
    assert_eq!(wave.frames(), 2);
    assert_eq!(
        wave.samples().collect::<Vec<_>>(),
        [0.0, 127.0 / 128.0, -1.0, -0.5]
    );
}

#[test]
fn damaged_and_unknown_wav_files_are_refused() {
    let good = wav::write(&mono(&[1, 2])).unwrap();
    let with = |at: usize, bytes: &[u8]| {
        let mut out = good.clone();
        out.splice(at..at + bytes.len(), bytes.iter().copied());
        out
    };
    // Not RIFF, not WAVE.
    assert!(matches!(
        wav::parse(b"RIFX\x04\0\0\0WAVE"),
        Err(FormatError::BadMagic { .. })
    ));
    assert!(matches!(
        wav::parse(&with(8, b"AVI ")),
        Err(FormatError::BadMagic { .. })
    ));
    // A RIFF size past the end.
    assert!(matches!(
        wav::parse(&with(4, &hex("ff000000"))),
        Err(FormatError::Truncated { .. })
    ));
    // Compressed samples (format 0x55, MPEG layer 3) are not read.
    let error = wav::parse(&with(20, &hex("5500"))).unwrap_err();
    assert!(matches!(error, FormatError::UnknownLayout { .. }));
    assert!(error.to_string().contains("format 85, 16 bits, 1 channels"));
    // 24 bits, 3 channels, a sample rate of 0, a frame size that does not fit.
    assert!(matches!(
        wav::parse(&with(34, &hex("1800"))),
        Err(FormatError::UnknownLayout { .. })
    ));
    assert!(matches!(
        wav::parse(&with(22, &hex("0300"))),
        Err(FormatError::UnknownLayout { .. })
    ));
    assert!(matches!(
        wav::parse(&with(24, &hex("00000000"))),
        Err(FormatError::Damaged { .. })
    ));
    assert!(matches!(
        wav::parse(&with(32, &hex("0400"))),
        Err(FormatError::Damaged { .. })
    ));
    // Half a frame of samples.
    let mut odd = good.clone();
    odd.truncate(good.len() - 1);
    odd.splice(40..44, hex("03000000"));
    odd.splice(4..8, ((odd.len() - 8) as u32).to_le_bytes());
    assert!(matches!(wav::parse(&odd), Err(FormatError::Damaged { .. })));
    // `data` before `fmt `, no `data`, a chunk running past the end.
    let mut swapped = b"RIFF\x14\0\0\0WAVE".to_vec();
    swapped.extend(b"data");
    swapped.extend(hex("00000000"));
    swapped.extend(b"fmt ");
    swapped.extend(hex("00000000"));
    assert!(
        wav::parse(&swapped)
            .unwrap_err()
            .to_string()
            .contains("before")
    );
    let mut no_data = good.clone();
    no_data.truncate(36);
    no_data.splice(4..8, 28u32.to_le_bytes());
    assert!(
        wav::parse(&no_data)
            .unwrap_err()
            .to_string()
            .contains("no `data`")
    );
    assert!(matches!(
        wav::parse(&with(40, &hex("40000000"))),
        Err(FormatError::Truncated { .. })
    ));
}

#[test]
fn what_cannot_be_written_as_wav_is_refused() {
    let mut wave = mono(&[1]);
    wave.bits = 24;
    assert!(wav::write(&wave).is_err());
    let mut half = mono(&[1]);
    half.data.pop();
    assert!(wav::write(&half).is_err());
}

#[test]
fn a_sound_script_byte_for_byte() {
    let script = SoundScript {
        sounds: vec![
            TimedSound {
                time: 0.759,
                file: "sound/common/walk_dirt_n.wav".into(),
            },
            TimedSound {
                time: 0.198,
                file: "sound/common/walk_dirt_n.wav".into(),
            },
        ],
    };
    let expected = "ScriptType\tCharacterSoundInformation\r\n\
                    SoundDataCount\t2\r\n\
                    SoundData00\t0.759000 \"sound/common/walk_dirt_n.wav\"\r\n\
                    SoundData01\t0.198000 \"sound/common/walk_dirt_n.wav\"\r\n";
    let bytes = script.write().unwrap();
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), expected);
    assert_eq!(SoundScript::parse(&bytes).unwrap(), script);
}

#[test]
fn sound_scripts_read_as_the_client_writes_them() {
    let text = b"ScriptType        CharacterSoundInformation\r\n\r\n\
                 SoundDataCount    2\r\n\
                 SoundData00       0.066000 \"sound/monster/wolf/drop_2.wav\"\r\n\
                 SoundData01       0.000000 \"sound/monster/wolf/dead_1.wav\"\r\n";
    let script = SoundScript::parse(text).unwrap();
    assert_eq!(script.sounds.len(), 2);
    assert_eq!(script.sounds[0].time, 0.066);
    assert_eq!(
        script.sounds[1].file.as_bytes(),
        b"sound/monster/wolf/dead_1.wav"
    );
    // No sounds at all.
    let empty = SoundScript::parse(b"ScriptType CharacterSoundInformation\r\nSoundDataCount 0\r\n")
        .unwrap();
    assert!(empty.sounds.is_empty());
}

#[test]
fn damaged_sound_scripts_are_refused() {
    let refused = |text: &str| SoundScript::parse(text.as_bytes()).unwrap_err().to_string();
    assert!(refused("ScriptType MotionData\r\n").contains("CharacterSoundInformation"));
    assert!(
        refused("ScriptType CharacterSoundInformation\r\nSoundDataCount 2\r\nSoundData00 0.1 \"a.wav\"\r\n")
            .contains("no SoundData01")
    );
    assert!(
        refused(
            "ScriptType CharacterSoundInformation\r\nSoundDataCount 1\r\nSoundData00 \"a.wav\"\r\n"
        )
        .contains("a time and a file")
    );
    assert!(
        refused("ScriptType CharacterSoundInformation\r\nSoundDataCount 1\r\nSoundData00 soon \"a.wav\"\r\n")
            .contains("not a number")
    );
    assert!(
        refused("ScriptType CharacterSoundInformation\r\nSoundDataCount 1000\r\n")
            .contains("limit")
    );
}

#[test]
fn a_motion_or_effect_names_its_sound_script_by_path() {
    let path =
        |file: &str| sound::script_path(file.as_bytes()).map(|p| String::from_utf8(p).unwrap());
    assert_eq!(
        path("d:/ymir work/pc/warrior/general/run.msa").as_deref(),
        Some("sound/pc/warrior/general/run.mss")
    );
    assert_eq!(
        path("D:\\Ymir Work\\effect\\hit\\blow_1\\blow_1_low.mse").as_deref(),
        Some("sound/effect/hit/blow_1/blow_1_low.mss")
    );
    assert_eq!(
        path("d:/ymir work/monster/wolf.v2/20").as_deref(),
        None,
        "the dot is in a folder's name"
    );
    assert_eq!(path("sound/pc/x.msa"), None);
    assert_eq!(path("d:/ymir work/.msa"), None);
}

fn spot(position: [f32; 3], range: f32, full_volume: f32) -> AmbienceSpot {
    AmbienceSpot {
        position,
        property: 2_293_772_605,
        range,
        full_volume,
    }
}

#[test]
fn sound_spots_byte_for_byte() {
    let file = AreaAmbience {
        spots: vec![spot([81_625.984, -104_513.33, 11_311.0], 8_950.0, 0.24)],
    };
    let expected = "AreaAmbienceDataFile\r\n\r\n\
                    Start Object000\r\n\
                    \x20   81625.984375 -104513.328125 11311.000000\r\n\
                    \x20   2293772605\r\n\
                    \x20   8950\r\n\
                    \x20   0.240000\r\n\
                    End Object\r\n\r\n\
                    ObjectCount 1\r\n";
    let bytes = file.write().unwrap();
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), expected);
    assert_eq!(AreaAmbience::parse(&bytes).unwrap(), file);
    // An area without spots, as most of the client's are.
    let empty = AreaAmbience::default().write().unwrap();
    assert_eq!(empty, b"AreaAmbienceDataFile\r\n\r\n\r\nObjectCount 0\r\n");
    assert!(AreaAmbience::parse(&empty).unwrap().spots.is_empty());
}

#[test]
fn damaged_sound_spots_are_refused() {
    let file = AreaAmbience {
        spots: vec![spot([1.0, -2.0, 3.0], 100.0, 0.5)],
    };
    let text = String::from_utf8(file.write().unwrap()).unwrap();
    let refused = |text: &str| {
        AreaAmbience::parse(text.as_bytes())
            .unwrap_err()
            .to_string()
    };
    assert!(refused(&text.replace("ObjectCount 1", "ObjectCount 2")).contains("ObjectCount is 2"));
    assert!(refused(&text.replace("    0.500000\r\n", "")).contains("expected 4 lines"));
    assert!(refused(&text.replace("    100\r\n", "    far\r\n")).contains("`far` is not a number"));
    assert!(
        refused(&text.replace("AreaAmbienceDataFile", "AreaDataFile"))
            .contains("AreaAmbienceDataFile")
    );
}

#[test]
fn ambience_properties_say_what_they_play() {
    let property = Property {
        id: 2_293_772_605,
        values: vec![
            (
                "ambiencesoundvector".into(),
                "sound/ambience/waterfall.wav".into(),
            ),
            ("maxvolumeareapercentage".into(), "0.300000".into()),
            ("playinterval".into(), "0.000000".into()),
            ("playintervalvariation".into(), "1.500000".into()),
            ("playtype".into(), "LOOP".into()),
            ("propertyname".into(), "waterfall".into()),
            ("propertytype".into(), "Ambience".into()),
        ],
    };
    let read = Property::parse(&property.write().unwrap()).unwrap();
    assert_eq!(
        read.ambience().unwrap(),
        Some(Ambience {
            sound: "sound/ambience/waterfall.wav".into(),
            play: PlayType::Loop,
            interval: 0.0,
            interval_variation: 1.5,
            full_volume: 0.3,
        })
    );
    // Another play type is kept as written; a number that is not one fails.
    let mut once = property.clone();
    once.values[4].1 = "ONCE".into();
    assert_eq!(
        once.ambience().unwrap().unwrap().play,
        PlayType::Other("ONCE".into())
    );
    let mut bad = property.clone();
    bad.values[1].1 = "loud".into();
    assert!(bad.ambience().is_err());
    // Not an ambience.
    let mut effect = property;
    effect.values[6].1 = "Effect".into();
    assert_eq!(effect.ambience().unwrap(), None);
}

proptest! {
    #[test]
    fn wav_files_round_trip(
        stereo in any::<bool>(),
        eight in any::<bool>(),
        rate in 1u32..96_000,
        frames in proptest::collection::vec(any::<u8>(), 0..200),
    ) {
        let channels = if stereo { 2 } else { 1 };
        let bits = if eight { 8 } else { 16 };
        let frame = usize::from(channels) * usize::from(bits / 8);
        let mut data = frames;
        data.truncate(data.len() / frame * frame);
        let wave = Wave { channels, sample_rate: rate, bits, data };
        prop_assert_eq!(wav::parse(&wav::write(&wave).unwrap()).unwrap(), wave);
    }

    #[test]
    fn sound_scripts_round_trip(
        sounds in proptest::collection::vec((0u32..10_000, "[a-z_/]{1,20}\\.wav"), 0..9),
    ) {
        let script = SoundScript {
            sounds: sounds
                .into_iter()
                .map(|(ms, file)| TimedSound { time: ms as f32 / 1000.0, file: file.as_str().into() })
                .collect(),
        };
        prop_assert_eq!(SoundScript::parse(&script.write().unwrap()).unwrap(), script);
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let _ = wav::parse(&bytes);
        let mut riff = b"RIFF".to_vec();
        riff.extend(((bytes.len() + 4) as u32).to_le_bytes());
        riff.extend(b"WAVE");
        riff.extend(&bytes);
        let _ = wav::parse(&riff);
        let _ = SoundScript::parse(&bytes);
        let _ = AreaAmbience::parse(&bytes);
    }

    #[test]
    fn damaged_wav_files_never_panic(
        samples in proptest::collection::vec(any::<i16>(), 0..20),
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..6),
    ) {
        let mut bytes = wav::write(&mono(&samples)).unwrap();
        for (at, value) in flips {
            let len = bytes.len();
            bytes[at % len] = value;
        }
        let _ = wav::parse(&bytes);
    }
}

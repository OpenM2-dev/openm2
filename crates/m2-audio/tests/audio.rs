//! Decoding (WAV through our reader, MP3 through nanomp3), the distance
//! rules, and the output driven through kira's mock backend: no audio device
//! needed. Sounds are built in the tests: WAV files by `m2_formats::wav`,
//! MP3 files as silent frames written out by hand.

use std::sync::Arc;

use glam::Vec3;
use kira::backend::mock::MockBackend;
use kira::sound::streaming::Decoder as _;
use m2_audio::output::decibels;
use m2_audio::spatial::Heard;
use m2_audio::{
    Audio, AudioError, Falloff, Listener, MusicStream, SoundKind, Volumes, decode, kind,
};
use m2_formats::wav::{self, Wave};
use proptest::prelude::*;

fn wav_file(channels: u16, samples: &[i16]) -> Vec<u8> {
    wav::write(&Wave {
        channels,
        sample_rate: 22_050,
        bits: 16,
        data: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
    })
    .unwrap()
}

/// `count` silent MPEG-1 layer III frames: 128 kbit/s, 44.1 kHz, mono.
///
/// The header is `FF FB` (sync, MPEG-1, layer III, no CRC), `90` (128
/// kbit/s, 44.1 kHz, no padding), `C0` (mono). A frame is
/// 144 × 128,000 / 44,100 = 417 bytes; side information and main data all
/// zero decode to silence, 1,152 samples a frame.
fn silent_mp3(count: usize) -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
    frame.repeat(count)
}

#[test]
fn wav_files_decode_to_stereo_frames() {
    assert_eq!(kind(&wav_file(1, &[0])), Some(SoundKind::Wav));
    let mono = decode(&wav_file(1, &[0, 16_384, -16_384])).unwrap();
    assert_eq!((mono.sample_rate, mono.channels), (22_050, 1));
    let frames: Vec<_> = mono.frames.iter().map(|f| (f.left, f.right)).collect();
    assert_eq!(frames, [(0.0, 0.0), (0.5, 0.5), (-0.5, -0.5)]);
    assert_eq!(mono.peak(), 0.5);
    assert!((mono.duration() - 3.0 / 22_050.0).abs() < 1e-12);

    let stereo = decode(&wav_file(2, &[16_384, -16_384, 0, 8_192])).unwrap();
    assert_eq!(stereo.channels, 2);
    let frames: Vec<_> = stereo.frames.iter().map(|f| (f.left, f.right)).collect();
    assert_eq!(frames, [(0.5, -0.5), (0.0, 0.25)]);
}

#[test]
fn mp3_files_decode() {
    let bytes = silent_mp3(20);
    assert_eq!(kind(&bytes), Some(SoundKind::Mp3));
    let sound = decode(&bytes).unwrap();
    assert_eq!((sound.sample_rate, sound.channels), (44_100, 1));
    assert!(sound.frames.len() >= 10 * 1152, "{}", sound.frames.len());
    assert_eq!(sound.frames.len() % 1152, 0);
    assert_eq!(sound.peak(), 0.0);
}

#[test]
fn what_is_not_a_sound_is_refused() {
    assert_eq!(kind(b"not a sound at all"), None);
    assert!(matches!(
        decode(b"not a sound at all"),
        Err(AudioError::UnknownKind)
    ));
    assert!(matches!(
        decode(b"RIFF\x04\0\0\0WAVX"),
        Err(AudioError::Format(_))
    ));
    assert!(MusicStream::new(Arc::from(wav_file(1, &[0]))).is_err());
}

#[test]
fn music_streams_frame_by_frame_and_pads_its_end() {
    let mut stream = MusicStream::new(Arc::from(silent_mp3(30))).unwrap();
    assert_eq!(stream.sample_rate(), 44_100);
    let total = stream.num_frames();
    assert!(total >= 20 * 1152, "{total}");
    assert!((stream.duration() - total as f64 / 44_100.0).abs() < 1e-9);
    let mut decoded = 0;
    loop {
        let chunk = stream.decode().unwrap();
        assert!(!chunk.is_empty(), "a stream never answers with nothing");
        decoded += chunk.len();
        if decoded > total + 4 * 1152 {
            break;
        }
    }
    // Back to the start, then it decodes again.
    assert_eq!(stream.seek(0).unwrap(), 0);
    assert_eq!(stream.decode().unwrap().len(), 1152);
}

#[test]
fn loudness_falls_with_distance() {
    let falloff = Falloff::SOUND;
    assert_eq!(falloff.gain(0.0), 1.0);
    assert_eq!(falloff.gain(1_000.0), 1.0);
    assert_eq!(falloff.gain(3_000.0), 0.5);
    assert_eq!(falloff.gain(5_000.0), 0.0);
    assert_eq!(falloff.gain(f32::NAN), 0.0);

    // A sound spot heard to 100 m, at full volume within its first quarter.
    let spot = Falloff::spot(10_000.0, 0.25);
    assert_eq!((spot.full, spot.silent), (2_500.0, 10_000.0));
    assert_eq!(spot.gain(2_000.0), 1.0);
    assert_eq!(spot.gain(6_250.0), 0.5);
    assert_eq!(Falloff::spot(f32::INFINITY, 2.0).gain(0.0), 0.0);
}

#[test]
fn sounds_are_heard_on_their_side() {
    // Facing north (−y in the world), so the right is east (+x).
    let listener = Listener::new(Vec3::new(100.0, -100.0, 0.0), Vec3::X * 3.0);
    assert_eq!(listener.right, Vec3::X);
    let east = listener
        .hear(Vec3::new(600.0, -100.0, 0.0), Falloff::SOUND)
        .unwrap();
    assert_eq!(
        east,
        Heard {
            gain: 1.0,
            pan: 0.7
        }
    );
    let west = listener
        .hear(Vec3::new(-2_900.0, -100.0, 0.0), Falloff::SOUND)
        .unwrap();
    assert_eq!(west.gain, 0.5);
    assert!((west.pan + 0.7).abs() < 1e-6);
    let ahead = listener
        .hear(Vec3::new(100.0, -600.0, 0.0), Falloff::SOUND)
        .unwrap();
    assert_eq!(ahead.pan, 0.0);
    // At the listener: centred. Too far: not heard.
    assert_eq!(
        listener
            .hear(listener.position, Falloff::SOUND)
            .unwrap()
            .pan,
        0.0
    );
    assert!(
        listener
            .hear(Vec3::new(100.0, 6_000.0, 0.0), Falloff::SOUND)
            .is_none()
    );
    // A right without length falls back to the world's x.
    assert_eq!(Listener::new(Vec3::ZERO, Vec3::ZERO).right, Vec3::X);
}

#[test]
fn volumes_are_decibels() {
    assert_eq!(decibels(1.0).0, 0.0);
    assert!((decibels(0.5).0 + 6.0206).abs() < 1e-3);
    assert_eq!(decibels(0.0), kira::Decibels::SILENCE);
    assert_eq!(decibels(f32::NAN), kira::Decibels::SILENCE);
    assert_eq!(decibels(4.0).0, 0.0);
}

#[test]
fn the_output_plays_sounds_loops_and_music() {
    let mut audio = Audio::<MockBackend>::with_backend(Volumes::default()).unwrap();
    let sound = decode(&wav_file(1, &[0, 1_000, 2_000, 1_000])).unwrap();
    let heard = Heard {
        gain: 0.5,
        pan: -0.2,
    };
    assert!(audio.play(&sound, heard));

    let id = audio.start_loop(&sound, heard).unwrap();
    assert_eq!(audio.loops(), 1);
    audio.set_loop(
        id,
        Heard {
            gain: 1.0,
            pan: 0.0,
        },
    );
    audio.stop_loop(id);
    assert_eq!(audio.loops(), 0);
    // Changing or stopping a loop that is gone does nothing.
    audio.set_loop(id, heard);
    audio.stop_loop(id);

    audio
        .play_music(MusicStream::new(Arc::from(silent_mp3(30))).unwrap())
        .unwrap();
    assert!(audio.playing_music());
    audio.set_volumes(Volumes {
        sounds: 0.3,
        music: 0.0,
    });
    audio.stop_music();
    assert!(!audio.playing_music());
    assert!(audio.music_error().is_none());

    // kira's mixer runs over what was started.
    audio.backend_mut().on_start_processing();
    for _ in 0..64 {
        audio.backend_mut().process();
    }
}

#[test]
fn loops_are_capped() {
    let mut audio = Audio::<MockBackend>::with_backend(Volumes::default()).unwrap();
    let sound = decode(&wav_file(1, &[1_000; 8])).unwrap();
    let heard = Heard {
        gain: 1.0,
        pan: 0.0,
    };
    let started = (0..m2_audio::output::MAX_LOOPS + 5)
        .filter(|_| audio.start_loop(&sound, heard).is_some())
        .count();
    assert_eq!(started, m2_audio::output::MAX_LOOPS);
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2_000)) {
        let _ = decode(&bytes);
        let mut mp3 = silent_mp3(3);
        mp3.extend(&bytes);
        let _ = decode(&mp3);
        let _ = MusicStream::new(Arc::from(mp3));
    }

    #[test]
    fn damaged_mp3_files_never_panic(
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..40),
    ) {
        let mut bytes = silent_mp3(6);
        for (at, value) in flips {
            let len = bytes.len();
            bytes[at % len] = value;
        }
        let _ = decode(&bytes);
        if let Ok(mut stream) = MusicStream::new(Arc::from(bytes)) {
            for _ in 0..12 {
                let _ = stream.decode();
            }
        }
    }
}

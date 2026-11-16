//! What `openm2 play` plays (`docs/specs/audio.md`): the sound scripts of
//! the actors' motions and of the effects that start, a sound for each hit,
//! the map's sound spots, and music for each screen. Without an audio
//! device (or with `--mute`, or for `--screenshot`) it is silent and reads
//! nothing.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use m2_audio::{Audio, Backend, DefaultBackend, Falloff, Listener, LoopId, MusicStream, Pcm};

use crate::actors::{Motion, MotionSound};
use crate::source::{Music, Source};
use crate::view::Files;
use crate::world_effects::{MotionClock, due};

/// Most sounds of effects waiting for their time.
const MAX_PENDING: usize = 256;

/// A map's sound spot, placed in the world.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SoundSpot {
    /// Where it is, world centimetres.
    pub position: Vec3,
    /// The sound's game path, normalized.
    pub sound: Vec<u8>,
    /// How far it is heard.
    pub falloff: Falloff,
}

struct Spot {
    spot: SoundSpot,
    playing: Option<LoopId>,
}

/// The sounds of the game, on kira's backend `B` (the audio device, or a
/// mock in tests).
pub(crate) struct WorldSounds<B: Backend = DefaultBackend> {
    audio: Option<Audio<B>>,
    /// The sounds of a hit, taken in turn.
    hit_sounds: Vec<Vec<u8>>,
    /// Decoded sounds by path; `None` for those that are missing or do not
    /// decode (said once).
    sounds: HashMap<Vec<u8>, Option<Pcm>>,
    /// The sounds of effects, by the effect's path.
    scripts: HashMap<Vec<u8>, Vec<MotionSound>>,
    spots: Vec<Spot>,
    /// Sounds of effects still to come: seconds left, game path, where.
    pending: Vec<(f32, Vec<u8>, Vec3)>,
    listener: Listener,
    /// The music asked for last: none, or a screen's.
    music: Option<Music>,
    /// What it is called, for notes.
    music_name: String,
    hits: usize,
}

impl WorldSounds {
    /// No sound at all.
    pub fn silent() -> Self {
        Self::new(None, Vec::new())
    }
}

impl<B: Backend> WorldSounds<B> {
    /// Sounds played on `audio`, a hit with each of `hit_sounds` in turn;
    /// silent without `audio`.
    pub fn new(audio: Option<Audio<B>>, hit_sounds: Vec<Vec<u8>>) -> Self {
        Self {
            audio,
            hit_sounds,
            sounds: HashMap::new(),
            scripts: HashMap::new(),
            spots: Vec::new(),
            pending: Vec::new(),
            listener: Listener::new(Vec3::ZERO, Vec3::X),
            music: None,
            music_name: String::new(),
            hits: 0,
        }
    }

    /// Plays at these volumes from now on (the Settings window).
    pub fn set_volumes(&mut self, volumes: m2_audio::Volumes) {
        if let Some(audio) = &mut self.audio {
            audio.set_volumes(volumes);
        }
    }

    /// Where the sounds are heard from, and which way is right.
    pub fn listen(&mut self, position: Vec3, right: Vec3) {
        self.listener = Listener::new(position, right);
    }

    /// The decoded sound at `path`, read the first time.
    fn sound(&mut self, files: &Files<'_>, path: &[u8]) -> Option<Pcm> {
        if let Some(sound) = self.sounds.get(path) {
            return sound.clone();
        }
        let decoded = files
            .read_bytes(path)
            .and_then(|bytes| Ok(m2_audio::decode(&bytes)?));
        let sound = match decoded {
            Ok(sound) => Some(sound),
            Err(error) => {
                eprintln!("note: sound {}: {error:#}", path.escape_ascii());
                None
            }
        };
        self.sounds.insert(path.to_vec(), sound.clone());
        sound
    }

    /// Plays the sound at `path` once, from `at`.
    pub fn play_at(&mut self, files: &Files<'_>, path: &[u8], at: Vec3) {
        if self.audio.is_none() {
            return;
        }
        let Some(heard) = self.listener.hear(at, Falloff::SOUND) else {
            return;
        };
        let Some(sound) = self.sound(files, path) else {
            return;
        };
        if let Some(audio) = &mut self.audio {
            audio.play(&sound, heard);
        }
    }

    /// The sounds of `motion` whose time came between `before` and `now`
    /// (as for its effects), from an actor at `at`.
    pub fn motion(
        &mut self,
        files: &Files<'_>,
        motion: &Motion,
        at: Vec3,
        before: Option<MotionClock>,
        now: MotionClock,
    ) {
        if self.audio.is_none() {
            return;
        }
        let due_files: Vec<Vec<u8>> = due(&motion.sounds, motion, before, now)
            .map(|sound| sound.file.clone())
            .collect();
        for file in due_files {
            self.play_at(files, &file, at);
        }
    }

    /// A hit at `at`.
    pub fn hit(&mut self, files: &Files<'_>, at: Vec3) {
        let file = self
            .hit_sounds
            .get(self.hits % self.hit_sounds.len().max(1))
            .cloned();
        self.hits = self.hits.wrapping_add(1);
        if let Some(file) = file {
            self.play_at(files, &file, at);
        }
    }

    /// An effect started at `at` (its path in the asset folder): its
    /// sounds, each at its time.
    pub fn effect(&mut self, files: &Files<'_>, effect: &[u8], at: Vec3) {
        if self.audio.is_none() {
            return;
        }
        let sounds = self
            .scripts
            .entry(effect.to_vec())
            .or_insert_with(|| MotionSound::of_effect(files, effect))
            .clone();
        for sound in sounds {
            if sound.time <= 0.0 {
                self.play_at(files, &sound.file, at);
            } else if self.pending.len() < MAX_PENDING {
                self.pending.push((sound.time, sound.file, at));
            }
        }
    }

    /// The map's sound spots, instead of the last map's.
    pub fn set_map(&mut self, spots: Vec<SoundSpot>) {
        self.stop_spots();
        self.spots = spots
            .into_iter()
            .map(|spot| Spot {
                spot,
                playing: None,
            })
            .collect();
    }

    fn stop_spots(&mut self) {
        for spot in &mut self.spots {
            if let (Some(id), Some(audio)) = (spot.playing.take(), &mut self.audio) {
                audio.stop_loop(id);
            }
        }
    }

    /// Forgets the world's sounds: a new game. The music goes on.
    pub fn clear(&mut self) {
        self.stop_spots();
        self.spots.clear();
        self.pending.clear();
    }

    /// One frame: the sounds of effects whose time came, and the sound
    /// spots in reach started, followed and stopped.
    pub fn advance(&mut self, files: &Files<'_>, seconds: f32) {
        if self.audio.is_none() {
            return;
        }
        let mut due = Vec::new();
        self.pending.retain_mut(|(left, file, at)| {
            *left -= seconds;
            if *left <= 0.0 {
                due.push((std::mem::take(file), *at));
                return false;
            }
            true
        });
        for (file, at) in due {
            self.play_at(files, &file, at);
        }
        for n in 0..self.spots.len() {
            let Some(spot) = self.spots.get(n) else {
                continue;
            };
            let heard = self.listener.hear(spot.spot.position, spot.spot.falloff);
            let playing = spot.playing;
            let path = spot.spot.sound.clone();
            match (heard, playing) {
                (Some(heard), None) => {
                    let Some(sound) = self.sound(files, &path) else {
                        continue;
                    };
                    let id = self
                        .audio
                        .as_mut()
                        .and_then(|audio| audio.start_loop(&sound, heard));
                    if let Some(spot) = self.spots.get_mut(n) {
                        spot.playing = id;
                    }
                }
                (Some(heard), Some(id)) => {
                    if let Some(audio) = &mut self.audio {
                        audio.set_loop(id, heard);
                    }
                }
                (None, Some(id)) => {
                    if let Some(audio) = &mut self.audio {
                        audio.stop_loop(id);
                    }
                    if let Some(spot) = self.spots.get_mut(n) {
                        spot.playing = None;
                    }
                }
                (None, None) => {}
            }
        }
        if let Some(error) = self.audio.as_mut().and_then(Audio::music_error) {
            eprintln!("note: music {}: {error}", self.music_name);
        }
    }

    /// Plays the music of a screen from `source`, or none; nothing changes
    /// when it is already playing.
    pub fn music(&mut self, source: &Source, music: Option<Music>) {
        if self.music == music {
            return;
        }
        self.music = music;
        let Some(audio) = &mut self.audio else {
            return;
        };
        let Some((name, bytes)) = music.and_then(|music| source.music(music)) else {
            audio.stop_music();
            return;
        };
        self.music_name.clone_from(&name);
        let stream = bytes.and_then(|bytes| Ok(MusicStream::new(Arc::from(bytes))?));
        match stream.and_then(|stream| Ok(audio.play_music(stream)?)) {
            Ok(()) => eprintln!("music: {name}"),
            Err(error) => {
                audio.stop_music();
                eprintln!("note: music {name}: {error:#}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use m2_assets::AssetFolder;
    use m2_audio::{MockBackend, Volumes};
    use m2_formats::wav::{self, Wave};

    use super::*;

    /// The sounds of a hit.
    const HIT_SOUNDS: [&[u8]; 2] = [
        b"sound/common/hit/hit_sword_1.wav",
        b"sound/common/hit/hit_sword_2.wav",
    ];

    /// An asset folder's files, removed when dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("openm2-world-sounds-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, file: &str, bytes: &[u8]) {
            let path = self.0.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn beep() -> Vec<u8> {
        wav::write(&Wave {
            channels: 1,
            sample_rate: 22_050,
            bits: 16,
            data: [0i16, 8_000, 0, -8_000]
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect(),
        })
        .unwrap()
    }

    /// Two sounds and an effect that plays them, the second half a second
    /// after the first.
    fn assets(name: &str) -> (Folder, AssetFolder) {
        let folder = Folder::new(name);
        folder.write("sound/ambience/waterfall.wav", &beep());
        folder.write("sound/common/hit/hit_sword_1.wav", &beep());
        folder.write(
            "effect/test/boom.effect.toml",
            b"format = 1\n\n[[sound]]\ntime = 0.0\nfile = \"/sound/common/hit/hit_sword_1.wav\"\n\n\
              [[sound]]\ntime = 0.5\nfile = \"/sound/ambience/waterfall.wav\"\n",
        );
        folder.write("effect/test/quiet.effect.toml", b"format = 1\n");
        let assets = AssetFolder::new(folder.path());
        (folder, assets)
    }

    fn hit_sounds() -> Vec<Vec<u8>> {
        HIT_SOUNDS.iter().map(|s| s.to_vec()).collect()
    }

    fn sounds() -> WorldSounds<MockBackend> {
        let audio = Audio::<MockBackend>::with_backend(Volumes::default()).unwrap();
        WorldSounds::new(Some(audio), hit_sounds())
    }

    fn loops(sounds: &WorldSounds<MockBackend>) -> usize {
        sounds.audio.as_ref().map_or(0, Audio::loops)
    }

    #[test]
    fn sound_spots_play_while_the_listener_is_in_reach() {
        let (_folder, assets) = assets("spots");
        let files = Files::Assets(&assets);
        let mut sounds = sounds();
        sounds.set_map(vec![SoundSpot {
            position: Vec3::ZERO,
            sound: b"sound/ambience/waterfall.wav".to_vec(),
            falloff: Falloff::spot(2_000.0, 0.5),
        }]);
        sounds.listen(Vec3::new(5_000.0, 0.0, 0.0), Vec3::X);
        sounds.advance(&files, 0.1);
        assert_eq!(loops(&sounds), 0);
        sounds.listen(Vec3::new(1_500.0, 0.0, 0.0), Vec3::X);
        sounds.advance(&files, 0.1);
        assert_eq!(loops(&sounds), 1);
        sounds.listen(Vec3::new(500.0, 0.0, 0.0), Vec3::X);
        sounds.advance(&files, 0.1);
        assert_eq!(loops(&sounds), 1, "followed, not started again");
        sounds.listen(Vec3::new(2_500.0, 0.0, 0.0), Vec3::X);
        sounds.advance(&files, 0.1);
        assert_eq!(loops(&sounds), 0);
        // A new game stops the spots.
        sounds.listen(Vec3::ZERO, Vec3::X);
        sounds.advance(&files, 0.1);
        assert_eq!(loops(&sounds), 1);
        sounds.clear();
        assert_eq!(loops(&sounds), 0);
    }

    #[test]
    fn an_effects_sounds_come_at_their_times() {
        let (_folder, assets) = assets("effect");
        let files = Files::Assets(&assets);
        let mut sounds = sounds();
        sounds.effect(&files, b"effect/test/boom.effect.toml", Vec3::ZERO);
        // The first at once, the second half a second later.
        assert!(
            sounds
                .sounds
                .contains_key(b"sound/common/hit/hit_sword_1.wav".as_slice())
        );
        assert_eq!(sounds.pending.len(), 1);
        sounds.advance(&files, 0.3);
        assert_eq!(sounds.pending.len(), 1);
        sounds.advance(&files, 0.3);
        assert!(sounds.pending.is_empty());
        assert!(
            sounds
                .sounds
                .contains_key(b"sound/ambience/waterfall.wav".as_slice())
        );
        // An effect without sounds plays nothing.
        sounds.effect(&files, b"effect/test/quiet.effect.toml", Vec3::ZERO);
        assert!(sounds.pending.is_empty());
    }

    #[test]
    fn missing_sounds_are_looked_for_once_and_silence_reads_nothing() {
        let (_folder, assets) = assets("missing");
        let files = Files::Assets(&assets);
        let mut sounds = sounds();
        sounds.play_at(&files, b"sound/missing.wav", Vec3::ZERO);
        assert!(matches!(
            sounds.sounds.get(b"sound/missing.wav".as_slice()),
            Some(None)
        ));
        // Too far to hear: not even read.
        sounds.play_at(
            &files,
            b"sound/common/hit/hit_sword_1.wav",
            Vec3::new(9_000.0, 0.0, 0.0),
        );
        assert_eq!(sounds.sounds.len(), 1);

        let mut silent = WorldSounds::<MockBackend>::new(None, hit_sounds());
        silent.play_at(&files, b"sound/common/hit/hit_sword_1.wav", Vec3::ZERO);
        silent.hit(&files, Vec3::ZERO);
        silent.effect(&files, b"effect/test/boom.effect.toml", Vec3::ZERO);
        silent.advance(&files, 1.0);
        assert!(silent.sounds.is_empty() && silent.pending.is_empty());
    }
}

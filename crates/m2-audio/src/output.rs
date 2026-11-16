//! The audio device, through kira: three tracks, for one-shot sounds, loops
//! (a map's sound spots) and music, each with its volume.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use kira::backend::Backend;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, Panning, Tween};

use crate::AudioError;
use crate::decode::{MusicStream, Pcm};
use crate::spatial::Heard;

/// Most one-shot sounds playing at once; more are dropped.
pub const MAX_SOUNDS: usize = 64;

/// Most loops playing at once.
pub const MAX_LOOPS: usize = 32;

/// How long a loop takes to follow a change of loudness or side.
const LOOP_FADE: Duration = Duration::from_millis(150);

/// How long music takes to fade in or out.
pub const MUSIC_FADE: Duration = Duration::from_secs(2);

/// Volumes, as amplitudes from 0 (silent) to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volumes {
    /// Sounds of actors, effects and the map's sound spots.
    pub sounds: f32,
    /// Music.
    pub music: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Self {
            sounds: 1.0,
            music: 1.0,
        }
    }
}

/// An amplitude (0 to 1) as kira's decibels; 0 and below is silence.
pub fn decibels(amplitude: f32) -> Decibels {
    if !amplitude.is_finite() || amplitude <= 0.001 {
        return Decibels::SILENCE;
    }
    Decibels((20.0 * amplitude.min(1.0).log10()).max(Decibels::SILENCE.0))
}

fn tween(duration: Duration) -> Tween {
    Tween {
        duration,
        ..Tween::default()
    }
}

/// A loop that is playing, to change or stop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LoopId(u64);

/// The audio output.
pub struct Audio<B: Backend = kira::DefaultBackend> {
    manager: AudioManager<B>,
    sounds: TrackHandle,
    loops: TrackHandle,
    music: TrackHandle,
    looping: BTreeMap<LoopId, StaticSoundHandle>,
    next_loop: u64,
    song: Option<StreamingSoundHandle<AudioError>>,
}

impl<B: Backend> fmt::Debug for Audio<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Audio")
            .field("loops", &self.looping.len())
            .field("music", &self.song.is_some())
            .finish_non_exhaustive()
    }
}

impl Audio<kira::DefaultBackend> {
    /// Opens the system's default audio device.
    pub fn open(volumes: Volumes) -> Result<Self, AudioError> {
        Self::with_backend(volumes)
    }
}

impl<B: Backend> Audio<B>
where
    B::Settings: Default,
    B::Error: fmt::Debug,
{
    /// Starts the output on backend `B` with its default settings (tests use
    /// kira's mock backend).
    pub fn with_backend(volumes: Volumes) -> Result<Self, AudioError> {
        let mut manager = AudioManager::<B>::new(AudioManagerSettings::default())
            .map_err(|e| AudioError::Output(format!("{e:?}")))?;
        let mut track = |volume: f32, capacity: usize| {
            manager
                .add_sub_track(
                    TrackBuilder::new()
                        .volume(decibels(volume))
                        .sound_capacity(capacity),
                )
                .map_err(|_| AudioError::Output("no room for a track".to_owned()))
        };
        let sounds = track(volumes.sounds, MAX_SOUNDS)?;
        let loops = track(volumes.sounds, MAX_LOOPS)?;
        let music = track(volumes.music, 4)?;
        Ok(Self {
            manager,
            sounds,
            loops,
            music,
            looping: BTreeMap::new(),
            next_loop: 0,
            song: None,
        })
    }
}

impl<B: Backend> Audio<B> {
    /// Plays a sound once, as it is `heard`. False if it was dropped (too
    /// many playing).
    pub fn play(&mut self, sound: &Pcm, heard: Heard) -> bool {
        let data = StaticSoundData {
            sample_rate: sound.sample_rate,
            frames: sound.frames.clone(),
            settings: StaticSoundSettings::new()
                .volume(decibels(heard.gain))
                .panning(Panning(heard.pan)),
            slice: None,
        };
        self.sounds.play(data).is_ok()
    }

    /// Starts playing a sound over and over, fading in. `None` if it could
    /// not start (too many loops).
    pub fn start_loop(&mut self, sound: &Pcm, heard: Heard) -> Option<LoopId> {
        if self.looping.len() >= MAX_LOOPS {
            return None;
        }
        let data = StaticSoundData {
            sample_rate: sound.sample_rate,
            frames: sound.frames.clone(),
            settings: StaticSoundSettings::new()
                .loop_region(..)
                .volume(decibels(heard.gain))
                .panning(Panning(heard.pan))
                .fade_in_tween(tween(LOOP_FADE)),
            slice: None,
        };
        let handle = self.loops.play(data).ok()?;
        let id = LoopId(self.next_loop);
        self.next_loop = self.next_loop.wrapping_add(1);
        self.looping.insert(id, handle);
        Some(id)
    }

    /// Changes how a loop is heard.
    pub fn set_loop(&mut self, id: LoopId, heard: Heard) {
        if let Some(handle) = self.looping.get_mut(&id) {
            handle.set_volume(decibels(heard.gain), tween(LOOP_FADE));
            handle.set_panning(Panning(heard.pan), tween(LOOP_FADE));
        }
    }

    /// Fades a loop out and forgets it.
    pub fn stop_loop(&mut self, id: LoopId) {
        if let Some(mut handle) = self.looping.remove(&id) {
            handle.stop(tween(LOOP_FADE));
        }
    }

    /// Loops playing.
    pub fn loops(&self) -> usize {
        self.looping.len()
    }

    /// Plays music over and over, fading out what played before.
    pub fn play_music(&mut self, stream: MusicStream) -> Result<(), AudioError> {
        self.stop_music();
        let data = StreamingSoundData::from_decoder(stream)
            .loop_region(..)
            .fade_in_tween(tween(MUSIC_FADE));
        let handle = self
            .music
            .play(data)
            .map_err(|e| AudioError::Output(format!("music: {e:?}")))?;
        self.song = Some(handle);
        Ok(())
    }

    /// Fades the music out.
    pub fn stop_music(&mut self) {
        if let Some(mut song) = self.song.take() {
            song.stop(tween(MUSIC_FADE));
        }
    }

    /// Whether music is playing (or fading in).
    pub fn playing_music(&self) -> bool {
        self.song.is_some()
    }

    /// An error the music hit while decoding, once.
    pub fn music_error(&mut self) -> Option<AudioError> {
        self.song.as_mut().and_then(StreamingSoundHandle::pop_error)
    }

    /// Changes the volumes.
    pub fn set_volumes(&mut self, volumes: Volumes) {
        let quick = tween(LOOP_FADE);
        self.sounds.set_volume(decibels(volumes.sounds), quick);
        self.loops.set_volume(decibels(volumes.sounds), quick);
        self.music.set_volume(decibels(volumes.music), quick);
    }

    /// The backend, for tests driving kira's mock backend.
    pub fn backend_mut(&mut self) -> &mut B {
        self.manager.backend_mut()
    }
}

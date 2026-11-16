//! The client's sounds decoded into stereo frames: WAV (PCM) through
//! `m2_formats::wav`, MP3 through nanomp3. Music is decoded while it plays
//! ([`MusicStream`]).

use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use kira::Frame;
use m2_formats::wav;
use nanomp3::Channels;

use crate::AudioError;

/// Most frames one sound decodes to (about 95 s at 44.1 kHz, 32 MiB);
/// longer sounds are music and are streamed.
pub const MAX_SOUND_FRAMES: usize = 4 * 1024 * 1024;

/// What kind of sound file some bytes are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    /// A RIFF WAV file.
    Wav,
    /// MPEG audio: an ID3 tag, or MP3 frames following each other.
    Mp3,
}

/// What kind of sound file `bytes` hold, `None` for neither.
pub fn kind(bytes: &[u8]) -> Option<SoundKind> {
    if bytes.starts_with(b"RIFF") {
        Some(SoundKind::Wav)
    } else if nanomp3::detect(bytes) {
        Some(SoundKind::Mp3)
    } else {
        None
    }
}

/// A decoded sound: stereo frames (a mono sound in both), ready to play.
#[derive(Clone)]
pub struct Pcm {
    /// Frames per second.
    pub sample_rate: u32,
    /// Channels as stored: 1 or 2.
    pub channels: u16,
    /// The frames.
    pub frames: Arc<[Frame]>,
}

impl fmt::Debug for Pcm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pcm")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames.len())
            .finish()
    }
}

impl Pcm {
    /// The length in seconds.
    pub fn duration(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames.len() as f64 / f64::from(self.sample_rate)
    }

    /// The loudest sample, 0 to 1 (silence is 0).
    pub fn peak(&self) -> f32 {
        self.frames
            .iter()
            .map(|f| f.left.abs().max(f.right.abs()))
            .fold(0.0, f32::max)
    }
}

/// Decodes a whole sound: a WAV or an MP3 file.
pub fn decode(bytes: &[u8]) -> Result<Pcm, AudioError> {
    match kind(bytes) {
        Some(SoundKind::Wav) => from_wave(&wav::parse(bytes)?),
        Some(SoundKind::Mp3) => decode_mp3(bytes),
        None => Err(AudioError::UnknownKind),
    }
}

/// A WAV file's samples as frames.
pub fn from_wave(wave: &wav::Wave) -> Result<Pcm, AudioError> {
    let frames = wave.frames();
    if frames > MAX_SOUND_FRAMES {
        return Err(AudioError::TooLong {
            frames,
            limit: MAX_SOUND_FRAMES,
        });
    }
    let samples: Vec<f32> = wave.samples().collect();
    let frames: Arc<[Frame]> = if wave.channels == 2 {
        samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[left, right]| Frame::new(*left, *right))
            .collect()
    } else {
        samples.iter().map(|s| Frame::from_mono(*s)).collect()
    };
    Ok(Pcm {
        sample_rate: wave.sample_rate,
        channels: wave.channels,
        frames,
    })
}

fn channel_count(channels: Option<Channels>) -> u16 {
    match channels {
        Some(Channels::Stereo) => 2,
        Some(Channels::Mono) | None => 1,
    }
}

/// Interleaved samples as frames.
fn push_frames(out: &mut Vec<Frame>, samples: &[f32], channels: u16) {
    if channels == 2 {
        out.extend(
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[left, right]| Frame::new(*left, *right)),
        );
    } else {
        out.extend(samples.iter().map(|s| Frame::from_mono(*s)));
    }
}

fn decode_mp3(bytes: &[u8]) -> Result<Pcm, AudioError> {
    let mut reader = nanomp3::SliceReader::<f32>::new(bytes);
    let mut frames = Vec::new();
    let mut channels = 0;
    let mut sample_rate = 0;
    while let Some(samples) = reader
        .read_frame()
        .map_err(|e| AudioError::Mp3(e.to_string()))?
        .map(<[f32]>::to_vec)
    {
        if channels == 0 {
            channels = channel_count(reader.channels());
            sample_rate = reader.sample_rate();
        }
        push_frames(&mut frames, &samples, channels);
        if frames.len() > MAX_SOUND_FRAMES {
            return Err(AudioError::TooLong {
                frames: frames.len(),
                limit: MAX_SOUND_FRAMES,
            });
        }
    }
    if frames.is_empty() || sample_rate == 0 {
        return Err(AudioError::Mp3("no audio frames".to_owned()));
    }
    Ok(Pcm {
        sample_rate,
        channels,
        frames: frames.into(),
    })
}

/// An MP3 file decoded while it plays, for music.
pub struct MusicStream {
    reader: nanomp3::Reader<Cursor<Arc<[u8]>>, f32>,
    channels: u16,
    sample_rate: u32,
    frames: usize,
}

/// Frames of silence a stream gives at a time past the end of its audio.
const PADDING: usize = 1152;

impl fmt::Debug for MusicStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MusicStream")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames)
            .finish()
    }
}

impl MusicStream {
    /// Opens an MP3 file; its length is found by scanning its frames.
    pub fn new(bytes: Arc<[u8]>) -> Result<Self, AudioError> {
        if kind(&bytes) != Some(SoundKind::Mp3) {
            return Err(AudioError::UnknownKind);
        }
        let reader = nanomp3::Reader::<_, f32>::new(Cursor::new(bytes))
            .map_err(|e| AudioError::Mp3(e.to_string()))?;
        let sample_rate = reader.sample_rate();
        let frames = reader
            .total_samples()
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        if sample_rate == 0 || frames == 0 {
            return Err(AudioError::Mp3("no audio frames".to_owned()));
        }
        Ok(Self {
            channels: channel_count(reader.channels()),
            reader,
            sample_rate,
            frames,
        })
    }

    /// The length in seconds.
    pub fn duration(&self) -> f64 {
        self.frames as f64 / f64::from(self.sample_rate)
    }

    /// Channels as stored: 1 or 2.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Decodes the whole piece at once, however long (for tools; music
    /// plays streamed).
    pub fn into_pcm(mut self) -> Result<Pcm, AudioError> {
        use kira::sound::streaming::Decoder as _;
        let mut frames = Vec::with_capacity(self.frames.min(MAX_SOUND_FRAMES));
        while frames.len() < self.frames {
            frames.extend(self.decode()?);
        }
        frames.truncate(self.frames);
        Ok(Pcm {
            sample_rate: self.sample_rate,
            channels: self.channels,
            frames: frames.into(),
        })
    }
}

impl kira::sound::streaming::Decoder for MusicStream {
    type Error = AudioError;

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn num_frames(&self) -> usize {
        self.frames
    }

    /// The next MP3 frame's samples. Past the end of the audio, silence:
    /// kira asks until it has the length the scan found, and an empty answer
    /// would make it ask forever.
    fn decode(&mut self) -> Result<Vec<Frame>, Self::Error> {
        let mut out = Vec::new();
        let channels = self.channels;
        match self
            .reader
            .read_frame()
            .map_err(|e| AudioError::Mp3(e.to_string()))?
        {
            Some(samples) => push_frames(&mut out, samples, channels),
            None => out.resize(PADDING, Frame::ZERO),
        }
        Ok(out)
    }

    fn seek(&mut self, index: usize) -> Result<usize, Self::Error> {
        self.reader
            .seek(index as u64)
            .map_err(|e| AudioError::Mp3(e.to_string()))?;
        Ok(index)
    }
}

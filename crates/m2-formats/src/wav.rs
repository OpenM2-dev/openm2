//! WAV sounds (`.wav`): a RIFF file holding PCM samples
//! (`docs/specs/formats/sound.md`, "WAV").
//!
//! ```text
//! "RIFF" u32 size  "WAVE"
//!   "fmt " u32 16   u16 format (1: PCM)  u16 channels  u32 sample rate
//!                   u32 bytes per second  u16 bytes per frame  u16 bits
//!   "data" u32 size  samples…
//!   other chunks (LIST, cue, …), skipped
//! ```
//!
//! Chunks are padded to an even length. Samples are interleaved by channel,
//! little-endian: 16-bit signed, or 8-bit unsigned around 128. Every sound
//! of the client is 16-bit PCM, mono but for three.

use crate::FormatError;
use crate::reader::{Reader, Writer};

const WHAT: &str = "wav";

/// The format number of PCM samples, the only one read.
pub const FORMAT_PCM: u16 = 1;

/// Largest sample data read (64 MiB; the client's largest sound is under
/// 4 MiB).
pub const MAX_DATA: usize = 64 * 1024 * 1024;

/// Highest sample rate read.
pub const MAX_SAMPLE_RATE: u32 = 384_000;

const KNOWN: &str = "PCM (format 1), 8 or 16 bits, 1 or 2 channels";

/// A parsed WAV file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wave {
    /// 1 (mono) or 2 (stereo).
    pub channels: u16,
    /// Frames per second.
    pub sample_rate: u32,
    /// 8 or 16.
    pub bits: u16,
    /// The samples as stored: interleaved, little-endian, a whole number of
    /// frames.
    pub data: Vec<u8>,
}

impl Wave {
    /// Bytes per frame (one sample of every channel).
    pub fn frame_len(&self) -> usize {
        usize::from(self.channels) * usize::from(self.bits / 8)
    }

    /// The number of frames.
    pub fn frames(&self) -> usize {
        self.data.len().checked_div(self.frame_len()).unwrap_or(0)
    }

    /// The length in seconds.
    pub fn duration(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f64 / f64::from(self.sample_rate)
    }

    /// The samples from −1 to 1, interleaved by channel.
    pub fn samples(&self) -> impl Iterator<Item = f32> + '_ {
        let sixteen = self.bits == 16;
        let step = if sixteen { 2 } else { 1 };
        self.data.chunks_exact(step).map(move |s| match s {
            [lo, hi] if sixteen => f32::from(i16::from_le_bytes([*lo, *hi])) / 32768.0,
            [b] => (f32::from(*b) - 128.0) / 128.0,
            _ => 0.0,
        })
    }
}

/// Parses a WAV file: the `fmt ` and `data` chunks, others skipped.
pub fn parse(bytes: &[u8]) -> Result<Wave, FormatError> {
    let mut r = Reader::new(WHAT, bytes);
    r.magic(*b"RIFF", "`RIFF`")?;
    let size = r.u32()? as usize;
    if size > r.remaining() {
        return Err(FormatError::Truncated {
            what: WHAT,
            at: r.position(),
            needed: size,
            remaining: r.remaining(),
        });
    }
    let mut r = Reader::new(WHAT, r.take(size)?);
    r.magic(*b"WAVE", "`WAVE`")?;
    let mut format = None;
    let mut wave = None;
    while r.remaining() > 0 {
        let id = r.array::<4>()?;
        let len = r.u32()? as usize;
        let body = r.take(len)?;
        if len % 2 == 1 && r.remaining() > 0 {
            r.skip(1)?;
        }
        match &id {
            b"fmt " => {
                if format.is_some() {
                    return Err(damaged("two `fmt ` chunks"));
                }
                format = Some(read_format(body)?);
            }
            b"data" => {
                let Some((channels, sample_rate, bits)) = format else {
                    return Err(damaged("`data` before `fmt `"));
                };
                if wave.is_some() {
                    return Err(damaged("two `data` chunks"));
                }
                if body.len() > MAX_DATA {
                    return Err(FormatError::TooLarge {
                        what: WHAT,
                        size: body.len() as u64,
                        limit: MAX_DATA as u64,
                    });
                }
                let out = Wave {
                    channels,
                    sample_rate,
                    bits,
                    data: body.to_vec(),
                };
                if !body.len().is_multiple_of(out.frame_len()) {
                    return Err(damaged(&format!(
                        "{} bytes of samples is not a whole number of {}-byte frames",
                        body.len(),
                        out.frame_len()
                    )));
                }
                wave = Some(out);
            }
            _ => {}
        }
    }
    match (format, wave) {
        (_, Some(wave)) => Ok(wave),
        (None, None) => Err(damaged("no `fmt ` chunk")),
        (Some(_), None) => Err(damaged("no `data` chunk")),
    }
}

/// The `fmt ` chunk: channels, sample rate, bits.
fn read_format(body: &[u8]) -> Result<(u16, u32, u16), FormatError> {
    let mut r = Reader::new(WHAT, body);
    let format = r.u16()?;
    let channels = r.u16()?;
    let sample_rate = r.u32()?;
    let _bytes_per_second = r.u32()?;
    let frame_len = r.u16()?;
    let bits = r.u16()?;
    if format != FORMAT_PCM || !matches!(channels, 1 | 2) || !matches!(bits, 8 | 16) {
        return Err(FormatError::UnknownLayout {
            what: WHAT,
            found: format!("format {format}, {bits} bits, {channels} channels"),
            known: KNOWN,
        });
    }
    if sample_rate == 0 || sample_rate > MAX_SAMPLE_RATE {
        return Err(damaged(&format!("a sample rate of {sample_rate}")));
    }
    if frame_len != channels * (bits / 8) {
        return Err(damaged(&format!(
            "{frame_len} bytes per frame for {channels} channels of {bits} bits"
        )));
    }
    Ok((channels, sample_rate, bits))
}

/// Writes a WAV file with a `fmt ` and a `data` chunk, for test data.
pub fn write(wave: &Wave) -> Result<Vec<u8>, FormatError> {
    if !matches!(wave.channels, 1 | 2) || !matches!(wave.bits, 8 | 16) {
        return Err(FormatError::Unsupported {
            what: WHAT,
            detail: format!(
                "{} bits, {} channels (known: {KNOWN})",
                wave.bits, wave.channels
            ),
        });
    }
    if wave.data.len() > MAX_DATA || !wave.data.len().is_multiple_of(wave.frame_len()) {
        return Err(FormatError::Unsupported {
            what: WHAT,
            detail: format!(
                "{} bytes of samples is not a whole number of frames under the limit",
                wave.data.len()
            ),
        });
    }
    let frame_len = wave.channels * (wave.bits / 8);
    let pad = wave.data.len() % 2;
    let mut w = Writer::new(WHAT);
    w.bytes(b"RIFF");
    w.u32((4 + 8 + 16 + 8 + wave.data.len() + pad) as u32);
    w.bytes(b"WAVE");
    w.bytes(b"fmt ");
    w.u32(16);
    w.u16(FORMAT_PCM);
    w.u16(wave.channels);
    w.u32(wave.sample_rate);
    w.u32(wave.sample_rate.saturating_mul(u32::from(frame_len)));
    w.u16(frame_len);
    w.u16(wave.bits);
    w.bytes(b"data");
    w.u32(wave.data.len() as u32);
    w.bytes(&wave.data);
    w.zeros(pad);
    Ok(w.into_bytes())
}

fn damaged(detail: &str) -> FormatError {
    FormatError::Damaged {
        what: WHAT,
        detail: detail.to_owned(),
    }
}

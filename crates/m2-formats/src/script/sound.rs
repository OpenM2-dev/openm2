//! Sound scripts (`.mss`, `ScriptType CharacterSoundInformation`): the
//! sounds a motion or an effect plays, and when
//! (`docs/specs/formats/sound.md`, "Sound scripts").
//!
//! ```text
//! ScriptType        CharacterSoundInformation
//!
//! SoundDataCount    2
//! SoundData00       0.759000 "sound/common/walk_dirt_n.wav"
//! SoundData01       0.198000 "sound/common/walk_dirt_n.wav"
//! ```
//!
//! Each `SoundData##` is a time in seconds from the start of the motion (or
//! effect) and a file, not in time order. The script of
//! `d:/ymir work/X.msa` (or `.mse`) is `sound/X.mss`.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, Script, Token, check_type};

/// The `ScriptType` of a sound script.
pub const SCRIPT_TYPE: &str = "CharacterSoundInformation";

/// Most sounds one script may have (the client's most is 8).
pub const MAX_SOUNDS: u32 = 256;

const WHAT: &str = "sound script";

/// A parsed sound script.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundScript {
    /// The sounds in file order.
    pub sounds: Vec<TimedSound>,
}

/// One sound of a script.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimedSound {
    /// Seconds from the start.
    pub time: f32,
    /// The sound file (`sound/…/x.wav`, sometimes `.mp3`).
    pub file: RawStr,
}

impl SoundScript {
    /// Parses a sound script.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed script. Fails if its `ScriptType` is not
    /// `CharacterSoundInformation`, a `SoundData##` below `SoundDataCount` is
    /// missing, or one is not a time and a file.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        check_type(script, SCRIPT_TYPE, WHAT)?;
        let count: u32 = script.number("SoundDataCount", WHAT)?.unwrap_or(0);
        if count > MAX_SOUNDS {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: u64::from(count),
                limit: u64::from(MAX_SOUNDS),
            });
        }
        let sounds = (0..count)
            .map(|n| {
                let key = format!("SoundData{n:02}");
                let damaged = |detail: &str| FormatError::Damaged {
                    what: WHAT,
                    detail: format!("{key}: {detail}"),
                };
                let values = script.values(&key).ok_or_else(|| {
                    damaged(&format!("SoundDataCount is {count}, but there is no {key}"))
                })?;
                let [time, file] = values else {
                    return Err(damaged("expected a time and a file"));
                };
                let time = std::str::from_utf8(time.text.as_bytes())
                    .ok()
                    .and_then(|t| t.parse::<f32>().ok())
                    .filter(|t| t.is_finite())
                    .ok_or_else(|| damaged("the time is not a number"))?;
                Ok(TimedSound {
                    time,
                    file: file.text.clone(),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { sounds })
    }

    /// The script, for test data.
    pub fn to_script(&self) -> Script {
        let mut script = Script::default();
        script.push_values("ScriptType", vec![Token::bare(SCRIPT_TYPE)]);
        script.push_values(
            "SoundDataCount",
            vec![Token::bare(self.sounds.len().to_string().as_str())],
        );
        for (n, sound) in self.sounds.iter().enumerate() {
            script.push_values(
                &format!("SoundData{n:02}"),
                vec![
                    Token::bare(format!("{:.6}", sound.time).as_str()),
                    Token::quoted(sound.file.clone()),
                ],
            );
        }
        script
    }

    /// Writes the script, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script())
    }
}

/// The sound script of a motion or effect file: `d:/ymir work/X.msa` (or
/// `.mse`) becomes `sound/X.mss`. `None` for a path outside `d:/ymir work/`
/// or without an extension. Case and slashes are normalized.
pub fn script_path(file: &[u8]) -> Option<Vec<u8>> {
    let lower: Vec<u8> = file
        .iter()
        .map(|b| {
            if *b == b'\\' {
                b'/'
            } else {
                b.to_ascii_lowercase()
            }
        })
        .collect();
    let rest = lower.strip_prefix(b"d:/ymir work/")?;
    let name_start = rest.iter().rposition(|b| *b == b'/').map_or(0, |n| n + 1);
    let dot = rest.iter().rposition(|b| *b == b'.')?;
    if dot <= name_start {
        return None;
    }
    let stem = rest.get(..dot)?;
    let mut out = b"sound/".to_vec();
    out.extend_from_slice(stem);
    out.extend_from_slice(b".mss");
    Some(out)
}

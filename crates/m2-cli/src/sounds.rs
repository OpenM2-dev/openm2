//! Sound commands: `sound info|wav|play|check`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use m2_assets::map::PropertyIndex;
use m2_assets::sound::{music_names, read_music, read_sound_settings};
use m2_audio::spatial::Heard;
use m2_audio::{Audio, MusicStream, Pcm, SoundKind, Volumes, kind};
use m2_formats::map::ambience::AreaAmbience;
use m2_formats::map::property::PlayType;
use m2_formats::normalize_name;
use m2_formats::wav::{self, Wave};

use crate::files::{KeysArg, open_vfs, read_input};
use crate::scripts::{SHOWN_PROBLEMS, Tally};

/// `openm2 sound …`
#[derive(Debug, Subcommand)]
pub enum SoundCommand {
    /// Print a sound's kind, sample rate, channels and length (WAV or MP3)
    Info {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode a sound or a piece of music to a 16-bit WAV file, to listen to
    /// it or look at its samples (keep it out of the repo)
    Wav {
        /// A file, or with --client a game path
        path: String,
        /// The WAV file to write
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Play a sound or a piece of music on the audio device, then exit
    Play {
        /// A file, or with --client a game path
        path: String,
        /// Volume, 0 to 1
        #[arg(long, default_value_t = 1.0)]
        volume: f32,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode every sound of a client (the archives' WAV and MP3 files, the
    /// music of its BGM folder) and check its maps' sound spots. Exit code 1
    /// if a sound does not decode
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// Runs a `sound` command.
pub fn sound(command: SoundCommand) -> Result<ExitCode> {
    match command {
        SoundCommand::Info { path, client, keys } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let sound = decode_whole(&bytes)?;
            println!(
                "{}, {} Hz, {}, {} frames, {:.3} s, peak {:.3}",
                match kind(&bytes) {
                    Some(SoundKind::Wav) => "WAV (PCM)",
                    Some(SoundKind::Mp3) => "MP3",
                    None => "unknown",
                },
                sound.sample_rate,
                if sound.channels == 2 {
                    "stereo"
                } else {
                    "mono"
                },
                sound.frames.len(),
                sound.duration(),
                sound.peak()
            );
        }
        SoundCommand::Wav {
            path,
            out,
            client,
            keys,
        } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let sound = decode_whole(&bytes)?;
            std::fs::write(&out, wav::write(&to_wave(&sound))?)
                .with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {} ({:.3} s)", out.display(), sound.duration());
        }
        SoundCommand::Play {
            path,
            volume,
            client,
            keys,
        } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let sound = decode_whole(&bytes)?;
            let mut audio = Audio::open(Volumes::default())?;
            let heard = Heard {
                gain: volume.clamp(0.0, 1.0),
                pan: 0.0,
            };
            if !audio.play(&sound, heard) {
                bail!("the sound could not be started");
            }
            println!("playing {path} ({:.1} s)", sound.duration());
            std::thread::sleep(Duration::from_secs_f64(sound.duration() + 0.3));
        }
        SoundCommand::Check { client, keys } => return check(&client, &keys),
    }
    Ok(ExitCode::SUCCESS)
}

/// Decodes all of a sound or a piece of music, however long.
fn decode_whole(bytes: &[u8]) -> Result<Pcm> {
    if kind(bytes) != Some(SoundKind::Mp3) {
        return Ok(m2_audio::decode(bytes)?);
    }
    Ok(MusicStream::new(Arc::from(bytes))?.into_pcm()?)
}

/// A decoded sound as a 16-bit WAV file: stereo if it was, else the left
/// channel.
fn to_wave(sound: &Pcm) -> Wave {
    let sample = |s: f32| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
    let data = sound
        .frames
        .iter()
        .flat_map(|f| {
            let mut out = sample(f.left).to_vec();
            if sound.channels == 2 {
                out.extend(sample(f.right));
            }
            out
        })
        .collect();
    Wave {
        channels: sound.channels.clamp(1, 2),
        sample_rate: sound.sample_rate,
        bits: 16,
        data,
    }
}

fn check(client: &Path, keys: &KeysArg) -> Result<ExitCode> {
    let vfs = open_vfs(client, &keys.load()?)?;
    let mut failures = Vec::new();
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rates: BTreeMap<(u32, u16), usize> = BTreeMap::new();
    let mut longest = (0.0f64, String::new());
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".wav") || name.ends_with(b".mp3"))
        .collect();
    for name in &names {
        let shown = name.escape_ascii().to_string();
        let decoded = vfs
            .read(name)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| {
                *kinds
                    .entry(match kind(&bytes) {
                        Some(SoundKind::Wav) => "WAV",
                        Some(SoundKind::Mp3) => "MP3",
                        None => "unknown",
                    })
                    .or_insert(0) += 1;
                decode_whole(&bytes)
            });
        match decoded {
            Ok(sound) => {
                *rates
                    .entry((sound.sample_rate, sound.channels))
                    .or_insert(0) += 1;
                if sound.duration() > longest.0 {
                    longest = (sound.duration(), shown);
                }
            }
            Err(error) => failures.push(format!("{shown}: {error:#}")),
        }
    }
    println!(
        "sounds: {} files ({}), decoded {}; longest {:.1} s ({})",
        names.len(),
        list(&kinds),
        names.len() - failures.len(),
        longest.0,
        longest.1
    );
    let rates: BTreeMap<String, usize> = rates
        .into_iter()
        .map(|((rate, channels), n)| {
            let layout = if channels == 2 { "stereo" } else { "mono" };
            (format!("{rate} Hz {layout}"), n)
        })
        .collect();
    println!("  {}", list(&rates));

    let music = music_names(client, &vfs);
    let mut playing = 0.0;
    for name in &music {
        match read_music(client, &vfs, name)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| decode_whole(&bytes))
        {
            Ok(sound) => playing += sound.duration(),
            Err(error) => failures.push(format!("music {name}: {error:#}")),
        }
    }
    println!(
        "music: {} files, {:.0} min in all",
        music.len(),
        playing / 60.0
    );
    match read_sound_settings(client) {
        Ok(Some(settings)) => println!(
            "volumes in {}: music {}, sounds {}",
            m2_assets::sound::CLIENT_SETTINGS,
            shown_volume(settings.music),
            shown_volume(settings.sounds)
        ),
        Ok(None) => println!("no {}", m2_assets::sound::CLIENT_SETTINGS),
        Err(error) => failures.push(format!("{error}")),
    }

    let properties = PropertyIndex::load(&vfs);
    let (mut files, mut spots) = (0usize, 0usize);
    let mut looked_up = Tally::default();
    let mut sounds = Tally::default();
    let mut plays: BTreeMap<String, usize> = BTreeMap::new();
    for name in vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b"areaambiencedata.txt"))
        .collect::<Vec<_>>()
    {
        files += 1;
        let shown = name.escape_ascii().to_string();
        let file = match vfs
            .read(&name)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(AreaAmbience::parse(&bytes)?))
        {
            Ok(file) => file,
            Err(error) => {
                failures.push(format!("{shown}: {error:#}"));
                continue;
            }
        };
        for spot in &file.spots {
            spots += 1;
            let ambience = properties
                .get(spot.property)
                .and_then(|(_, property)| property.ambience().ok().flatten());
            looked_up.record(ambience.is_some(), || {
                format!("{} in {shown}", spot.property)
            });
            let Some(ambience) = ambience else {
                continue;
            };
            let play = match &ambience.play {
                PlayType::Loop => "LOOP".to_owned(),
                PlayType::Other(other) => other.to_string(),
            };
            *plays.entry(play).or_insert(0) += 1;
            let path = normalize_name(ambience.sound.as_bytes());
            sounds.record(vfs.source(&path).is_some(), || {
                format!("{} in {shown}", ambience.sound)
            });
        }
    }
    println!(
        "sound spots: {spots} in {files} files; {} with an ambience property{}, played {}; sounds{}",
        looked_up.total - looked_up.bad,
        looked_up.problems("without"),
        list(&plays),
        if sounds.bad == 0 {
            " all found".to_owned()
        } else {
            sounds.problems("missing")
        }
    );

    if failures.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    println!("{} failed:", failures.len());
    for failure in failures.iter().take(SHOWN_PROBLEMS) {
        println!("  {failure}");
    }
    Ok(ExitCode::FAILURE)
}

fn list<K: std::fmt::Display>(counted: &BTreeMap<K, usize>) -> String {
    counted
        .iter()
        .map(|(key, n)| format!("{key} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn shown_volume(volume: Option<f32>) -> String {
    volume.map_or_else(|| "not set".to_owned(), |v| v.to_string())
}

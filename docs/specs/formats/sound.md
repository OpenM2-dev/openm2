# Sounds

How openm2 reads the client's sounds: the sound files (WAV, MP3), the sound scripts (`.mss`) that say which sounds a motion or an effect plays and when, the maps' sound spots (`areaambiencedata.txt` with `Ambience` properties), the music, and the volumes of the original client's settings. How they are played is in [`../audio.md`](../audio.md).

**Code:** `m2_formats::wav` (`Wave`), `m2_formats::script::sound` (`SoundScript`, `script_path`), `m2_formats::map::ambience` (`AreaAmbience`), `m2_formats::map::property` (`Property::ambience`); `m2_assets::sound` (sound scripts by motion, music, `metin2.cfg`), `m2_assets::map` (an area's sound spots); `m2_audio::decode` (WAV and MP3 into frames, music streamed). Commands: `openm2 sound info|wav|play|check`, `script check` (sound scripts), `map info` (a map's sound spots).

**Source:** the files of a TMP4 40250 client, examined with `openm2 vfs cat`, `script check --census` and a census of the sound files' headers (`docs/source-policy.md`, "Allowed inputs"). The WAV (RIFF) and MPEG audio layouts are public formats; MP3 is decoded by the nanomp3 crate. The client program's strings (`AreaAmbienceData.txt`, `SoundDataCount`, `PlayMusic`, …) only confirmed which files it reads. No other source.

**Verified against:** that client: all 1,578 WAV files, 9 MP3 files in the archives and 25 in the `BGM` folder decode, all 3,509 sound scripts and 1,177 area sound files read, and every sound spot finds its property and sound (`openm2 sound check`, `script check`, `crates/m2-assets/tests/real_client.rs`).

## Sound files
| Kind | Where | In the client |
|---|---|---|
| WAV, 16-bit PCM | `sound/…` in the archives (`Sound.eix`, `sound2.eix`, `metin2_patch_w20_sound.eix`, …) | 1,578: mono 22,050 Hz 1,355, 32,000 Hz 112, 44,100 Hz 106, 22,000 Hz 2; stereo 44,100 Hz 3 |
| MP3 (MPEG-1 layer III) | `sound/ambience/`, `sound/common/etc/`, `sound/effect/etc/`; `bgm/` | 9 in the archives (4 of them music) |
| MP3 music | the `BGM` folder beside the archives, loose | 25 pieces, 75 minutes |

Folders: `sound/pc/` and `sound/pc2/` (the four classes), `sound/monster/` and `sound/monster2/`, `sound/npc/` (horses, mounts), `sound/common/` (steps, swings, hits, voices, spells), `sound/effect/`, `sound/ambience/` (waterfalls, a water mill), `sound/ui/` (36: clicks, windows, items, quests).

### WAV
A RIFF file: `RIFF`, a u32 size (the rest of the file), `WAVE`, then chunks, each a four-letter id, a u32 size and that many bytes, padded to an even length:

| Chunk | What | openm2 |
|---|---|---|
| `fmt ` | u16 format (1 is PCM), u16 channels, u32 sample rate, u32 bytes per second, u16 bytes per frame, u16 bits per sample (16 bytes; 18 with a u16 extra size) | read: PCM only, 8 or 16 bits, 1 or 2 channels |
| `data` | the samples, interleaved by channel, little-endian: 16-bit signed, or 8-bit unsigned around 128 | read |
| `LIST`, `cue `, `bext`, `CDif`, `JUNK`, `smpl`, … | tags, markers, editor data | skipped |

In the client: every file is PCM, 16 bits, its RIFF size the file's length less 8, its chunks ending at the file's end; about half carry other chunks too.

**openm2 refuses** another format (MP3 or ADPCM in a WAV), 24 or 32 bits, more than two channels, a sample rate of 0 or over 384,000, a frame size that does not match, `data` before `fmt `, two of either, sample data that is not whole frames, more than 64 MiB of samples, and a size past the end. The writer (for tests) writes `fmt ` and `data` only.

### MP3
MPEG-1 layer III at 44,100 Hz, mono, stereo or joint stereo; five pieces of music have 626 bytes before their first frame. openm2 hands them to nanomp3, a pure-Rust port of minimp3, and never decodes more than about 95 s of a sound at once; music is decoded while it plays.

## Sound scripts (`.mss`)
The script syntax ([script.md](script.md)), `ScriptType CharacterSoundInformation`:

```text
ScriptType        CharacterSoundInformation

SoundDataCount    3
SoundData00       0.759000 "sound/common/walk_dirt_n.wav"
SoundData01       0.528000 "sound/common/walk_dirt_n.wav"
SoundData02       0.198000 "sound/common/walk_dirt_n.wav"
```

`SoundDataCount`, then `SoundData00` … each a time (seconds from the start of the motion or effect) and a sound file, not in time order.

**Where they are:** the script of the motion `d:/ymir work/X.msa` is `sound/X.mss`; of the effect `d:/ymir work/X.mse`, `sound/X.mss` too. So `d:/ymir work/pc/warrior/general/run.msa` plays `sound/pc/warrior/general/run.mss`, and `effect/hit/blow_1/blow_1_low.mse` plays `sound/effect/hit/blow_1/blow_1_low.mss`. 3,335 scripts are beside a motion script, 37 beside an effect script; the rest name motions this client does not have (monster motions numbered otherwise, unused variants).

**In the client:** 3,509 scripts, 6,019 sounds; 14 scripts have none, most have one or two, at most 8; times from 0 to 3.663 s. Every file named is under `sound/`, 8 of them MP3. 42 name one of 25 files the client does not have. Runs and walks play steps (`walk_dirt_n.wav`, three a cycle), attacks their swings and cries, flinches and falls theirs (`sound/monster/wolf/damage_1.wav` in the wolf's `30.mss`).

**openm2 refuses** another script type, a `SoundData##` missing below the count, one that is not a time and a file, a time that is not a finite number, and more than 256 sounds.

## Sound spots (`areaambiencedata.txt`)
Each area may have one, in the map editor's block layout ([map.md](map.md), "Objects"):

```text
AreaAmbienceDataFile

Start Object000
    81625.984375 -104513.328125 11311.000000
    2293772605
    8950
    0.240000
End Object

ObjectCount 1
```

Each spot: a position (x, −y, z in centimetres, as an object's), the id of an `Ambience` property, the distance it is heard to (centimetres, written without decimals in the client), and the part of that distance where it is at full volume (openm2's reading: 0.06 to 0.58 in the client, with the property's own `maxvolumeareapercentage` beside it).

**In the client:** 1,177 files, 7 spots (in `metin2_map_a1`, `a3`, `b1`, `b3`): six waterfalls, heard to 64 to 130 m, and a warp gate's hum on `metin2_map_a3`, heard to 33 m. A file without spots is the header, two blank lines and `ObjectCount 0`.

### `Ambience` properties (`.pra`)
A property file ([map.md](map.md), "Properties") with `propertytype "Ambience"`:

| Key | What | In the client |
|---|---|---|
| `ambiencesoundvector` | the sound file | `sound/ambience/waterfall.wav`, `sound/ambience/warp_test.mp3` |
| `playtype` | how it plays | `LOOP` in both |
| `playinterval`, `playintervalvariation` | seconds between plays, and how much that varies (openm2's reading) | 0 |
| `maxvolumeareapercentage` | the part of a spot's distance at full volume (openm2's reading) | 0.3, 0.5 |

Two files, `property/b/ambience/waterfall.pra` and `warp.pra`.

## Music
25 MP3 pieces in the client's `BGM` folder (`m2bg.mp3`, `login_window.mp3`, `characterselect.mp3`, `desert.mp3`, `misty_forest.mp3`, `monkey_temple.mp3`, …), 5 of them also under `bgm/` in `BGM.eix`. openm2 reads a piece from the folder first (any case), then the archives. Which piece the original plays where is not in the client's data files (its interface scripts choose it); openm2's choice is in [`../audio.md`](../audio.md).

## The original client's volumes (`metin2.cfg`)
A loose file beside the archives with `KEY<tabs>value` lines (window size, gamma, shadows, …). `openm2 sound check` reads two and shows them; `play` doesn't (its own flags give the volumes):

| Key | openm2 |
|---|---|
| `MUSIC_VOLUME` | the music's volume, 0 to 1 |
| `VOICE_VOLUME` | the other sounds' volume, 0 to 1 (openm2's reading of the name) |

A value outside 0 to 1 is clamped; one that is not a number is refused; a missing file or key leaves openm2's default.

## Open
- What the spots' last number and the property's `maxvolumeareapercentage` each do, and how `playinterval` and play types other than `LOOP` play: the client has only looping spots.
- How far the original hears the actors' sounds, and from where (the character, the camera).
- Which music plays on which map.

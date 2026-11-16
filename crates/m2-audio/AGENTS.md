# m2-audio — rules for agents

`m2-audio` plays the client's sounds: WAV files (read by `m2_formats::wav`) and MP3 files (nanomp3) decoded into frames, how loud and on which side a sound is heard from where it is, and the output through kira on the system's audio device. No clock, no window, no file I/O: the caller (`openm2 play`, `world_sounds.rs`) hands in bytes and says what plays when. The rules are in `docs/specs/audio.md`, the files in `docs/specs/formats/sound.md`, the choices in `docs/architecture.md` (Design choices).

## Layout
`src/`:
- `decode.rs`: `kind` (WAV or MP3 by their first bytes), `decode` (a whole sound into a `Pcm`, capped at `MAX_SOUND_FRAMES`), `from_wave`, `MusicStream` (an MP3 decoded while it plays: kira's streaming `Decoder`, silence past its end, `into_pcm` for tools).
- `spatial.rs`: `Falloff` (full volume, then linearly to silence; `SOUND`, `spot`), `Listener` (`hear`: gain and side), `Heard`, `PAN`.
- `output.rs`: `Audio` (kira's manager and three tracks: sounds, loops, music; `play`, `start_loop`/`set_loop`/`stop_loop`, `play_music`/`stop_music`, `set_volumes`), `Volumes`, `LoopId`, `decibels`.
- `lib.rs`: `AudioError`; kira's backends re-exported (`DefaultBackend`, `MockBackend`).

`tests/audio.rs`: decoding (WAV from our writer, MP3 as silent frames written out by hand), the distance rules, and the output on kira's mock backend.

## Rules
- **No panics:** `unwrap`, `expect`, indexing and `panic!` are denied in `src/`. Sound files are untrusted: return `AudioError`, cap what is decoded, and never let a stream answer kira with nothing (its decoder thread would ask forever).
- **No device in tests:** use `Audio::<MockBackend>::with_backend`. `Audio::open` only in `openm2`; without a device the caller goes on silent.
- **No binary fixtures:** build WAV files with `m2_formats::wav::write` and MP3 data from hand-written silent frames, as `tests/audio.rs` does.
- **Agents cannot hear.** Test what is decoded and started; leave how it sounds to the human (`play`'s window, `openm2 sound play`) and say so in the handoff.
- **kira stays behind this crate's API.** `m2-cli` uses `Audio`, `Pcm`, `MusicStream` and the spatial types, not kira directly, so kira can be upgraded or replaced in one place.
- **Licenses:** kira comes without its decoders (symphonia); `triple_buffer` is the one MPL-2.0 exception (`deny.toml`). A new audio dependency goes through `just deny` and, for a new license, a design choice in `docs/architecture.md`.

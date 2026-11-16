# Audio

How openm2 plays an asset folder's sounds and music ([`assets.md`](assets.md), "Monsters, music and sounds"): the client's, once `assets import` has copied them. The files are described in [`formats/sound.md`](formats/sound.md); why these libraries: [`../architecture.md`](../architecture.md#design-choices).

**Code:** `m2_audio` (`decode`: sounds into frames, `MusicStream`; `spatial`: `Falloff`, `Listener`; `output`: `Audio` on kira), `crates/m2-cli/src/world_sounds.rs` (what `play` plays when), `actors.rs` (`MotionSound`: a motion's or effect's sounds), `world_effects.rs` (`due`: the events a motion's time passed; `take_started`: the effects that started). Commands: `openm2 play` (sounds and music), `openm2 sound play` (one sound).

**Status:** the rules below are openm2's. The original client's sound settings (how far a sound is heard, where from, which music plays where) are not known. Agents cannot hear: the tests check what is decoded, scheduled and started (kira's mock backend), and people check the rest by listening in `play`'s window.

## Output
- kira on the system's default audio device (cpal: WASAPI on Windows, ALSA on Linux, Core Audio on macOS). Without a device `play` says so and goes on silent; `--mute` plays nothing, and `--screenshot` never opens the device.
- Three tracks: one-shot sounds (at most 64 at once; more are dropped), loops (the map's sound spots, at most 32) and music. Sounds and loops share the sounds' volume.
- Volumes are amplitudes from 0 to 1: the player's settings file's ([`client.md`](client.md), "The player's settings"), else 1 for sounds and 0.5 for music; `--sound-volume` and `--music-volume` set them for the session. The Settings window changes them while playing, with a short fade, and its Mute silences everything, the device staying open ([`ui.md`](ui.md), "The game menu"). An asset folder holds no player's settings, and the original client's `metin2.cfg` isn't read.
- Each sound plays at its own sample rate; kira resamples to the device's. A mono sound plays in both speakers.

## Where a sound is heard from
- **The listener** is our character, its right the camera's right: a sound to the right of the screen is heard on the right.
- **Loudness falls with distance,** linearly from full volume to silence: an actor's or effect's sound is at full volume within 10 m and silent from 50 m (`Falloff::SOUND`). A sound out of reach when it starts is not played (nor read).
- **The side:** the direction to the sound against the listener's right, times 0.7 (kira's panning: −1 is the left speaker only). A sound at the listener is centred.
- One-shot sounds keep the loudness and side they started with; loops follow the listener every frame (in 0.15 s).

## What plays
| When | What | Where from |
|---|---|---|
| An actor's motion passes a time of its sound script | that sound (`sound/X.mss` beside `d:/ymir work/X.msa`, [formats/sound.md](formats/sound.md); an asset folder's motion file's `[[sound]]`) | the actor's feet |
| An effect starts once (a hit's, a motion event's) | the sounds of its sound script (`sound/X.mss` beside `X.mse`), or of an asset folder's effect file, each at its time | where the effect starts |
| A hit (any Damage but a miss) | `sound/common/hit/hit_sword_1.wav`, `_2`, `_3` in turn (an asset folder's `hit_sounds`) | the hit's flash |
| Our character drinks a potion that gives health or mana back (EntityEffect) | `sound/ui/eat_potion.wav`, once a drink (an asset folder's `potion_sound`), as the client's scripts set it and its source plays it, for its own character only | our feet |
| A character gains a level | the level-up effect's own sound (`sound/effect/etc/levelup_1/level_up.mss`), as any effect's | its feet |
| Our character comes within a sound spot's reach | its property's sound, looped, until we leave it | the spot |
| A screen shows | its music, looped: below | — |

- **Motion sounds** follow the motion's clock exactly as its effects do (`render.md`, "Effects"): a looping motion (run, wait) plays its sounds every cycle (a run's steps), a motion played once (an attack, a flinch, a fall) plays them once, a motion just started plays the sounds at its time 0. A sound timed after the motion's end never plays (openm2): the warrior's run (0.667 s) lists steps at 0.198, 0.528 and 0.759 s, and playing the third each time the run goes round would make the steps limp, three uneven ones a cycle; so it is two a cycle, a third of a second apart at speed 100. Sounds of our character, monsters and other characters alike: steps, swings, attack cries and bites, flinches, falls.
- **Hits:** the weapons' item scripts name hit sounds under `d:/ymir work/sound/battle/`, which this client does not have, and the general hit effect openm2 plays (`blow_1.mse`) has no sound script; so openm2 plays the hit sounds that the hit effect `blow_1_low` names (its second), taking the three in turn (openm2's choice). The one hit flinches the actor too, whose damage motion plays its own cry.
- **Sound spots:** the spot's distance (`areaambiencedata.txt`) is where the sound fades out; its full-volume part (the spot's last number) is heard at full volume (`Falloff::spot`). All of the client's spots loop (`LOOP`); other play types play as loops too.
- **Effects placed on maps** loop and play no sound scripts; nor do the twinkles over items.
- A sound that is missing from the client or does not decode is said once on the console (`note: sound …`) and skipped.

## Music
An asset folder names each screen's in `assets.toml`'s `[music]` ([`assets.md`](assets.md)); the import copies the client's below.

| Screen | Music (from the client's `BGM` folder, else `bgm/` in the archives) |
|---|---|
| Login | `login_window.mp3` |
| Character select and create | `characterselect.mp3` |
| Loading | what played before goes on |
| Game, every map | `m2bg.mp3` |

openm2's choice by the files' names: which piece the original plays on which map is chosen by its interface scripts, not by data files. A new piece fades in over 2 s while the last one fades out; music is decoded while it plays and loops.

## Not yet
- Interface sounds (`sound/ui/`: clicks, windows, picking up items, wearing): which action plays which is in the original's interface scripts. The potion's is played (above).
- Music by map, the spots' `playinterval`. (Skills play their motions' sound scripts like any motion.)
- Sounds in `view map` and `view model`.

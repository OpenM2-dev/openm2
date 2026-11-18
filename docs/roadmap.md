# openm2 — Roadmap

What openm2 does today, what comes next, and what is still unknown. When you finish an item, remove it from **Next**. When you find a new question, add it to **Open questions**. Design changes go in [architecture.md](architecture.md#design-choices), under Design choices.

## What works

**The game client (`openm2 play`)**
- **Screens:** the Servers board (servers by address, trust by key), sign-in with an OpenID Connect provider in the system's browser, character select and create, and loading screens.
- **The world:**
  - The map around the character: terrain, water, buildings with their floors, the sky, and trees grown from openm2's own recipes.
  - The maps' effects and sound spots, with music.
- **Moving:** click to walk around the map's walls, or run with W/A/S/D. The camera follows and can be turned.
- **Characters and monsters:**
  - Animated, wearing their armour and weapons, with names and levels over them.
  - Mounts that follow their owner and can be ridden.
- **Fighting:**
  - Weapon combos, targets with a ring, damage numbers and misses.
  - Monsters that fight back, flinch, fall and shoot.
  - Death and restart, and the experience's flight.
- **Items:**
  - The inventory, wearing and using items, dropping and picking up.
  - Quickslots on four pages.
  - NPC shops, upgrades at smiths and with scrolls, with the shine and glows of upgraded items.
  - Potions' and level-ups' effects.
- **Skills:**
  - The character window with stats and status points, and the Skills tab.
  - Skill groups chosen at a teacher, and skill books and soul stones.
  - Skills on quickslots with cooldowns; their motions, effects, states and the affects bar.
- **Social:** chat, whispers with envelopes, and a friends list.
- **Other features:**
  - Fishing.
  - Warps to other maps.
  - The minimap and the atlas.
  - The game menu with settings (volumes, interface size, names), Change character, Log out and Quit.
  - Paste and copy in text fields.
  - The interface at the display's scale.

**Tools and formats**
- **The client's files:** archives (EterPack, MCOZ/LZO1X/XTEA), proto tables, GR2 models (Oodle1), DDS, TGA, JPEG, WAV/MP3, text scripts, maps, effects and sounds. Every parser has a writer.
- **`openm2 assets import`:** converts the player's own client into an asset folder of open formats (glTF, PNG, TOML), on their machine. `assets check` and `assets current` read such a folder back.
- **`openm2 content import`:** writes the server's content (maps, walk masks, monsters, items, skills and spawn lists) from the operator's own client.
- **Viewers:** `openm2 view model|map|tree|effect|ui`, in a window or into a PNG.
- **Headless:** `openm2 bot`, a headless client for tests and checks. `openm2 status` checks a server.

**The server (`openm2-server`)**
- **Accounts:** sign-in through OpenID Connect providers, or, where the operator allows it, with the device's own key (no browser, no internet), with no passwords. Accounts and characters live in Postgres, in a save's folder (one file, for a server on the player's own machine), or in memory for tests.
- **The world:** monsters and NPCs from a content folder. Spawns, packs, and stones that call packs. Walk masks hold for players and monsters, and moves are checked.
- **Gameplay:** fights, deaths and levels, drops and gold, shops, skills and their effects, teachers, master training, mounts and stables, upgrades, fishing, warps, whispers and friends.
- **Game masters:** named by the operator (by account, or the owner's device key), with `/level`, `/gold`, `/item`, `/heal`, `/speed`, `/gm on|off`, and their choice of empire.
- **Operations:** OpenTelemetry, a Docker image and a compose file with Postgres. One world per database or save. A ready line and a stop on stdin's close for a program that runs the server.

**Protocol and security**
- **The protocol:** openm2's own, versioned, generated from `crates/m2-protocol/packets/openm2.toml`, with byte-exact vectors for every message.
- **Transport:** TLS 1.3 with ALPN `openm2/1`. Servers are trusted by their certificate or by a pinned key. Logins use single-use 32-byte tickets.

**Releases and updates**
- **Releases:** a tag publishes the server's image to GHCR, and the Windows client with `openm2-launcher` to a GitHub Release, with third-party notices.
- **`openm2-launcher`:** a window that keeps an install up to date from a channel's signed manifests and bundles. It imports the player's client once (Browse buttons for the folder and the keys file on Windows), imports it again when a version reads clients differently, then starts the game.
- **The release updates itself:** `openm2-setup.exe` installs it for the player alone (or the zip, unzipped anywhere), and its launcher follows the latest GitHub release, a flat channel of the programs only, signed with openm2's own key.
- **`openm2 release`:** makes keys, writes, signs and verifies manifests.

## Next
- **Local saves in the launcher:** a button that hosts a named save on the player's own machine, the player its game master. The steps left: the launcher running `openm2-server` on a save (its ready line, stopped when the game ends), the content imported beside the assets, the saves' window, and the server in the Windows release.
  - Then LAN: a save opened to the network, its link (address and key) to copy, and a "This device" button on the login screen for friends' games.
- **An original sample world** in the repo, under `docs/source-policy.md`, "Original content". It needs a rule that lets it through `just no-assets`.
  - The gate: `play` on our own server with only the sample world, and no Metin2 files on the machine.
- **A WebGPU build.**
- **Fuzz targets** for the parsers and the protocol, run in Linux CI (nightly and cargo-fuzz).
- **The chat window:** the taskbar's other button.
- **Mounts in the interface:**
  - a Mount row in the Skills tab for horseback skills
  - mount lines in the Character tab
  - clicking our own mount to ride
  - name tags raised over riders
  - horseback skills' own motions
- **Deleting characters:** the original asks for a code typed on the select screen.
- **The server's image carries the third-party notices** (`just notices --package m2-server --target x86_64-unknown-linux-gnu`). Today it carries only webpki-roots' terms.
- **The launcher on a real channel on Windows:** an update to a new version, and a manifest edited by hand refused. The first release with its own channel is the first that can show it: the one after it updates from it.

## Open questions
- **Run speeds with a weapon:** walks are timed with the race's run speed without a weapon, on the server and in the client alike. A character holding a weapon runs with that weapon's motion, so its feet may slide a little. Timing by the weapon would need the import to take each weapon kind's run speed, and the server to send it when the weapon changes.
- **Clocks drifting apart:** the client sets its estimate of the server's clock once, from GameTime. The server makes up for moves up to 500 ms old. Over a long session, clocks drifting apart by more than that could get paths refused at their corners. A GameTime now and then would fix it.
- **Monsters that cannot reach a player:** an aggressive monster with no path to a player searches for one again about every tick while the player stays in sight. Remembering whom it could not reach would spare the searches.
- **Floors in the original:** how the original picks among a building's height planes is unknown. openm2 uses the highest floor over the character or the terrain (`render.md`, "Floors"). Also open: whether the collision shapes' plane sizes are sides or half sides, what a shape of kind 1 is, and what `isattributedata` means (`mdatr.md`).
- **Combat and item numbers:** openm2 chooses these itself (`world.md`): the fists, experience by level difference, the level table, points per level, hit checks, drop ownership and lifetime, pick-up reach, stack size and drop limits. Monster gold may be generous with imported monsters; `drops.toml` could take a gold chance.
- **Sky in the original:**
  - How the original lays the gradient's bands on its box.
  - Whether `CloudScale` is the cloud layer's width or half of it, and whether the layer is flat.
  - What unit `CloudSpeed` is in, and what `CloudColor` does.
  - Whether the fog covers the sky.
  - openm2's choices are in `render.md`, "Maps: the sky".
- **One device key everywhere:** a device's key names its account on every server that takes keys, so two such servers can tell it is the same player. A key per server (derived from the server's key) would not, but a new server key would then mean a new account.
- **Servers open to devices' keys:** anyone who reaches one makes accounts, as many as they like. A cap on accounts, or a list of the keys allowed, may be wanted before such a server is opened to more than friends.
- **Linking a provider to an account:** a provider's first sign-in makes a new account. Adding a second provider to it would need the player signed in with both at once, never by e-mail.
- **Names in other codepages:** the import decodes names with codepage 1252 only. Other locales' names keep their ASCII and get `?` for the rest. A full decoder (for example `encoding_rs`) is a new dependency, partly under BSD-3, which is a licence decision.
- **Item names in other codepages:** `play` shows an asset folder's names, which are UTF-8. The protocol carries UTF-8, so names from the server show as they are. Only the import's decoding is limited (see "Names in other codepages").
- **Effects in the original:** some effect script values are openm2's reading (`effect.md`):
  - billboard types 0, 4 and 5
  - the rotation and texture animation types
  - `EmittingDirection`
  - whether `MaxEmissionCount` caps the particles alive or born
  - the Bézier positions
  - the units of gravity and air resistance

  openm2 draws effects in the order they were added, unsorted, so blended ones can cover each other wrongly.
- **Sound in the original:**
  - How far the actors' sounds carry, and whether they are heard from the character or the camera.
  - What a sound spot's last number and `maxvolumeareapercentage` do.
  - How `playinterval` and play types other than `LOOP` play.
  - Which hit sound a weapon makes, and which interface action plays which `sound/ui/` sound.
  - openm2's rules are in `audio.md`.
- **Skill numbers:** openm2 chooses these itself (`world.md`): a skill's power per level, how long hits count, hits per monster, the states' chances and times, pushes, the mana shield's share, toggles' costs. Two things in the skill table are not read: what its second amount means where the second point is `NONE`, and its splash damage.
- **Skills in the original:**
  - What the original checks before using a skill (range, mana, cooldown).
  - Whether it walks up to a far target first (openm2 uses the skill where the character stands).
  - What its move's argument and motion number are at a master's grade.
  - Which `_me`/`_target` motion a shaman's blessing plays.
  - Which icon a perfect master shows.
  - openm2's rules are in `ui.md`, "Skills".
- **Shops, minimap and items on the ground in the original:**
  - **Shops:** how near the original walks to an NPC, whether and from how far it closes a shop, what it sends as ShopBuy's count, and how it asks before buying or selling.
  - **Minimap:** its zoom steps and its marks.
  - **Items on the ground:** how items lie on the ground, and its key for picking up.
  - openm2's rules are in `ui.md` and `render.md`.
- **Equipment backdrop:** which place of `equipment_bg_without_ring.tga` belongs to which worn item is openm2's reading of the silhouettes (`ui.md`, "The inventory"). Three places hold nothing.
- **Item and monster list duplicates:**
  - 203 numbers are listed twice in each locale's `item_list.txt`, and 7 in `npclist.txt`. openm2 takes the first line (`script.md`).
  - 21 sub-images go past the image they name. openm2 cuts them to the image.
- **The original's interface font:** its face, size and text effects. openm2 uses the first installed of Tahoma, Verdana, Arial, … at 12 px, plain.
- **`ttf-parser` advisory:** ignored in `deny.toml` until cosmic-text moves to fontdb 0.24. Check at each cosmic-text release.
- **GR2 format 7:** two costume models (`shaman_lord.gr2`, `warrior_rabbit1.gr2`) use Granny's format 7. Read them when a costume needs them.
- **`MotionDuration`:** in 450 of 5,992 motion scripts it differs from the animation's length, by up to 33 s. openm2 plays the animation's length. Which one the original uses matters for combo timing.
- **Terrain texture repeat:** a texture with scale s repeats every 3,200 / s cm, chosen by eye (`render.md`, "Terrain").
- **Attribute bits `0x04`, `0x08`, `0x40`, `0x80`:** `0x04` marks circles on a1's town square and gates (community documentation says no fighting). The others cover outlines or whole maps (`map.md`). openm2 reads none of them.
- **Trees:** wind, simpler trees far away, and the trees' own shadows.
- **Map details** (`map.md`):
  - the extra object lines
  - which of two property files with the same id the client keeps
  - how the original blends tiles and draws water
  - whether it adds ambient light
  - a separate light for characters
- **Long walks:** the client sends one move to the end point and one wait at arrival. Whether the original sends more on a long walk is unknown.
- **Motion list weights:** a motion and its variants usually add up to 100, but 282 of 3,437 do not. Whether the weight is a chance of picking the variant is unknown.
- **Weapons in other hands:** unchecked:
  - which hand holds a bell or a fan (openm2: the right)
  - whether daggers and bows sit right at the bones' origins

  Costume bodies and polymorph shapes' motions are not drawn.
- **Combo steps:**
  - Which `argument` the original sends with a combo step's move, and whether it sends `kind` 2 for single attacks.
  - In which order a weapon's combo motions follow each other.
  - openm2 sends the step from 1 (`combat.md`). The motion scripts say `combo_04` and `combo_07` end a chain (`script.md`, "Combo input"). How the original reaches `combo_05` is unknown.
- **Combo and attack timing:** what `ComboInputData`'s and `AttackingData`'s times and types do in the original, and what motion event types 2 and 7 to 9 are. openm2's reading of the combo window is in `script.md`; the rest is kept as numbers.
- **Hit bodies:** model scripts carry collision spheres, mostly two with the same centre, of types 1 and 3 (`script.md`, "Race data"). openm2's swings hit any of them, placed where the model stands. Open:
  - which type the original's swings hit
  - what the other type is for
  - whether an offset from a bone turns with the bone
- **Attacking with the mouse:** whether a click on a monster attacks once or until it dies, how near the original walks first, what Space does without a target, and how big it takes an entity to be. openm2's rules are in `render.md`, "Our fighting".
- **The client's interface scripts:** the client's `root/*.py` would say:
  - which combo motion follows which
  - what the restart window sends
  - which music plays on which map

  openm2 has not read them yet.
- **`LOCAL_` keys:** four `season1/pc/*.msm` scripts name some models only with `LOCAL_*` keys, a locale variant that openm2 does not read.
- **Lighting maps:** some dungeon and guild models name second textures (`…LightingMap`), mostly not shipped. Whether the client uses them is unknown.
- **Archive priority:** the VFS mounts archives in the client's `pack/Index` order, and the first one wins. That is inferred, not confirmed. What the list's path prefixes mean is unknown (`eterpack.md`, "Archive list").
- **Proto details:** the meaning of the item fields at offsets 4 and 60, and the signedness of limit and apply values.

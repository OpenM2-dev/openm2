# Movement

How our character walks, and how the client shows other entities walking.

## Coordinates
Positions are world coordinates in centimetres: `x` grows to the east, `y` to the south. The client's atlas says which map holds a position ([`formats/map.md`](../../formats/map.md), "Atlas"); the map's corner is subtracted to place it on the map.

## Rotation
- `angle` (CharacterMove, EntityMove, EntitySpawn) is the direction faced, in whole degrees in a u16: 0 faces south (+y), 90 east, 180 north, 270 west.
- It is the angle of the way faced from (0, 1), turning towards +x. `Point::angle_to` works it out from two points, rounded to the nearest degree.
- Senders send 0 to 359; readers take the value modulo 360.
- openm2's server turns its monsters in steps of 5 degrees ([`server/world.md`](../../server/world.md), "Strolling").

## Kinds
Move kinds keep the values the client's game files use:

| `kind` | Meaning |
|---|---|
| 0 | wait: the entity stands at `x`, `y` |
| 1 | move: the entity walks to `x`, `y` |
| 2 | attack |
| 3 | combo attack; `argument` is the step, from 1 |
| 4 | monster skill |
| 128 + n | skill motion n: the skill's place in its job's numbering, 1 to 6 or 16 to 21 ([`skills.md`](skills.md), "Using a skill") |

- `argument` is 0 for the other kinds.
- The server closes the connection for any other kind from the client: past 4 without bit 7 set.

## Our character walks
The client leads its own character; the server follows.
1. **Start:** to walk to a point, the client sends CharacterMove `kind` 1 with the **end point**, the direction, and `time`: its estimate of the server's clock (GameTime's value plus the time since it arrived; 0 before GameTime), which the server checks the move at. It moves its own character there at once, in a straight line.
2. **Arrive:** when the walk is over, it sends CharacterMove `kind` 0 with the end point, on the first tick after (ticks come about every 100 ms).
3. **Stop:** stopping halfway sends `kind` 0 with the point reached.
4. A new walk while walking starts from the point reached.

### Paths
A walk may be a **path**: points to walk to in turn, each leg a straight line (`m2_core::Command::MoveTo`). `openm2 play` finds its paths around the map's walls ([`render.md`](../../render.md), "Walking around walls"); the bot walks one leg.
- The first leg starts at once, with its CharacterMove `kind` 1 as above.
- Each next leg starts **where and when the one before it ended**, with its own CharacterMove `kind` 1 to its end point, facing along it. The client sends it on the first tick after, or before any command it handles in between, but its `time` is the server clock at the moment the leg began, not when it was sent.
- Only the last leg ends with the `kind` 0 of "Arrive"; a stop halfway stops on whichever leg the character is on, and the rest of the path is dropped.
- The server sees one walk after another and needs nothing new.

What the server does with them ([`server/session.md`](../../server/session.md), "Moving"):
- In the game only: a move outside it closes the connection. The moves of a dead character are ignored.
- **The move's time:** `time` is read as the server's clock (the nearest time with those 32 bits), then held to no earlier than the mover's last move nor **500 ms** before the move arrived, and no later than its arrival. Lag up to half a second is made up for, and saying a move is older gains a client nothing. The move is checked from where the server has the character at that time, along its walk.
- `kind` 1: the walk starts there, at that time, and must stay on the character's map and cross no blocked square of the map's walk mask ([`server/world.md`](../../server/world.md), "Walls"). It takes the time "Speed" gives. A path's legs, each sent with the time it began, start where the one before ended.
- Any other kind must be on the map, within **2 m** of where the server has the character at that time, with no blocked square between; it puts the character at the move's point.
- A `kind` 0 at the end of the walk the server is still timing lets the walk go on: the client arrived first.
- It sends EntityMove with the mover's `vid`, the same `kind`, `argument`, `angle` and point, and what is left of the walk's `duration` (0 for other kinds), to the players that see the mover. Nothing goes back to the mover.
- A move that breaks these rules is refused (below).

### Refused moves
- The server stops the character where it has it at that moment: the players that see it get EntityMove `kind` 0 there, and the mover gets **MoveRefused** {x, y} with that point.
- The core ends the walk and its path at once, with no wait sent back: our character stands at the point, and `Event::MoveRefused` says where. Moves already on their way are checked from there and may be refused too.
- `openm2 play` draws the character there and does not walk to what the walk went for again from there ([`render.md`](../../render.md), "Walking around walls"); the bot goes on with its next walk from there.

The client ignores an EntityMove for its own vid: it leads its own moves.

## Speed
A walk takes as long as the entity's run motion needs for the distance, scaled by its movement speed. The client and the server use the same rule, so both agree:
- `speed` = the run motion's forward distance a second: its `Accumulation` (a motion script value, [`formats/script.md`](../../formats/script.md)) over its duration. A warrior covers 300 cm in 0.667 s: 450 cm/s.
- `duration` = distance / `speed` × 1000, in whole milliseconds, times `percent` / 100. `percent` is 100 at movement speed 100, 200 − speed below it, and 10000 / speed above it (whole numbers). Players have movement speed 150, which gives 66 %.
- The server takes `speed` from the race's `run_speed` in its content, else 450 cm/s ([`server/content.md`](../../server/content.md), "Race"), and tells the client in MainCharacter's `run_speed`. The core times our walks with it (`World::run_speed`); without it (0) it walks nowhere. `openm2 play` draws its run motion at the motion's own pace, which a weapon's run can make a little faster or slower than the walk.

## Other entities walk
EntityMove: the entity walks in a straight line from where the client shows it now to `x`, `y`, taking `duration` milliseconds from the message's arrival, and faces `angle`. A `duration` of 0 puts it there at once.
- The client walks every EntityMove the same way, whatever its kind; the kind and the argument say which motion plays.
- openm2's server sends:
  - a player's moves as above;
  - a monster's stroll or chase as `kind` 1 with its end point and duration;
  - a monster's bite as `kind` 2 where it stands, facing its victim, with `duration` 0 ([`combat.md`](combat.md), "Monsters fight back");
  - a monster stopping as `kind` 0 where it stands;
  - for a monster that comes into view while it walks, an EntityMove `kind` 1 right after its EntitySpawn, with the milliseconds left.
- `m2-core` puts an entity whose move has no duration at its new point at once; `openm2 play` draws such a jump as a run ([`render.md`](../../render.md), "Moves without a time").

## What the client keeps
Every entity in `m2_core::World::entities` has a `Track`: from, to, start and duration. Its position at any time is on the straight line between the two points (`Track::at`). The core never reads a clock: `now` comes in with every input.
- Our character's track is the leg it is on; `World::path` holds the points after it, and the speed. `World::follow_path(now)` starts the legs that have begun by `now`: the session does it on every tick and command, and `openm2 play` on its copy of the world before each frame, so the character turns its corners on time between ticks.

## Not covered yet
- **Clocks:** the client sets its estimate of the server's clock once, from GameTime. Over a long session the two can drift apart; past half a second the server stops making up for the difference, and paths may be refused at their corners (`docs/roadmap.md`, "Open questions").

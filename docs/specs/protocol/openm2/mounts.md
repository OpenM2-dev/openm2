# Mounts (openm2 protocol)

openm2's own. A player's mount: calling it, getting on and off, what others see, its stamina, training it at a stable. The server's rules are in [`../../server/world.md`](../../server/world.md), "Mounts"; the layouts in [`messages.md`](messages.md).

## Messages
| Dir | Id | Message | What |
|---|---|---|---|
| c2s | 37 | RideMount `{ride}` | get on (`true`) or off |
| c2s | 38 | TrainMount `{vid}` | train our mount at the stable `vid` |
| s2c | 48 | EntityMount `{vid, look}` | a player rides a mount that looks like monster `look`; 0 on foot |
| s2c | 49 | MountState `{level, grade, look, stamina, most, ride_from, fights, skills, skill_points, summoned}` | our mount; level 0 is none |
| s2c | 50 | RunSpeed `{run_speed}` | our run speed changed: centimetres a second at movement speed 100 |
| s2c | 51 | StableOpened `{vid, next}` | a stable's offer: the next level (`MountOffer`) or none past the last |

`MountOffer` is `{level, grade, look, move_speed, stamina, rider_level, gold, items, skill_points}`, `items` a list of `MountCost {vnum, count}` (at most 8).

## The flow
- **Calling:** using the mount's call item (UseItem) calls it: the mount appears 1.5 m beside its rider as an ordinary EntitySpawn of kind 8 (a horse), its race the look, and follows its rider with ordinary EntityMoves. MountState says which vid (`summoned`). Using the call again sends it away (EntityRemove).
- **Getting on:** RideMount `true`. The standing mount goes (EntityRemove); the rider gets, in order, EntityMount (also to the players that see it), EntityUpdate with the riding speed, PointsChanged, RunSpeed, MountState. A player coming into view of a rider gets EntityMount after its EntitySpawn and EntityInfo.
- **Getting off:** RideMount `false`: EntityMount with look 0, EntityUpdate, PointsChanged, RunSpeed; the mount appears beside the rider again; MountState.
- **Riding walks** are CharacterMoves as on foot. The server and the client time them with the run speed RunSpeed gave and the movement speed of EntityUpdate, so both agree; a walk under way when the speed changes keeps its timing.
- **A charge:** a horseback skill with the `charge` flag lets one skill move (a CharacterMove with the skill bit) go as far as the skill's target range, at once; the client sends it to the point the skill takes its rider.
- **Stables:** Talk to a stable NPC answers StableOpened, in place of any shop or teacher open. TrainMount answers PointsChanged, ItemSets, MountState, StableOpened again and an info line; a refusal is an info line.
- Refusals (no mount, too tired, too far, the bag full, the costs) are info lines (Chat kind 1), as a teacher's are.

## Client rules
- The client draws a rider's mount under it, the rider at the mount's `saddle` bone ([`../../render.md`](../../render.md), "Riding").
- It sends no RideMount without a mount, and no attack while riding a mount that does not fight; job skills only on foot, horseback skills only riding a mount whose grade has them.
- A stable's window closes with the X, Escape, walking away, the NPC leaving, or a shop or teacher opening.

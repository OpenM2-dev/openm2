# Fishing (openm2 protocol)

openm2's own. Casting a worn fishing pole's line onto water, the bite, reeling in, and what items used raise for a while. The server's rules are in [`../../server/world.md`](../../server/world.md), "Fishing"; the layouts in [`messages.md`](messages.md).

## Messages
| Dir | Id | Message | What |
|---|---|---|---|
| c2s | 42 | Cast `{angle}` | cast our line that way: degrees as CharacterMove's (0 south, 90 east) |
| c2s | 43 | ReelIn | reel our line in: after a bite to catch what bit, before one to stop |
| s2c | 57 | Fishing `{vid, step, x, y}` | a step of player `vid`'s fishing, its float at (`x`, `y`) in world units |
| s2c | 58 | ItemBoosts `{boosts}` | what items we used raise on us for now: the whole list whenever it changes |

Fishing's `step`: 0 it cast (or it was fishing already when it came into view), 1 it reeled in before a bite or stopped, 2 a fish bit, 3 it caught something, 4 what bit got away. A client that knows no other number ignores it. The float is where the server checked the water: every client draws it there.

`boosts` is a list of `ItemBoost {item, point, amount, seconds}` (at most 16): the item used, the point raised as Affect's numbers (1 attack speed, 2 movement speed, 12 hidden, 13 strength, 14 dexterity), by how much, and the seconds left. An empty list comes once when the last boost ends; a player that never had one gets none.

## The flow
- **The pole:** a fishing pole is worn as the weapon (UseItem). Its sockets in ItemSet say its state: socket 0 its points, socket 2 the bait on it (an item number, 0 for none). Bait is used from the bag (UseItem) and goes onto the worn pole; bait already on it comes back to the bag.
- **Casting:** Cast. The server answers Fishing (cast) to us and to the players that see us, or an info line (Chat kind 1) that says why not: no pole, no bait, riding, walking, a map without fish, no water at the float. A Cast while a line is out is ignored.
- **The bite:** Fishing (bite), some seconds later, to the same players.
- **Reeling in:** ReelIn. Before the bite: Fishing (reeled in), the bait kept. After it: the pole's ItemSet (the bait gone, a point maybe), Fishing (caught) or Fishing (lost), then for a catch the ItemSet of where it went, and info lines: what was caught or that it got away, and the pole's points.
- **Waiting too long:** after the bite's time, Fishing (lost) with the pole's ItemSet and an info line.
- **What ends a line:** a move other than a stop where we stand, an attack, a skill, mounting, the pole's place changing, dying, a warp or a restart end it with Fishing (reeled in); after a bite the bait is gone.
- **Coming into view:** a player we start to see with a line out comes with Fishing (cast), and Fishing (bite) when a fish bit.
- **The Fisherman:** a pole dropped onto a fisher NPC is GiveItem ([`upgrades.md`](upgrades.md)): UpgradeOffer with no gold or items, the pole's chance, `on_fail` 1 (or 2 for the first pole); Upgrade makes the next pole, with no points or bait.
- **Opening and the fire:** an item that opens (a live fish, a clam) is used with UseItem: the ItemSets that changed and an info line. The fire's item lights a fire NPC (EntitySpawn) a metre before us; an item dropped onto it with GiveItem is grilled, the whole stack, in its place.
- **Boosts:** an item that raises a point for a while, used with UseItem, answers its ItemSet, ItemBoosts, then EntityUpdate and PointsChanged as a skill's raise does.

## Client rules
- The client casts only with a pole worn, on foot, with no line out, the way it finds water 6 m out (`docs/specs/ui.md`, "Fishing"); the server checks it all again with its own reach.
- While our line is out, the attack key, a move key or a click on the ground sends ReelIn instead (at most every 0.5 s), and the character doesn't move.
- A Fishing step plays its motion and the fishing effects at the float; the bite shows a fish over the fisher's head.

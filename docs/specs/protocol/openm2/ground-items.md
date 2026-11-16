# Items on the ground

Items lying on the ground: dropped by a player, left by a monster, picked up.

## Appearing and leaving
- A ground item has a `vid` from the same series as players and monsters, and players see it by the same rules: from within 100 m of where it lies, until it is more than 120 m away ([`server/world.md`](../../server/world.md), "View").
- Coming into view, a player gets **GroundItemAdd** (`vid`, `vnum`, `x`, `y`), then **ItemOwnership** (`vid`, `owner`).
- `vnum` 1 is gold. How much gold lies there is not sent.
- There is no height: the client puts the item on the ground.
- Going out of view, picked up or vanished: **GroundItemRemove**.
- Items lying near where we enter come after the entities around us ([`session.md`](session.md), "Entering the game").

## Owners
- ItemOwnership's `owner` is the name of the character who alone may pick the item up; empty for anyone.
- A player's drop has no owner. A monster's drop belongs to its killer for 30 s; then the players that see it get ItemOwnership with an empty `owner`.
- Every item vanishes 5 min after it lands.
- These times are the server's ([`server/world.md`](../../server/world.md), "The ground").

## Picking up
- **PickUp** `vid`. The client sends it only for an item it knows, in the game, with our character in the world and alive. Walking up to it first is the caller's business; `openm2 play` does ([`render.md`](../../render.md), "Items on the ground").
- The server takes an item the player sees, within 5 m, that is not someone else's while it is owned ([`server/world.md`](../../server/world.md), "The ground"):
  - **gold** goes to our gold (point 11), then PointsChanged;
  - **an item** goes onto the bag's stacks of it first, then to the first free place: an ItemSet for each slot that changed, then the info line `You picked up <name>.` (`You picked up <name> (<count>).` for more than one);
  - then GroundItemRemove goes to everyone who sees the item.
- Refused with an info line, the item staying where it is: someone else's (`That is not yours yet.`), more gold than a character holds (`You cannot carry more gold.`), a full bag (`Your bag is full.`). Anything else is ignored.

## Dropping
- **DropItem** `window`, `slot`, `count`: some or all of an item of the bag, where our character stands. The client sends it for a bag slot (below 90) that holds an item, with `count` from 1 to the stack's count.
- **DropGold** `amount`: some of our gold, from 1 to what we have.
- What the server does ([`server/world.md`](../../server/world.md), "The ground"):
  - an item: an ItemSet for the slot (what is left, or empty), then the item lands;
  - gold: PointsChanged, then the gold lands;
  - it lands where the player is, with no owner, and the players that see the spot get GroundItemAdd and ItemOwnership;
  - only in the game and alive, never a worn item, and at most 10 drops in 10 s; refusals are silent.

## What the client keeps
- `m2_core::World::ground`, by vid: the item number, where it lies, and its owner (`None` for anyone, and until ItemOwnership says).
- A GroundItemAdd of a vid already there replaces it; ItemOwnership and GroundItemRemove of a vid the client does not have are ignored (an `Ignored` event).
- Phase 1 or 2 empties it, with the entities.
- Events: `GroundItemAppeared` (vid, number, where), `GroundItemLeft` (vid).
- `Command::PickUp { vid }`, `Command::Drop { slot, count }` and `Command::DropGold { amount }` send their messages in the game only, with our character in the world and alive; otherwise, or for anything the rules above refuse, an `Ignored` event. The world changes when the server answers.

## Not covered yet
- Items that may not be dropped (the item table's anti flags).
- How much gold lies on the ground.
- Parties.

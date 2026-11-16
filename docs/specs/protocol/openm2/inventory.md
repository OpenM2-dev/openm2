# Inventory

The items in our character's bag and on its body, using and moving them, and the quickslots.

## Slots
Window numbers keep the values the client's game files use. openm2 keeps everything in window 1 (inventory): the bag, then what the character wears. Window 2 (equipment) is never sent.

| Slots of window 1 | What |
|---|---|
| 0–89 | the bag: two pages of 5 columns × 9 rows; slot = page × 45 + row × 5 + column |
| 90 and up | worn items: 90 + the wear position ("Worn items") |

An item is 1 to 3 cells tall: its size in the item table ([`formats/proto.md`](../../formats/proto.md); the server's content has the same, [`server/content.md`](../../server/content.md), "Item"). It sits in its top slot and covers the slots 5 and 10 below on the same page; it never crosses into the next page.

## Worn items
| Wear position | Slot | What | Worn by items with |
|---|---|---|---|
| 0 | 90 | armour | wear flag 1 |
| 1 | 91 | helmet | wear flag 2 |
| 2 | 92 | shoes | wear flag 4 |
| 3 | 93 | bracelet | wear flag 8 |
| 4 | 94 | weapon | wear flag 16 |
| 5 | 95 | necklace | wear flag 32 |
| 6 | 96 | earrings | wear flag 64 |
| 7, 8 | 97, 98 | two unique items | wear flag 128 |
| 9 | 99 | arrows | wear flag 512 |
| 10 | 100 | shield | wear flag 256 |

- Positions and flags keep the values the client's game files use. Which flag counts when an item has several, and when a unique item takes the second place, are the server's rules ([`server/world.md`](../../server/world.md), "Items").
- The game's numbering goes on with costumes (19 a body costume, 20 a hairstyle); openm2 does not use them yet.
- `openm2 play` shows positions 0 to 10 ([`ui.md`](../../ui.md), "The inventory").

## Items arriving
- **Entering the game:** an ItemSet for each item, the bag's then the worn ones, after our own EntitySpawn and EntityInfo ([`session.md`](session.md), "Entering the game").
- **A slot changes** (an item used, moved, dropped, picked up, bought or sold): an ItemSet for each slot that changed. A slot emptied: ItemSet with `vnum` 0 and `count` 0.
- openm2's server fills `window`, `slot`, `vnum`, `count` and `sockets` (a fishing pole's points in socket 0 and its bait's item number in socket 2, [`fishing.md`](fishing.md); 0 for every other item); flags are 0, and the bonuses empty.

## Using an item
- **UseItem** `window`, `slot`: wear the item, take it off, or use it up. The client sends it in the game only. The server works out what happens ([`server/world.md`](../../server/world.md), "Items"):
  - a worn item comes off to the first free place of the bag;
  - an item that can be worn goes on at its position, and what was worn there goes back to the bag;
  - a potion gives back health and mana, and one is used up;
  - anything else does nothing.
- **A change on the body** is answered with EntityUpdate (what we wear and our speeds) to us and to the players that see us, then PointsChanged, then an ItemSet clearing each slot an item left and one for each slot it went to.
- **A potion** is answered with its ItemSet, then PointsChanged, then EntityEffect for the health and the mana it gave back, to us and the players that see us ([`entities.md`](entities.md), "Effects").
- **Refusals:** a full bag and an item the character may not wear get an info line in the chat (`Your bag is full.`, `Your character cannot wear this.`, `You need level N to wear this.`); other refusals are silent.
- Outside the game the server closes the connection; while our character is dead it ignores the message.

## Moving an item
- **MoveItem** `from_window`, `from_slot`, `to_window`, `to_slot`, `count`. The client sends the item's whole count.
- The server moves the whole stack ([`server/world.md`](../../server/world.md), "Items"):
  - into the bag, where all the cells it would cover are free, its own not counting;
  - onto the body, to an empty position that suits it, if the character may wear it;
  - a stackable item onto a stack of the same item tops that stack up, and the rest stays.
- **The answer:** EntityUpdate and PointsChanged when what we wear changed, then an ItemSet for the old slot and one for the new.
- Otherwise nothing changes and nothing is sent, but the info line for an item the character may not wear.

## Quickslots
- There are 8, `index` 0 to 7. Each holds a `kind` and a `position`: kind 1 an item (`position` is its slot in the bag), kind 2 a skill (`position` is the skill's number). The kinds keep the values the client's game files use.
- Each request is answered with the matching message: SetQuickSlot with QuickSlotSet, ClearQuickSlot with QuickSlotCleared, SwapQuickSlots with QuickSlotsSwapped.
- Quickslots are 0 to 31: four pages of eight, as the original's taskbar has them (eight, 0 to 7, before). The server ignores an index of 32 or more, another kind, or an item slot past the bag, without an answer. The dead may change their quickslots.
- Quickslots do not follow items: after a move, a quickslot still points at the old slot.
- They are kept with the character and sent while loading ([`session.md`](session.md), "Entering the game").

## What the client keeps
- `m2_core::World::items`, by window and slot: `vnum`, `count`, flags, anti flags, sockets, and the bonuses whose kind is not 0. An ItemSet with `vnum` or `count` 0 removes the slot.
- `m2_core::World::quickslots`, by index: kind and position. A QuickSlotSet of index 32 or more is ignored (an `Ignored` event).
- Both are emptied by Phase 1 or 2; the server sends them again.
- `Command::UseItem`, `MoveItem`, `SetQuickSlot`, `ClearQuickSlot` and `SwapQuickSlots` send their messages in the game only (an `Ignored` event otherwise, and for a quickslot index of 32 or more). The world changes when the server answers.

## Not covered yet
- Splitting a stack, costumes and the hairstyle slot.

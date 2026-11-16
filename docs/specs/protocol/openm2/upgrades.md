# Upgrades (openm2 protocol)

openm2's own. Upgrading an item of the bag to its next item: dropped onto a smith NPC, or with a scroll put onto it. The server's rules are in [`../../server/world.md`](../../server/world.md), "Upgrades"; the layouts in [`messages.md`](messages.md).

## Messages
| Dir | Id | Message | What |
|---|---|---|---|
| c2s | 39 | GiveItem `{vid, slot}` | an item of the bag dropped onto NPC `vid`: what does upgrading it there take? |
| c2s | 40 | UseItemOnItem `{slot, onto}` | the item at `slot` put onto the one at `onto`, both of the bag: an upgrade scroll onto what it upgrades |
| c2s | 41 | Upgrade `{smith, scroll, slot, vnum}` | upgrade the item an offer showed, at the smith `smith` or with the scroll at `scroll` (one of them), if `vnum` is still at `slot` |
| s2c | 53 | UpgradeOffer `{smith, scroll, slot, vnum, next, gold, items, chance, on_fail}` | what upgrading the item `vnum` at `slot` to `next` takes, at a smith or with a scroll |

`items` is a list of `UpgradeCost {vnum, count}` (at most 4): what an upgrade uses up, as `gold` is, whether it works or not. `chance` is in percent. `on_fail` says what a failure does: 0 the item is gone, 1 it goes down a level, 2 it stays as it is; a client that knows no other number shows one it doesn't know as 0.

## The flow
- **At a smith:** Talk to a smith NPC answers an info line that says how it upgrades. An item of the bag dropped onto it is GiveItem; the smith answers UpgradeOffer, or an info line that says why not. Another NPC answers `Only a smith upgrades items.`
- **With a scroll:** an item of the bag put onto another of the bag is UseItemOnItem. A scroll answers UpgradeOffer for the item it was put on, or an info line; any other item does nothing. (Putting an item onto the same item still stacks it with MoveItem.)
- **Upgrading:** Upgrade with the offer's smith or scroll, place and item. The server keeps nothing between the offer and it, and checks it all again. It answers PointsChanged (the gold), an ItemSet for each place that changed (the item, the scroll, the materials) and an info line: the upgrade worked, or failed with what that did. A refusal is an info line; an Upgrade whose item is no longer at its place is ignored.
- Refusals (not a smith, a worn item, an item that does not upgrade, a scroll's limit, the gold and the items) are info lines (Chat kind 1), as a teacher's are.

## Client rules
- The client sends GiveItem only for an item of the bag, once its character is near the NPC, and UseItemOnItem only for two different places of the bag.
- Upgrade sends what the open offer showed and closes it; the next offer comes only when asked.
- The offer closes with the X, Cancel, Escape, walking away from the smith, the smith leaving, the item's or the scroll's place changing, or a shop, teacher or stable opening.

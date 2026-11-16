# NPCs and their shops

Talking to an NPC, its shop opening, buying from it, selling to it and closing it.

## Goods
- A shop's places are a grid 5 wide and 8 high: place `x + 5 y`, 0 to 39. A tall item (its size, 1 to 3) covers the places below it.
- ShopOpened's `goods` list the places that hold something: `position`, `vnum`, `price` (gold for the whole stack) and `count` (how many one purchase gives). Empty places are left out.
- Which goods a shop has, where each lies and what it costs are the server's ([`server/world.md`](../../server/world.md), "Shops").

## Talking
- **Talk** `vid`. The client sends it only for an NPC it knows (kind 1), in the game, with our character in the world and alive (an `Ignored` event otherwise). `openm2 play` walks up to 2.5 m of the NPC first ([`ui.md`](../../ui.md), "The shop").
- The server takes an NPC the player sees, within 5 m. A teacher answers **TeacherOpened** and the server forgets any shop open ([`skills.md`](skills.md), "Teachers"). An NPC with goods opens its shop: **ShopOpened**. An NPC without goods, or anything else, does nothing.

## The shop
- **ShopOpened** always names the NPC (`vid`) and carries its goods. It replaces any shop open before.
- From then on the server holds this shop as ours until ShopClose, until another shop or a teacher opens, or until a purchase or sale finds its NPC gone or more than 12 m away, which forgets it.
- The server never sends a close of its own.

## Buying
- **ShopBuy** `position`. The client asks only for a place that holds a good whose price our gold pays.
- What the server does ([`server/world.md`](../../server/world.md), "Shops"):
  - less gold than the price: **ShopRefused** `reason` 1;
  - no room in the bag (onto stacks of the item first, then the first free place): ShopRefused `reason` 2;
  - after a refusal nothing is taken and nothing else is sent;
  - otherwise the gold goes and the item comes into the bag: PointsChanged, then an ItemSet for each slot that changed;
  - an empty place, or no shop open, is ignored.

## Selling
- **ShopSell** `slot`, `count`: `count` (1 to the stack's) of the item in a slot of the bag. The client sells only items of the bag (slots below 90), with the stack's whole count.
- The server pays the item's sell value for each, then sends the slot's ItemSet (what is left, or empty), then PointsChanged.
- Gold past the most a character holds is refused with the info line `You cannot carry more gold.`; anything else is ignored. Worn items cannot be sold.

## Closing
- **ShopClose**: the server forgets our shop.
- The client sends it when the window closes (`Command::CloseShop`), and when the shop's NPC leaves (EntityRemove): the core forgets the shop and says so. `openm2 play` closes the shop when our character walks more than 10 m from the NPC.
- The server handles requests in the order they come, so buys and sells sent just before a ShopClose still count.
- Outside the game, Talk and the shop messages close the connection.

## What the client keeps
- `m2_core::World::shop`: the NPC's vid and what it sells by place (item number, price, count). Set by ShopOpened; cleared by `Command::CloseShop`, by the NPC's EntityRemove, and by Phase 1 or 2.
- Events: `ShopOpened` (the NPC's vid), `ShopClosed`, `ShopRefused` (`NotEnoughGold` or `BagFull`).
- `Command::Talk { vid }`, `Buy { position }` and `Sell { slot }` work in the game only, with our character in the world and alive; `CloseShop` needs only an open shop. Otherwise, or for anything the rules above refuse, an `Ignored` event. The bag and our gold change when the server answers.

## Not covered yet
- Quests and dialogs at NPCs.
- Players' own shops.
- Items that may not be sold.

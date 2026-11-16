# Friends

Friendships between characters: asking, answering, ending one, and seeing friends come and go. The server's rules: [`server/session.md`](../../server/session.md), "Friends"; what it stores: [`server/storage.md`](../../server/storage.md), "Friends".

## A friendship
- It is between two characters, both ways at once, and stored: it lasts across sessions until either side ends it.
- A character has at most `Config::max_friends` (50; at most 64, what Friends carries).

## Asking and answering
- **AskFriend** (`name`): to a player in the game, by its character's name in any case. The server answers with **FriendNotice** (`name`, `notice`), and the asked player gets **FriendAsked** (`name`, ours).
- **AnswerFriend** (`name`, `accept`): the asked player's answer. Accepting makes the friendship when both lists have room: both get **FriendChanged** status 2. Declining tells the asker (notice 7).
- Asking a player who asked us first accepts their request. A request waits a minute (`Config::friend_ask_time`) and ends when either player leaves the game.
- The client keeps the requests waiting for our answer, oldest first, eight at most (`World::friend_requests`); answering one, or notice 9 about it, takes it away.

| Notice | Meaning |
|---|---|
| 1 | our request went out |
| 2 | nobody of that name is in the game |
| 3 | we are friends already |
| 4 | our list is full |
| 5 | their list is full |
| 6 | too many requests: ours in a short time (`Config::friend_ask_burst` in `Config::friend_ask_window`, 5 a minute), or waiting for them (`Config::max_friend_asks`, 5) |
| 7 | they declined |
| 8 | our request ended unanswered: it expired, or they left |
| 9 | their request to us ended: it expired, they left, or there was none to answer |

## Ending a friendship
- **RemoveFriend** (`name`): a friend of ours, in the game or not. Both lose the friendship at once; we get **FriendChanged** status 0, and so does the friend if it is in the game with that character.

## Friends coming and going
- **Friends** (`friends`: `name`, `online`): our friends after entering the game, by name in lowercase; not sent when we have none.
- **FriendChanged** (`name`, `status`): 0 no longer a friend, 1 a friend out of the game, 2 a friend in the game. A friend entering or leaving the game sends it to its friends in the game whose lists hold it.
- The client keeps the friends by name in lowercase (`World::friends`) and reports each change (`Event::FriendChanged`: added, in the game, out of it, removed). The phase going to the character list or loading forgets them: they are the character's.

## Out of turn
Each of AskFriend, AnswerFriend and RemoveFriend outside the game closes the connection. A name no character could have, or our own, is ignored; RemoveFriend of someone not in our list too.

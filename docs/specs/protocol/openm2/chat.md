# Chat

Sending a chat line or a chat command, receiving chat lines, and whispers to one player.

## Kinds
Chat kinds keep the values the client's game files use:

| Value | Kind | Notes |
|---|---|---|
| 0 | talk | players nearby |
| 1 | info | lines from the server |
| 2 | notice | a server notice |
| 3 | party | |
| 4 | guild | |
| 5 | command | the answer to a command |
| 6 | shout | everyone in the game |
| 8 | big notice | shown in the middle of the screen |

openm2's server takes 0 and 6, and sends 0, 1 and 6.

## Sending
- **SendChat** carries `kind` and `text`, in the game only. The client refuses it before (an `Ignored` event), and refuses a line over 512 bytes.
- The server closes the connection for a SendChat outside the game. It drops empty lines, lines over 512 bytes and lines with control characters, without an answer ([`server/session.md`](../../server/session.md), "Chat").
- **Talk** (0): the line goes out as `Name: text`, kind 0, with the speaker's `vid` and `empire`, to the speaker and to every player that sees it.
- **Shout** (6): the same line, kind 6, to every player in the game.
- Other kinds are ignored.
- So the speaker gets its own line back.

## Commands
A talk line starting with `/` is a command, not a line for others:
- `/logout` and `/quit`: leave the game ([`session.md`](session.md), "Leaving");
- `/skillgroup 1` or `/skillgroup 2`: choose the skill group in a world without teachers, a game master's command in one with them ([`skills.md`](skills.md), "Skill numbers and groups");
- anything else: the info line `Unknown command <name>`.

`/w` and `/whisper` are the client's own: `play` sends them as whispers (below), never as chat.

Restarting and spending skill and status points are messages of their own (Restart, RaiseSkill, RaiseStatus), not commands.

## Receiving
- **Chat** carries `kind`, `vid` (the speaker; 0 for the server), `empire` (0 for the server) and `text`, the line as shown.
- The server's own lines (answers, refusals) are info lines: kind 1, `vid` 0, `empire` 0.
- The client marks lines whose `vid` is our own character's as **mine** (`ChatLine::mine`) and keeps the last 100 lines (`World::chat`). Chat lines are taken in any stage.

## Text
- Text is UTF-8 both ways.
- Text from the server is untrusted. `openm2 bot` prints it with control characters escaped, and `openm2 play` shows them as `?`, so a server cannot send terminal control sequences.
- The server writes its own lines with control characters as `?`.

## Whispers
A line to one player, by its character's name. Whispers are never stored: the player must be in the game.
- **SendWhisper** carries `name` (string<24>) and `text` (string<512>), in the game only. The client checks it as the server does and sends nothing for a name no character could have (1 to 24 ASCII letters and digits), our own name, or a line that is empty, over 512 bytes or with control characters (an `Ignored` event).
- The server closes the connection for a SendWhisper outside the game, drops the text checks' failures without an answer, and ignores the name's ([`server/session.md`](../../server/session.md), "Whispers").
- **Whisper** carries `name`, `mine` and `text`:
  - to the other player, `mine` false and our character's name;
  - back to us, `mine` true and the other's name as it was created.
- **WhisperRefused** carries the `name` as we wrote it and `reason`: 1 nobody of that name is in the game (whether or not a character has it), 2 too many whispers in a short time (`Config::whisper_burst` in `Config::whisper_window`, 10 in 10 s, each whisper counted whether or not it arrives).
- The client keeps eight conversations, a hundred lines each (`World::whispers`, by the other's name in any case; past eight, the one whose last line is oldest goes). A refusal is a line of its conversation when there is one. The phase going to the character list or loading forgets them.
- `play`: `/w NAME TEXT` and `/whisper NAME TEXT` in the chat send one; `/w NAME` opens its conversation ([`ui.md`](../../ui.md), "Whispers").

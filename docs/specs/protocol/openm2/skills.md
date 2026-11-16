# Skills

Our character's skills: their numbers and groups, their levels, spending skill points, and using a skill.

## Skill numbers and groups
- A skill has a number, as in the client's skill tables. Each job has twelve, from 30 × job + 1: the warrior's (job 0) 1 to 6 and 16 to 21, the ninja's 31 to 36 and 46 to 51, the sura's 61 to 66 and 76 to 81, the shaman's 91 to 96 and 106 to 111. The job is the class value modulo 4 ([`session.md`](session.md), "Characters").
- A character uses one of two **skill groups** (1 or 2; 0 before one is chosen): group 1 has the job's first six skills, group 2 the six from 15 further on (`m2_core::group_skills`).
- The group comes with MainCharacter, and **SkillGroup** says when it changes.
- The player chooses it once, from level 5, at a **teacher** (below), or with the chat command **`/skillgroup 1`** or **`/skillgroup 2`** in a world without teachers (a game master's command in one with them). The server answers SkillGroup, or an info line that says why not ([`server/world.md`](../../server/world.md), "Skills").

## Teachers
- A teacher is an NPC of the server's content that teaches one job's skill group ([`server/content.md`](../../server/content.md), "Teachers"). The client can't tell one by its number.
- **Talk** to a teacher (as to a shop's NPC, [`shops.md`](shops.md)) answers **TeacherOpened** {`vid`, `job` (0 warrior, 1 ninja, 2 sura, 3 shaman), `group` (1 or 2), `level` (the level from which a group is chosen)} in place of ShopOpened. The server forgets any shop open; the client closes its shop too.
- **LearnSkillGroup** {`vid`} learns the teacher's group. The server checks it again, with nothing kept from the talk: in game and alive, the teacher seen and within 12 m, of the character's job, the level reached, no group yet. It answers SkillGroup and an info line; a teacher of another class, too low a level or a group chosen already get an info line, anything else is ignored.
- Closing a teacher tells the server nothing.
- Other numbers are skills of any job (support, horse and guild skills); openm2 does not use them yet.

## Levels
- **Skills** lists the skills our character has learned: `skill`, `level`, `grade` and `next_read`. Skills left out are not learned.
- The server sends it after entering, after the ItemSets (an empty list for none), and after each skill point spent.
- **Level:** 1 to 19 normal, 20 to 29 master (M1 to M10), 30 to 39 grand master (G1 to G10), 40 perfect master.
- **Grade:** 0 normal, 1 master, 2 grand master, 3 perfect master; it follows the level.
- `next_read`: when the next skill book or soul stone may be used on it, in seconds since 1970; 0 for any time, and a time gone by means now ("Training").

## Training
Decision 039; the server's rules in [`server/world.md`](../../server/world.md), "Training".
- **A skill book** (item kind 17, its skill in `values[0]`) is used with **UseItem**, as any item ([`inventory.md`](inventory.md)). A master's skill of our group (M1 to M10) whose wait is over may rise a level, M10 to G1; one book goes either way, and the skill waits again (`next_read`). The server answers ItemSet, an info line, then Skills; a refusal is an info line and uses nothing.
- **A soul stone** (an item the server's content names) used with UseItem answers **ChooseSkill** {`slot` (the stone's place in the bag), `skills` (the skills it may train now: our group's, G1 to G10, their wait over)}, or an info line when there are none. Nothing is used yet.
- **UseItemOnSkill** {`slot`, `skill`} trains the skill picked with the item there: the server checks it all again, with nothing kept from ChooseSkill (in game and alive, a soul stone at that place of the bag, the skill trainable now). Then as a book: ItemSet, an info line, Skills; G10 to P. A refusal of the skill is an info line; anything else is ignored.
- **Items that help** (named by the content) are used with UseItem: one ends every skill's wait (Skills follows), one makes the next training likelier. An info line says what each did, or why it was kept.
- **A book that forgets** (item kind 22) used with UseItem takes every skill of our job and our group: QuickSlotCleared for each quickslot that held one of them, SkillGroup 0, PointsChanged (our skill points: one a level-up), Skills, an info line. With nothing to forget, an info line and nothing used.

## Spending a skill point
- **RaiseSkill** `skill` spends one skill point (point 28, [`entities.md`](entities.md), "Points") on one of our skills.
- The server takes a skill of the player's group that its content has, once the character's level reaches the skill's own, while the skill is below master. A point raises it one level, and near master may make it a master early. It answers PointsChanged, then Skills ([`server/world.md`](../../server/world.md), "Skills").
- Refusals get an info line: no group yet, a skill it cannot learn, its level too low, a master's skill, no points left.

## Using a skill
1. **The motion:** CharacterMove with `kind` 128 + the skill's motion number, `argument` 0, our position and the direction faced. The motion number is the skill's place in its job's numbering, skill − 30 × job: 1 to 6 or 16 to 21 (`m2_core::skill_motion`). That is the skill descriptions' motion number for every job skill of the client ([`formats/script.md`](../../formats/script.md), "Skill descriptions"). The server passes the move on to the players around as EntityMove with the same kind.
2. **The skill:** **UseSkill** with `skill` and `target`, the entity it is used on, if any. The server takes a learned skill of the player's group, in the game and alive, once its cooldown is over and its mana pays the cost. It takes the mana, starts the cooldown and sends PointsChanged. A skill used on oneself raises some points for a while: EntityUpdate (our speeds) to us and to the players that see us, then PointsChanged and Affects (below). An attack skill's hits name their targets; a skill used on others (a shaman's) goes on the player `target` names, else on us: a monster gets an info line, and so does a player too far for the skill, with nothing spent. A skill that stays on and is on goes off at its next UseSkill.

## What skills raise
Decision 042; the server's rules in [`server/world.md`](../../server/world.md), "Skill effects".
- **Affects** lists what skills raise on our character for now, ours and others' on us: `skill`, `point` (1 attack speed, 2 movement speed, 3 attack grade, 4 defence, 5 resistance, 6 reflection, 7 critical hits, 8 dodging, 9 health from hits, 10 mana shield, 11 casting speed, 12 hidden; 0 for a skill that stays on and raises nothing), `amount` (percent for speeds and chances, points for grades), `seconds` (left, rounded up; 0 while a skill that stays on is on). A skill that raises two points is listed twice.
- The server sends the whole list whenever it changes: a use, a raised point ending, a skill turned off, our death (an empty list). Nothing comes at entering: raised points end at a logout.
- What others see of a player's or a monster's skills is EntitySpawn's and EntityUpdate's `affects` ([`entities.md`](entities.md), "State and parts"), and which skills are on a player, by number, EntitySkills ([`entities.md`](entities.md), "Skills on players").
3. **The hits:** when the skill motion's attack spheres touch an entity, **Hit** with `skill` set to the skill's number. The server counts them while the skill's use lasts (3 s), within its ranges and limits ([`server/world.md`](../../server/world.md), "Skills"). Then as any hit: Damage, TargetHealth, the monster's answer or its death ([`combat.md`](combat.md)).

- A cooldown not over, or too little mana, gets an info line (`The skill is ready again in N s.`, `You do not have enough mana.`); anything else is ignored.
- The client sends the motion first, then UseSkill, only for a job's skill that is learned, with our character in the world and alive; a walk in progress stops first.
- Other players' skill moves come as EntityMove with bit 7 set: `openm2 play` plays the skill of that player's job with that motion number ([`render.md`](../../render.md), "Playing").

## What the client keeps
- `m2_core::World::skill_group`: from MainCharacter, then SkillGroup.
- `m2_core::World::teacher`: the teacher we look at (vid, job, group, level), from TeacherOpened until `Command::CloseTeacher`, its NPC leaving, a ShopOpened, or Phase 1 or 2. A TeacherOpened closes the shop open (`ShopClosed`). Events: `TeacherOpened` (the vid), `TeacherClosed`.
- `Command::LearnSkillGroup { vid }` sends LearnSkillGroup for the teacher open, in the game only; `Command::CloseTeacher` forgets it. Otherwise an `Ignored` event.
- `m2_core::World::skills`: the skills learned, by number (level, grade, `next_read`). Replaced by each Skills; cleared by Phase 1 or 2.
- `m2_core::World::skill_choice`: what a soul stone offers (its slot, the skills: none numbered 0, each once), from ChooseSkill until `Command::TrainSkill` or `Command::CloseSkillChoice`, an ItemSet at its slot, or Phase 1 or 2. Events: `SkillChoiceOpened` (the slot), `SkillChoiceClosed`.
- `Command::TrainSkill { skill }` sends UseItemOnSkill with the choice's slot for a skill it offers, in the game only, and closes it; `Command::CloseSkillChoice` forgets it. Otherwise an `Ignored` event.
- Events: `SkillGroupChanged` (the group); `SkillChanged` (number, level, grade) for each skill that changed, and after entering for each one learned.
- `Command::SkillUp { skill }` sends RaiseSkill, in the game only. `Command::UseSkill { skill, angle, target }` works in the game, alive, for a learned job skill. Otherwise an `Ignored` event. `Command::Hit { vid, skill }` tells a hit, `skill` 0 for an attack.
- The quickslots hold skills as kind 2 with the skill's number ([`inventory.md`](inventory.md), "Quickslots").

## Not covered yet
- Passive and support skills, horse and guild skills, skills on other players.
- How a master's or grand master's motion would be numbered in the move: openm2 sends the same number at every grade.

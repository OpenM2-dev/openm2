//! Writing content back as files: the same text for the same content, in a
//! fixed order, readable by [`crate::parse`].

use std::fmt::Write as _;

use crate::FORMAT;
use crate::fishing::Fishing;
use crate::model::{
    Content, Drop, GoldDrop, Group, Item, ItemBonus, Level, Map, Monster, Mounts, Npc, OnFail,
    Pack, Race, ShopGood, Skill, StartItem, Upgrades,
};
use crate::sources::SPAWNS_DIR;
use crate::walk::WalkError;

/// The first line of every file `openm2 content import` writes: the data
/// comes from the operator's own client and must never be committed
/// (`docs/source-policy.md`). `just no-assets` refuses files that start with it.
pub const IMPORT_MARKER: &str =
    "# openm2 content import: from the operator's own client; never commit (docs/source-policy.md)";

/// What goes above the format line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// Nothing.
    Plain,
    /// [`IMPORT_MARKER`].
    Imported,
}

impl Content {
    /// The folder's files as (path, text): `content.toml`, `maps.toml`,
    /// `monsters.toml`, `items.toml`, `races.toml`, `levels.toml` if the
    /// content has levels, `drops.toml` if it has drops, `start.toml` if
    /// new characters start with anything, `shops.toml` if NPCs sell
    /// anything, `teachers.toml` if NPCs teach, `training.toml` if skills
    /// train otherwise than by default, `mounts.toml` if there are mounts,
    /// `skills.toml` if there are skills, `packs.toml` if there are packs,
    /// then one file per spawn file.
    pub fn to_files(&self, header: Header) -> Vec<(String, String)> {
        let mut files = Vec::new();

        let mut text = start(header);
        let _ = writeln!(text, "name = {}", string(&self.name));
        files.push(("content.toml".to_owned(), text));

        let mut text = start(header);
        for map in self.maps.values() {
            write_map(&mut text, map);
        }
        files.push(("maps.toml".to_owned(), text));

        let mut text = start(header);
        for monster in self.monsters.values() {
            write_monster(&mut text, monster);
        }
        files.push(("monsters.toml".to_owned(), text));

        let mut text = start(header);
        for item in self.items.values() {
            write_item(&mut text, item);
        }
        files.push(("items.toml".to_owned(), text));

        let mut text = start(header);
        for race in self.races.values() {
            write_race(&mut text, race);
        }
        files.push(("races.toml".to_owned(), text));

        if !self.levels.is_empty() {
            let mut text = start(header);
            for level in &self.levels {
                write_level(&mut text, level);
            }
            files.push(("levels.toml".to_owned(), text));
        }

        if self.all_drops().next().is_some() || self.all_golds().next().is_some() {
            let mut text = start(header);
            for drop in self.all_drops() {
                write_drop(&mut text, drop);
            }
            for gold in self.all_golds() {
                write_gold(&mut text, gold);
            }
            files.push(("drops.toml".to_owned(), text));
        }

        if self.start != crate::model::Start::default() {
            let mut text = start(header);
            if self.start.level != 1 {
                let _ = writeln!(text, "level = {}", self.start.level);
            }
            if self.start.gold > 0 {
                let _ = writeln!(text, "gold = {}", self.start.gold);
            }
            for item in &self.start.items {
                write_start_item(&mut text, item);
            }
            files.push(("start.toml".to_owned(), text));
        }

        if !self.skills.is_empty() {
            let mut text = start(header);
            for skill in self.skills.values() {
                write_skill(&mut text, skill);
            }
            files.push(("skills.toml".to_owned(), text));
        }

        if !self.shops.is_empty() {
            let mut text = start(header);
            for (npc, goods) in &self.shops {
                for good in goods {
                    write_good(&mut text, *npc, good);
                }
            }
            files.push(("shops.toml".to_owned(), text));
        }

        if !self.teachers.is_empty() {
            let mut text = start(header);
            for teacher in self.teachers.values() {
                let _ = writeln!(text, "\n[[teacher]]");
                let _ = writeln!(text, "npc = {}", teacher.npc);
                let _ = writeln!(text, "job = {}", string(teacher.job.name()));
                let _ = writeln!(text, "group = {}", teacher.group);
            }
            files.push(("teachers.toml".to_owned(), text));
        }

        if self.training != crate::model::Training::default() {
            let mut text = start(header);
            let training = &self.training;
            if training.books != crate::model::Books::default() {
                let _ = writeln!(text, "\n[books]");
                let _ = writeln!(text, "chance = {}", training.books.chance);
                let _ = writeln!(text, "wait = {}", training.books.wait);
            }
            if training.stones != crate::model::Stones::default() {
                let stones = &training.stones;
                let items: Vec<String> = stones.items.iter().map(u32::to_string).collect();
                let _ = writeln!(text, "\n[stones]");
                let _ = writeln!(text, "items = [{}]", items.join(", "));
                let _ = writeln!(text, "chance = {}", stones.chance);
                let _ = writeln!(text, "wait = {}", stones.wait);
            }
            for help in &training.helps {
                let _ = writeln!(text, "\n[[help]]");
                let _ = writeln!(text, "item = {}", help.item);
                if help.skip_wait {
                    let _ = writeln!(text, "skip_wait = true");
                }
                if help.chance > 0 {
                    let _ = writeln!(text, "chance = {}", help.chance);
                }
            }
            files.push(("training.toml".to_owned(), text));
        }

        if let Some(mounts) = &self.mounts {
            let mut text = start(header);
            write_mounts(&mut text, mounts);
            files.push(("mounts.toml".to_owned(), text));
        }

        if !self.upgrades.is_empty() {
            let mut text = start(header);
            write_upgrades(&mut text, &self.upgrades);
            files.push(("upgrades.toml".to_owned(), text));
        }

        if !self.fishing.is_empty() {
            let mut text = start(header);
            write_fishing(&mut text, &self.fishing);
            files.push(("fishing.toml".to_owned(), text));
        }

        if !self.packs.is_empty() {
            let mut text = start(header);
            for pack in self.packs.values() {
                write_pack(&mut text, pack);
            }
            files.push(("packs.toml".to_owned(), text));
        }

        for (name, spawns) in &self.spawns {
            let mut text = start(header);
            for group in &spawns.groups {
                write_group(&mut text, group);
            }
            for npc in &spawns.npcs {
                write_npc(&mut text, npc);
            }
            files.push((format!("{SPAWNS_DIR}/{name}.toml"), text));
        }
        files
    }

    /// The walk masks and water masks as (path, PNG bytes), one per path
    /// the maps name; an imported mask carries [`IMPORT_MARKER`] as its
    /// text.
    pub fn walk_files(&self, header: Header) -> Result<Vec<(String, Vec<u8>)>, WalkError> {
        let comment = (header == Header::Imported).then(|| IMPORT_MARKER.trim_start_matches("# "));
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for (name, map) in &self.maps {
            let masks = [
                (&map.walk, self.walks.get(name)),
                (&map.water, self.waters.get(name)),
            ];
            for (path, mask) in masks {
                let (Some(path), Some(mask)) = (path, mask) else {
                    continue;
                };
                if files.iter().any(|(written, _)| written == path) {
                    continue;
                }
                files.push((path.clone(), mask.to_png(comment)?));
            }
        }
        Ok(files)
    }
}

fn start(header: Header) -> String {
    let mut text = String::new();
    if header == Header::Imported {
        text.push_str(IMPORT_MARKER);
        text.push('\n');
    }
    let _ = writeln!(text, "format = {FORMAT}");
    text
}

fn write_level(out: &mut String, level: &Level) {
    let _ = writeln!(
        out,
        "
[[level]]"
    );
    let _ = writeln!(out, "level = {}", level.level);
    let _ = writeln!(out, "experience = {}", level.experience);
}

fn write_gold(out: &mut String, gold: &GoldDrop) {
    let _ = writeln!(
        out,
        "
[[gold]]"
    );
    let _ = writeln!(out, "monster = {}", gold.monster);
    let [least, most] = gold.amount;
    let _ = writeln!(out, "amount = [{least}, {most}]");
    let _ = writeln!(out, "chance = {}", float(gold.chance));
}

fn write_drop(out: &mut String, drop: &Drop) {
    let _ = writeln!(out, "\n[[drop]]");
    let _ = writeln!(out, "monster = {}", drop.monster);
    let _ = writeln!(out, "item = {}", drop.item);
    let _ = writeln!(out, "count = {}", drop.count);
    let _ = writeln!(out, "chance = {}", float(drop.chance));
}

fn write_skill(out: &mut String, s: &Skill) {
    let _ = writeln!(out, "\n[[skill]]");
    let _ = writeln!(out, "vnum = {}", s.vnum);
    let _ = writeln!(out, "name = {}", string(&s.name));
    let _ = writeln!(out, "job = {}", string(s.job.name()));
    if s.level > 0 {
        let _ = writeln!(out, "level = {}", s.level);
    }
    let _ = writeln!(out, "kind = {}", string(s.kind.name()));
    if !s.flags.is_empty() {
        let flags: Vec<String> = s.flags.iter().map(|f| string(f)).collect();
        let _ = writeln!(out, "flags = [{}]", flags.join(", "));
    }
    let words = [("point", &s.point), ("point2", &s.point2)];
    let formulas = [
        ("amount", &s.amount),
        ("duration", &s.duration),
        ("amount2", &s.amount2),
        ("duration2", &s.duration2),
        ("sp_cost", &s.sp_cost),
        ("duration_sp_cost", &s.duration_sp_cost),
        ("cooldown", &s.cooldown),
    ];
    if let (_, Some(point)) = words[0] {
        let _ = writeln!(out, "point = {}", string(point));
    }
    for (key, formula) in formulas.iter().take(2) {
        if let Some(formula) = formula {
            let _ = writeln!(out, "{key} = {}", string(formula.text()));
        }
    }
    if let (_, Some(point)) = words[1] {
        let _ = writeln!(out, "point2 = {}", string(point));
    }
    for (key, formula) in formulas.iter().skip(2) {
        if let Some(formula) = formula {
            let _ = writeln!(out, "{key} = {}", string(formula.text()));
        }
    }
    if s.max_hit > 0 {
        let _ = writeln!(out, "max_hit = {}", s.max_hit);
    }
    if s.target_range > 0.0 {
        let _ = writeln!(out, "target_range = {}", float(s.target_range));
    }
    if s.splash_range > 0.0 {
        let _ = writeln!(out, "splash_range = {}", float(s.splash_range));
    }
}

fn write_good(out: &mut String, npc: u32, good: &ShopGood) {
    let _ = writeln!(out, "\n[[good]]");
    let _ = writeln!(out, "npc = {npc}");
    let _ = writeln!(out, "item = {}", good.item);
    let _ = writeln!(out, "count = {}", good.count);
    let _ = writeln!(out, "price = {}", good.price);
}

fn write_start_item(out: &mut String, item: &StartItem) {
    let _ = writeln!(out, "\n[[item]]");
    let _ = writeln!(out, "item = {}", item.item);
    let _ = writeln!(out, "count = {}", item.count);
    if !item.races.is_empty() {
        let races: Vec<String> = item.races.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "races = [{}]", races.join(", "));
    }
    if item.wear {
        let _ = writeln!(out, "wear = true");
    }
}

fn write_map(out: &mut String, map: &Map) {
    let _ = writeln!(out, "\n[[map]]");
    let _ = writeln!(out, "name = {}", string(&map.name));
    let _ = writeln!(out, "base = {}", floats(&map.base));
    let _ = writeln!(out, "size = {}", floats(&map.size));
    if let Some(walk) = &map.walk {
        let _ = writeln!(out, "walk = {}", string(walk));
    }
    if let Some(water) = &map.water {
        let _ = writeln!(out, "water = {}", string(water));
    }
}

fn write_monster(out: &mut String, m: &Monster) {
    let _ = writeln!(out, "\n[[monster]]");
    let _ = writeln!(out, "vnum = {}", m.vnum);
    let _ = writeln!(out, "name = {}", string(&m.name));
    let _ = writeln!(out, "kind = {}", string(m.kind.name()));
    if let Some([x, y]) = m.to {
        let _ = writeln!(out, "to = [{}, {}]", float(x), float(y));
    }
    let _ = writeln!(out, "rank = {}", m.rank);
    let _ = writeln!(out, "battle = {}", m.battle);
    let _ = writeln!(out, "level = {}", m.level);
    let _ = writeln!(out, "hp = {}", m.hp);
    if let Some(regen) = m.hp_regen {
        let _ = writeln!(
            out,
            "hp_regen = {{ every = {}, percent = {} }}",
            float(regen.every),
            regen.percent
        );
    }
    let _ = writeln!(out, "defence = {}", m.defence);
    let _ = writeln!(out, "experience = {}", m.experience);
    let [least, most] = m.gold;
    let _ = writeln!(out, "gold = [{least}, {most}]");
    let [least, most] = m.damage;
    let _ = writeln!(out, "damage = [{least}, {most}]");
    let _ = writeln!(out, "damage_multiplier = {}", float(m.damage_multiplier));
    let s = m.stats;
    let _ = writeln!(
        out,
        "stats = {{ st = {}, dx = {}, ht = {}, iq = {} }}",
        s.st, s.dx, s.ht, s.iq
    );
    let _ = writeln!(out, "attack_speed = {}", m.attack_speed);
    let _ = writeln!(out, "move_speed = {}", m.move_speed);
    let _ = writeln!(out, "attack_range = {}", float(m.attack_range));
    let _ = writeln!(out, "aggressive = {}", m.aggressive);
    let _ = writeln!(out, "aggressive_sight = {}", float(m.aggressive_sight));
    let _ = writeln!(out, "moves = {}", m.moves);
    if let Some(speed) = m.run_speed {
        let _ = writeln!(out, "run_speed = {}", float(speed));
    }
    if !m.calls.is_empty() {
        let calls: Vec<String> = m.calls.iter().map(u32::to_string).collect();
        let _ = writeln!(out, "calls = [{}]", calls.join(", "));
    }
}

fn write_item(out: &mut String, i: &Item) {
    let _ = writeln!(out, "\n[[item]]");
    let _ = writeln!(out, "vnum = {}", i.vnum);
    let _ = writeln!(out, "name = {}", string(&i.name));
    let _ = writeln!(out, "kind = {}", i.kind);
    let _ = writeln!(out, "subtype = {}", i.subtype);
    let _ = writeln!(out, "size = {}", i.size);
    let _ = writeln!(out, "anti_flags = {}", i.anti_flags);
    let _ = writeln!(out, "flags = {}", i.flags);
    let _ = writeln!(out, "wear_flags = {}", i.wear_flags);
    let _ = writeln!(out, "immune_flags = {}", i.immune_flags);
    let _ = writeln!(out, "buy_price = {}", i.buy_price);
    let _ = writeln!(out, "sell_price = {}", i.sell_price);
    if !i.limits.is_empty() {
        let _ = writeln!(out, "limits = {}", bonuses(&i.limits));
    }
    if !i.applies.is_empty() {
        let _ = writeln!(out, "applies = {}", bonuses(&i.applies));
    }
    let _ = writeln!(out, "values = {}", ints(&i.values));
    let _ = writeln!(out, "sockets = {}", ints(&i.sockets));
    let _ = writeln!(out, "upgrade = {}", i.upgrade);
    let _ = writeln!(out, "upgrade_set = {}", i.upgrade_set);
    let _ = writeln!(out, "magic_percent = {}", i.magic_percent);
    let _ = writeln!(out, "socket_percent = {}", i.socket_percent);
    let _ = writeln!(out, "specular = {}", i.specular);
}

fn write_race(out: &mut String, race: &Race) {
    let _ = writeln!(out, "\n[[race]]");
    let _ = writeln!(out, "race = {}", race.race);
    let _ = writeln!(out, "run_speed = {}", float(race.run_speed));
}

fn write_group(out: &mut String, g: &Group) {
    let _ = writeln!(out, "\n[[group]]");
    let _ = writeln!(out, "map = {}", string(&g.map));
    let _ = writeln!(out, "at = {}", floats(&g.at));
    match &g.size {
        Some(size) => {
            let _ = writeln!(out, "size = {}", floats(size));
        }
        None => {
            let _ = writeln!(out, "radius = {}", float(g.radius));
        }
    }
    if let Some(monster) = g.monster {
        let _ = writeln!(out, "monster = {monster}");
    }
    if let Some(pack) = g.pack {
        let _ = writeln!(out, "pack = {pack}");
    }
    let _ = writeln!(out, "count = {}", g.count);
    let _ = writeln!(out, "respawn = {}", float(g.respawn));
    if let Some(rotation) = g.rotation {
        let _ = writeln!(out, "rotation = {}", float(rotation));
    }
}

fn write_pack(out: &mut String, p: &Pack) {
    let _ = writeln!(out, "\n[[pack]]");
    let _ = writeln!(out, "number = {}", p.number);
    let _ = writeln!(out, "leader = {}", p.leader);
    if !p.members.is_empty() {
        let members: Vec<String> = p
            .members
            .iter()
            .map(|m| format!("{{ monster = {}, count = {} }}", m.monster, m.count))
            .collect();
        let _ = writeln!(out, "members = [{}]", members.join(", "));
    }
}

fn write_npc(out: &mut String, n: &Npc) {
    let _ = writeln!(out, "\n[[npc]]");
    let _ = writeln!(out, "map = {}", string(&n.map));
    let _ = writeln!(out, "at = {}", floats(&n.at));
    let _ = writeln!(out, "monster = {}", n.monster);
    let _ = writeln!(out, "rotation = {}", float(n.rotation));
}

/// A TOML basic string: quoted, with `"`, `\` and control characters
/// escaped.
fn write_mounts(text: &mut String, mounts: &Mounts) {
    let rules = &mounts.rules;
    let _ = writeln!(text, "\n[mount]");
    let _ = writeln!(text, "summon = {}", rules.summon);
    let _ = writeln!(text, "drain = {}", rules.drain);
    let _ = writeln!(text, "rest = {}", rules.rest);
    for grade in &mounts.grades {
        let _ = writeln!(text, "\n[[grade]]");
        let _ = writeln!(text, "grade = {}", grade.grade);
        let _ = writeln!(text, "look = {}", grade.look);
        if grade.fights {
            let _ = writeln!(text, "fights = true");
        }
        if grade.skills {
            let _ = writeln!(text, "skills = true");
        }
    }
    for level in &mounts.levels {
        let _ = writeln!(text, "\n[[level]]");
        let _ = writeln!(text, "level = {}", level.level);
        let _ = writeln!(text, "grade = {}", level.grade);
        let _ = writeln!(text, "move_speed = {}", level.move_speed);
        let _ = writeln!(text, "stamina = {}", level.stamina);
        if level.rider_level > 0 {
            let _ = writeln!(text, "rider_level = {}", level.rider_level);
        }
        if level.gold > 0 {
            let _ = writeln!(text, "gold = {}", level.gold);
        }
        if !level.items.is_empty() {
            let items: Vec<String> = level
                .items
                .iter()
                .map(|cost| format!("{{ item = {}, count = {} }}", cost.item, cost.count))
                .collect();
            let _ = writeln!(text, "items = [{}]", items.join(", "));
        }
        if level.skill_points > 0 {
            let _ = writeln!(text, "skill_points = {}", level.skill_points);
        }
    }
    for feed in mounts.feeds.values() {
        let _ = writeln!(text, "\n[[feed]]");
        let _ = writeln!(text, "item = {}", feed.item);
        let _ = writeln!(text, "stamina = {}", feed.stamina);
    }
    for stable in mounts.stables.values() {
        let _ = writeln!(text, "\n[[stable]]");
        let _ = writeln!(text, "npc = {}", stable.npc);
    }
}

fn write_upgrades(text: &mut String, upgrades: &Upgrades) {
    for smith in upgrades.smiths.values() {
        let _ = writeln!(text, "\n[[smith]]");
        let _ = writeln!(text, "npc = {}", smith.npc);
        if smith.on_fail != OnFail::default() {
            let _ = writeln!(text, "on_fail = {}", string(smith.on_fail.name()));
        }
    }
    for scroll in upgrades.scrolls.values() {
        let _ = writeln!(text, "\n[[scroll]]");
        let _ = writeln!(text, "item = {}", scroll.item);
        if scroll.on_fail != OnFail::default() {
            let _ = writeln!(text, "on_fail = {}", string(scroll.on_fail.name()));
        }
        if scroll.chance > 0 {
            let _ = writeln!(text, "chance = {}", scroll.chance);
        }
        if scroll.sure {
            let _ = writeln!(text, "sure = true");
        }
        if scroll.up_to > 0 {
            let _ = writeln!(text, "up_to = {}", scroll.up_to);
        }
    }
    for set in upgrades.sets.values() {
        let _ = writeln!(text, "\n[[set]]");
        let _ = writeln!(text, "set = {}", set.set);
        let _ = writeln!(text, "chance = {}", set.chance);
        if set.gold > 0 {
            let _ = writeln!(text, "gold = {}", set.gold);
        }
        if !set.items.is_empty() {
            let items: Vec<String> = set
                .items
                .iter()
                .map(|cost| format!("{{ item = {}, count = {} }}", cost.item, cost.count))
                .collect();
            let _ = writeln!(text, "items = [{}]", items.join(", "));
        }
    }
}

fn write_fishing(text: &mut String, fishing: &Fishing) {
    if let Some(rules) = fishing.written_rules() {
        let _ = writeln!(text, "\n[rules]");
        let _ = writeln!(text, "bite = [{}, {}]", rules.bite[0], rules.bite[1]);
        let _ = writeln!(text, "react = {}", float(rules.react));
        let _ = writeln!(text, "reach = {}", float(rules.reach));
    }
    for npc in fishing.fishers() {
        let _ = writeln!(text, "\n[[fisher]]");
        let _ = writeln!(text, "npc = {npc}");
    }
    for timing in fishing.timings() {
        let _ = writeln!(text, "\n[[timing]]");
        let _ = writeln!(text, "name = {}", string(&timing.name));
        let points: Vec<String> = timing
            .points
            .iter()
            .map(|[seconds, percent]| format!("[{}, {}]", float(*seconds), float(*percent)))
            .collect();
        let _ = writeln!(text, "points = [{}]", points.join(", "));
    }
    for fish in fishing.all_fish() {
        let _ = writeln!(text, "\n[[fish]]");
        let _ = writeln!(text, "item = {}", fish.item);
        let _ = writeln!(text, "difficulty = {}", fish.difficulty);
        let _ = writeln!(text, "timing = {}", string(&fish.timing));
    }
    for water in fishing.waters() {
        let _ = writeln!(text, "\n[[water]]");
        let maps: Vec<String> = water.maps.iter().map(|map| string(map)).collect();
        let _ = writeln!(text, "maps = [{}]", maps.join(", "));
        if water.nothing > 0 {
            let _ = writeln!(text, "nothing = {}", water.nothing);
        }
        if !water.catch.is_empty() {
            let catches: Vec<String> = water
                .catch
                .iter()
                .map(|c| format!("{{ item = {}, weight = {} }}", c.item, c.weight))
                .collect();
            let _ = writeln!(text, "catch = [{}]", catches.join(", "));
        }
    }
    for open in fishing.opens() {
        let _ = writeln!(text, "\n[[open]]");
        let _ = writeln!(text, "item = {}", open.item);
        if !open.gives.is_empty() {
            let gives: Vec<String> = open
                .gives
                .iter()
                .map(|g| {
                    format!(
                        "{{ item = {}, count = {}, chance = {} }}",
                        g.item,
                        g.count,
                        float(g.chance)
                    )
                })
                .collect();
            let _ = writeln!(text, "gives = [{}]", gives.join(", "));
        }
    }
    if let Some(fire) = fishing.fire() {
        let _ = writeln!(text, "\n[fire]");
        let _ = writeln!(text, "item = {}", fire.item);
        let _ = writeln!(text, "npc = {}", fire.npc);
        let _ = writeln!(text, "seconds = {}", fire.seconds);
    }
    for grill in fishing.grills() {
        let _ = writeln!(text, "\n[[grill]]");
        let _ = writeln!(text, "item = {}", grill.item);
        let _ = writeln!(text, "into = {}", grill.into);
    }
}

fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A float that reads back as the same `f64` (Rust's shortest form), with
/// `.0` added to whole numbers so it stays a TOML float.
fn float(value: f64) -> String {
    let text = format!("{value}");
    if text.contains(['.', 'e', 'E', 'n', 'i']) {
        text
    } else {
        format!("{text}.0")
    }
}

fn floats(values: &[f64]) -> String {
    let parts: Vec<String> = values.iter().map(|v| float(*v)).collect();
    format!("[{}]", parts.join(", "))
}

fn ints(values: &[i32]) -> String {
    let parts: Vec<String> = values.iter().map(i32::to_string).collect();
    format!("[{}]", parts.join(", "))
}

fn bonuses(values: &[ItemBonus]) -> String {
    let parts: Vec<String> = values
        .iter()
        .map(|b| format!("{{ kind = {}, value = {} }}", b.kind, b.value))
        .collect();
    format!("[{}]", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_escaped() {
        assert_eq!(string("a\"b\\c\n\u{1}é"), "\"a\\\"b\\\\c\\n\\u0001é\"");
    }

    #[test]
    fn floats_stay_floats() {
        assert_eq!(float(3.0), "3.0");
        assert_eq!(float(-0.0), "-0.0");
        assert_eq!(float(0.1), "0.1");
        assert_eq!(float(1e21), "1000000000000000000000.0");
    }
}

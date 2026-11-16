//! Skill formulas: small arithmetic expressions over a character's numbers,
//! such as `-(2*atk+(0.5*atk+3*str)*k)` (`docs/specs/server/content.md`,
//! "Formulas"). openm2's own reader: numbers, the names of [`Name`], `+ - *
//! /`, brackets and the functions of [`Function`].
//!
//! Values are integers in thousandths, so a formula gives the same result
//! on every machine and the server sees no float.

use std::fmt;

use serde::Deserialize;

/// The longest formula, in bytes.
pub const MAX_FORMULA: usize = 256;
/// The deepest nesting of brackets, signs and calls.
const MAX_DEPTH: usize = 32;
/// One, in thousandths.
pub const ONE: i64 = 1_000;
/// openm2's skill power per skill level, in thousandths: `k` grows by
/// 0.025 a level, to 0.5 at a master's first level and 1 at a perfect
/// master's. The server and the client work cooldowns out with it alike.
pub const SKILL_POWER: u32 = 25;

/// `k` for a skill at `level`, `per_level` thousandths a level, at most 1.
pub fn skill_power(level: u8, per_level: u32) -> i64 {
    (i64::from(level) * i64::from(per_level)).min(ONE)
}

/// A name a formula may use: one of the character's numbers, set by the
/// server when it works the formula out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Name {
    /// `k`: the skill's power at its level, 0 to 1.
    Power,
    /// `lv`: the character's level.
    Level,
    /// `str`: its strength.
    St,
    /// `dex`: its dexterity.
    Dx,
    /// `con`: its vitality.
    Ht,
    /// `iq`: its intelligence.
    Iq,
    /// `atk`: its attack against the target.
    Attack,
    /// `mwep`: its weapon's magic attack.
    MagicWeapon,
    /// `ar`: its hit rating, 0 to 0.9.
    Rating,
    /// `odef`: the target's defence.
    TargetDefence,
    /// `maxhp`: its maximum health.
    MaxHp,
    /// `maxsp`: its maximum mana.
    MaxSp,
    /// `chain`: how many targets this use of the skill has hit before.
    Chain,
}

impl Name {
    /// Every name with the word formulas write for it.
    pub const ALL: [(Name, &'static str); 13] = [
        (Name::Power, "k"),
        (Name::Level, "lv"),
        (Name::St, "str"),
        (Name::Dx, "dex"),
        (Name::Ht, "con"),
        (Name::Iq, "iq"),
        (Name::Attack, "atk"),
        (Name::MagicWeapon, "mwep"),
        (Name::Rating, "ar"),
        (Name::TargetDefence, "odef"),
        (Name::MaxHp, "maxhp"),
        (Name::MaxSp, "maxsp"),
        (Name::Chain, "chain"),
    ];

    fn from_word(word: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, w)| *w == word)
            .map(|(name, _)| *name)
    }
}

/// A function a formula may call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    /// `number(a, b)`: a whole number from a to b, at random.
    Number,
    /// `floor(x)`: x without its fraction, rounded down.
    Floor,
    /// `min(a, b)`: the lower of the two.
    Min,
    /// `max(a, b)`: the higher of the two.
    Max,
}

impl Function {
    fn from_word(word: &str) -> Option<(Self, usize)> {
        match word {
            "number" => Some((Self::Number, 2)),
            "floor" => Some((Self::Floor, 1)),
            "min" => Some((Self::Min, 2)),
            "max" => Some((Self::Max, 2)),
            _ => None,
        }
    }
}

/// The values of the names, in thousandths (`3 * ONE` for 3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Values {
    /// `k`.
    pub power: i64,
    /// `lv`.
    pub level: i64,
    /// `str`.
    pub st: i64,
    /// `dex`.
    pub dx: i64,
    /// `con`.
    pub ht: i64,
    /// `iq`.
    pub iq: i64,
    /// `atk`.
    pub attack: i64,
    /// `mwep`.
    pub magic_weapon: i64,
    /// `ar`.
    pub rating: i64,
    /// `odef`.
    pub target_defence: i64,
    /// `maxhp`.
    pub max_hp: i64,
    /// `maxsp`.
    pub max_sp: i64,
    /// `chain`.
    pub chain: i64,
}

impl Values {
    fn get(&self, name: Name) -> i64 {
        match name {
            Name::Power => self.power,
            Name::Level => self.level,
            Name::St => self.st,
            Name::Dx => self.dx,
            Name::Ht => self.ht,
            Name::Iq => self.iq,
            Name::Attack => self.attack,
            Name::MagicWeapon => self.magic_weapon,
            Name::Rating => self.rating,
            Name::TargetDefence => self.target_defence,
            Name::MaxHp => self.max_hp,
            Name::MaxSp => self.max_sp,
            Name::Chain => self.chain,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
}

/// A node of a parsed formula; children are indexes into the formula's
/// nodes, always lower than the node's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Node {
    Number(i64),
    Name(Name),
    Neg(u16),
    Binary(Op, u16, u16),
    Call(Function, u16, Option<u16>),
}

/// A checked formula.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Formula {
    text: String,
    nodes: Vec<Node>,
}

impl Formula {
    /// Reads `text`, or says what is wrong with it and where.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_FORMULA {
            return Err(format!("a formula is longer than {MAX_FORMULA} bytes"));
        }
        let mut parser = Parser {
            text,
            at: 0,
            nodes: Vec::new(),
            depth: 0,
        };
        parser.skip_spaces();
        if parser.peek().is_none() {
            return Err("an empty formula".to_owned());
        }
        parser.expression()?;
        parser.skip_spaces();
        if let Some(c) = parser.peek() {
            return Err(format!("{c:?} at {} is not expected", parser.at + 1));
        }
        Ok(Self {
            text: text.to_owned(),
            nodes: parser.nodes,
        })
    }

    /// The formula as written.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The names it uses.
    pub fn names(&self) -> impl Iterator<Item = Name> + '_ {
        self.nodes.iter().filter_map(|node| match node {
            Node::Name(name) => Some(*name),
            _ => None,
        })
    }

    /// Works the formula out with `values`, in thousandths. `roll(a, b)`
    /// gives a whole number from a to b for `number`. Every step is
    /// rounded towards zero and kept within ±10^15; dividing by 0 gives 0.
    pub fn eval(&self, values: &Values, roll: &mut dyn FnMut(i64, i64) -> i64) -> i64 {
        let mut results: Vec<i64> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let get = |index: u16| results.get(usize::from(index)).copied().unwrap_or(0);
            let value = match *node {
                Node::Number(value) => value,
                Node::Name(name) => values.get(name),
                Node::Neg(a) => -get(a),
                Node::Binary(op, a, b) => {
                    let (a, b) = (i128::from(get(a)), i128::from(get(b)));
                    let value = match op {
                        Op::Add => a + b,
                        Op::Sub => a - b,
                        Op::Mul => a * b / i128::from(ONE),
                        Op::Div if b == 0 => 0,
                        Op::Div => a * i128::from(ONE) / b,
                    };
                    clamp(value)
                }
                Node::Call(function, a, b) => {
                    let a = get(a);
                    let b = b.map_or(0, get);
                    match function {
                        Function::Number => {
                            let (low, high) = (a.div_euclid(ONE), b.div_euclid(ONE));
                            let (low, high) = (low.min(high), low.max(high));
                            clamp(i128::from(roll(low, high).clamp(low, high)) * i128::from(ONE))
                        }
                        Function::Floor => a.div_euclid(ONE) * ONE,
                        Function::Min => a.min(b),
                        Function::Max => a.max(b),
                    }
                }
            };
            results.push(value);
        }
        results.last().copied().unwrap_or(0)
    }
}

/// The largest value a step keeps.
const LIMIT: i128 = 1_000_000_000_000_000;

fn clamp(value: i128) -> i64 {
    // Within ±10^15, which fits an i64.
    value.clamp(-LIMIT, LIMIT) as i64
}

impl TryFrom<String> for Formula {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl fmt::Debug for Formula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Formula({:?})", self.text)
    }
}

impl fmt::Display for Formula {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
    nodes: Vec<Node>,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.text
            .get(self.at..)
            .and_then(|rest| rest.chars().next())
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.at += 1;
        }
    }

    fn push(&mut self, node: Node) -> Result<u16, String> {
        let index = u16::try_from(self.nodes.len()).map_err(|_| "a formula too long".to_owned())?;
        self.nodes.push(node);
        Ok(index)
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(format!("a formula nested deeper than {MAX_DEPTH}"));
        }
        Ok(())
    }

    /// expression = term { ("+" | "-") term }
    fn expression(&mut self) -> Result<u16, String> {
        let mut left = self.term()?;
        loop {
            self.skip_spaces();
            let op = match self.peek() {
                Some('+') => Op::Add,
                Some('-') => Op::Sub,
                _ => return Ok(left),
            };
            self.at += 1;
            let right = self.term()?;
            left = self.push(Node::Binary(op, left, right))?;
        }
    }

    /// term = unary { ("*" | "/") unary }
    fn term(&mut self) -> Result<u16, String> {
        let mut left = self.unary()?;
        loop {
            self.skip_spaces();
            let op = match self.peek() {
                Some('*') => Op::Mul,
                Some('/') => Op::Div,
                _ => return Ok(left),
            };
            self.at += 1;
            let right = self.unary()?;
            left = self.push(Node::Binary(op, left, right))?;
        }
    }

    /// unary = ("-" | "+") unary | primary
    fn unary(&mut self) -> Result<u16, String> {
        self.enter()?;
        self.skip_spaces();
        let index = match self.peek() {
            Some('-') => {
                self.at += 1;
                let inner = self.unary()?;
                self.push(Node::Neg(inner))?
            }
            Some('+') => {
                self.at += 1;
                self.unary()?
            }
            _ => self.primary()?,
        };
        self.depth -= 1;
        Ok(index)
    }

    /// primary = number | name | function "(" arguments ")" | "(" expression ")"
    fn primary(&mut self) -> Result<u16, String> {
        let start = self.at;
        match self.peek() {
            Some('(') => {
                self.at += 1;
                let inner = self.expression()?;
                self.close(start)?;
                Ok(inner)
            }
            Some(c) if c.is_ascii_digit() || c == '.' => {
                let value = self.number()?;
                self.push(Node::Number(value))
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    self.at += 1;
                }
                let word = self.text.get(start..self.at).unwrap_or_default();
                if let Some(name) = Name::from_word(word) {
                    return self.push(Node::Name(name));
                }
                let Some((function, arguments)) = Function::from_word(word) else {
                    return Err(format!("{word:?} at {} is not a known name", start + 1));
                };
                self.skip_spaces();
                if self.peek() != Some('(') {
                    return Err(format!("{word} at {} needs brackets", start + 1));
                }
                self.at += 1;
                let first = self.expression()?;
                let second = if arguments == 2 {
                    self.skip_spaces();
                    if self.peek() != Some(',') {
                        return Err(format!("{word} at {} takes two values", start + 1));
                    }
                    self.at += 1;
                    Some(self.expression()?)
                } else {
                    None
                };
                self.close(start)?;
                self.push(Node::Call(function, first, second))
            }
            Some(c) => Err(format!("{c:?} at {} is not expected", start + 1)),
            None => Err("a formula ends too soon".to_owned()),
        }
    }

    fn close(&mut self, start: usize) -> Result<(), String> {
        self.skip_spaces();
        if self.peek() != Some(')') {
            return Err(format!("the bracket at {} is not closed", start + 1));
        }
        self.at += 1;
        Ok(())
    }

    /// A number with at most three decimals, in thousandths.
    fn number(&mut self) -> Result<i64, String> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
            self.at += 1;
        }
        let word = self.text.get(start..self.at).unwrap_or_default();
        let (whole, fraction) = word.split_once('.').unwrap_or((word, ""));
        if (whole.is_empty() && fraction.is_empty())
            || fraction.contains('.')
            || fraction.len() > 3
            || whole.len() > 12
        {
            return Err(format!(
                "{word:?} at {}: a number with at most 12 digits and 3 decimals",
                start + 1
            ));
        }
        let whole: i64 = if whole.is_empty() {
            0
        } else {
            whole
                .parse()
                .map_err(|_| format!("{word:?} at {}", start + 1))?
        };
        let mut thousandths: i64 = 0;
        let mut scale = 100;
        for digit in fraction.chars() {
            thousandths += i64::from(digit.to_digit(10).unwrap_or(0)) * scale;
            scale /= 10;
        }
        Ok(whole * ONE + thousandths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(text: &str, values: &Values) -> i64 {
        Formula::parse(text)
            .unwrap()
            .eval(values, &mut |low, _| low)
    }

    #[test]
    fn arithmetic_in_thousandths() {
        let v = Values::default();
        assert_eq!(eval("1+2*3", &v), 7_000);
        assert_eq!(eval("(1+2)*3", &v), 9_000);
        assert_eq!(eval("-(1.5)", &v), -1_500);
        assert_eq!(eval("10/4", &v), 2_500);
        assert_eq!(eval("1/3", &v), 333);
        assert_eq!(eval("5/0", &v), 0);
        assert_eq!(eval("0.6", &v), 600);
        assert_eq!(eval(".5", &v), 500);
        assert_eq!(eval("- - 2", &v), 2_000);
    }

    #[test]
    fn names_and_functions() {
        let v = Values {
            power: 500,
            attack: 100 * ONE,
            st: 20 * ONE,
            ..Values::default()
        };
        // -(2·100 + (0.5·100 + 3·20)·0.5) = -(200 + 55) = -255
        assert_eq!(eval("-(2*atk+(0.5*atk+3*str)*k)", &v), -255_000);
        assert_eq!(eval("10+20*k", &v), 20_000);
        assert_eq!(eval("floor(3+k*5)", &v), 5_000);
        assert_eq!(eval("floor(-0.5)", &v), -1_000);
        assert_eq!(eval("min(1, 2) + max(1, 2)", &v), 3_000);
        let f = Formula::parse("number(100,300)").unwrap();
        assert_eq!(f.eval(&v, &mut |_, high| high), 300_000);
        assert_eq!(
            f.eval(&v, &mut |_, _| 9_999),
            300_000,
            "kept within its range"
        );
        assert_eq!(
            Formula::parse("atk*k+str")
                .unwrap()
                .names()
                .collect::<Vec<_>>(),
            [Name::Attack, Name::Power, Name::St]
        );
    }

    #[test]
    fn refusals_say_where() {
        for (text, message) in [
            ("", "an empty formula"),
            ("1+", "a formula ends too soon"),
            ("(1+2", "the bracket at 1 is not closed"),
            ("1+foo", "\"foo\" at 3 is not a known name"),
            ("number(1)", "number at 1 takes two values"),
            ("floor 2", "floor at 1 needs brackets"),
            (
                "1.2345",
                "\"1.2345\" at 1: a number with at most 12 digits and 3 decimals",
            ),
            (
                "1.2.3",
                "\"1.2.3\" at 1: a number with at most 12 digits and 3 decimals",
            ),
            ("2 3", "'3' at 3 is not expected"),
            ("2$", "'$' at 2 is not expected"),
        ] {
            assert_eq!(Formula::parse(text).unwrap_err(), message, "{text}");
        }
        let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
        assert!(Formula::parse(&deep).unwrap_err().contains("deeper"));
        let long = "1+".repeat(200);
        assert!(Formula::parse(&long).unwrap_err().contains("longer"));
    }

    #[test]
    fn huge_values_stay_bounded() {
        let v = Values::default();
        let big = eval("999999999999*999999999999*999999999999", &v);
        assert_eq!(i128::from(big), LIMIT);
    }
}

//! The time and the connection numbers, as the edge hands them in.

use std::fmt;

/// Milliseconds since the server started. The edge reads the clock and passes
/// the time in with every input; the core only compares times.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Millis(pub u64);

impl Millis {
    /// Milliseconds from `earlier` to `self`, 0 if `earlier` is later.
    pub fn since(self, earlier: Millis) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// `ms` milliseconds later.
    pub fn plus(self, ms: u64) -> Millis {
        Millis(self.0.saturating_add(ms))
    }

    /// The 32-bit clock the protocol carries (GameTime): the low
    /// bits of the uptime, wrapping after 49 days.
    pub fn wire(self) -> u32 {
        // Truncation is the point: the wire clock wraps.
        self.0 as u32
    }
}

impl fmt::Display for Millis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

/// One accepted connection, numbered by the edge. Numbers are never reused
/// while the server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConnId(pub u64);

impl fmt::Display for ConnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conn#{}", self.0)
    }
}

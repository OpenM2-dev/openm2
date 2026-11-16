//! Time and connection identifiers passed into the state machine.

use std::fmt;

/// A point in time, in milliseconds, supplied by the caller.
///
/// Core never reads a clock. Only differences between two `Millis` values mean
/// anything; where zero is depends on the caller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Millis(pub u64);

impl Millis {
    /// Milliseconds from `earlier` to `self`, or 0 if `earlier` is later.
    pub fn since(self, earlier: Millis) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// This time plus `ms` milliseconds (saturating).
    pub fn plus(self, ms: u64) -> Millis {
        Millis(self.0.saturating_add(ms))
    }
}

impl fmt::Display for Millis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

/// One TCP connection (to the auth server or a game server).
///
/// Packets and effects carry the id so bytes arriving late on a connection that
/// was already replaced are never taken for the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnId(pub u32);

impl ConnId {
    /// The id after this one.
    pub fn next(self) -> ConnId {
        ConnId(self.0.wrapping_add(1))
    }
}

impl fmt::Display for ConnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conn#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_never_goes_negative() {
        assert_eq!(Millis(1_500).since(Millis(1_000)), 500);
        assert_eq!(Millis(1_000).since(Millis(1_500)), 0);
    }

    #[test]
    fn plus_saturates() {
        assert_eq!(Millis(10).plus(5), Millis(15));
        assert_eq!(Millis(u64::MAX).plus(1), Millis(u64::MAX));
    }

    #[test]
    fn conn_ids_advance() {
        assert_eq!(ConnId(7).next(), ConnId(8));
        assert_eq!(ConnId(7).to_string(), "conn#7");
    }
}

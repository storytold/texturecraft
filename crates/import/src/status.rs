//! [`Status`]: how far one part of an imported stack (a layer's content for a channel, a mask, a blend mode) can be
//! reproduced from the file alone, with the reasons when it cannot. What makes a part not computable is always a typed
//! [`Blocker`] from the reader's vocabulary, so locks and the report group by its key (`graph:<name>`, `strokes:<tool>`,
//! `feature:<id>`, `class:<class>`).

use texturecraft_core::LockReason;
use texturecraft_import_reader::blockers::Blocker;

/// At most this many distinct reasons are kept per part (the report stays readable).
const MAX_REASONS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    /// Reproduced as the original computes it.
    #[default]
    Exact,
    /// Reproduced with a stand-in (a guessed blend mode, an approximated projection, …).
    Approximated(Vec<String>),
    /// Needs something TextureCraft cannot evaluate yet (compiled graphs, strokes, …), by blocker.
    NotComputable(Vec<Blocker>),
}

impl Status {
    pub fn approximated(reason: impl Into<String>) -> Status {
        Status::Approximated(vec![reason.into()])
    }

    pub fn not_computable(blocker: Blocker) -> Status {
        Status::NotComputable(vec![blocker])
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Status::Exact)
    }

    /// Exact or approximated: there are pixels to show.
    pub fn is_computable(&self) -> bool {
        !matches!(self, Status::NotComputable(_))
    }

    /// The reasons as text (a not computable part's are its blockers' texts).
    pub fn reasons(&self) -> Vec<String> {
        match self {
            Status::Exact => Vec::new(),
            Status::Approximated(r) => r.clone(),
            Status::NotComputable(b) => b.iter().map(|b| b.text.clone()).collect(),
        }
    }

    /// What makes it not computable (empty when it is computable).
    pub fn blockers(&self) -> &[Blocker] {
        match self {
            Status::NotComputable(b) => b,
            _ => &[],
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Status::Exact => 0,
            Status::Approximated(_) => 1,
            Status::NotComputable(_) => 2,
        }
    }

    /// Combines with another part: the worse state wins, reasons of that state are merged (distinct, at most
    /// [`MAX_REASONS`]).
    pub fn and(&mut self, other: &Status) {
        if other.rank() > self.rank() {
            *self = match other {
                Status::Approximated(_) => Status::Approximated(Vec::new()),
                Status::NotComputable(_) => Status::NotComputable(Vec::new()),
                Status::Exact => Status::Exact,
            };
        }
        if other.rank() != self.rank() {
            return;
        }
        match (self, other) {
            (Status::Approximated(mine), Status::Approximated(theirs)) => {
                for r in theirs {
                    if mine.len() < MAX_REASONS && !mine.contains(r) {
                        mine.push(r.clone());
                    }
                }
            }
            (Status::NotComputable(mine), Status::NotComputable(theirs)) => {
                for b in theirs {
                    if mine.len() < MAX_REASONS && !mine.iter().any(|m| m.key == b.key && m.text == b.text) {
                        mine.push(b.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// The lock reasons of a not computable part: each blocker's key and text.
    pub fn lock_reasons(&self) -> Vec<LockReason> {
        self.blockers().iter().map(|b| LockReason::new(b.key.to_string(), b.text.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worse_state_wins_and_reasons_merge() {
        let mut s = Status::Exact;
        s.and(&Status::approximated("a"));
        s.and(&Status::approximated("a"));
        s.and(&Status::approximated("b"));
        assert_eq!(s.reasons(), ["a", "b"]);
        s.and(&Status::not_computable(Blocker::graph("dirt", "fill \"dirt\"")));
        assert!(!s.is_computable());
        s.and(&Status::approximated("c"));
        assert_eq!(s.reasons(), ["fill \"dirt\""]);
        assert_eq!(s.lock_reasons(), [LockReason::new("graph:dirt", "fill \"dirt\"")]);
        let mut many = Status::Exact;
        for i in 0..20 {
            many.and(&Status::approximated(format!("r{i}")));
        }
        assert_eq!(many.reasons().len(), MAX_REASONS);
    }
}

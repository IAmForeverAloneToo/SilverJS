//! Sets of characters for the regexp engine: sorted, disjoint, merged
//! inclusive ranges of code points (or of code units outside Unicode
//! mode), with the operations a class needs: union, intersection,
//! subtraction and complement within a mode's range.

/// A set of characters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CharSet {
    ranges: Vec<(u32, u32)>,
}

impl CharSet {
    pub(crate) fn new() -> CharSet {
        CharSet { ranges: Vec::new() }
    }

    /// The set of `ranges`, in any order, overlapping or not.
    pub(crate) fn of(ranges: &[(u32, u32)]) -> CharSet {
        let mut s = CharSet::new();
        for &(lo, hi) in ranges {
            s.add_range(lo, hi);
        }
        s
    }

    /// The set of a generated table's ranges, written flat (first, last,
    /// first, last, ...), sorted and merged as the generator writes them.
    pub(crate) fn from_flat(flat: &[u32]) -> CharSet {
        CharSet {
            ranges: flat
                .chunks_exact(2)
                .filter_map(|p| match *p {
                    [first, last] => Some((first, last)),
                    _ => None,
                })
                .collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn ranges(&self) -> &[(u32, u32)] {
        &self.ranges
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub(crate) fn add(&mut self, c: u32) {
        self.add_range(c, c);
    }

    /// Adds `lo..=hi`, keeping the ranges sorted and merged.
    pub(crate) fn add_range(&mut self, lo: u32, hi: u32) {
        if lo > hi {
            return;
        }
        // The first range that could touch `lo..=hi`.
        let at = self
            .ranges
            .partition_point(|&(_, h)| h.saturating_add(1) < lo);
        let mut end = at;
        let (mut nlo, mut nhi) = (lo, hi);
        while let Some(&(a, b)) = self.ranges.get(end) {
            if a > nhi.saturating_add(1) {
                break;
            }
            nlo = nlo.min(a);
            nhi = nhi.max(b);
            end += 1;
        }
        self.ranges.splice(at..end, [(nlo, nhi)]);
    }

    pub(crate) fn union(&mut self, other: &CharSet) {
        for &(lo, hi) in &other.ranges {
            self.add_range(lo, hi);
        }
    }

    pub(crate) fn contains(&self, c: u32) -> bool {
        let at = self.ranges.partition_point(|&(_, h)| h < c);
        self.ranges.get(at).is_some_and(|&(lo, _)| lo <= c)
    }

    /// Everything up to `max` that is not in the set.
    pub(crate) fn complement(&self, max: u32) -> CharSet {
        let mut out = Vec::new();
        let mut next = 0u32;
        for &(lo, hi) in &self.ranges {
            if lo > max {
                break;
            }
            if lo > next {
                out.push((next, lo - 1));
            }
            next = hi.saturating_add(1);
        }
        if next <= max && !self.ranges.last().is_some_and(|&(_, h)| h == u32::MAX) {
            out.push((next, max));
        }
        CharSet { ranges: out }
    }

    pub(crate) fn intersect(&self, other: &CharSet) -> CharSet {
        let mut out = Vec::new();
        let (mut i, mut j) = (0, 0);
        while let (Some(&(a, b)), Some(&(c, d))) = (self.ranges.get(i), other.ranges.get(j)) {
            let (lo, hi) = (a.max(c), b.min(d));
            if lo <= hi {
                out.push((lo, hi));
            }
            if b < d {
                i += 1;
            } else {
                j += 1;
            }
        }
        CharSet { ranges: out }
    }

    pub(crate) fn subtract(&self, other: &CharSet) -> CharSet {
        self.intersect(&other.complement(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::CharSet;

    #[test]
    fn ranges_merge_and_operate() {
        let mut s = CharSet::of(&[(10, 20), (30, 40), (21, 25)]);
        assert_eq!(s.ranges(), &[(10, 25), (30, 40)]);
        s.add_range(26, 29);
        assert_eq!(s.ranges(), &[(10, 40)]);
        s.add(5);
        assert!(s.contains(5) && s.contains(40) && !s.contains(41) && !s.contains(6));
        assert_eq!(s.complement(50).ranges(), &[(0, 4), (6, 9), (41, 50)]);
        let t = CharSet::of(&[(0, 12), (38, 60)]);
        assert_eq!(s.intersect(&t).ranges(), &[(5, 5), (10, 12), (38, 40)]);
        assert_eq!(s.subtract(&t).ranges(), &[(13, 37)]);
        assert_eq!(CharSet::new().complement(0xFFFF).ranges(), &[(0, 0xFFFF)]);
    }
}

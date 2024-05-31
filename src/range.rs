use core::ops::RangeInclusive;

pub struct NRangeInclusive<Idx> {
    ranges: Vec<RangeInclusive<Idx>>,
}

impl<Idx> NRangeInclusive<Idx>
where
    Idx: PartialOrd<Idx>,
{
    pub fn new() -> Self {
        Self { ranges: vec![] }
    }

    pub fn push(&mut self, range: RangeInclusive<Idx>) {
        self.ranges.push(range);
    }

    pub fn contains(&self, item: &Idx) -> bool {
        for range in &self.ranges {
            if range.contains(item) {
                return true;
            }
        }

        false
    }
}

//! The random 5–20 minute spacing Auto keeps between two submissions (AC4).
//!
//! The gap is derived from a sample the caller supplies, so this stays pure:
//! tests pick the sample, the app layer draws it from the system clock.

/// The shortest gap between two automatic submissions (AC4).
pub const MIN_GAP_MINUTES: u32 = 5;
/// The longest gap between two automatic submissions (AC4).
pub const MAX_GAP_MINUTES: u32 = 20;

/// A gap in `[5, 20]` minutes, uniformly mapped from `sample` (AC4).
pub fn gap_minutes(sample: u64) -> u32 {
    // `%` on the span keeps a u64::MAX sample from overflowing: the result is
    // already inside [0, span) before the offset is added.
    let span = MAX_GAP_MINUTES - MIN_GAP_MINUTES + 1;
    MIN_GAP_MINUTES + (sample % u64::from(span)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gap_is_between_five_and_twenty_minutes() {
        // The endpoints are reachable: sample 0 and the maximum remainder.
        assert_eq!(gap_minutes(0), 5);
        assert_eq!(gap_minutes(15), 20);
        for sample in 0..1_000 {
            let gap = gap_minutes(sample);
            assert!(
                (MIN_GAP_MINUTES..=MAX_GAP_MINUTES).contains(&gap),
                "{sample} → {gap}"
            );
        }
        // A 64-bit sample still lands in the window, not in an overflow.
        for sample in [u64::MAX, u64::MAX - 7, 1 << 63] {
            let gap = gap_minutes(sample);
            assert!(
                (MIN_GAP_MINUTES..=MAX_GAP_MINUTES).contains(&gap),
                "{sample} → {gap}"
            );
        }
    }

    #[test]
    fn the_gap_varies_with_the_sample() {
        let gaps: std::collections::BTreeSet<u32> = (0..64).map(gap_minutes).collect();
        assert!(gaps.len() > 1, "spacing must not be a constant");
    }
}

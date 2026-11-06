//! Grid layout math.

/// How many tiles fit per row, given the space and a target tile width.
pub fn columns(available: f32, target: f32, gap: f32, count: usize) -> usize {
    let fit = ((available + gap) / (target + gap)).floor().max(1.0) as usize;
    fit.min(count.max(1))
}

/// Round a requested pixel size up to a bucket so small window resizes
/// don't restart streams.
pub fn bucket(px: f32) -> u32 {
    const STEP: f32 = 80.0;
    ((px / STEP).ceil() * STEP).clamp(160.0, 1600.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fit_and_cap_at_count() {
        assert_eq!(columns(1000.0, 220.0, 10.0, 6), 4);
        assert_eq!(columns(1000.0, 220.0, 10.0, 2), 2);
        assert_eq!(columns(100.0, 220.0, 10.0, 6), 1);
        assert_eq!(columns(1000.0, 220.0, 10.0, 0), 1);
    }

    #[test]
    fn buckets_round_up() {
        assert_eq!(bucket(1.0), 160);
        assert_eq!(bucket(161.0), 240);
        assert_eq!(bucket(400.0), 400);
        assert_eq!(bucket(5000.0), 1600);
    }
}

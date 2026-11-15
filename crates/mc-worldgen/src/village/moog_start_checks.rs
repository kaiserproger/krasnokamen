//! Moog's overworld start checks, before jigsaw assembly.

/// `height_at` returns the selected projection heightmap's first-free height;
/// `liquid_at` inspects the highest non-air block of the unmodified noise column.
/// Radius is measured in chunks and samples each chunk at its (7, 7) column.
pub(super) fn passes(
    chunk_x: i32,
    chunk_z: i32,
    cannot_spawn_in_liquid: bool,
    terrain_check: Option<(i32, i32)>,
    y_bounds: (Option<i32>, Option<i32>),
    height_at: &dyn Fn(i32, i32) -> Option<i32>,
    liquid_at: &dyn Fn(i32, i32) -> Option<bool>,
) -> bool {
    let column = |chunk: i32| chunk.checked_mul(16)?.checked_add(7);
    let Some(x) = column(chunk_x) else { return false };
    let Some(z) = column(chunk_z) else { return false };
    if cannot_spawn_in_liquid && liquid_at(x, z) != Some(false) {
        return false;
    }
    let Some((radius, max_range)) = terrain_check else {
        return true;
    };
    let (Some(min_x), Some(max_x), Some(min_z), Some(max_z)) = (
        chunk_x.checked_sub(radius),
        chunk_x.checked_add(radius),
        chunk_z.checked_sub(radius),
        chunk_z.checked_add(radius),
    ) else {
        return false;
    };
    let (mut lowest, mut highest) = (i32::MAX, i32::MIN);
    for cx in min_x..=max_x {
        for cz in min_z..=max_z {
            let Some(x) = column(cx) else { return false };
            let Some(z) = column(cz) else { return false };
            let Some(height) = height_at(x, z) else { return false };
            lowest = lowest.min(height);
            highest = highest.max(height);
            if y_bounds.0.is_some_and(|min| lowest < min)
                || y_bounds.1.is_some_and(|max| lowest > max)
            {
                return false;
            }
            if i64::from(highest) - i64::from(lowest) > i64::from(max_range) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::passes;

    #[test]
    fn rejects_liquid_at_start_column_only_when_forbidden() {
        let height = |_, _| Some(65);
        let liquid = |x, z| Some((x, z) == (23, -9));
        assert!(!passes(1, -1, true, None, (None, None), &height, &liquid));
        assert!(passes(1, -1, false, None, (None, None), &height, &liquid));
        assert!(passes(0, 0, true, None, (None, None), &height, &liquid));
        assert!(!passes(0, 0, true, None, (None, None), &height, &|_, _| None));
    }

    #[test]
    fn terrain_radius_samples_chunk_centres_and_allows_range_boundary() {
        let height = |x, z| Some(if (x, z) == (23, -9) { 68 } else { 65 });
        let dry = |_, _| Some(false);
        assert!(passes(0, 0, true, Some((1, 3)), (None, None), &height, &dry));
        assert!(!passes(0, 0, true, Some((1, 2)), (None, None), &height, &dry));
        assert!(passes(0, 0, true, Some((0, 2)), (None, None), &height, &dry));
        assert!(!passes(i32::MAX, 0, true, None, (None, None), &height, &dry));
        assert!(!passes(0, 0, true, Some((1, 3)), (None, None), &|_, _| None, &dry));
        assert!(!passes(0, 0, true, Some((1, 3)), (Some(66), None), &height, &dry));
    }
}

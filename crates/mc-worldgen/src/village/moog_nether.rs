//! Occupied-land placement after Moog's shared growth and recentering.
use super::height_query::HeightQuery;
use super::solver::VillageAssembly;
use mc_data::village_data::{MoogStartChecks, StructureSpec};

pub(super) fn place_on_land(
    assembly: &mut VillageAssembly,
    spec: &StructureSpec,
    checks: MoogStartChecks,
    offset_y: i32,
    bounds: (i32, i32),
    query: &dyn HeightQuery,
) -> Option<()> {
    let first = assembly.pieces.first()?;
    let base = first.bounds_min.y;
    let (mut min, mut max) = (first.bounds_min, first.bounds_max);
    for piece in &assembly.pieces[1..] {
        min.x = min.x.min(piece.bounds_min.x);
        min.y = min.y.min(piece.bounds_min.y);
        min.z = min.z.min(piece.bounds_min.z);
        max.x = max.x.max(piece.bounds_max.x);
        max.y = max.y.max(piece.bounds_max.y);
        max.z = max.z.max(piece.bounds_max.z);
    }
    let x = min
        .x
        .checked_add(max.x.checked_sub(min.x)?.checked_add(1)? / 2)?;
    let z = min
        .z
        .checked_add(max.z.checked_sub(min.z)?.checked_add(1)? / 2)?;
    let ground = query.highest_land(x, z, bounds.0, bounds.1)?;
    let mut target = ground.checked_add(offset_y)?;
    let below = min.y.checked_sub(base)?;
    if let Some(top) = checks.max_y {
        target = target.min(
            top.checked_sub(max.y.checked_sub(min.y)?)?
                .checked_sub(below)?,
        );
    }
    if let Some(bottom) = checks.min_y {
        target = target.max(bottom.checked_sub(below)?);
    }
    let dy = target.checked_sub(base)?;
    let final_min = min.y.checked_add(dy)?;
    let final_max = max.y.checked_add(dy)?;
    if final_min < bounds.0.checked_add(spec.dimension_padding.bottom)?
        || final_max >= bounds.1.checked_sub(spec.dimension_padding.top)?
    {
        return None;
    }
    if checks.min_y.is_some_and(|bottom| final_min < bottom)
        || checks.max_y.is_some_and(|top| final_max > top)
    {
        return None;
    }
    for piece in &mut assembly.pieces {
        piece.position.y = piece.position.y.checked_add(dy)?;
        piece.bounds_min.y = piece.bounds_min.y.checked_add(dy)?;
        piece.bounds_max.y = piece.bounds_max.y.checked_add(dy)?;
    }
    for beard in &mut assembly.beard_pieces {
        beard.min.y = beard.min.y.checked_add(dy)?;
        beard.max.y = beard.max.y.checked_add(dy)?;
    }
    assembly.origin.y = assembly.origin.y.checked_add(dy)?;
    assembly.origin.x = x;
    assembly.origin.z = z;
    Some(())
}

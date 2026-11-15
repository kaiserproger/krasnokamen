//! Plan-free height and liquid queries used by jigsaw start checks.

/// A generator's unmodified terrain. Optional facts are unavailable rather
/// than guessed by callers that only supply the legacy world-surface query.
pub trait HeightQuery {
    fn world_surface(&self, x: i32, z: i32) -> i32;

    fn ocean_floor(&self, _x: i32, _z: i32) -> Option<i32> {
        None
    }

    fn top_is_liquid(&self, _x: i32, _z: i32) -> Option<bool> {
        None
    }
}

impl<F: Fn(i32, i32) -> i32> HeightQuery for F {
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self(x, z)
    }
}


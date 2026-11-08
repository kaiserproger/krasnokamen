//! Offline operations on stopped native worlds, above the runtime core.
//!
//! Each operation owns the world lease and uses the runtime's generation and
//! persistence contracts; the core has no dependency on these operator tools.

pub mod pregenerate;
pub mod world_maintenance;
pub mod worldgen_upgrade;

//! Tamed cats relax beside sleeping owners and may bring a morning gift.
use mc_data::entity_types::entity_type_contract_26_1_2_by_name;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Relax {
    pub(crate) owner: Uuid,
    pub(crate) target: [f64; 3],
    pub(crate) on_bed_ticks: u16,
}

pub(crate) static TYPE: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| {
    entity_type_contract_26_1_2_by_name("minecraft:cat")
        .expect("cat type")
        .protocol_id
});

pub(crate) fn is_cat(type_id: u32) -> bool {
    type_id == *TYPE
}

pub(crate) fn drive(relax: Option<&Relax>, speed: f32) -> Option<([f64; 3], f64)> {
    let relax = relax?;
    Some((
        relax.target,
        if relax.on_bed_ticks > 0 {
            0.0
        } else {
            f64::from(speed) * 1.1
        },
    ))
}

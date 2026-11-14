//! Vanilla common entity flags retained in both native records and WAL images.
use crate::config::CoreError;
use mc_nbt::Tag;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Controls {
    pub no_ai: bool,
    pub no_gravity: bool,
    pub invulnerable: bool,
    /// Manually lit swell fuse; unlike proximity ignition this does not cancel.
    pub ignited: bool,
    pub fall_distance: f64,
}
impl Controls {
    /// Journal dirty comparison of a baseline (`self`) with the live row.
    /// `fall_distance` is a transient mid-air accumulator (see `mob_fall`):
    /// growth rides the next image emitted for another reason and
    /// checkpoints, but never forces one. The reset to zero on landing is
    /// journaled so replay cannot keep a stale non-zero distance.
    pub(crate) fn journal_eq(&self, live: &Self) -> bool {
        self.no_ai == live.no_ai
            && self.no_gravity == live.no_gravity
            && self.invulnerable == live.invulnerable
            && self.ignited == live.ignited
            && !(live.fall_distance == 0.0 && self.fall_distance != 0.0)
    }

    pub(crate) fn from_fields(fields: &[(String, Tag)]) -> Result<Self, CoreError> {
        let mut state = Self::default();
        for name in [
            "NoAI",
            "NoGravity",
            "Invulnerable",
            "ignited",
            "FallDistance",
        ] {
            let mut found = fields.iter().filter(|(key, _)| key == name);
            let Some((_, value)) = found.next() else {
                continue;
            };
            if found.next().is_some() {
                return Err(CoreError::Storage(format!("{name}: duplicate field")));
            }
            match (name, value) {
                ("NoAI", Tag::Byte(value)) => state.no_ai = *value != 0,
                ("NoGravity", Tag::Byte(value)) => state.no_gravity = *value != 0,
                ("Invulnerable", Tag::Byte(value)) => state.invulnerable = *value != 0,
                ("ignited", Tag::Byte(value)) => state.ignited = *value != 0,
                ("FallDistance", Tag::Float(value)) if value.is_finite() => {
                    state.fall_distance = f64::from(*value)
                }
                ("FallDistance", _) => {
                    return Err(CoreError::Storage(
                        "FallDistance: expected finite float".into(),
                    ));
                }
                _ => return Err(CoreError::Storage(format!("{name}: expected byte"))),
            }
        }
        Ok(state)
    }
    pub(crate) fn to_fields(self) -> Vec<(String, Tag)> {
        vec![
            ("NoAI".into(), Tag::Byte(i8::from(self.no_ai))),
            ("NoGravity".into(), Tag::Byte(i8::from(self.no_gravity))),
            ("ignited".into(), Tag::Byte(i8::from(self.ignited))),
            (
                "Invulnerable".into(),
                Tag::Byte(i8::from(self.invulnerable)),
            ),
            ("FallDistance".into(), Tag::Float(self.fall_distance as f32)),
        ]
    }
}

/// Entity invulnerability also admits bypass-tag damage and creative attackers.
pub(crate) fn blocks_damage(
    state: &crate::tick::TickState,
    slot: crate::entities::EntitySlot,
    kind: crate::survival::DamageKind,
    attacker: Option<crate::boundary::EntityId>,
) -> bool {
    state.entities.row(slot).controls.invulnerable
        && !kind.bypasses_invulnerability()
        && !attacker
            .and_then(|id| state.players.get(&id))
            .is_some_and(|player| player.creative())
}

/// 26.3 Entity.<clinit> registers DATA_NO_GRAVITY as the sixth
/// base-entity accessor, with the BOOLEAN serializer.
pub(crate) fn sync_no_gravity(appearance: &mut crate::boundary::EntityAppearance, value: bool) {
    const DATA_NO_GRAVITY: u8 = 5;
    if let Some((_, current)) = appearance
        .extra
        .iter_mut()
        .find(|(index, _)| *index == DATA_NO_GRAVITY)
    {
        *current = crate::boundary::EntityDataValue::Boolean(value);
    } else if value {
        appearance.extra.push((
            DATA_NO_GRAVITY,
            crate::boundary::EntityDataValue::Boolean(true),
        ));
    }
}

pub(crate) fn set_no_gravity(
    state: &mut crate::tick::TickState,
    slot: crate::entities::EntitySlot,
    value: bool,
) {
    let row = state.entities.row_mut(slot);
    if row.controls.no_gravity == value {
        return;
    }
    row.controls.no_gravity = value;
    sync_no_gravity(std::sync::Arc::make_mut(&mut row.appearance), value);
    state.appearance_changed.push(state.entities.body(slot).id);
}

//! Explicit 776 -> 777 presentations and 777 -> 776 inputs. Registry maps are
//! built from the opposite source reports, never from a lossy map's inverse.
//! Layout/default authority: protocol-chain's local 26.2/26.3 javap captures.
use std::borrow::Cow;

use anyhow::{Result, bail, ensure};
use mc_protocol::codec::{ReadMc, WriteMc};

use super::items::copy_nbt;

pub(super) fn clientbound_name<'a>(name: &'a str, body: &[u8]) -> Result<&'a str> {
    if name != "minecraft:animate" {
        return Ok(name);
    }
    let mut r = body;
    r.read_varint()?;
    Ok(match r.read_u8()? {
        0 | 3 => "minecraft:swing_animation",
        2 | 4 | 5 => "minecraft:animate",
        action => bail!("unsupported 776 animation {action}"),
    })
}

/// Registry-bearing bodies have already been name-paired by play.rs. Only the
/// measured layout differences below need another representation, not another
/// frame decode or another registry traversal.
pub(super) fn clientbound<'a>(name: &str, body: Cow<'a, [u8]>) -> Result<Cow<'a, [u8]>> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    if !matches!(
        name,
        "animate"
            | "entity_position_sync"
            | "move_entity_pos"
            | "move_entity_pos_rot"
            | "move_entity_rot"
            | "level_particles"
            | "set_player_team"
    ) {
        return Ok(body);
    }
    let mut r = body.as_ref();
    let mut w = Vec::with_capacity(r.len() + 16);
    match name {
        "animate" => {
            w.write_varint(r.read_varint()?);
            match r.read_u8()? {
                hand @ (0 | 3) => {
                    w.write_varint(i32::from(hand == 3));
                    // SwingAnimation.DEFAULT = WHACK(id 1), duration 6 (javap).
                    w.write_varint(1);
                    w.write_varint(6);
                }
                2 => w.write_u8(0),
                4 => w.write_u8(1),
                5 => w.write_u8(2),
                action => bail!("unsupported 776 animation {action}"),
            }
        }
        "entity_position_sync" => {
            w.write_varint(r.read_varint()?);
            w.write_varint(0); // PositionPath's absolute single-position variant
            for _ in 0..3 {
                w.write_f64(r.read_f64()?);
            }
            for _ in 0..3 {
                r.read_f64()?;
            } // old velocity has no 777 field
            w.write_f32(r.read_f32()?);
            w.write_f32(r.read_f32()?);
            w.write_bool(r.read_bool()?);
        }
        "move_entity_pos" | "move_entity_pos_rot" => {
            w.write_varint(r.read_varint()?);
            let delta = [r.read_i16()?, r.read_i16()?, r.read_i16()?];
            let rotation = if name.ends_with("_rot") {
                Some([r.read_u8()?, r.read_u8()?])
            } else {
                None
            };
            w.write_varint(i32::from(r.read_bool()?)); // onGround, zero extra steps
            for value in delta {
                w.write_i16(value);
            }
            if let Some(rotation) = rotation {
                w.extend(rotation);
            }
        }
        "move_entity_rot" => {
            w.write_varint(r.read_varint()?);
            let rotation = [r.read_u8()?, r.read_u8()?];
            w.write_bool(r.read_bool()?);
            w.extend(rotation);
        }
        "level_particles" => {
            let override_limiter = r.read_bool()?;
            let always_show = r.read_bool()?;
            let position = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
            let distance = [r.read_f32()?, r.read_f32()?, r.read_f32()?];
            let speed = r.read_f32()?;
            let count = r.read_i32()?;
            w.extend_from_slice(r); // already mapped ParticleOptions, now first
            r = &[];
            w.write_bool(override_limiter);
            w.write_bool(always_show);
            for value in position {
                w.write_f64(value);
            }
            for value in distance {
                w.write_f32(value);
            }
            for _ in 0..3 {
                w.write_f32(speed);
            } // old scalar applies to all axes
            w.write_varint(count);
            w.write_varint(0); // RandomizationType.DEFAULT, as the old constructor
        }
        "set_player_team" => team(&mut r, &mut w)?,
        _ => unreachable!(),
    }
    ensure!(r.is_empty(), "776 -> 777 {name}: trailing bytes");
    Ok(Cow::Owned(w))
}

pub(super) fn serverbound(name: &str, body: &[u8]) -> Result<Option<Vec<u8>>> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    if !matches!(
        name,
        "punch" | "accept_teleportation" | "player_action" | "sign_update"
    ) {
        return Ok(None);
    }
    let mut r = body;
    let mut w = Vec::with_capacity(body.len());
    match name {
        "punch" => w.write_varint(0), // 777 has no hand; old punch uses main hand
        "accept_teleportation" => {
            w.write_varint(r.read_varint()?);
            for _ in 0..3 {
                r.read_f64()?;
            }
            r.read_f32()?;
            r.read_f32()?;
        }
        "player_action" => {
            let action = r.read_varint()?;
            ensure!(
                action != 1,
                "777 CHANGE_DESTROY_DIRECTION has no 776 operation"
            );
            w.write_varint(if action > 1 { action - 1 } else { action });
            w.extend_from_slice(r);
            r = &[];
        }
        "sign_update" => {
            w.write_i64(r.read_i64()?);
            let lines = [
                r.read_string(384)?,
                r.read_string(384)?,
                r.read_string(384)?,
                r.read_string(384)?,
            ];
            let front = r.read_varint()?;
            ensure!((0..=1).contains(&front), "777 sign text slot {front}");
            w.write_bool(front == 1);
            for line in lines {
                w.write_string(&line, 384)?;
            }
        }
        _ => unreachable!(),
    }
    ensure!(r.is_empty(), "777 -> 776 {name}: trailing bytes");
    Ok(Some(w))
}

fn team(r: &mut &[u8], w: &mut Vec<u8>) -> Result<()> {
    w.write_string(&r.read_string(32767)?, 32767)?;
    let method = r.read_u8()?;
    w.write_u8(method);
    if matches!(method, 0 | 2) {
        copy_nbt(r, w)?;
        let options = r.read_u8()?;
        let visibility = r.read_varint()?;
        let collision = r.read_varint()?;
        let colour = r.read_varint()?;
        copy_nbt(r, w)?; // prefix
        copy_nbt(r, w)?; // suffix
        w.write_varint(visibility);
        w.write_varint(collision);
        ensure!(
            colour == 21 || (0..=15).contains(&colour),
            "776 team colour {colour}"
        );
        w.write_bool(colour != 21);
        if colour != 21 {
            w.write_varint(colour);
        }
        w.write_u8(options);
    }
    if matches!(method, 0 | 3 | 4) {
        let count = r.read_varint()?;
        ensure!((0..=65536).contains(&count), "team player count");
        w.write_varint(count);
        for _ in 0..count {
            w.write_string(&r.read_string(32767)?, 32767)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "upgrade_tests.rs"]
mod tests;

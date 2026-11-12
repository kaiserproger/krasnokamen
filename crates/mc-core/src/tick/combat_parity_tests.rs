use super::{flat_player_fixture, join};
use crate::boundary::{ChangeSet, ConnectionId, EntityId, Hand, PlayerChange, PlayerInput};
use crate::commands::GameMode;
use crate::intent::DamageSource;
use crate::survival::{DamageKind, Difficulty};
use crate::tick::{DamagePiece, TickState, run_tick};

fn hit(state: &mut TickState, target: EntityId, kind: DamageKind, source: Option<DamageSource>) {
    let slot = state.entities.slot_of(target).unwrap();
    state.entities.row_mut(slot).protected_until = 0;
    state.merge_damage.push((
        slot,
        DamagePiece {
            amount: 100.0,
            kind,
            source,
            physical: None,
            custom_armor: false,
        },
    ));
}

#[test]
fn damage_observer_counts_real_admitted_victims_and_thorns_reaction() {
    use crate::tick::damage::DamageObservation;
    use mc_data::{Identifier, ItemEnchantment, ItemStack};

    let (mut state, actor, pos) = flat_player_fixture();
    state.tick = 20;
    let actor_slot = state.entities.slot_of(actor).unwrap();
    state.entities.row_mut(actor_slot).protected_until = 0;
    let target = state.spawn_mob(
        state.registry.zombie_type_id,
        [pos[0] + 1.0, pos[1], pos[2]],
        100.0,
    );
    let target_slot = state.entities.slot_of(target).unwrap();
    let item = state
        .registry
        .items
        .id_of(&Identifier::parse("minecraft:diamond_chestplate").unwrap())
        .unwrap();
    let mut armor = ItemStack::new(item, 1);
    armor.enchantments.push(ItemEnchantment {
        id: Identifier::parse("minecraft:thorns").unwrap(),
        level: 10,
    });
    std::sync::Arc::make_mut(&mut state.entities.row_mut(target_slot).appearance).equipment[4] =
        Some(armor);
    let blocked = state.spawn_mob(
        state.registry.zombie_type_id,
        [pos[0] + 2.0, pos[1], pos[2]],
        100.0,
    );
    let blocked_slot = state.entities.slot_of(blocked).unwrap();
    state.entities.row_mut(blocked_slot).controls.invulnerable = true;
    let source = DamageSource::Melee {
        attacker: actor,
        position: pos,
        yaw: 0.0,
        bonus_knockback: 0.0,
        sweep: false,
        fire_ticks: 0,
        bane_ticks: 0,
    };
    hit(&mut state, target, DamageKind::Melee, Some(source));
    hit(&mut state, blocked, DamageKind::Melee, Some(source));
    let mut observed = Vec::new();
    state.commit_damage(Some(&mut |_, slot, fact| observed.push((slot, fact))));
    assert_eq!(
        observed
            .iter()
            .filter(|(_, fact)| *fact == DamageObservation::AcceptedReactive)
            .count(),
        1
    );
    assert!(observed.contains(&(actor_slot, DamageObservation::AcceptedReactive)));
    assert_eq!(
        observed
            .iter()
            .filter(|(_, fact)| *fact == DamageObservation::AcceptedVictim)
            .count(),
        2
    );
    assert!(observed.contains(&(actor_slot, DamageObservation::AcceptedVictim)));
    assert!(observed.contains(&(target_slot, DamageObservation::AcceptedVictim)));
    assert!(!observed.iter().any(|(slot, _)| *slot == blocked_slot));
    assert!(state.players[&actor].vitals.health < 20.0);
}

#[test]
fn damage_observer_excludes_command_only_groups_without_changing_damage() {
    use crate::tick::damage::DamageObservation;

    let (mut state, _, pos) = flat_player_fixture();
    let command_only = state.spawn_mob(state.registry.zombie_type_id, pos, 1000.0);
    let mixed = state.spawn_mob(
        state.registry.zombie_type_id,
        [pos[0] + 1.0, pos[1], pos[2]],
        1000.0,
    );
    let command = DamageSource::Command {
        connection: ConnectionId(1),
        damage_type: "minecraft:generic",
        direct: None,
        cause: None,
        position: None,
    };
    hit(&mut state, command_only, DamageKind::Generic, Some(command));
    hit(&mut state, mixed, DamageKind::Generic, None);
    hit(&mut state, mixed, DamageKind::Generic, Some(command));
    state.merge_damage.last_mut().unwrap().1.amount = 200.0;
    let mut observed = Vec::new();
    state.commit_damage(Some(&mut |_, slot, fact| observed.push((slot, fact))));
    let command_slot = state.entities.slot_of(command_only).unwrap();
    let mixed_slot = state.entities.slot_of(mixed).unwrap();
    assert_eq!(
        observed,
        vec![(mixed_slot, DamageObservation::AcceptedVictim)]
    );
    assert_eq!(state.entities.row(command_slot).health, 900.0);
    assert_eq!(state.entities.row(mixed_slot).health, 800.0);
}

fn shield(state: &mut TickState, player: EntityId) {
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:shield").unwrap())
        .unwrap();
    let actor = state.players.get_mut(&player).unwrap();
    actor.inventory.slots[0] = mc_data::ItemStack::new(item, 1);
    actor.use_item(&state.registry.item_rules(), Hand::Main);
    for _ in 0..5 {
        actor.tick_item_use(&state.registry.item_rules());
    }
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.row_mut(slot).protected_until = 0;
    state.entities.set_rot(slot, 0.0, 0.0);
}

#[test]
fn parity_creative_and_spectator_ignore_damage_but_not_void() {
    for mode in [GameMode::Creative, GameMode::Spectator] {
        let (mut state, player, _) = flat_player_fixture();
        state.players.get_mut(&player).unwrap().game_mode = mode;
        for kind in [DamageKind::Melee, DamageKind::Projectile, DamageKind::Lava] {
            hit(&mut state, player, kind, None);
            state.commit_merge_effects();
            assert_eq!(
                state.players[&player].vitals.health, 20.0,
                "{mode:?} {kind:?}"
            );
            assert_eq!(
                state
                    .entities
                    .body(state.entities.slot_of(player).unwrap())
                    .hurt_at,
                None
            );
        }
        hit(&mut state, player, DamageKind::Void, None);
        assert_eq!(state.commit_merge_effects().0, vec![player]);
    }
}

#[test]
fn parity_death_orbs_once_keep_inventory_and_fire_reset() {
    for keep in [false, true] {
        let (mut state, player, _) = flat_player_fixture();
        state
            .gamerules
            .insert("keep_inventory".into(), keep.to_string());
        state
            .players
            .get_mut(&player)
            .unwrap()
            .vitals
            .experience
            .level = 10;
        let slot = state.entities.slot_of(player).unwrap();
        state.entities.body_mut(slot).burn_ticks = 300;
        std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).shared_flags |= 1;
        hit(&mut state, player, DamageKind::Void, None);
        let (_, _, spawned, _) = state.commit_merge_effects();
        let xp: i32 = spawned
            .iter()
            .filter_map(|id| state.entities.slot_of(*id))
            .filter_map(|slot| state.entities.row(slot).appearance.experience_orb)
            .map(|(value, count)| value * count)
            .sum();
        assert_eq!(xp, if keep { 0 } else { 70 });
        assert!(state.commit_merge_effects().2.is_empty());
        assert!(crate::tick::respawn_player(
            &mut state,
            ConnectionId(1),
            &mut ChangeSet::default()
        ));
        assert_eq!(state.entities.body(slot).burn_ticks, 0);
        assert_eq!(state.entities.row(slot).appearance.shared_flags & 1, 0);
    }
}

#[test]
fn parity_spectator_moves_through_wall() {
    let (mut state, player, pos) = flat_player_fixture();
    let slot = state.entities.slot_of(player).unwrap();
    state.players.get_mut(&player).unwrap().game_mode = GameMode::Spectator;
    let x = pos[0].floor() as i32 + 1;
    for y in pos[1].floor() as i32..=pos[1].floor() as i32 + 2 {
        state
            .world
            .set_block(x, y, pos[2].floor() as i32, state.registry.states.stone);
    }
    let dest = [pos[0] + 2.0, pos[1], pos[2]];
    run_tick(
        &mut state,
        &[PlayerInput::Move {
            connection: ConnectionId(1),
            position: dest,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        }],
    );
    assert_eq!(state.entities.pos(slot), dest);
}

#[test]
fn parity_peaceful_recovers_without_starvation() {
    let (mut state, player, _) = flat_player_fixture();
    state.difficulty = Difficulty::Peaceful;
    let p = state.players.get_mut(&player).unwrap();
    p.vitals.food = 0;
    p.vitals.saturation = 0.0;
    p.vitals.health = 10.0;
    for _ in 0..80 {
        run_tick(&mut state, &[]);
    }
    assert!(state.players[&player].vitals.health > 10.0);
    assert!(state.players[&player].vitals.food >= 8);
}

#[test]
fn parity_mob_melee_cannot_cross_solid_wall() {
    let (mut state, player, pos) = flat_player_fixture();
    let kind = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:zombie").unwrap())
        .unwrap();
    let mob = state.spawn_mob(kind, [pos[0], pos[1], pos[2] + 1.8], 20.0);
    let z = pos[2].floor() as i32 + 1;
    for y in pos[1].floor() as i32..=pos[1].floor() as i32 + 2 {
        state
            .world
            .set_block(pos[0].floor() as i32, y, z, state.registry.states.stone);
    }
    let source = DamageSource::Melee {
        attacker: mob,
        position: [pos[0], pos[1], pos[2] + 1.8],
        yaw: 0.0,
        bonus_knockback: 0.0,
        sweep: false,
        fire_ticks: 0,
        bane_ticks: 0,
    };
    hit(&mut state, player, DamageKind::Melee, Some(source));
    state.commit_merge_effects();
    assert_eq!(state.players[&player].vitals.health, 20.0);
}

#[test]
fn parity_death_announced_to_other_players() {
    let (mut state, player, _) = flat_player_fixture();
    join(&mut state, 2, "observer");
    state.outbox.clear();
    hit(&mut state, player, DamageKind::Void, None);
    state.commit_merge_effects();
    assert!(state.outbox[&ConnectionId(2)].iter().any(
        |change| matches!(change, PlayerChange::StyledChat { content } if format!("{content:?}").contains("death.attack.outOfWorld"))
    ));
}

#[test]
fn parity_special_mob_melee_death_names_the_mob_for_the_two_argument_template() {
    let (mut state, player, pos) = flat_player_fixture();
    let mob = state.spawn_mob(
        state.registry.zombie_type_id,
        [pos[0] + 1.0, pos[1], pos[2]],
        100.0,
    );
    // The source `special_mobs::damage` submits for warden/guardian-style attacks.
    let source = DamageSource::Projectile {
        projectile: mob,
        direct_type_id: state.registry.zombie_type_id,
        owner: mob,
        velocity: [0.0; 2],
        punch: 0,
        flame: false,
    };
    state.outbox.clear();
    hit(&mut state, player, DamageKind::Melee, Some(source));
    state.commit_merge_effects();
    let message = state.outbox[&ConnectionId(1)]
        .iter()
        .find_map(|change| match change {
            PlayerChange::Died { message } => Some(format!("{message:?}")),
            _ => None,
        })
        .expect("the player died");
    assert!(message.contains("death.attack.mob"), "{message}");
    assert!(message.contains("entity.minecraft.zombie"), "{message}");
}

#[test]
fn parity_death_message_keys_match_the_damage_type_registry_and_language_file() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/vanilla");
    let lang: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{root}/assets/minecraft/lang/en_us.json")).unwrap(),
    )
    .unwrap();
    let mut mismatches = Vec::new();
    for kind in [
        DamageKind::Generic,
        DamageKind::Melee,
        DamageKind::Thorns,
        DamageKind::Projectile,
        DamageKind::Fireball,
        DamageKind::Lightning,
        DamageKind::Explosion,
        DamageKind::InFire,
        DamageKind::OnFire,
        DamageKind::Lava,
        DamageKind::Suffocation,
        DamageKind::OutsideBorder,
        DamageKind::Cactus,
        DamageKind::SweetBerryBush,
        DamageKind::HotFloor,
        DamageKind::Campfire,
        DamageKind::Fall,
        DamageKind::FallingAnvil,
        DamageKind::FallingStalactite,
        DamageKind::Drowning,
        DamageKind::Starvation,
        DamageKind::Wither,
        DamageKind::Magic,
        DamageKind::GenericKill,
        DamageKind::Freeze,
        DamageKind::FlyIntoWall,
        DamageKind::Cramming,
        DamageKind::SonicBoom,
        DamageKind::IndirectMagic,
        DamageKind::Stalagmite,
        DamageKind::DragonBreath,
        DamageKind::Void,
    ] {
        let (mut state, player, _) = flat_player_fixture();
        hit(&mut state, player, kind, None);
        state.commit_merge_effects();
        let message = state.outbox[&ConnectionId(1)]
            .iter()
            .find_map(|change| match change {
                PlayerChange::Died { message } => Some(format!("{message:?}")),
                _ => None,
            })
            .expect("the player died");
        let type_name = kind.type_name(None, false);
        let registry: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(format!(
                "{root}/data/minecraft/damage_type/{}.json",
                type_name.strip_prefix("minecraft:").unwrap()
            ))
            .unwrap(),
        )
        .unwrap();
        let key = format!("death.attack.{}", registry["message_id"].as_str().unwrap());
        if !message.contains(&format!("\"{key}\"")) || lang.get(&key).is_none() {
            mismatches.push(format!("{kind:?}: want {key}, got {message}"));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn parity_shield_blocks_front_explosion() {
    let (mut state, player, pos) = flat_player_fixture();
    state.difficulty = Difficulty::Normal;
    shield(&mut state, player);
    crate::explosion::apply_no_blocks(&mut state, [pos[0], pos[1], pos[2] + 2.0], 3.0, &[]);
    state.commit_merge_effects();
    assert_eq!(state.players[&player].vitals.health, 20.0);
}

#[test]
fn parity_crit_requires_fall_distance_not_air_below() {
    for (height, fall, expected) in [(2.0, 0.0, 19.0), (0.2, 0.2, 18.5)] {
        let (mut state, player, mut pos) = flat_player_fixture();
        pos[1] += height;
        let slot = state.entities.slot_of(player).unwrap();
        state.entities.set_pos(slot, pos);
        state.entities.set_ground(slot, false);
        state.players.get_mut(&player).unwrap().fall_distance = fall;
        state.tick = 30;
        let target = state.spawn_mob(
            state.registry.cow_type_id,
            [pos[0] + 1.0, pos[1], pos[2]],
            20.0,
        );
        run_tick(
            &mut state,
            &[PlayerInput::Attack {
                connection: ConnectionId(1),
                target,
            }],
        );
        let slot = state.entities.slot_of(target).unwrap();
        assert_eq!(
            state.entities.row(slot).health,
            expected,
            "height {height} fall {fall}"
        );
    }
}

#[test]
fn parity_zero_and_large_respawn_radius_are_safe() {
    let (mut state, _, _) = flat_player_fixture();
    let spawn = state.world.world_spawn();
    state.gamerules.insert("respawn_radius".into(), "0".into());
    for seed in 0..20 {
        let pos = crate::natural_spawn::player_spawn(&state, seed);
        assert_eq!(pos[0].floor() as i32, spawn[0]);
        assert_eq!(pos[2].floor() as i32, spawn[2]);
    }
    state
        .gamerules
        .insert("respawn_radius".into(), i32::MAX.to_string());
    assert_eq!(
        crate::natural_spawn::player_spawn(&state, 7),
        crate::natural_spawn::player_spawn(&state, 7)
    );
}

#[test]
fn parity_invalid_overworld_spawn_is_cleared() {
    let (mut state, player, pos) = flat_player_fixture();
    state.players.get_mut(&player).unwrap().respawn_point = Some(pos.map(|v| v.floor() as i32));
    hit(&mut state, player, DamageKind::Void, None);
    state.commit_merge_effects();
    assert!(crate::tick::respawn_player(
        &mut state,
        ConnectionId(1),
        &mut ChangeSet::default()
    ));
    assert_eq!(state.players[&player].respawn_point, None);
}

#[test]
fn parity_piercing_bypasses_shield_without_wear_and_arrows_rebound() {
    use crate::projectiles::{ProjectileKind, ProjectileState};
    for piercing in [0, 1] {
        let (mut state, player, pos) = flat_player_fixture();
        shield(&mut state, player);
        let mut record =
            ProjectileState::new(ProjectileKind::Arrow, None, mc_data::ItemStack::EMPTY);
        record.piercing = piercing;
        let arrow = crate::projectiles::launch(
            &mut state,
            record,
            [pos[0], pos[1] + 1.0, pos[2] + 2.0],
            [0.0, 0.0, -3.0],
        )
        .unwrap();
        run_tick(&mut state, &[]);
        if piercing == 0 {
            assert_eq!(state.players[&player].vitals.health, 20.0);
            let slot = state.entities.slot_of(arrow).unwrap();
            assert!(state.entities.vel(slot)[2] > 0.0);
            assert!(state.players[&player].inventory.slots[0].damage.is_some());
        } else {
            assert!(state.players[&player].vitals.health < 20.0);
            assert!(state.players[&player].inventory.slots[0].damage.is_none());
        }
    }
}

#[test]
fn fix_mobs_totem_intercepts_lethal_damage_in_either_hand_but_not_void() {
    use crate::effects::{ActiveEffect, EffectId};
    for index in [0, crate::inventory::OFF_HAND] {
        let (mut state, player, _) = flat_player_fixture();
        let totem = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:totem_of_undying").unwrap())
            .unwrap();
        let actor = state.players.get_mut(&player).unwrap();
        actor.inventory.slots[index] = mc_data::ItemStack::new(totem, 1);
        actor.give_effect(ActiveEffect::new(EffectId::Poison, 600, 0));
        hit(&mut state, player, DamageKind::Fall, None);
        let (deaths, events, spawned, _) = state.commit_merge_effects();
        assert!(deaths.is_empty(), "held totem must prevent death");
        assert!(spawned.is_empty(), "no drops or death XP");
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, crate::boundary::EntityEvent::Death { .. }))
        );
        assert!(events.iter().any(|event| matches!(event, crate::boundary::EntityEvent::Status { entity, event: 35 } if *entity == player)));
        let actor = &state.players[&player];
        assert_eq!(actor.vitals.health, 1.0);
        assert!(actor.inventory.slots[index].is_empty());
        assert!(!actor.active_effects.has(EffectId::Poison));
        for (id, duration, amplifier) in [
            (EffectId::Regeneration, 900, 1),
            (EffectId::Absorption, 100, 1),
            (EffectId::FireResistance, 800, 0),
        ] {
            let effect = actor.active_effects.get(id).unwrap();
            assert_eq!((effect.duration, effect.amplifier), (duration, amplifier));
        }
        state.players.get_mut(&player).unwrap().inventory.slots[index] =
            mc_data::ItemStack::new(totem, 1);
        state
            .gamerules
            .insert("keep_inventory".into(), "true".into());
        hit(&mut state, player, DamageKind::Void, None);
        assert_eq!(state.commit_merge_effects().0, vec![player]);
        assert_eq!(state.players[&player].inventory.slots[index].count, 1);
    }
}

#[test]
fn fix_mobs_totem_checks_main_hand_first_and_never_searches_backpack() {
    for held in [false, true] {
        let (mut state, player, _) = flat_player_fixture();
        let totem = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:totem_of_undying").unwrap())
            .unwrap();
        let actor = state.players.get_mut(&player).unwrap();
        actor.inventory.slots[9] = mc_data::ItemStack::new(totem, 1);
        if held {
            actor.inventory.slots[0] = mc_data::ItemStack::new(totem, 1);
            actor.inventory.slots[crate::inventory::OFF_HAND] = mc_data::ItemStack::new(totem, 1);
        }
        state
            .gamerules
            .insert("keep_inventory".into(), "true".into());
        hit(&mut state, player, DamageKind::Fall, None);
        let deaths = state.commit_merge_effects().0;
        assert_eq!(deaths.is_empty(), held);
        let actor = &state.players[&player];
        assert_eq!(actor.inventory.slots[9].count, 1);
        if held {
            assert!(actor.inventory.slots[0].is_empty());
            assert_eq!(actor.inventory.slots[crate::inventory::OFF_HAND].count, 1);
        }
    }
}

#[test]
fn death_protection_component_rescues_any_held_item_without_default_totem_effects() {
    use crate::command_items::ItemArgument;
    use crate::effects::{ActiveEffect, EffectId};
    for item in ["stick", "totem_of_undying"] {
        for hand in [0, crate::inventory::OFF_HAND] {
            let (mut state, player, _) = flat_player_fixture();
            let argument =
                ItemArgument::parse(&format!("{item}[death_protection={{death_effects:[]}}]"))
                    .unwrap();
            let mut stack = argument.stack(&state.registry.items).unwrap();
            let count = if item == "stick" { 2 } else { 1 };
            stack.count = count;
            let actor = state.players.get_mut(&player).unwrap();
            actor.inventory.slots[hand] = stack;
            actor.give_effect(ActiveEffect::new(EffectId::Poison, 600, 0));
            hit(&mut state, player, DamageKind::Fall, None);
            let (deaths, events, drops, _) = state.commit_merge_effects();
            assert!(deaths.is_empty(), "{item} in slot {hand}");
            assert!(drops.is_empty());
            assert!(events.iter().any(|event| matches!(
                event,
                crate::boundary::EntityEvent::Status { entity, event: 35 } if *entity == player
            )));
            let actor = &state.players[&player];
            assert_eq!(actor.vitals.health, 1.0);
            assert_eq!(actor.inventory.slots[hand].count, count - 1);
            assert!(actor.active_effects.has(EffectId::Poison));
            assert!(!actor.active_effects.has(EffectId::Regeneration));
            assert!(!actor.active_effects.has(EffectId::Absorption));
            assert!(!actor.active_effects.has(EffectId::FireResistance));
        }
    }
}

#[test]
fn custom_death_protection_applies_actions_in_order_and_broadcasts_sound() {
    use crate::command_items::ItemArgument;
    use crate::effects::{ActiveEffect, EffectId};
    let (mut state, player, _) = flat_player_fixture();
    let stack = ItemArgument::parse(
        "stick[death_protection={death_effects:[\
         {type:'minecraft:apply_effects',effects:[{id:'minecraft:speed',duration:200}]},\
         {type:'minecraft:remove_effects',effects:['minecraft:speed']},\
         {type:'minecraft:clear_all_effects'},\
         {type:'minecraft:apply_effects',effects:[{id:'minecraft:regeneration',duration:120,amplifier:1}]},\
         {type:'minecraft:apply_effects',probability:0.0f,effects:[{id:'minecraft:poison',duration:120}]},\
         {type:'minecraft:play_sound',sound:{sound_id:'minecraft:block.bell.use',range:32.0f}}]}]",
    ).unwrap().stack(&state.registry.items).unwrap();
    let actor = state.players.get_mut(&player).unwrap();
    actor.inventory.slots[0] = stack;
    actor.give_effect(ActiveEffect::new(EffectId::Poison, 400, 0));
    hit(&mut state, player, DamageKind::Fall, None);
    assert!(state.commit_merge_effects().0.is_empty());
    let actor = &state.players[&player];
    assert_eq!(actor.vitals.health, 1.0);
    assert!(!actor.active_effects.has(EffectId::Poison));
    assert!(!actor.active_effects.has(EffectId::Speed));
    let regen = actor.active_effects.get(EffectId::Regeneration).unwrap();
    assert_eq!((regen.duration, regen.amplifier), (120, 1));
    assert!(state.outbox[&ConnectionId(1)].iter().any(|change| matches!(
        change, PlayerChange::CommandSound { sound, fixed_range: Some(range), .. }
            if sound.as_str() == "minecraft:block.bell.use" && *range == 32.0
    )));
}

#[test]
fn custom_death_protection_removes_only_effects_in_named_tag() {
    use crate::command_items::ItemArgument;
    use crate::effects::{ActiveEffect, EffectId};
    let (mut state, player, _) = flat_player_fixture();
    let registry = std::sync::Arc::make_mut(&mut state.registry);
    std::sync::Arc::make_mut(&mut registry.tags)
        .registries
        .entry(mc_data::Identifier::parse("minecraft:mob_effect").unwrap())
        .or_default()
        .insert(
            mc_data::Identifier::parse("example:cleansed").unwrap(),
            vec![EffectId::Poison as i32],
        );
    let stack = ItemArgument::parse(
        "stick[death_protection={death_effects:[{type:'minecraft:remove_effects',effects:'#example:cleansed'}]}]",
    ).unwrap().stack(&state.registry.items).unwrap();
    let actor = state.players.get_mut(&player).unwrap();
    actor.inventory.slots[0] = stack;
    actor.give_effect(ActiveEffect::new(EffectId::Poison, 400, 0));
    actor.give_effect(ActiveEffect::new(EffectId::Speed, 400, 0));
    hit(&mut state, player, DamageKind::Fall, None);
    assert!(state.commit_merge_effects().0.is_empty());
    assert!(!state.players[&player].active_effects.has(EffectId::Poison));
    assert!(state.players[&player].active_effects.has(EffectId::Speed));
}

#[test]
fn death_protection_rescue_wal_replays_health_effects_and_inventory_custody_together() {
    use crate::command_items::ItemArgument;
    use crate::effects::EffectId;
    use crate::journal::{JournalWriter, Replay, read_complete_batches_after, replay_batches};
    let (mut state, player, _) = flat_player_fixture();
    let mut protection = ItemArgument::parse(
        "stick[death_protection={death_effects:[{type:'minecraft:apply_effects',effects:[{id:'minecraft:regeneration',duration:120}]}]}]",
    ).unwrap().stack(&state.registry.items).unwrap();
    protection.count = 2;
    let diamond = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:diamond").unwrap())
        .unwrap();
    let actor = state.players.get_mut(&player).unwrap();
    actor.inventory.slots[0] = protection.clone();
    actor.inventory_menu.carried = mc_data::ItemStack::new(diamond, 3);
    actor.inventory_menu.grid[0] = mc_data::ItemStack::new(diamond, 4);
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.row_mut(slot).protected_until = 0;
    let uuid = state.entities.row(slot).uuid.to_string();
    let (output, _, _) = run_tick(
        &mut state,
        &[PlayerInput::Command {
            connection: ConnectionId(1),
            permission_level: 4,
            command: "damage @s 30 minecraft:fall".into(),
        }],
    );
    let expected = crate::core::player_record_for(&state, player, output.tick).unwrap();
    assert_eq!(expected.health, 1.0);
    assert!(expected.active_effects.has(EffectId::Regeneration));
    assert_eq!(expected.inventory[0].count, 1);
    assert_eq!(expected.carried.count, 3);
    assert_eq!(expected.grid[0].count, 4);
    let entries = crate::core::journal_entries_for_tick(&mut state, &output);
    let dir = tempfile::tempdir().unwrap();
    let mut writer = JournalWriter::open(dir.path(), 1).unwrap();
    writer.append(1, output.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let batches = read_complete_batches_after(&dir.path().join("journal"), 0).unwrap();
    let replay = replay_batches(Replay::default(), &batches);
    assert!(replay.players[&uuid].eq_except_statistics(&expected));
    let replay = replay_batches(replay, &batches);
    assert!(replay.players[&uuid].eq_except_statistics(&expected));
}

#[test]
fn death_protection_changes_fire_admission_before_the_next_same_tick_hit() {
    use crate::command_items::ItemArgument;
    use crate::effects::{ActiveEffect, EffectId};
    for clear in [true, false] {
        let (mut state, player, _) = flat_player_fixture();
        let action = if clear {
            "{type:'minecraft:clear_all_effects'}"
        } else {
            "{type:'minecraft:apply_effects',effects:[{id:'minecraft:fire_resistance',duration:200}]}"
        };
        let item = ItemArgument::parse(&format!(
            "stick[death_protection={{death_effects:[{action}]}}]"
        ))
        .unwrap()
        .stack(&state.registry.items)
        .unwrap();
        let actor = state.players.get_mut(&player).unwrap();
        actor.inventory.slots[0] = item;
        if clear {
            actor.give_effect(ActiveEffect::new(EffectId::FireResistance, 200, 0));
        }
        let slot = state.entities.slot_of(player).unwrap();
        state.entities.row_mut(slot).protected_until = 0;
        for (kind, amount) in [(DamageKind::Fall, 30.0), (DamageKind::Lava, 40.0)] {
            state.merge_damage.push((
                slot,
                DamagePiece {
                    amount,
                    kind,
                    source: None,
                    physical: None,
                    custom_armor: false,
                },
            ));
        }
        let (deaths, events, _, _) = state.commit_merge_effects();
        assert_eq!(deaths.contains(&player), clear, "clear={clear}");
        let actor = &state.players[&player];
        assert_eq!(actor.vitals.is_dead(), clear);
        assert_eq!(actor.active_effects.has(EffectId::FireResistance), !clear);
        if !clear {
            assert_eq!(actor.vitals.health, 1.0);
            assert_eq!(
                state.entities.row(slot).last_hurt,
                30.0,
                "resisted lava must not enter hurt admission"
            );
            assert!(!events.iter().any(|event| matches!(event,
                crate::boundary::EntityEvent::Damage { entity, source_type_id, .. }
                if *entity == player && *source_type_id == DamageKind::Lava.type_id(None, false)
            )));
        }
    }
}

#[test]
fn custom_death_protection_teleport_clears_fall_and_impulse_context() {
    use crate::command_items::ItemArgument;
    let (mut state, player, origin) = flat_player_fixture();
    let stack = ItemArgument::parse(
        "stick[death_protection={death_effects:[{type:'minecraft:teleport_randomly',diameter:16.0f}]}]",
    ).unwrap().stack(&state.registry.items).unwrap();
    let actor = state.players.get_mut(&player).unwrap();
    actor.inventory.slots[0] = stack;
    actor.fall_distance = 8.0;
    actor.current_impulse_impact_pos = Some(origin);
    hit(&mut state, player, DamageKind::Fall, None);
    assert!(state.commit_merge_effects().0.is_empty());
    let actor = &state.players[&player];
    assert_eq!(actor.fall_distance, 0.0);
    assert_eq!(actor.current_impulse_impact_pos, None);
    let destination = state.entities.pos(state.entities.slot_of(player).unwrap());
    assert_ne!(destination, origin);
    assert!((destination[0] - origin[0]).abs() <= 8.0);
    assert!((destination[2] - origin[2]).abs() <= 8.0);
    assert!(state.outbox[&ConnectionId(1)].iter().any(|change| matches!(
        change, PlayerChange::CommandSound { sound, .. }
            if sound.as_str() == "minecraft:item.chorus_fruit.teleport"
    )));
}

#[test]
fn death_teleport_keeps_mount_on_failure_and_dismounts_only_on_success() {
    use crate::command_items::ItemArgument;
    for succeeds in [false, true] {
        let (mut state, player, origin) = flat_player_fixture();
        let boat_type = state
            .registry
            .entity_types
            .id_of(&mc_data::Identifier::parse("minecraft:oak_boat").unwrap())
            .unwrap();
        let boat = state.spawn_mob(boat_type, [origin[0], origin[1], origin[2] + 1.0], 20.0);
        run_tick(
            &mut state,
            &[PlayerInput::InteractEntity {
                connection: ConnectionId(1),
                target: boat,
                hand: Hand::Main,
                location: [0.0; 3],
            }],
        );
        let player_slot = state.entities.slot_of(player).unwrap();
        let boat_slot = state.entities.slot_of(boat).unwrap();
        assert_eq!(crate::mounts::vehicle(&state, player), Some(boat_slot));
        if !succeeds {
            state
                .entities
                .set_pos(player_slot, [10_000.5, 90.0, 10_000.5]);
            state
                .entities
                .set_pos(boat_slot, [10_000.5, 89.0, 10_000.5]);
        }
        let old = state.entities.pos(player_slot);
        state.outbox.clear();
        state.players.get_mut(&player).unwrap().inventory.slots[0] = ItemArgument::parse(
            "stick[death_protection={death_effects:[{type:'minecraft:teleport_randomly',diameter:16.0f}]}]",
        ).unwrap().stack(&state.registry.items).unwrap();
        hit(&mut state, player, DamageKind::Fall, None);
        assert!(state.commit_merge_effects().0.is_empty());
        let destination = state.entities.pos(player_slot);
        if succeeds {
            assert_eq!(crate::mounts::vehicle(&state, player), None);
            assert_ne!(destination, old);
            assert!((destination[0] - old[0]).abs() <= 8.0);
            assert!((destination[2] - old[2]).abs() <= 8.0);
            assert!(state.outbox.values().flatten().any(|change| matches!(
                change, PlayerChange::PositionCorrected { position, .. } if *position == destination
            )));
        } else {
            assert_eq!(crate::mounts::vehicle(&state, player), Some(boat_slot));
            assert_eq!(destination, old);
            assert!(
                !state
                    .outbox
                    .values()
                    .flatten()
                    .any(|change| matches!(change, PlayerChange::PositionCorrected { .. }))
            );
        }
    }
}

#[test]
fn death_protection_removal_skips_main_hand_and_can_use_offhand_default() {
    use crate::command_items::ItemArgument;
    for offhand in [false, true] {
        let (mut state, player, _) = flat_player_fixture();
        let removed = ItemArgument::parse("totem_of_undying[!death_protection]")
            .unwrap()
            .stack(&state.registry.items)
            .unwrap();
        let totem = removed.item_id;
        let actor = state.players.get_mut(&player).unwrap();
        actor.inventory.slots[0] = removed;
        if offhand {
            actor.inventory.slots[crate::inventory::OFF_HAND] = mc_data::ItemStack::new(totem, 1);
        }
        state
            .gamerules
            .insert("keep_inventory".into(), "true".into());
        hit(&mut state, player, DamageKind::Fall, None);
        let deaths = state.commit_merge_effects().0;
        assert_eq!(deaths.is_empty(), offhand);
        let actor = &state.players[&player];
        assert_eq!(actor.inventory.slots[0].count, 1);
        assert!(actor.inventory.slots[crate::inventory::OFF_HAND].is_empty());
        if offhand {
            assert_eq!(actor.vitals.health, 1.0);
            assert!(
                actor
                    .active_effects
                    .has(crate::effects::EffectId::Regeneration)
            );
        }
    }
}

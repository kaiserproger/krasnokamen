use super::*;

#[test]
fn geyser_commands_refuse_missing_options_before_emitting() {
    let (mut state, player, _) = crate::hostile_tests::setup(8.0);
    let context = Context::source(&state, crate::command_console::CONNECTION, player, 4);
    for name in ["geyser", "geyser_base", "geyser_poof", "geyser_plume"] {
        let command = format!("particle minecraft:{name} ~ ~1 ~ 0 0 0 0 1 force @a");
        assert_eq!(run(&mut state, context, &command), Err("Particle options are not supported".into()), "{name}");
    }
}

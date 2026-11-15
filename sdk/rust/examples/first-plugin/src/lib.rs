use krasnokamen_plugin_sdk::{
    export_plugin, message_session, Command, Event, EventContext, Failure, Plugin,
};

#[derive(Default)]
struct Hello;

impl Plugin for Hello {
    fn on_events(&mut self, _: &EventContext, events: &[Event]) -> Result<Vec<Command>, Failure> {
        let mut commands = Vec::new();
        for event in events {
            if let Event::CommandInvoked(invoked) = event {
                if invoked.name == "hello" {
                    commands.push(message_session(
                        invoked.session,
                        "Hello from a WASM plugin.",
                    ));
                }
            }
        }
        Ok(commands)
    }
}

export_plugin!(Hello);

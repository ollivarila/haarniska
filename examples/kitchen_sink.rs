//! Kitchen sink: an agent with everything enabled, running in the terminal.

use haarniska::Agent;
use haarniska::harness::plugin::Plugin;
use haarniska::inference::Inference;
use haarniska::tui::Tui;

// TODO: replace with a built-in inference implementation configured from an
// API key in the environment.
struct TodoInference;

impl Inference for TodoInference {}

// TODO: replace with the built-in tools: read, write, edit, shell.
struct TodoBuiltinTools;

impl Plugin for TodoBuiltinTools {}

// TODO: show a custom tool the model can call.
struct CustomPlugin;

impl Plugin for CustomPlugin {}

#[tokio::main]
async fn main() {
    let agent = haarniska::builder()
        .inference(TodoInference)
        .plugin(TodoBuiltinTools)
        .plugin(CustomPlugin)
        .build();

    agent.run(Tui::new()).await;
}

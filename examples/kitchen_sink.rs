//! Kitchen sink: an agent with everything enabled, running in the terminal.

use haarniska::Agent;
use haarniska::harness::plugin::Plugin;
use haarniska::inference::anthropic::AnthropicInference;
use haarniska::tui::Tui;

// TODO: replace with the built-in tools: read, write, edit, shell.
struct TodoBuiltinTools;

impl Plugin for TodoBuiltinTools {}

// TODO: show a custom tool the model can call.
struct CustomPlugin;

impl Plugin for CustomPlugin {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = haarniska::builder()
        .inference(AnthropicInference::new("claude-opus-5")?)
        .plugin(TodoBuiltinTools)
        .plugin(CustomPlugin)
        .build();

    agent.run(Tui::new()).await;
    Ok(())
}

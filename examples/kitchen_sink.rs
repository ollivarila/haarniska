//! Kitchen sink: an agent with everything enabled, running in the terminal.

use haarniska::Agent;
use haarniska::inference::anthropic::{AnthropicInference, model};
use haarniska::tui::Tui;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = haarniska::builder()
        .inference(AnthropicInference::new(model::CLAUDE_HAIKU_4_5)?)
        .build();

    agent.run(Tui::new()).await;
    Ok(())
}

//! Kitchen sink: an agent with everything enabled, running in the terminal.

use std::time::Instant;

use haarniska::Agent;
use haarniska::inference::anthropic::{AnthropicInference, model};
use haarniska::tui::Tui;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    // Loads ANTHROPIC_API_KEY from a `.env` file, if there is one.
    dotenvy::dotenv().ok();
    let agent = haarniska::builder()
        .with_inference(AnthropicInference::new(model::CLAUDE_HAIKU_4_5)?)
        .with_default_tools()
        .build();

    agent.run(Tui::new(started)?).await;
    Ok(())
}

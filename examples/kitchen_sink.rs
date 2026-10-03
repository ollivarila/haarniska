//! Kitchen sink: an agent with everything enabled, running in the terminal.

use std::time::Instant;

use haarniska::Agent;
use haarniska::harness::instructions::Instructions;
use haarniska::harness::skills::Skills;
use haarniska::inference::anthropic::{AnthropicInference, model};
use haarniska::tui::Tui;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    // Loads ANTHROPIC_API_KEY from a `.env` file, if there is one.
    dotenvy::dotenv().ok();
    // User-wide skills first, then the project's, which win on a name clash.
    let home = std::env::home_dir().unwrap_or_default();
    let skills = Skills::new()
        .layer_dir(home.join(".claude/skills"))?
        .layer_dir(".claude/skills")?;

    let agent = haarniska::builder()
        .with_inference(AnthropicInference::new(model::CLAUDE_HAIKU_4_5)?)
        .with_default_tools()
        .with_instructions(Instructions::new().layer_dir(".")?)
        .with_skills(skills)
        .build();

    agent.run(Tui::new(started)?).await;
    Ok(())
}

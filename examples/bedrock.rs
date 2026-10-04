//! An agent on Amazon Bedrock, running in the terminal.
//!
//! `BEDROCK_MODEL` names the model or inference profile. AWS credentials come
//! from `AWS_PROFILE` and the region from `AWS_REGION`. If
//! `BEDROCK_AUTH_REFRESH` is set, it is run when the credentials expire.

use std::time::Instant;

use haarniska::Agent;
use haarniska::harness::hook::AskBefore;
use haarniska::harness::instructions::Instructions;
use haarniska::inference::bedrock::BedrockInference;
use haarniska::tui::Tui;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let tui = Tui::new(started)?;
    dotenvy::dotenv().ok();

    let model = std::env::var("BEDROCK_MODEL").map_err(|_| "BEDROCK_MODEL is not set")?;
    let mut inference = BedrockInference::new(&model);
    if let Ok(command) = std::env::var("BEDROCK_AUTH_REFRESH") {
        inference = inference.with_auth_refresh(&command);
    }

    let agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_instructions(Instructions::new().layer_dir(".")?)
        .with_hook(AskBefore::tools(["shell", "write", "edit"]))
        .with_approver(tui.approver())
        .build();

    agent.run(tui).await;
    Ok(())
}

//! The agent loop, driven by scripted model replies and real tools in a
//! temporary directory.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures_util::{Stream, StreamExt, stream};
use haarniska::Agent;
use haarniska::harness::Event;
use haarniska::harness::hook::{Answer, ApprovalRequest, Approver, AskBefore, Decision, Hook};
use haarniska::harness::instructions::Instructions;
use haarniska::harness::skills::{SkillProblem, Skills};
use haarniska::harness::tool::{Tool, ToolCx, ToolError};
use haarniska::inference::{
    Block, Chunk, Error, Inference, Message, Reply, Request, StopReason, ToolCall, ToolResult,
    Usage,
};
use haarniska::ui::{Input, Ui};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;

#[tokio::test]
async fn text_reply_is_streamed_and_ends_the_turn() {
    let (inference, _requests) = ScriptedInference::new([text_reply("Hello")]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let events: Vec<Event> = agent.prompt("Say hello").collect().await;

    assert_eq!(
        events,
        [
            Event::Text("Hello".into()),
            Event::Done {
                stop: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ]
    );
}

#[tokio::test]
async fn tool_call_runs_and_its_result_goes_back_to_the_model() {
    let sandbox = TempDir::new().unwrap();
    let call = ToolCall {
        id: "call_1".into(),
        name: "write".into(),
        input: json!({"path": "a.txt", "content": "hi"}),
    };
    let (inference, requests) =
        ScriptedInference::new([tool_reply(call.clone()), text_reply("Written.")]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    let result = ToolResult {
        call_id: "call_1".into(),
        output: "Wrote 2 bytes to a.txt".into(),
        is_error: false,
    };
    assert_eq!(
        events,
        [
            Event::ToolStarted(call.clone()),
            Event::ToolFinished(result.clone()),
            Event::Text("Written.".into()),
            Event::Done {
                stop: StopReason::EndTurn,
                usage: Usage::default(),
            },
        ]
    );
    let written = std::fs::read_to_string(sandbox.path().join("a.txt")).unwrap();
    assert_eq!(written, "hi");
    assert_eq!(
        requests.lock().unwrap()[1].messages,
        [
            Message::User("Write a.txt".into()),
            Message::Assistant(vec![Block::ToolCall(call)]),
            Message::ToolResults(vec![result]),
        ]
    );
}

#[tokio::test]
async fn failing_tool_gives_the_model_an_error_and_the_turn_goes_on() {
    let sandbox = TempDir::new().unwrap();
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("read", json!({"path": "missing.txt"}))),
        text_reply("No such file."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Read missing.txt").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert!(result.output.starts_with("Error: "));
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn unknown_tool_gives_the_model_an_error() {
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("nope", json!({}))),
        text_reply("Sorry."),
    ]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let events: Vec<Event> = agent.prompt("Do it").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(result.output, "Error: no tool named `nope`");
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn input_that_does_not_fit_gives_the_model_an_error() {
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("read", json!({"file": "a.txt"}))),
        text_reply("Sorry."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .build();

    let events: Vec<Event> = agent.prompt("Read a.txt").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert!(result.output.starts_with("Error: invalid input"));
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn panicking_tool_gives_the_model_an_error_and_the_turn_goes_on() {
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("panic", json!({}))),
        text_reply("It crashed."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_tool(Panic)
        .build();

    let events: Vec<Event> = agent.prompt("Crash").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(result.output, "Error: the tool crashed");
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn failing_model_call_ends_the_turn_with_a_failure() {
    let (inference, _requests) = ScriptedInference::new([]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let events: Vec<Event> = agent.prompt("Hello").collect().await;

    assert_eq!(
        events,
        [Event::Failed(Error::new("no scripted reply left"))]
    );
}

#[tokio::test]
async fn next_prompt_continues_the_conversation() {
    let (inference, requests) = ScriptedInference::new([text_reply("Hi."), text_reply("Fine.")]);
    let mut agent = haarniska::builder().with_inference(inference).build();

    let _: Vec<Event> = agent.prompt("Hello").collect().await;
    let _: Vec<Event> = agent.prompt("How are you?").collect().await;

    assert_eq!(
        requests.lock().unwrap()[1].messages,
        [
            Message::User("Hello".into()),
            Message::Assistant(vec![Block::Text("Hi.".into())]),
            Message::User("How are you?".into()),
        ]
    );
}

#[tokio::test]
async fn hook_blocks_a_call_and_the_model_is_told_why() {
    let sandbox = TempDir::new().unwrap();
    let call = tool_call("write", json!({"path": "a.txt", "content": "hi"}));
    let (inference, _requests) =
        ScriptedInference::new([tool_reply(call.clone()), text_reply("Understood.")]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(BlockWrites)
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(
        result.output,
        "Error: blocked by a hook: writing is not allowed"
    );
    assert!(!sandbox.path().join("a.txt").exists());
    assert_eq!(events[0], Event::ToolStarted(call));
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn hook_changes_the_input_a_tool_runs_with() {
    let sandbox = TempDir::new().unwrap();
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call(
            "write",
            json!({"path": "a.txt", "content": "hi"}),
        )),
        text_reply("Written."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(WriteElsewhere)
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    assert!(!sandbox.path().join("a.txt").exists());
    let written = std::fs::read_to_string(sandbox.path().join("b.txt")).unwrap();
    assert_eq!(written, "hi");
    assert_eq!(
        events[0],
        Event::ToolStarted(tool_call(
            "write",
            json!({"path": "b.txt", "content": "hi"})
        ))
    );
}

#[tokio::test]
async fn hooks_change_the_result_in_the_order_they_were_added() {
    let (inference, requests) = ScriptedInference::new([
        tool_reply(tool_call("shell", json!({"command": "printf out"}))),
        text_reply("Done."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(Append("-first"))
        .with_hook(Append("-second"))
        .build();

    let events: Vec<Event> = agent.prompt("Run it").collect().await;

    assert_eq!(tool_result(&events).output, "out-first-second");
    let Message::ToolResults(results) = &requests.lock().unwrap()[1].messages[2] else {
        panic!("the model is sent the tool results");
    };
    assert_eq!(results[0].output, "out-first-second");
}

#[tokio::test]
async fn hook_crashing_before_a_call_blocks_it() {
    let sandbox = TempDir::new().unwrap();
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call(
            "write",
            json!({"path": "a.txt", "content": "hi"}),
        )),
        text_reply("Sorry."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(CrashBefore)
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(result.output, "Error: blocked: a hook crashed");
    assert!(!sandbox.path().join("a.txt").exists());
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn hook_crashing_after_a_call_leaves_the_result_alone() {
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("shell", json!({"command": "printf out"}))),
        text_reply("Done."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(CrashAfter)
        .with_hook(Append("-kept"))
        .build();

    let events: Vec<Event> = agent.prompt("Run it").collect().await;

    let result = tool_result(&events);
    assert!(!result.is_error);
    assert_eq!(result.output, "out-kept");
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn call_that_needs_consent_runs_when_the_user_allows_it() {
    let sandbox = TempDir::new().unwrap();
    let (approver, asked) = ScriptedApprover::new([Answer::Allow]);
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call(
            "write",
            json!({"path": "a.txt", "content": "hi"}),
        )),
        text_reply("Written."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(AskBefore::tools(["write"]))
        .with_approver(approver)
        .with_cwd(sandbox.path())
        .build();

    let _: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    assert!(sandbox.path().join("a.txt").exists());
    assert_eq!(
        *asked.lock().unwrap(),
        [ApprovalRequest {
            tool: "write".into(),
            input: json!({"path": "a.txt", "content": "hi"}),
            reasons: vec!["write needs your consent".into()],
        }]
    );
}

#[tokio::test]
async fn call_the_user_denies_does_not_run_and_the_model_is_told() {
    let sandbox = TempDir::new().unwrap();
    let (approver, _asked) = ScriptedApprover::new([Answer::Deny]);
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call(
            "write",
            json!({"path": "a.txt", "content": "hi"}),
        )),
        text_reply("Understood."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(AskBefore::tools(["write"]))
        .with_approver(approver)
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(result.output, "Error: the user declined this call");
    assert!(!sandbox.path().join("a.txt").exists());
    assert_eq!(events.last(), Some(&done()));
}

#[tokio::test]
async fn call_that_needs_consent_is_blocked_when_there_is_no_one_to_ask() {
    let sandbox = TempDir::new().unwrap();
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call(
            "write",
            json!({"path": "a.txt", "content": "hi"}),
        )),
        text_reply("Understood."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(AskBefore::tools(["write"]))
        .with_cwd(sandbox.path())
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    assert_eq!(
        tool_result(&events).output,
        "Error: this call needs the user's consent, and there is no one to ask"
    );
    assert!(!sandbox.path().join("a.txt").exists());
}

#[tokio::test]
async fn always_allow_stops_the_asking_for_that_tool() {
    let sandbox = TempDir::new().unwrap();
    let (approver, asked) = ScriptedApprover::new([Answer::AllowAlways]);
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call(
            "write",
            json!({"path": "a.txt", "content": "one"}),
        )),
        tool_reply(tool_call(
            "write",
            json!({"path": "b.txt", "content": "two"}),
        )),
        text_reply("Written."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(AskBefore::tools(["write"]))
        .with_approver(approver)
        .with_cwd(sandbox.path())
        .build();

    let _: Vec<Event> = agent.prompt("Write both").collect().await;

    assert!(sandbox.path().join("a.txt").exists());
    assert!(sandbox.path().join("b.txt").exists());
    assert_eq!(asked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn tools_that_were_not_named_run_without_asking() {
    let (approver, asked) = ScriptedApprover::new([]);
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("shell", json!({"command": "printf out"}))),
        text_reply("Done."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(AskBefore::tools(["write"]))
        .with_approver(approver)
        .build();

    let events: Vec<Event> = agent.prompt("Run it").collect().await;

    assert_eq!(tool_result(&events).output, "out");
    assert!(asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn block_from_a_later_hook_wins_and_the_user_is_not_asked() {
    let (approver, asked) = ScriptedApprover::new([Answer::Allow]);
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call(
            "write",
            json!({"path": "a.txt", "content": "hi"}),
        )),
        text_reply("Understood."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_default_tools()
        .with_hook(AskBefore::tools(["write"]))
        .with_hook(BlockWrites)
        .with_approver(approver)
        .build();

    let events: Vec<Event> = agent.prompt("Write a.txt").collect().await;

    assert_eq!(
        tool_result(&events).output,
        "Error: blocked by a hook: writing is not allowed"
    );
    assert!(asked.lock().unwrap().is_empty());
}

/// An [`Approver`] that gives `answers` in order, denies after that, and
/// records what it was asked.
struct ScriptedApprover {
    answers: Mutex<VecDeque<Answer>>,
    asked: Asked,
}

type Asked = Arc<Mutex<Vec<ApprovalRequest>>>;

impl ScriptedApprover {
    fn new(answers: impl IntoIterator<Item = Answer>) -> (Self, Asked) {
        let asked = Asked::default();
        let approver = Self {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: asked.clone(),
        };
        (approver, asked)
    }
}

impl Approver for ScriptedApprover {
    async fn approve(&self, request: ApprovalRequest) -> Answer {
        self.asked.lock().unwrap().push(request);
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Answer::Deny)
    }
}

struct BlockWrites;

impl Hook for BlockWrites {
    async fn before_tool(&self, _cx: &ToolCx, tool: &str, _input: &mut Value) -> Decision {
        if tool == "write" {
            return Decision::Block("writing is not allowed".into());
        }
        Decision::Continue
    }
}

/// Sends every write to `b.txt`.
struct WriteElsewhere;

impl Hook for WriteElsewhere {
    async fn before_tool(&self, _cx: &ToolCx, _tool: &str, input: &mut Value) -> Decision {
        input["path"] = json!("b.txt");
        Decision::Continue
    }
}

/// Adds its text to the end of every result.
struct Append(&'static str);

impl Hook for Append {
    async fn after_tool(&self, _cx: &ToolCx, _call: &ToolCall, result: &mut ToolResult) {
        result.output.push_str(self.0);
    }
}

struct CrashBefore;

impl Hook for CrashBefore {
    async fn before_tool(&self, _cx: &ToolCx, _tool: &str, _input: &mut Value) -> Decision {
        panic!("boom")
    }
}

/// Spoils the result, then crashes.
struct CrashAfter;

impl Hook for CrashAfter {
    async fn after_tool(&self, _cx: &ToolCx, _call: &ToolCall, result: &mut ToolResult) {
        result.output = "spoiled".into();
        panic!("boom")
    }
}

#[tokio::test]
async fn skills_are_listed_after_the_system_prompt() {
    let dir = TempDir::new().unwrap();
    write_skill(
        dir.path(),
        "release",
        "Steps for cutting a release.",
        "Tag it.",
    );
    write_skill(
        dir.path(),
        "write-docs",
        "Conventions for design docs.",
        "Be brief.",
    );
    let (inference, requests) = ScriptedInference::new([text_reply("Hi.")]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_system_prompt("You are a coding agent.")
        .with_skills(Skills::new().layer_dir(dir.path()).unwrap())
        .build();

    let _: Vec<Event> = agent.prompt("Hello").collect().await;

    let requests = requests.lock().unwrap();
    assert_eq!(
        requests[0].system,
        "You are a coding agent.\n\n\
         Skills are available for the tasks below. \
         Load one with the skill tool before doing a task it covers.\n\n\
         - release: Steps for cutting a release.\n\
         - write-docs: Conventions for design docs."
    );
    assert_eq!(requests[0].tools, ["skill"]);
}

#[tokio::test]
async fn model_loads_a_skill_and_gets_its_instructions() {
    let dir = TempDir::new().unwrap();
    write_skill(
        dir.path(),
        "release",
        "Steps for cutting a release.",
        "Tag it.",
    );
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("skill", json!({"name": "release"}))),
        text_reply("Released."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_skills(Skills::new().layer_dir(dir.path()).unwrap())
        .build();

    let events: Vec<Event> = agent.prompt("Cut a release").collect().await;

    let result = tool_result(&events);
    assert!(!result.is_error);
    assert_eq!(
        result.output,
        format!(
            "Tag it.\n\nThis skill's files are in {}",
            dir.path().join("release").display()
        )
    );
}

#[tokio::test]
async fn unknown_skill_gives_the_model_the_names_that_exist() {
    let dir = TempDir::new().unwrap();
    write_skill(
        dir.path(),
        "release",
        "Steps for cutting a release.",
        "Tag it.",
    );
    let (inference, _requests) = ScriptedInference::new([
        tool_reply(tool_call("skill", json!({"name": "deploy"}))),
        text_reply("Sorry."),
    ]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_skills(Skills::new().layer_dir(dir.path()).unwrap())
        .build();

    let events: Vec<Event> = agent.prompt("Deploy").collect().await;

    let result = tool_result(&events);
    assert!(result.is_error);
    assert_eq!(
        result.output,
        "Error: no skill named `deploy`. Available: release"
    );
}

#[tokio::test]
async fn skill_in_a_later_layer_replaces_one_with_the_same_name() {
    let user = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    write_skill(user.path(), "release", "The usual release.", "Tag it.");
    write_skill(
        project.path(),
        "release",
        "This project's release.",
        "Ship it.",
    );
    let skills = Skills::new()
        .layer_dir(user.path())
        .unwrap()
        .layer_dir(project.path())
        .unwrap();
    let (inference, requests) = ScriptedInference::new([text_reply("Hi.")]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_skills(skills)
        .build();

    let _: Vec<Event> = agent.prompt("Hello").collect().await;

    let system = &requests.lock().unwrap()[0].system;
    assert!(system.ends_with("\n- release: This project's release."));
    assert!(!system.contains("The usual release."));
}

#[tokio::test]
async fn broken_skill_is_skipped_and_reported() {
    let dir = TempDir::new().unwrap();
    write_skill(
        dir.path(),
        "release",
        "Steps for cutting a release.",
        "Tag it.",
    );
    std::fs::create_dir(dir.path().join("broken")).unwrap();
    std::fs::write(dir.path().join("broken/SKILL.md"), "No header here.").unwrap();
    std::fs::create_dir(dir.path().join("not-a-skill")).unwrap();

    let skills = Skills::new().layer_dir(dir.path()).unwrap();

    assert_eq!(
        skills.problems(),
        [SkillProblem {
            path: dir.path().join("broken/SKILL.md"),
            reason: "there is no header".into(),
        }]
    );
}

#[tokio::test]
async fn without_skills_there_is_no_listing_and_no_skill_tool() {
    let empty = TempDir::new().unwrap();
    let skills = Skills::new()
        .layer_dir(empty.path())
        .unwrap()
        .layer_dir(empty.path().join("missing"))
        .unwrap();
    let (inference, requests) = ScriptedInference::new([text_reply("Hi.")]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_system_prompt("You are a coding agent.")
        .with_skills(skills)
        .build();

    let _: Vec<Event> = agent.prompt("Hello").collect().await;

    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].system, "You are a coding agent.");
    assert!(requests[0].tools.is_empty());
}

#[tokio::test]
async fn instructions_follow_the_system_prompt() {
    let project = TempDir::new().unwrap();
    let file = project.path().join("AGENTS.md");
    std::fs::write(&file, "Always use tabs.\n").unwrap();

    let instructions = Instructions::new().layer_dir(project.path()).unwrap();

    assert_eq!(
        system_prompt_with(instructions).await,
        format!(
            "You are a coding agent.\n\nInstructions from {}:\n\nAlways use tabs.",
            file.display()
        )
    );
}

#[tokio::test]
async fn directory_without_instructions_leaves_the_system_prompt_alone() {
    let project = TempDir::new().unwrap();

    let instructions = Instructions::new().layer_dir(project.path()).unwrap();

    assert_eq!(
        system_prompt_with(instructions).await,
        "You are a coding agent."
    );
}

#[tokio::test]
async fn layers_are_given_in_order_with_the_conflict_rule() {
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let user_file = home.path().join("rules.md");
    let project_file = project.path().join("AGENTS.md");
    std::fs::write(&user_file, "Answer briefly.").unwrap();
    std::fs::write(&project_file, "Always use tabs.").unwrap();

    let instructions = Instructions::new()
        .layer_file(&user_file)
        .unwrap()
        .layer_dir(project.path())
        .unwrap();

    assert_eq!(
        system_prompt_with(instructions).await,
        format!(
            "You are a coding agent.\n\n\
             Instructions follow, from general to specific. \
             Where they conflict, the later one applies.\n\n\
             Instructions from {}:\n\nAnswer briefly.\n\n\
             Instructions from {}:\n\nAlways use tabs.",
            user_file.display(),
            project_file.display()
        )
    );
}

#[tokio::test]
async fn claude_md_is_read_when_there_is_no_agents_md() {
    let project = TempDir::new().unwrap();
    std::fs::write(project.path().join("CLAUDE.md"), "Use spaces.").unwrap();

    let instructions = Instructions::new().layer_dir(project.path()).unwrap();

    assert!(
        system_prompt_with(instructions)
            .await
            .ends_with("Use spaces.")
    );
}

#[tokio::test]
async fn claude_md_is_ignored_when_there_is_an_agents_md() {
    let project = TempDir::new().unwrap();
    std::fs::write(project.path().join("AGENTS.md"), "Use tabs.").unwrap();
    std::fs::write(project.path().join("CLAUDE.md"), "Use spaces.").unwrap();

    let instructions = Instructions::new().layer_dir(project.path()).unwrap();

    let system_prompt = system_prompt_with(instructions).await;
    assert!(system_prompt.ends_with("Use tabs."));
    assert!(!system_prompt.contains("Use spaces."));
}

#[tokio::test]
async fn session_runs_each_prompt_and_shows_its_events() {
    let (inference, requests) = ScriptedInference::new([text_reply("Hi."), text_reply("Fine.")]);
    let (ui, shown) = ScriptedUi::new(["Hello", "How are you?"]);
    let agent = haarniska::builder().with_inference(inference).build();

    agent.run(ui).await;

    assert_eq!(
        *shown.lock().unwrap(),
        [
            Event::Text("Hi.".into()),
            done(),
            Event::Text("Fine.".into()),
            done(),
        ]
    );
    assert_eq!(requests.lock().unwrap()[1].messages.len(), 3);
}

#[tokio::test]
async fn cancel_stops_the_running_turn_and_the_session_goes_on() {
    let call = tool_call("hang", json!({}));
    let (inference, _requests) =
        ScriptedInference::new([tool_reply(call.clone()), text_reply("Hi.")]);
    let (mut ui, shown) = ScriptedUi::new(["Hang", "Hello"]);
    ui.cancel_when_a_tool_starts = true;
    let agent = haarniska::builder()
        .with_inference(inference)
        .with_tool(Hang)
        .build();

    agent.run(ui).await;

    assert_eq!(
        *shown.lock().unwrap(),
        [Event::ToolStarted(call), Event::Text("Hi.".into()), done()]
    );
}

/// A [`Ui`] that sends `prompts` one per turn, then quits, and records
/// every event it is shown.
struct ScriptedUi {
    prompts: VecDeque<String>,
    shown: Shown,
    turn_running: bool,
    cancel_when_a_tool_starts: bool,
    cancel_now: bool,
}

type Shown = Arc<Mutex<Vec<Event>>>;

impl ScriptedUi {
    fn new<const N: usize>(prompts: [&str; N]) -> (Self, Shown) {
        let shown = Shown::default();
        let ui = Self {
            prompts: prompts.into_iter().map(String::from).collect(),
            shown: shown.clone(),
            turn_running: false,
            cancel_when_a_tool_starts: false,
            cancel_now: false,
        };
        (ui, shown)
    }
}

impl Ui for ScriptedUi {
    async fn next(&mut self) -> Input {
        if self.cancel_now {
            self.cancel_now = false;
            self.turn_running = false;
            return Input::Cancel;
        }
        if self.turn_running {
            return std::future::pending().await;
        }
        match self.prompts.pop_front() {
            Some(prompt) => {
                self.turn_running = true;
                Input::Prompt(prompt)
            }
            None => Input::Quit,
        }
    }

    fn show(&mut self, event: Event) {
        match event {
            Event::Done { .. } | Event::Failed(_) => self.turn_running = false,
            Event::ToolStarted(_) => self.cancel_now = self.cancel_when_a_tool_starts,
            _ => {}
        }
        self.shown.lock().unwrap().push(event);
    }
}

/// A tool that never finishes.
struct Hang;

impl Tool for Hang {
    type Input = NoInput;

    const NAME: &str = "hang";
    const DESCRIPTION: &str = "Never finishes.";

    async fn call(&self, _cx: &ToolCx, _input: NoInput) -> Result<String, ToolError> {
        std::future::pending().await
    }
}

/// A tool that always panics.
struct Panic;

#[derive(Deserialize, JsonSchema)]
struct NoInput {}

impl Tool for Panic {
    type Input = NoInput;

    const NAME: &str = "panic";
    const DESCRIPTION: &str = "Panics.";

    async fn call(&self, _cx: &ToolCx, _input: NoInput) -> Result<String, ToolError> {
        panic!("boom")
    }
}

/// An [`Inference`] that plays back `replies` in order and records the
/// messages of every request it receives.
struct ScriptedInference {
    replies: Mutex<VecDeque<Reply>>,
    requests: Requests,
}

type Requests = Arc<Mutex<Vec<Seen>>>;

/// What the model was sent in one call.
struct Seen {
    system: String,
    messages: Vec<Message>,
    tools: Vec<String>,
}

impl ScriptedInference {
    fn new(replies: impl IntoIterator<Item = Reply>) -> (Self, Requests) {
        let requests = Requests::default();
        let inference = Self {
            replies: Mutex::new(replies.into_iter().collect()),
            requests: requests.clone(),
        };
        (inference, requests)
    }
}

impl Inference for ScriptedInference {
    fn infer(&self, request: Request<'_>) -> impl Stream<Item = Result<Chunk, Error>> {
        self.requests.lock().unwrap().push(Seen {
            system: request.system.to_owned(),
            messages: request.messages.to_vec(),
            tools: request.tools.iter().map(|tool| tool.name.clone()).collect(),
        });
        let reply = self.replies.lock().unwrap().pop_front();

        let chunks: Vec<_> = match reply {
            Some(reply) => {
                let texts = reply.content.iter().filter_map(|block| match block {
                    Block::Text(text) => Some(Ok(Chunk::Text(text.clone()))),
                    _ => None,
                });
                texts.chain([Ok(Chunk::Done(reply.clone()))]).collect()
            }
            None => vec![Err(Error::new("no scripted reply left"))],
        };
        stream::iter(chunks)
    }
}

fn text_reply(text: &str) -> Reply {
    Reply {
        content: vec![Block::Text(text.into())],
        stop: StopReason::EndTurn,
        usage: Usage::default(),
    }
}

fn tool_reply(call: ToolCall) -> Reply {
    Reply {
        content: vec![Block::ToolCall(call)],
        stop: StopReason::ToolUse,
        usage: Usage::default(),
    }
}

fn tool_call(name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: "call_1".into(),
        name: name.into(),
        input,
    }
}

/// The result of the only tool call in `events`.
fn tool_result(events: &[Event]) -> &ToolResult {
    let mut results = events.iter().filter_map(|event| match event {
        Event::ToolFinished(result) => Some(result),
        _ => None,
    });
    let result = results.next().expect("a tool call finished");
    assert!(results.next().is_none(), "only one tool call finished");
    result
}

fn done() -> Event {
    Event::Done {
        stop: StopReason::EndTurn,
        usage: Usage::default(),
    }
}

/// The system prompt the model receives from an agent with `instructions`.
async fn system_prompt_with(instructions: Instructions) -> String {
    let (inference, requests) = ScriptedInference::new([text_reply("Hi.")]);
    let mut agent = haarniska::builder()
        .with_inference(inference)
        .with_system_prompt("You are a coding agent.")
        .with_instructions(instructions)
        .build();

    let _: Vec<Event> = agent.prompt("Hello").collect().await;

    requests.lock().unwrap()[0].system.clone()
}

/// Writes the skill `name` into the skills directory `dir`.
fn write_skill(dir: &std::path::Path, name: &str, description: &str, body: &str) {
    let folder = dir.join(name);
    std::fs::create_dir(&folder).unwrap();
    let text = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
    std::fs::write(folder.join("SKILL.md"), text).unwrap();
}

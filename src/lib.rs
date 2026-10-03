//! haarniska: library-first, extensible coding agent harness.

pub mod harness;
pub mod inference;
pub mod tools;
pub mod tui;
pub mod ui;

use std::path::PathBuf;

use futures_util::Stream;
use harness::tool::{DynTool, Tool};
use harness::{Event, Harness};
use inference::Inference;
use ui::Ui;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant for coding tasks";

pub trait Agent {
    /// One turn of the agentic loop.
    fn prompt(&mut self, text: &str) -> impl Stream<Item = Event>;

    /// Interactive session loop driving `ui`.
    fn run(self, ui: impl Ui) -> impl Future<Output = ()>;
}

pub fn builder() -> AgentBuilder {
    AgentBuilder {
        inference: (),
        tools: Vec::new(),
        system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
        cwd: ".".into(),
    }
}

pub struct AgentBuilder<I = ()> {
    inference: I,
    tools: Vec<Box<dyn DynTool>>,
    system_prompt: String,
    cwd: PathBuf,
}

impl AgentBuilder {
    pub fn with_inference<I: Inference>(self, inference: I) -> AgentBuilder<I> {
        AgentBuilder {
            inference,
            tools: self.tools,
            system_prompt: self.system_prompt,
            cwd: self.cwd,
        }
    }
}

impl<I> AgentBuilder<I> {
    pub fn with_tool(mut self, tool: impl Tool) -> Self {
        self.tools.push(Box::new(tool));
        self
    }

    /// Adds the built-in tools: read, write, edit, shell.
    pub fn with_default_tools(self) -> Self {
        self.with_tool(tools::Read)
            .with_tool(tools::Write)
            .with_tool(tools::Edit)
            .with_tool(tools::Shell)
    }

    /// Replaces the default system prompt.
    pub fn with_system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = system_prompt.into();
        self
    }

    /// Where tools resolve relative paths. Defaults to the process's
    /// working directory.
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }
}

impl<I: Inference> AgentBuilder<I> {
    pub fn build(self) -> impl Agent {
        Harness::new(self.inference, self.tools, self.system_prompt, self.cwd)
    }
}

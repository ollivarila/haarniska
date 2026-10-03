//! haarniska: library-first, extensible coding agent harness.

pub mod harness;
pub mod inference;
pub mod tui;
pub mod ui;

use futures_util::Stream;
use harness::tool::{DynTool, Tool};
use harness::{Event, Harness};
use inference::Inference;
use ui::Ui;

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
    }
}

pub struct AgentBuilder<I = ()> {
    inference: I,
    tools: Vec<Box<dyn DynTool>>,
}

impl AgentBuilder {
    pub fn inference<I: Inference>(self, inference: I) -> AgentBuilder<I> {
        AgentBuilder {
            inference,
            tools: self.tools,
        }
    }
}

impl<I> AgentBuilder<I> {
    pub fn tool(mut self, tool: impl Tool) -> Self {
        self.tools.push(Box::new(tool));
        self
    }
}

impl<I: Inference> AgentBuilder<I> {
    pub fn build(self) -> impl Agent {
        Harness {
            inference: self.inference,
            tools: self.tools,
        }
    }
}

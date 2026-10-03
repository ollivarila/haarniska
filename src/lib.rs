//! haarniska: library-first, extensible coding agent harness.

pub mod harness;
pub mod inference;
pub mod tui;
pub mod ui;

use futures_util::Stream;
use harness::plugin::Plugin;
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
        plugins: Vec::new(),
    }
}

pub struct AgentBuilder<I = ()> {
    inference: I,
    plugins: Vec<Box<dyn Plugin>>,
}

impl AgentBuilder {
    pub fn inference<I: Inference>(self, inference: I) -> AgentBuilder<I> {
        AgentBuilder {
            inference,
            plugins: self.plugins,
        }
    }
}

impl<I> AgentBuilder<I> {
    pub fn plugin(mut self, plugin: impl Plugin + 'static) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }
}

impl<I: Inference> AgentBuilder<I> {
    pub fn build(self) -> impl Agent {
        Harness {
            inference: self.inference,
            plugins: self.plugins,
        }
    }
}

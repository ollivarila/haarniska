//! haarniska: library-first, extensible coding agent harness.

pub mod harness;
pub mod provider;
pub mod tui;
pub mod ui;

use harness::Event;
use harness::plugin::Plugin;
use provider::Provider;
use ui::Ui;

pub struct AgentBuilder {}

impl AgentBuilder {
    pub fn provider(self, _provider: impl Provider) -> Self {
        todo!()
    }

    pub fn plugin(self, _plugin: impl Plugin) -> Self {
        todo!()
    }

    pub fn build(self) -> Agent {
        todo!()
    }
}

/// A harness combined with a model provider.
pub struct Agent {}

impl Agent {
    pub fn builder() -> AgentBuilder {
        AgentBuilder {}
    }

    /// One turn of the agentic loop.
    pub fn prompt(&mut self, _text: &str) -> impl Iterator<Item = Event> {
        std::iter::empty()
    }

    /// Interactive session loop driving `ui`.
    pub fn run(self, _ui: impl Ui) {
        todo!()
    }
}

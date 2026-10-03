//! The harness: agent loop and everything around the model.

pub mod plugin;

use crate::Agent;
use crate::inference::Inference;
use crate::ui::Ui;
use plugin::Plugin;

pub enum Event {}

/// Plugins and the loops that run them against a model.
pub(crate) struct Harness<P> {
    pub(crate) inference: P,
    pub(crate) plugins: Vec<Box<dyn Plugin>>,
}

impl<P: Inference> Agent for Harness<P> {
    fn prompt(&mut self, _text: &str) -> impl Iterator<Item = Event> {
        std::iter::empty()
    }

    fn run(self, _ui: impl Ui) {
        todo!()
    }
}

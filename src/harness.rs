//! The harness: agent loop and everything around the model.

pub mod tool;

use crate::Agent;
use crate::inference::Inference;
use crate::ui::Ui;
use futures_util::{Stream, stream};
use tool::DynTool;

pub enum Event {}

/// The loops that run tools against a model.
pub(crate) struct Harness<P> {
    pub(crate) inference: P,
    pub(crate) tools: Vec<Box<dyn DynTool>>,
}

impl<P: Inference> Agent for Harness<P> {
    fn prompt(&mut self, _text: &str) -> impl Stream<Item = Event> {
        stream::empty()
    }

    async fn run(self, _ui: impl Ui) {
        todo!()
    }
}

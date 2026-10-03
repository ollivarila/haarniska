//! Terminal UI.

use crate::harness::Event;
use crate::ui::{Input, Ui};

#[derive(Default)]
pub struct Tui {}

impl Tui {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Ui for Tui {
    async fn next(&mut self) -> Input {
        todo!()
    }

    fn show(&mut self, _event: Event) {
        todo!()
    }
}

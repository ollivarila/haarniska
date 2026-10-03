//! Terminal UI.

use crate::ui::Ui;

#[derive(Default)]
pub struct Tui {}

impl Tui {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Ui for Tui {}

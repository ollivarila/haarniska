//! The interface the harness drives a frontend through.

use crate::harness::Event;

/// A frontend. The harness asks it for input and tells it what happens.
pub trait Ui {
    /// Waits for the user.
    ///
    /// Also called while a turn runs, and dropped each time an event
    /// arrives. Keep state in the UI, not in the returned future.
    fn next(&mut self) -> impl Future<Output = Input>;

    /// Shows what happened in the running turn. Must not block.
    fn show(&mut self, event: Event);
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Input {
    /// Start a turn. Ignored while one is running.
    Prompt(String),
    /// Stop the running turn.
    Cancel,
    /// End the session.
    Quit,
}

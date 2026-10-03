//! Built-in tools: read, write, and edit files; run shell commands.
//!
//! They are written against the public [`Tool`](crate::harness::tool::Tool)
//! API, like any other tool.

mod edit;
mod read;
mod shell;
mod write;

pub use edit::{Edit, EditInput};
pub use read::{Read, ReadInput};
pub use shell::{Shell, ShellInput};
pub use write::{Write, WriteInput};

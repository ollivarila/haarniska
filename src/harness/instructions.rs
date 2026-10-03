//! Project instructions: `AGENTS.md`.

use std::io;
use std::path::{Path, PathBuf};

/// Read from a directory layer; the first that exists is used.
const FILE_NAMES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
const CONFLICT_RULE: &str = "Instructions follow, from general to specific. \
    Where they conflict, the later one applies.";

/// What the user and the project tell the agent to do.
///
/// A stack of layers, from general to specific. Every layer reaches the
/// model; it is told that a later one applies where two conflict.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Instructions {
    layers: Vec<Layer>,
}

impl Instructions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the file at `path` as the next layer. A missing or empty file is
    /// skipped. A leading `~` is not expanded.
    pub fn layer_file(mut self, path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(content) = read(path)? {
            self.push(path, content);
        }
        Ok(self)
    }

    /// Adds `dir`'s `AGENTS.md` as the next layer, or its `CLAUDE.md` if
    /// there is no `AGENTS.md`. A directory with neither is skipped.
    pub fn layer_dir(mut self, dir: impl AsRef<Path>) -> io::Result<Self> {
        for name in FILE_NAMES {
            let path = dir.as_ref().join(name);
            if let Some(content) = read(&path)? {
                self.push(&path, content);
                break;
            }
        }
        Ok(self)
    }

    /// `system_prompt` followed by every layer, each under its path.
    pub(crate) fn append_to(&self, mut system_prompt: String) -> String {
        if self.layers.len() > 1 {
            system_prompt.push_str(&format!("\n\n{CONFLICT_RULE}"));
        }
        for layer in &self.layers {
            let path = layer.path.display();
            let content = layer.content.trim();
            system_prompt.push_str(&format!("\n\nInstructions from {path}:\n\n{content}"));
        }
        system_prompt
    }

    fn push(&mut self, path: &Path, content: String) {
        if !content.trim().is_empty() {
            self.layers.push(Layer {
                path: path.to_owned(),
                content,
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Layer {
    path: PathBuf,
    content: String,
}

/// The file's text, or `None` if there is no such file.
fn read(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

//! Skills: written-down ways of doing recurring tasks, loaded on demand.

use std::io;
use std::path::{Path, PathBuf};

use gray_matter::Matter;
use gray_matter::engine::YAML;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::harness::tool::{Tool, ToolCx, ToolError};

const FILE_NAME: &str = "SKILL.md";
const LISTING_INTRO: &str = "Skills are available for the tasks below. \
    Load one with the skill tool before doing a task it covers.";

/// The skills an agent can load.
///
/// Built from layers of directories, from general to specific. A skill in a
/// later layer replaces one of the same name from an earlier layer.
#[derive(Debug, Clone, Default)]
pub struct Skills {
    skills: Vec<Skill>,
    problems: Vec<SkillProblem>,
}

impl Skills {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the skills in `dir` as the next layer: every subfolder with a
    /// `SKILL.md`. A missing directory is an empty layer. A leading `~` is
    /// not expanded.
    pub fn layer_dir(mut self, dir: impl AsRef<Path>) -> io::Result<Self> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(self),
            Err(error) => return Err(error),
        };
        let mut folders = entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        folders.sort();

        for folder in folders {
            match read_header(&folder) {
                Ok(Some(skill)) => {
                    self.skills.retain(|other| other.name != skill.name);
                    self.skills.push(skill);
                }
                Ok(None) => {}
                Err(reason) => self.problems.push(SkillProblem {
                    path: folder.join(FILE_NAME),
                    reason,
                }),
            }
        }
        Ok(self)
    }

    /// The skills that were skipped, and why.
    pub fn problems(&self) -> &[SkillProblem] {
        &self.problems
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// `system_prompt` followed by the name and description of every skill.
    pub(crate) fn append_to(&self, mut system_prompt: String) -> String {
        if self.is_empty() {
            return system_prompt;
        }
        system_prompt.push_str(&format!("\n\n{LISTING_INTRO}\n"));
        for skill in &self.skills {
            system_prompt.push_str(&format!("\n- {}: {}", skill.name, skill.description));
        }
        system_prompt
    }

    /// The tool the model loads a skill with.
    pub(crate) fn into_tool(self) -> SkillTool {
        SkillTool {
            skills: self.skills,
        }
    }
}

/// A skill that could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}: {reason}", path.display())]
pub struct SkillProblem {
    pub path: PathBuf,
    pub reason: String,
}

/// Gives the model a skill's instructions, by name.
pub(crate) struct SkillTool {
    skills: Vec<Skill>,
}

impl Tool for SkillTool {
    type Input = SkillInput;

    const NAME: &str = "skill";
    const DESCRIPTION: &str = "Load a skill's instructions before doing a task it covers.";

    async fn call(&self, _cx: &ToolCx, input: SkillInput) -> Result<String, ToolError> {
        let Some(skill) = self.skills.iter().find(|skill| skill.name == input.name) else {
            let names: Vec<_> = self
                .skills
                .iter()
                .map(|skill| skill.name.as_str())
                .collect();
            return Err(ToolError::other(format!(
                "no skill named `{}`. Available: {}",
                input.name,
                names.join(", ")
            )));
        };

        // Read now, not at startup, so an edited skill is used as it is.
        let text = tokio::fs::read_to_string(skill.folder.join(FILE_NAME)).await?;
        let body = parse(&text).map_err(ToolError::other)?.1;
        let folder = skill.folder.display();
        Ok(format!(
            "{}\n\nThis skill's files are in {folder}",
            body.trim()
        ))
    }
}

#[derive(Deserialize, JsonSchema)]
pub(crate) struct SkillInput {
    /// Name of the skill, as listed.
    name: String,
}

#[derive(Debug, Clone)]
struct Skill {
    name: String,
    description: String,
    folder: PathBuf,
}

#[derive(Deserialize)]
struct Header {
    name: Option<String>,
    description: Option<String>,
}

/// The skill in `folder`, `None` if it has no `SKILL.md`, or why it cannot
/// be used.
fn read_header(folder: &Path) -> Result<Option<Skill>, String> {
    let text = match std::fs::read_to_string(folder.join(FILE_NAME)) {
        Ok(text) => text,
        Err(error) if is_absent(&error) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let (header, _body) = parse(&text)?;

    // One line each, whatever the YAML layout was.
    let one_line = |field: Option<String>| -> Option<String> {
        let line = field?.split_whitespace().collect::<Vec<_>>().join(" ");
        (!line.is_empty()).then_some(line)
    };
    Ok(Some(Skill {
        name: one_line(header.name).ok_or("the header has no name")?,
        description: one_line(header.description).ok_or("the header has no description")?,
        folder: folder.to_owned(),
    }))
}

/// Splits a `SKILL.md` into its header and body.
fn parse(text: &str) -> Result<(Header, String), String> {
    let parsed = Matter::<YAML>::new()
        .parse::<Header>(text)
        .map_err(|error| format!("the header is not valid: {error}"))?;
    let header = parsed.data.ok_or("there is no header")?;
    Ok((header, parsed.content))
}

/// No such file, or `folder` is a file and not a folder.
fn is_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

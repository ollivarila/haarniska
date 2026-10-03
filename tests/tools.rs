//! The built-in tools, called the way the harness calls them.

use haarniska::harness::tool::{Tool, ToolCx};
use haarniska::tools::{Edit, EditInput, Read, ReadInput, Shell, ShellInput, Write, WriteInput};
use tempfile::TempDir;

fn workspace() -> (TempDir, ToolCx) {
    let dir = TempDir::new().unwrap();
    let cx = ToolCx::new(dir.path());
    (dir, cx)
}

#[tokio::test]
async fn read_returns_the_file_contents() {
    let (dir, cx) = workspace();
    std::fs::write(dir.path().join("a.txt"), "hello").unwrap();

    let output = Read
        .call(
            &cx,
            ReadInput {
                path: "a.txt".into(),
            },
        )
        .await;

    assert_eq!(output.unwrap(), "hello");
}

#[tokio::test]
async fn read_fails_for_a_missing_file() {
    let (_dir, cx) = workspace();

    let output = Read
        .call(
            &cx,
            ReadInput {
                path: "missing.txt".into(),
            },
        )
        .await;

    assert!(output.is_err());
}

#[tokio::test]
async fn write_creates_the_file_and_its_parent_directories() {
    let (dir, cx) = workspace();

    let input = WriteInput {
        path: "nested/dir/a.txt".into(),
        content: "hello".into(),
    };
    Write.call(&cx, input).await.unwrap();

    let written = std::fs::read_to_string(dir.path().join("nested/dir/a.txt")).unwrap();
    assert_eq!(written, "hello");
}

#[tokio::test]
async fn edit_replaces_text_that_occurs_once() {
    let (dir, cx) = workspace();
    std::fs::write(dir.path().join("a.txt"), "one two three").unwrap();

    let input = EditInput {
        path: "a.txt".into(),
        old: "two".into(),
        new: "2".into(),
    };
    Edit.call(&cx, input).await.unwrap();

    let edited = std::fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert_eq!(edited, "one 2 three");
}

#[tokio::test]
async fn edit_fails_and_leaves_the_file_alone_when_the_text_is_missing() {
    let (dir, cx) = workspace();
    std::fs::write(dir.path().join("a.txt"), "one two three").unwrap();

    let input = EditInput {
        path: "a.txt".into(),
        old: "four".into(),
        new: "4".into(),
    };
    let output = Edit.call(&cx, input).await;

    assert!(output.is_err());
    let content = std::fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert_eq!(content, "one two three");
}

#[tokio::test]
async fn edit_fails_and_leaves_the_file_alone_when_the_text_is_ambiguous() {
    let (dir, cx) = workspace();
    std::fs::write(dir.path().join("a.txt"), "two two").unwrap();

    let input = EditInput {
        path: "a.txt".into(),
        old: "two".into(),
        new: "2".into(),
    };
    let output = Edit.call(&cx, input).await;

    assert!(output.is_err());
    let content = std::fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert_eq!(content, "two two");
}

#[tokio::test]
async fn shell_runs_in_the_working_directory_and_returns_output() {
    let (dir, cx) = workspace();
    std::fs::write(dir.path().join("a.txt"), "").unwrap();

    let output = Shell
        .call(
            &cx,
            ShellInput {
                command: "ls".into(),
            },
        )
        .await;

    assert_eq!(output.unwrap(), "a.txt\n");
}

#[tokio::test]
async fn shell_reports_output_and_exit_code_of_a_failing_command() {
    let (_dir, cx) = workspace();

    let input = ShellInput {
        command: "echo oops >&2; exit 3".into(),
    };
    let output = Shell.call(&cx, input).await;

    assert_eq!(output.unwrap(), "oops\n\n[exit code: 3]");
}

//! Terminal UI.

mod markdown;

use std::io;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::harness::Event;
use crate::harness::hook::{Answer, ApprovalRequest, Approver};
use crate::inference::{StopReason, Usage};
use crate::ui::{Input, Ui};

const RENDER_THREAD: &str = "haarniska-tui";
/// How long the render thread waits for a key before looking for news
/// from the harness.
const TICK: Duration = Duration::from_millis(50);
/// Lines of a tool result shown in the transcript.
const RESULT_LINES: usize = 5;
/// Lines the transcript moves per notch of the mouse wheel.
const WHEEL_LINES: usize = 3;
/// Shown before a tool call, and before its result and the lines after
/// the result's first.
const TOOL_MARKER: &str = "● ";
const RESULT_MARKER: &str = "  ⎿ ";
const RESULT_INDENT: &str = "    ";
/// Stands for what was left out.
const ELLIPSIS: &str = "…";
/// Shown before the text being typed.
const INPUT_MARKER: &str = "> ";
/// Lines of a consent question shown at once.
const QUESTION_LINES: usize = 6;

/// A full-screen terminal UI. Keys are read and the screen is drawn on its
/// own thread, so a tool that blocks cannot freeze it.
pub struct Tui {
    messages: mpsc::Sender<Message>,
    inputs: UnboundedReceiver<Input>,
    /// When the program started, until the ready time has been reported.
    started: Option<Instant>,
    render_thread: Option<JoinHandle<()>>,
    hook: Arc<PanicHookState>,
}

impl Tui {
    /// Takes over the terminal. `started` is when the program started. Two
    /// startup times are shown, both counted from then: to the first frame,
    /// and to the agent being ready for input.
    pub fn new(started: Instant) -> io::Result<Self> {
        let terminal = ratatui::try_init()?;
        // Needed to see the mouse wheel.
        execute!(io::stdout(), EnableMouseCapture)?;
        let hook = install_panic_hook();
        let (messages_tx, messages_rx) = mpsc::channel();
        let (inputs_tx, inputs_rx) = unbounded_channel();

        let render_thread = thread::Builder::new()
            .name(RENDER_THREAD.into())
            .spawn(move || {
                render(terminal, App::new(started), messages_rx, inputs_tx);
                restore();
            })?;

        Ok(Self {
            messages: messages_tx,
            inputs: inputs_rx,
            started: Some(started),
            render_thread: Some(render_thread),
            hook,
        })
    }

    /// Asks the user, on this UI, when a hook wants their consent. Pass it
    /// to the agent builder.
    pub fn approver(&self) -> TuiApprover {
        TuiApprover {
            messages: self.messages.clone(),
        }
    }
}

impl Ui for Tui {
    async fn next(&mut self) -> Input {
        // The first time the harness waits for input, the agent is ready.
        if let Some(started) = self.started.take() {
            let _ = self.messages.send(Message::Ready(started.elapsed()));
        }
        // A closed channel means the render thread is gone.
        self.inputs.recv().await.unwrap_or(Input::Quit)
    }

    fn show(&mut self, event: Event) {
        let _ = self.messages.send(Message::Event(event));
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // The render thread restores the terminal on its way out.
        let _ = self.messages.send(Message::Stop);
        if let Some(render_thread) = self.render_thread.take() {
            let _ = render_thread.join();
        }

        self.hook.active.store(false, Ordering::Relaxed);
        if thread::panicking()
            && let Some(message) = self.hook.last_panic.lock().unwrap().take()
        {
            eprintln!("{message}");
        }
    }
}

/// Puts consent questions to the user of a [`Tui`].
#[derive(Clone)]
pub struct TuiApprover {
    messages: mpsc::Sender<Message>,
}

impl Approver for TuiApprover {
    async fn approve(&self, request: ApprovalRequest) -> Answer {
        let (answer, answered) = oneshot::channel();
        let _ = self
            .messages
            .send(Message::Question(Question { request, answer }));
        // No answer means the UI is gone.
        answered.await.unwrap_or(Answer::Deny)
    }
}

/// What the render thread is told.
enum Message {
    Event(Event),
    Question(Question),
    /// The agent took this long to be ready for input.
    Ready(Duration),
    Stop,
}

/// A consent question waiting for a key.
struct Question {
    request: ApprovalRequest,
    answer: oneshot::Sender<Answer>,
}

struct PanicHookState {
    /// False once the UI is gone and panics can print normally again.
    active: AtomicBool,
    last_panic: Mutex<Option<String>>,
}

/// Keeps panic messages off the screen while the UI is up.
///
/// A panic hook runs before the panic is caught, so a crashing tool would
/// otherwise print over the UI even though the harness recovers from it.
/// Panics on the render thread still restore the terminal and print.
fn install_panic_hook() -> Arc<PanicHookState> {
    let state = Arc::new(PanicHookState {
        active: AtomicBool::new(true),
        last_panic: Mutex::new(None),
    });

    // Installed by `ratatui::try_init`: restores the terminal, then prints.
    let restore_and_print = panic::take_hook();
    let hook_state = state.clone();
    panic::set_hook(Box::new(move |info| {
        let on_render_thread = thread::current().name() == Some(RENDER_THREAD);
        if on_render_thread || !hook_state.active.load(Ordering::Relaxed) {
            let _ = execute!(io::stdout(), DisableMouseCapture);
            restore_and_print(info);
        } else {
            *hook_state.last_panic.lock().unwrap() = Some(info.to_string());
        }
    }));

    state
}

/// Gives the terminal back as it was.
fn restore() {
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

/// The render thread: applies events, draws, and turns keys into input,
/// until the user quits or the [`Tui`] is dropped.
fn render(
    mut terminal: DefaultTerminal,
    mut app: App,
    messages: mpsc::Receiver<Message>,
    inputs: UnboundedSender<Input>,
) {
    // Whether the screen is out of date.
    let mut dirty = true;
    loop {
        loop {
            match messages.try_recv() {
                Ok(Message::Event(event)) => app.apply(event),
                Ok(Message::Question(question)) => app.question = Some(question),
                Ok(Message::Ready(after)) => app.ready = Some(after),
                Ok(Message::Stop) | Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => break,
            }
            dirty = true;
        }
        // A question nobody waits for any more, as after a cancelled turn.
        let abandoned = app.question.take_if(|question| question.answer.is_closed());
        dirty |= abandoned.is_some();

        if dirty && terminal.draw(|frame| app.draw(frame)).is_err() {
            return;
        }
        dirty = false;

        // Everything waiting is handled before the next draw, so a paste or
        // a moving mouse costs one frame, not one per event.
        let mut wait = TICK;
        loop {
            match event::poll(wait) {
                Ok(true) => wait = Duration::ZERO,
                Ok(false) => break,
                Err(_) => return,
            }
            match event::read() {
                Ok(event::Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    dirty = true;
                    if let Some(input) = app.key(key) {
                        let quit = input == Input::Quit;
                        if inputs.send(input).is_err() || quit {
                            return;
                        }
                    }
                }
                Ok(event::Event::Mouse(mouse)) => dirty |= app.mouse(mouse),
                Ok(event::Event::Resize(..)) => dirty = true,
                Ok(_) => {}
                Err(_) => return,
            }
        }
    }
}

/// Everything on screen.
struct App {
    transcript: Vec<Entry>,
    input: String,
    turn_running: bool,
    usage: Usage,
    started: Instant,
    /// Time to the first frame, and to the agent being ready for input.
    first_paint: Option<Duration>,
    ready: Option<Duration>,
    /// The transcript line at the top of the screen, or `None` to follow
    /// the newest output.
    top: Option<usize>,
    /// From the last draw: lines on a screen, and the `top` that shows the
    /// end of the transcript.
    page: usize,
    max_top: usize,
    /// Shown in place of the input line until answered.
    question: Option<Question>,
}

struct Entry {
    kind: EntryKind,
    text: String,
    /// The entry as screen lines at a width, until its text changes.
    rows: Option<(usize, Vec<Line<'static>>)>,
}

impl Entry {
    /// The entry as screen lines no wider than `width`, with the blank
    /// line that ends it.
    fn rows(&mut self, width: usize) -> &[Line<'static>] {
        let rows = match self.rows.take() {
            Some((at, rows)) if at == width => rows,
            _ => self.render(width),
        };
        &self.rows.insert((width, rows)).1
    }

    fn render(&self, width: usize) -> Vec<Line<'static>> {
        // The first line starts with `prefix`, the others with `indent`.
        let (prefix, indent, style, breaks) = match self.kind {
            EntryKind::Reply => {
                let mut rows = markdown::lines(&self.text, width);
                rows.push(Line::default());
                return rows;
            }
            // No blank line after it: its result belongs to it.
            EntryKind::Tool => return vec![tool_line(&self.text, width)],
            EntryKind::User => {
                let style = Style::new().fg(Color::Cyan).bold();
                (INPUT_MARKER, "  ", style, Break::AtSpaces)
            }
            EntryKind::Output => (
                RESULT_MARKER,
                RESULT_INDENT,
                Style::new().dim(),
                Break::Anywhere,
            ),
            EntryKind::Failure => {
                let style = Style::new().fg(Color::Red);
                (RESULT_MARKER, RESULT_INDENT, style, Break::Anywhere)
            }
            EntryKind::Error => ("  ", "  ", Style::new().fg(Color::Red), Break::AtSpaces),
            EntryKind::Note => ("  ", "  ", Style::new().dim(), Break::AtSpaces),
        };
        let mut rows = Vec::new();
        for (index, line) in self.text.lines().enumerate() {
            let start = if index == 0 { prefix } else { indent };
            let line = Line::styled(format!("{start}{line}"), style);
            rows.extend(wrap(&line, width, breaks));
        }
        rows.push(Line::default());
        rows
    }
}

/// A tool call on one line: a marker, the tool's name, and as much of its
/// input as fits.
fn tool_line(text: &str, width: usize) -> Line<'static> {
    let (name, input) = text.split_once(' ').unwrap_or((text, ""));
    let room = width.saturating_sub(TOOL_MARKER.chars().count() + name.chars().count() + 1);
    let first_line = input.lines().next().unwrap_or_default();
    let mut shown: String = first_line.chars().take(room).collect();
    if shown != input {
        // The marker takes the last column if there is none left.
        if shown.chars().count() == room {
            shown.pop();
        }
        shown.push_str(ELLIPSIS);
    }
    Line::from(vec![
        TOOL_MARKER.yellow(),
        name.to_owned().bold(),
        " ".into(),
        shown.dim(),
    ])
}

#[derive(Clone, Copy, PartialEq)]
enum EntryKind {
    User,
    Reply,
    /// A tool call, by name and input.
    Tool,
    /// What a tool call gave back, and the same for one that failed.
    Output,
    Failure,
    /// A turn that failed.
    Error,
    Note,
}

impl App {
    fn new(started: Instant) -> Self {
        Self {
            transcript: Vec::new(),
            input: String::new(),
            turn_running: false,
            usage: Usage::default(),
            started,
            first_paint: None,
            ready: None,
            top: None,
            page: 1,
            max_top: 0,
            question: None,
        }
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Text(text) => match self.transcript.last_mut() {
                Some(entry) if entry.kind == EntryKind::Reply => {
                    entry.text.push_str(&text);
                    entry.rows = None;
                }
                _ => self.push(EntryKind::Reply, text),
            },
            Event::ToolStarted(call) => {
                let text = format!("{} {}", call.name, describe(&call.input));
                self.push(EntryKind::Tool, text);
            }
            Event::ToolFinished(result) => {
                let kind = if result.is_error {
                    EntryKind::Failure
                } else {
                    EntryKind::Output
                };
                self.push(kind, first_lines(&result.output, RESULT_LINES));
            }
            Event::Done { stop, usage } => {
                self.turn_running = false;
                self.usage += usage;
                match stop {
                    StopReason::MaxTokens => self.push(EntryKind::Note, "(reply cut off)".into()),
                    StopReason::Refusal => self.push(EntryKind::Note, "(refused)".into()),
                    StopReason::EndTurn | StopReason::ToolUse => {}
                }
            }
            Event::Failed(error) => {
                self.turn_running = false;
                self.push(EntryKind::Error, error.to_string());
            }
        }
    }

    fn key(&mut self, key: KeyEvent) -> Option<Input> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c' | 'd') if ctrl => Some(Input::Quit),
            KeyCode::Esc if self.turn_running => {
                self.turn_running = false;
                self.question = None;
                self.push(EntryKind::Note, "(cancelled)".into());
                Some(Input::Cancel)
            }
            // Nothing is typed while a question is open.
            // Nor is Enter an answer: it is too easy to press out of habit.
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Enter if self.question.is_some() => {
                match key.code {
                    KeyCode::Char('y') => self.answer(Answer::Allow),
                    KeyCode::Char('n') => self.answer(Answer::Deny),
                    KeyCode::Char('a') => self.answer(Answer::AllowAlways),
                    _ => {}
                }
                None
            }
            KeyCode::Enter if !self.turn_running && !self.input.trim().is_empty() => {
                let prompt = std::mem::take(&mut self.input);
                self.push(EntryKind::User, prompt.clone());
                self.turn_running = true;
                self.top = None;
                Some(Input::Prompt(prompt))
            }
            KeyCode::PageUp => {
                self.scroll_up(self.page);
                None
            }
            KeyCode::PageDown => {
                self.scroll_down(self.page);
                None
            }
            KeyCode::End => {
                self.top = None;
                None
            }
            KeyCode::Char(c) if !ctrl => {
                self.input.push(c);
                None
            }
            KeyCode::Backspace => {
                self.input.pop();
                None
            }
            _ => None,
        }
    }

    fn answer(&mut self, answer: Answer) {
        if let Some(question) = self.question.take() {
            let _ = question.answer.send(answer);
        }
    }

    /// Whether the mouse changed what is on screen.
    fn mouse(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_up(WHEEL_LINES),
            MouseEventKind::ScrollDown => self.scroll_down(WHEEL_LINES),
            _ => return false,
        }
        true
    }

    fn scroll_up(&mut self, lines: usize) {
        let top = self.top.unwrap_or(self.max_top);
        self.top = Some(top.saturating_sub(lines));
    }

    /// Reaching the end goes back to following.
    fn scroll_down(&mut self, lines: usize) {
        self.top = self
            .top
            .map(|top| top + lines)
            .filter(|top| *top < self.max_top);
    }

    fn draw(&mut self, frame: &mut Frame) {
        let first_paint = *self
            .first_paint
            .get_or_insert_with(|| self.started.elapsed());
        let question_lines = self
            .question
            .as_ref()
            .map(|question| question_lines(&question.request, frame.area().width));
        let input_height = question_lines.as_ref().map_or(1, Vec::len) as u16 + 2;
        let [transcript_area, input_area, status_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(input_height),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        self.draw_transcript(frame, transcript_area);
        match (&self.question, question_lines) {
            (Some(question), Some(lines)) => {
                draw_question(frame, input_area, &question.request.tool, lines);
            }
            _ => self.draw_input(frame, input_area),
        }
        frame.render_widget(Paragraph::new(self.status_line(first_paint)), status_area);
    }

    /// One screen of the transcript: from `top` when scrolled back,
    /// otherwise the newest lines, so it follows the reply.
    fn draw_transcript(&mut self, frame: &mut Frame, area: Rect) {
        let padding = Padding::left(1);
        let width = area.width.saturating_sub(padding.left) as usize;
        let total: usize = self
            .transcript
            .iter_mut()
            .map(|entry| entry.rows(width).len())
            .sum();
        self.page = (area.height as usize).max(1);
        self.max_top = total.saturating_sub(self.page);
        self.top = self.top.filter(|top| *top < self.max_top);
        let top = self.top.unwrap_or(self.max_top);
        let visible: Vec<_> = self
            .transcript
            .iter_mut()
            .flat_map(|entry| entry.rows(width))
            .skip(top)
            .take(self.page)
            .cloned()
            .collect();
        let block = Block::new().padding(padding);
        frame.render_widget(Paragraph::new(visible).block(block), area);
    }

    fn draw_input(&self, frame: &mut Frame, area: Rect) {
        // The end of the input that fits, so the cursor stays visible.
        // One column is kept free for the cursor.
        let marker_width = INPUT_MARKER.chars().count() as u16;
        let width = area.width.saturating_sub(marker_width + 1) as usize;
        let hidden = self.input.chars().count().saturating_sub(width);
        let visible: String = self.input.chars().skip(hidden).collect();
        let cursor_x = area.x + marker_width + visible.chars().count() as u16;
        let line = Line::from(vec![INPUT_MARKER.cyan().bold(), visible.into()]);
        let block = Block::new()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::new().fg(Color::DarkGray));
        frame.render_widget(Paragraph::new(line).block(block), area);
        frame.set_cursor_position((cursor_x, area.y + 1));
    }

    fn status_line(&self, first_paint: Duration) -> Line<'static> {
        let state = if self.question.is_some() {
            "waiting for you".magenta().bold()
        } else if self.turn_running {
            "working".yellow().bold()
        } else {
            "ready".green().bold()
        };
        let keys = if self.top.is_some() {
            "scrolled back, End to follow".yellow()
        } else {
            "Enter send, Esc cancel, PgUp/PgDn or wheel scroll, Ctrl-C quit".dim()
        };
        let separator = || " | ".dim();
        Line::from(vec![
            " ".into(),
            state,
            separator(),
            "paint ".dim(),
            format!("{} ms", first_paint.as_millis()).cyan(),
            ", ready ".dim(),
            match self.ready {
                Some(ready) => format!("{} ms", ready.as_millis()).cyan(),
                None => "...".dim(),
            },
            separator(),
            "tokens ".dim(),
            self.usage.input_tokens.to_string().cyan(),
            " in, ".dim(),
            self.usage.output_tokens.to_string().cyan(),
            " out".dim(),
            separator(),
            keys,
        ])
    }

    fn push(&mut self, kind: EntryKind, text: String) {
        self.transcript.push(Entry {
            kind,
            text,
            rows: None,
        });
    }
}

/// A consent question about `tool`, in place of the input line.
fn draw_question(frame: &mut Frame, area: Rect, tool: &str, lines: Vec<Line<'static>>) {
    let keys = Line::from(vec![
        " ".into(),
        "y".green().bold(),
        " allow, ".into(),
        "n".red().bold(),
        " deny, ".into(),
        "a".cyan().bold(),
        format!(" always allow {tool} ").into(),
    ]);
    let block = Block::new()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::new().fg(Color::Yellow))
        .title(format!(" Allow {tool}? ").yellow().bold())
        .title_bottom(keys);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// What a consent question shows between its lines: the call, then each reason.
fn question_lines(request: &ApprovalRequest, screen_width: u16) -> Vec<Line<'static>> {
    let width = screen_width as usize;
    let input = describe(&request.input);

    let mut lines: Vec<_> =
        wrap_text(&input, Style::new().bold(), width, Break::Anywhere).collect();
    for reason in &request.reasons {
        lines.extend(wrap_text(
            reason,
            Style::new().dim(),
            width,
            Break::AtSpaces,
        ));
    }
    if lines.len() > QUESTION_LINES {
        lines.truncate(QUESTION_LINES - 1);
        lines.push(Line::from("...").dim());
    }
    lines
}

/// Where a line too long for the screen may be broken.
#[derive(Clone, Copy, PartialEq)]
enum Break {
    /// At a space where there is one: for text.
    AtSpaces,
    /// At the width, so the layout holds: for code and commands.
    Anywhere,
}

/// `text` in one style, as screen lines no wider than `width`.
fn wrap_text(
    text: &str,
    style: Style,
    width: usize,
    breaks: Break,
) -> impl Iterator<Item = Line<'static>> {
    text.lines()
        .flat_map(move |line| wrap(&Line::styled(line, style), width, breaks))
}

/// `line` as screen lines no wider than `width`. Styles are kept. Counts
/// characters, so wide ones may overflow.
fn wrap(line: &Line, width: usize, breaks: Break) -> Vec<Line<'static>> {
    let width = width.max(1);
    let chars: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|span| span.content.chars().map(|c| (c, span.style)))
        .collect();

    let mut rows = Vec::new();
    let mut rest = chars.as_slice();
    while rest.len() > width {
        // The last space that still leaves the row within the width.
        let space = rest[..=width].iter().rposition(|(c, _)| *c == ' ');
        let (row, skip) = match space {
            Some(at) if breaks == Break::AtSpaces && at > 0 => (&rest[..at], 1),
            _ => (&rest[..width], 0),
        };
        rows.push(row);
        rest = &rest[row.len() + skip..];
    }
    rows.push(rest);

    rows.into_iter()
        .map(|row| {
            let spans: Vec<Span> = row
                .chunk_by(|(_, a), (_, b)| a == b)
                .map(|run| {
                    let text: String = run.iter().map(|(c, _)| c).collect();
                    Span::styled(text, run[0].1)
                })
                .collect();
            Line {
                spans,
                style: line.style,
                alignment: line.alignment,
            }
        })
        .collect()
}

/// A tool's input for reading: a lone text field, like a shell command,
/// bare; otherwise each field by name.
fn describe(input: &serde_json::Value) -> String {
    let Some(fields) = input.as_object() else {
        return input.to_string();
    };
    let bare = |value: &serde_json::Value| match value.as_str() {
        Some(text) => text.to_owned(),
        None => value.to_string(),
    };
    match fields.values().next() {
        Some(value) if fields.len() == 1 => bare(value),
        _ => {
            let fields: Vec<_> = fields
                .iter()
                .map(|(name, value)| format!("{name}: {}", bare(value)))
                .collect();
            fields.join(", ")
        }
    }
}

/// The first `count` lines of `text`, saying how many more were left out.
/// A text with nothing in it says so.
fn first_lines(text: &str, count: usize) -> String {
    if text.trim().is_empty() {
        return "(no output)".into();
    }
    let mut lines: Vec<String> = text.lines().take(count).map(str::to_owned).collect();
    let left_out = text.lines().count().saturating_sub(count);
    if left_out > 0 {
        lines.push(format!("{ELLIPSIS} +{left_out} lines"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::inference::{ToolCall, ToolResult};

    /// Ends a line in markdown; a newline alone does not.
    const LINE_BREAK: &str = "  \n";

    /// An app showing a reply of the lines "1" to `count`, on a screen with
    /// room for five transcript lines.
    fn app_with_reply(count: usize) -> (App, Terminal<TestBackend>) {
        let mut app = App::new(Instant::now());
        let reply: Vec<String> = (1..=count).map(|n| n.to_string()).collect();
        app.apply(Event::Text(reply.join(LINE_BREAK)));
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();
        transcript(&mut app, &mut terminal);
        (app, terminal)
    }

    /// Draws and returns the transcript lines on screen.
    fn transcript(app: &mut App, terminal: &mut Terminal<TestBackend>) -> Vec<String> {
        terminal.draw(|frame| app.draw(frame)).unwrap();
        rows(terminal, 0..5)
            .iter()
            .map(|row| row.trim().to_owned())
            .collect()
    }

    /// The screen rows `ys`, as text.
    fn rows(terminal: &Terminal<TestBackend>, ys: std::ops::Range<u16>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        ys.map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::from(code));
    }

    /// Opens a question about `git push`; the receiver gets the answer.
    fn ask(app: &mut App) -> oneshot::Receiver<Answer> {
        let (answer, answered) = oneshot::channel();
        app.question = Some(Question {
            request: ApprovalRequest {
                tool: "shell".into(),
                input: serde_json::json!({"command": "git push"}),
                reasons: vec!["pushes to the remote".into()],
            },
            answer,
        });
        answered
    }

    #[test]
    fn question_takes_the_place_of_the_input_line() {
        let (mut app, mut terminal) = app_with_reply(1);
        let _answered = ask(&mut app);

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let rows = rows(&terminal, 4..8);
        assert!(rows[0].contains(" Allow shell? "));
        assert!(rows[1].contains("git push"));
        assert!(rows[2].contains("pushes to the remote"));
        assert!(rows[3].contains(" y allow, n deny, a always allow shell "));
    }

    #[test]
    fn keys_answer_the_question() {
        for (key, answer) in [
            ('y', Answer::Allow),
            ('n', Answer::Deny),
            ('a', Answer::AllowAlways),
        ] {
            let mut app = App::new(Instant::now());
            let mut answered = ask(&mut app);

            press(&mut app, KeyCode::Char(key));

            assert_eq!(answered.try_recv(), Ok(answer));
            assert!(app.question.is_none());
        }
    }

    #[test]
    fn enter_does_not_answer_the_question() {
        let mut app = App::new(Instant::now());
        let mut answered = ask(&mut app);

        press(&mut app, KeyCode::Enter);

        assert!(answered.try_recv().is_err());
        assert!(app.question.is_some());
    }

    #[test]
    fn nothing_is_typed_while_a_question_is_open() {
        let mut app = App::new(Instant::now());
        let mut answered = ask(&mut app);

        press(&mut app, KeyCode::Char('x'));

        assert_eq!(app.input, "");
        assert!(answered.try_recv().is_err());
    }

    #[test]
    fn tool_call_is_shown_on_one_line_above_its_result() {
        let mut app = App::new(Instant::now());
        let mut terminal = Terminal::new(TestBackend::new(30, 9)).unwrap();

        app.apply(Event::ToolStarted(ToolCall {
            id: "call_1".into(),
            name: "shell".into(),
            input: serde_json::json!({"command": "cargo test --all-targets --quiet"}),
        }));
        app.apply(Event::ToolFinished(ToolResult {
            call_id: "call_1".into(),
            output: "ok\nexit code 0".into(),
            is_error: false,
        }));

        assert_eq!(
            transcript(&mut app, &mut terminal),
            [
                "● shell cargo test --all-tar…",
                "⎿ ok",
                "exit code 0",
                "",
                ""
            ]
        );
    }

    #[test]
    fn long_tool_result_says_how_much_was_left_out() {
        let mut app = App::new(Instant::now());
        let mut terminal = Terminal::new(TestBackend::new(30, 12)).unwrap();
        let output: Vec<String> = (1..=8).map(|n| n.to_string()).collect();

        app.apply(Event::ToolFinished(ToolResult {
            call_id: "call_1".into(),
            output: output.join("\n"),
            is_error: false,
        }));

        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rows = rows(&terminal, 4..6);
        assert_eq!(rows[0].trim(), "5");
        assert_eq!(rows[1].trim(), "… +3 lines");
    }

    #[test]
    fn reply_is_shown_as_markdown() {
        let mut app = App::new(Instant::now());
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();

        app.apply(Event::Text("# Title\nUse **bold** and `code`.".into()));

        assert_eq!(
            transcript(&mut app, &mut terminal),
            ["Title", "", "Use bold and code.", "", ""]
        );
    }

    #[test]
    fn code_block_is_shown_without_fences_and_keeps_its_layout() {
        let mut app = App::new(Instant::now());
        let mut terminal = Terminal::new(TestBackend::new(12, 9)).unwrap();

        app.apply(Event::Text("```rust\nlet a = 1 + 22;\n  b();\n```".into()));

        // After the one column of padding: cut at the width, indent kept.
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let rows = rows(&terminal, 0..3);
        assert_eq!(rows, [" let a = 1 +", "  22;       ", "   b();     "]);
    }

    #[test]
    fn typescript_is_highlighted() {
        let mut app = App::new(Instant::now());
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();

        app.apply(Event::Text("```typescript\nconst a = 1;\n```".into()));

        assert_eq!(transcript(&mut app, &mut terminal)[0], "const a = 1;");
        let colour = terminal.backend().buffer()[(1, 0)].fg;
        assert!(matches!(colour, Color::Rgb(..)), "{colour:?}");
    }

    #[test]
    fn long_reply_lines_break_at_spaces() {
        let mut app = App::new(Instant::now());
        let mut terminal = Terminal::new(TestBackend::new(12, 9)).unwrap();

        app.apply(Event::Text("alpha beta gamma delta".into()));

        let rows = transcript(&mut app, &mut terminal);
        assert_eq!(rows[..2], ["alpha beta", "gamma delta"]);
    }

    #[test]
    fn transcript_follows_the_newest_output() {
        let (mut app, mut terminal) = app_with_reply(20);

        app.apply(Event::Text(format!("{LINE_BREAK}21")));

        // The last line on screen is the blank one that ends an entry.
        assert_eq!(
            transcript(&mut app, &mut terminal),
            ["18", "19", "20", "21", ""]
        );
    }

    #[test]
    fn page_up_shows_earlier_output_and_stays_there() {
        let (mut app, mut terminal) = app_with_reply(20);

        press(&mut app, KeyCode::PageUp);
        app.apply(Event::Text(format!("{LINE_BREAK}21")));

        assert_eq!(
            transcript(&mut app, &mut terminal),
            ["12", "13", "14", "15", "16"]
        );
    }

    #[test]
    fn mouse_wheel_scrolls_a_few_lines() {
        let (mut app, mut terminal) = app_with_reply(20);

        app.mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(
            transcript(&mut app, &mut terminal),
            ["14", "15", "16", "17", "18"]
        );
    }

    #[test]
    fn page_up_stops_at_the_start() {
        let (mut app, mut terminal) = app_with_reply(20);

        for _ in 0..10 {
            press(&mut app, KeyCode::PageUp);
        }

        assert_eq!(
            transcript(&mut app, &mut terminal),
            ["1", "2", "3", "4", "5"]
        );
    }

    #[test]
    fn paging_down_to_the_end_follows_again() {
        let (mut app, mut terminal) = app_with_reply(20);
        press(&mut app, KeyCode::PageUp);
        transcript(&mut app, &mut terminal);

        press(&mut app, KeyCode::PageDown);
        app.apply(Event::Text(format!("{LINE_BREAK}21")));

        assert_eq!(
            transcript(&mut app, &mut terminal),
            ["18", "19", "20", "21", ""]
        );
    }

    #[test]
    fn end_follows_again() {
        let (mut app, mut terminal) = app_with_reply(20);
        press(&mut app, KeyCode::PageUp);
        press(&mut app, KeyCode::PageUp);

        press(&mut app, KeyCode::End);

        assert_eq!(
            transcript(&mut app, &mut terminal),
            ["17", "18", "19", "20", ""]
        );
    }
}

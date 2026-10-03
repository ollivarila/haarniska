//! Terminal UI.

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
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::harness::Event;
use crate::harness::hook::{Answer, ApprovalRequest, Approver};
use crate::inference::StopReason;
use crate::ui::{Input, Ui};

const RENDER_THREAD: &str = "haarniska-tui";
/// How long the render thread waits for a key before redrawing.
const TICK: Duration = Duration::from_millis(50);
/// Lines of a tool result shown in the transcript.
const RESULT_LINES: usize = 8;
/// Lines the transcript moves per notch of the mouse wheel.
const WHEEL_LINES: usize = 3;
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
    loop {
        loop {
            match messages.try_recv() {
                Ok(Message::Event(event)) => app.apply(event),
                Ok(Message::Question(question)) => app.question = Some(question),
                Ok(Message::Ready(after)) => app.ready = Some(after),
                Ok(Message::Stop) | Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        // A question nobody waits for any more, as after a cancelled turn.
        app.question.take_if(|question| question.answer.is_closed());

        if terminal.draw(|frame| app.draw(frame)).is_err() {
            return;
        }

        let key = match event::poll(TICK) {
            Ok(true) => match event::read() {
                Ok(event::Event::Key(key)) if key.kind == KeyEventKind::Press => key,
                Ok(event::Event::Mouse(mouse)) => {
                    app.mouse(mouse);
                    continue;
                }
                Ok(_) => continue,
                Err(_) => return,
            },
            Ok(false) => continue,
            Err(_) => return,
        };
        if let Some(input) = app.key(key) {
            let quit = input == Input::Quit;
            if inputs.send(input).is_err() || quit {
                return;
            }
        }
    }
}

/// Everything on screen.
struct App {
    transcript: Vec<Entry>,
    input: String,
    turn_running: bool,
    input_tokens: u64,
    output_tokens: u64,
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
}

#[derive(Clone, Copy, PartialEq)]
enum EntryKind {
    User,
    Reply,
    Tool,
    Error,
    Note,
}

impl App {
    fn new(started: Instant) -> Self {
        Self {
            transcript: Vec::new(),
            input: String::new(),
            turn_running: false,
            input_tokens: 0,
            output_tokens: 0,
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
                Some(entry) if entry.kind == EntryKind::Reply => entry.text.push_str(&text),
                _ => self.push(EntryKind::Reply, text),
            },
            Event::ToolStarted(call) => {
                self.push(EntryKind::Tool, format!("{} {}", call.name, call.input));
            }
            Event::ToolFinished(result) => {
                let kind = if result.is_error {
                    EntryKind::Error
                } else {
                    EntryKind::Note
                };
                self.push(kind, first_lines(&result.output, RESULT_LINES));
            }
            Event::Done { stop, usage } => {
                self.turn_running = false;
                self.input_tokens += usage.input_tokens;
                self.output_tokens += usage.output_tokens;
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
            KeyCode::Char('y') if self.question.is_some() => self.answer(Answer::Allow),
            KeyCode::Char('n') if self.question.is_some() => self.answer(Answer::Deny),
            KeyCode::Char('a') if self.question.is_some() => self.answer(Answer::AllowAlways),
            // Nothing is typed while a question is open.
            // Nor is Enter an answer: it is too easy to press out of habit.
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Enter if self.question.is_some() => {
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

    fn answer(&mut self, answer: Answer) -> Option<Input> {
        if let Some(question) = self.question.take() {
            let _ = question.answer.send(answer);
        }
        None
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_up(WHEEL_LINES),
            MouseEventKind::ScrollDown => self.scroll_down(WHEEL_LINES),
            _ => {}
        }
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
        let question = self.question.as_ref().map(|question| &question.request);
        let question_lines = question.map(|request| question_lines(request, frame.area().width));
        let input_height = question_lines.as_ref().map_or(1, Vec::len) as u16 + 2;
        let [transcript_area, input_area, status_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(input_height),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        // One screen of the transcript: from `top` when scrolled back,
        // otherwise the newest lines, so it follows the reply.
        let padding = Padding::left(1);
        let lines = self.lines(transcript_area.width.saturating_sub(padding.left) as usize);
        self.page = (transcript_area.height as usize).max(1);
        self.max_top = lines.len().saturating_sub(self.page);
        self.top = self.top.filter(|top| *top < self.max_top);
        let top = self.top.unwrap_or(self.max_top);
        let visible: Vec<_> = lines.into_iter().skip(top).take(self.page).collect();
        let block = Block::new().padding(padding);
        frame.render_widget(Paragraph::new(visible).block(block), transcript_area);

        if let (Some(request), Some(lines)) = (question, question_lines) {
            let tool = request.tool.as_str();
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
            frame.render_widget(Paragraph::new(lines).block(block), input_area);
        } else {
            // The end of the input that fits, so the cursor stays visible.
            // One column is kept free for the cursor.
            let marker_width = INPUT_MARKER.chars().count() as u16;
            let width = input_area.width.saturating_sub(marker_width + 1) as usize;
            let hidden = self.input.chars().count().saturating_sub(width);
            let visible: String = self.input.chars().skip(hidden).collect();
            let cursor_x = input_area.x + marker_width + visible.chars().count() as u16;
            let line = Line::from(vec![INPUT_MARKER.cyan().bold(), visible.into()]);
            let block = Block::new()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::new().fg(Color::DarkGray));
            frame.render_widget(Paragraph::new(line).block(block), input_area);
            frame.set_cursor_position((cursor_x, input_area.y + 1));
        }

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
        let status = Line::from(vec![
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
            self.input_tokens.to_string().cyan(),
            " in, ".dim(),
            self.output_tokens.to_string().cyan(),
            " out".dim(),
            separator(),
            keys,
        ]);
        frame.render_widget(Paragraph::new(status), status_area);
    }

    /// The transcript as screen lines, wrapped to `width`.
    fn lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for entry in &self.transcript {
            let (prefix, style) = match entry.kind {
                EntryKind::User => ("> ", Style::new().fg(Color::Cyan).bold()),
                EntryKind::Reply => ("", Style::new()),
                EntryKind::Tool => ("* ", Style::new().fg(Color::Yellow)),
                EntryKind::Error => ("  ", Style::new().fg(Color::Red)),
                EntryKind::Note => ("  ", Style::new().dim()),
            };
            for row in wrap(&format!("{prefix}{}", entry.text), width) {
                lines.push(Line::styled(row, style));
            }
            lines.push(Line::default());
        }
        lines
    }

    fn push(&mut self, kind: EntryKind, text: String) {
        self.transcript.push(Entry { kind, text });
    }
}

/// What a consent question shows between its lines: the call, then each reason.
fn question_lines(request: &ApprovalRequest, screen_width: u16) -> Vec<Line<'static>> {
    let width = screen_width as usize;
    // A lone text field, like a shell command, reads better bare.
    let input = match request.input.as_object().map(|fields| fields.values()) {
        Some(mut values) if values.len() == 1 => match values.next() {
            Some(serde_json::Value::String(text)) => text.clone(),
            _ => request.input.to_string(),
        },
        _ => request.input.to_string(),
    };

    let mut lines: Vec<_> = wrap(&input, width)
        .into_iter()
        .map(|row| Line::from(row).bold())
        .collect();
    for reason in &request.reasons {
        lines.extend(
            wrap(reason, width)
                .into_iter()
                .map(|row| Line::from(row).dim()),
        );
    }
    if lines.len() > QUESTION_LINES {
        lines.truncate(QUESTION_LINES - 1);
        lines.push(Line::from("...").dim());
    }
    lines
}

/// `text` as screen rows no wider than `width`. Counts characters, so wide
/// ones may overflow.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            rows.push(String::new());
        }
        for row in chars.chunks(width.max(1)) {
            rows.push(row.iter().collect());
        }
    }
    rows
}

/// The first `count` lines of `text`, with a marker if more were left out.
fn first_lines(text: &str, count: usize) -> String {
    let mut lines: Vec<&str> = text.lines().take(count).collect();
    if text.lines().count() > count {
        lines.push("...");
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    /// An app showing a reply of the lines "1" to `count`, on a screen with
    /// room for five transcript lines.
    fn app_with_reply(count: usize) -> (App, Terminal<TestBackend>) {
        let mut app = App::new(Instant::now());
        let reply: Vec<String> = (1..=count).map(|n| n.to_string()).collect();
        app.apply(Event::Text(reply.join("\n")));
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();
        transcript(&mut app, &mut terminal);
        (app, terminal)
    }

    /// Draws and returns the transcript lines on screen.
    fn transcript(app: &mut App, terminal: &mut Terminal<TestBackend>) -> Vec<String> {
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..5)
            .map(|y| {
                let row: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim().to_owned()
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

        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (4..8)
            .map(|y| (0..60).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect();
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
    fn transcript_follows_the_newest_output() {
        let (mut app, mut terminal) = app_with_reply(20);

        app.apply(Event::Text("\n21".into()));

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
        app.apply(Event::Text("\n21".into()));

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
        app.apply(Event::Text("\n21".into()));

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

//! The TUI's application state — a small, flat struct plus pure reducers.
//!
//! Nothing here touches the terminal or crossterm: [`App::handle`] is a pure
//! function of `(state, event)`, so the whole layer is testable headless. The
//! event loop feeds it [`AppEvent`]s and draws when [`App::dirty`] is set.

/// A key the app understands — decoupled from crossterm so this module stays
/// terminal-free (`event.rs` translates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Enter,
    Esc,
    /// A control chord, e.g. `Ctrl('c')` for Ctrl-C.
    Ctrl(char),
}

/// Everything the app can react to, from any source.
#[derive(Clone, Debug, PartialEq)]
pub enum AppEvent {
    Key(Key),
    Paste(String),
    Tick,
}

/// A committed transcript block. The live streaming block (assistant text in
/// flight) joins this set in P0c, alongside `Thinking` and `Tool`.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    User(String),
    Notice(String),
}

/// Status-line fields. `model`/`effort` are wired to the session in P0c.
#[derive(Clone, Debug)]
pub struct Status {
    pub model: String,
    pub effort: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            model: "wcode".to_string(),
            effort: None,
        }
    }
}

/// The whole UI state. Flat by design — grow submodules only when it hurts.
#[derive(Default)]
pub struct App {
    transcript: Vec<Block>,
    input: String,
    /// Cursor position as a *character* index into `input`.
    cursor: usize,
    status: Status,
    running: bool,
    dirty: bool,
    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        App {
            dirty: true,
            ..App::default()
        }
    }

    /// Apply one event. Pure state transition; sets [`App::dirty`] on a change.
    pub fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.on_key(key),
            AppEvent::Paste(text) => self.insert_str(&text),
            AppEvent::Tick => {}
        }
    }

    fn on_key(&mut self, key: Key) {
        match key {
            Key::Char(c) => self.insert_char(c),
            Key::Backspace => self.backspace(),
            Key::Delete => {}
            Key::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                self.dirty = true;
            }
            Key::Right => {
                if self.cursor < self.input.chars().count() {
                    self.cursor += 1;
                }
                self.dirty = true;
            }
            Key::Home => {
                self.cursor = 0;
                self.dirty = true;
            }
            Key::End => {
                self.cursor = self.input.chars().count();
                self.dirty = true;
            }
            Key::Enter => self.submit(),
            Key::Esc => self.should_quit = true,
            // Ctrl-C cancels a run; idle, it quits. Cancellation lands in P0c.
            Key::Ctrl('c') => self.should_quit = true,
            _ => {}
        }
    }

    fn submit(&mut self) {
        let text = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.dirty = true;
        if text.trim().is_empty() {
            return;
        }
        self.transcript.push(Block::User(text));
        self.transcript
            .push(Block::Notice("no session connected yet (P0c wires the backend)".into()));
    }

    fn insert_char(&mut self, c: char) {
        let at = self.byte_index(self.cursor);
        self.input.insert(at, c);
        self.cursor += 1;
        self.dirty = true;
    }

    fn insert_str(&mut self, text: &str) {
        let at = self.byte_index(self.cursor);
        self.input.insert_str(at, text);
        self.cursor += text.chars().count();
        self.dirty = true;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_index(self.cursor);
        let start = self.byte_index(self.cursor - 1);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
        self.dirty = true;
    }

    /// Byte offset of the `n`th character, or the string end.
    fn byte_index(&self, n: usize) -> usize {
        self.input
            .char_indices()
            .nth(n)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    pub fn transcript(&self) -> &[Block] {
        &self.transcript
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
    }

    #[test]
    fn typing_builds_the_input_buffer() {
        let mut app = App::new();
        typed(&mut app, "hello");
        assert_eq!(app.input(), "hello");
        assert_eq!(app.cursor(), 5);
    }

    #[test]
    fn backspace_removes_the_char_before_the_cursor() {
        let mut app = App::new();
        typed(&mut app, "abc");
        app.handle(AppEvent::Key(Key::Left));
        app.handle(AppEvent::Key(Key::Backspace));
        assert_eq!(app.input(), "ac");
        assert_eq!(app.cursor(), 1);
    }

    #[test]
    fn enter_commits_a_user_block_and_clears_the_input() {
        let mut app = App::new();
        typed(&mut app, "hi there");
        app.handle(AppEvent::Key(Key::Enter));
        assert_eq!(app.input(), "");
        assert_eq!(app.cursor(), 0);
        assert_eq!(
            app.transcript()[0],
            Block::User("hi there".to_string())
        );
    }

    #[test]
    fn empty_enter_commits_nothing() {
        let mut app = App::new();
        typed(&mut app, "   ");
        app.handle(AppEvent::Key(Key::Enter));
        assert!(app.transcript().is_empty());
    }

    #[test]
    fn paste_inserts_at_the_cursor() {
        let mut app = App::new();
        typed(&mut app, "ac");
        app.handle(AppEvent::Key(Key::Left));
        app.handle(AppEvent::Paste("b".into()));
        assert_eq!(app.input(), "abc");
        assert_eq!(app.cursor(), 2);
    }

    #[test]
    fn esc_quits_and_ctrl_c_quits_when_idle() {
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::Esc));
        assert!(app.should_quit());

        let mut app = App::new();
        app.handle(AppEvent::Key(Key::Ctrl('c')));
        assert!(app.should_quit());
    }

    #[test]
    fn dirty_is_set_on_change_and_cleared_explicitly() {
        let mut app = App::new();
        app.clear_dirty();
        assert!(!app.dirty());
        typed(&mut app, "x");
        assert!(app.dirty());
        app.clear_dirty();
        assert!(!app.dirty());
    }
}

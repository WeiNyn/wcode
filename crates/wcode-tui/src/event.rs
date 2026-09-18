//! Translate terminal events into [`AppEvent`]s.
//!
//! The only module that knows crossterm's input types; the app sees our own
//! [`Key`] instead, so it stays terminal-free.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::app::{AppEvent, Key};

/// Translate one crossterm event into zero or more app events.
pub fn translate(event: Event) -> Vec<AppEvent> {
    match event {
        // Ignore key *release* so we act on press only.
        Event::Key(k) if k.kind != KeyEventKind::Release => {
            translate_key(k).map(AppEvent::Key).into_iter().collect()
        }
        Event::Mouse(mouse) => translate_mouse(mouse)
            .map(AppEvent::Key)
            .into_iter()
            .collect(),
        Event::Paste(text) => vec![AppEvent::Paste(text)],
        Event::Resize(_, _) => vec![AppEvent::Resize],
        _ => Vec::new(),
    }
}

fn translate_key(k: KeyEvent) -> Option<Key> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    Some(match k.code {
        // Ctrl-J is a portable newline: many terminals deliver it as a bare LF,
        // so map the chord (Shift-Enter still works) to an explicit Newline.
        KeyCode::Char('j') if ctrl => Key::Newline,
        KeyCode::Char(c) if ctrl => Key::Ctrl(c),
        // An Alt chord is its own thing — it must never fall through to the
        // plain `Char` arm and insert the character.
        KeyCode::Char(c) if alt => Key::Alt(c),
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        // Tab accepts the inline command completion; BackTab (Shift-Tab) focuses
        // the previous surface — but only once the popup is closed, since the
        // popup's own handler (`App::on_completion_key`) consumes it first.
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::F(n) => Key::F(n),
        KeyCode::Enter if k.modifiers.contains(KeyModifiers::SHIFT) => Key::Newline,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        _ => return None,
    })
}

/// The mouse has no click targets (P3 panels will add them); only the wheel
/// matters — it scrolls the transcript.
fn translate_mouse(m: MouseEvent) -> Option<Key> {
    match m.kind {
        MouseEventKind::ScrollUp => Some(Key::ScrollUp),
        MouseEventKind::ScrollDown => Some(Key::ScrollDown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn mouse(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// `AppEvent` carries `AgentEvent` (not `PartialEq`), so compare on `Key`.
    fn keys(mut events: Vec<AppEvent>) -> Vec<Key> {
        events
            .drain(..)
            .filter_map(|e| match e {
                AppEvent::Key(k) => Some(k),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn plain_char_and_ctrl_chord() {
        assert_eq!(
            keys(translate(key(KeyCode::Char('a'), KeyModifiers::NONE))),
            vec![Key::Char('a')]
        );
        assert_eq!(
            keys(translate(key(KeyCode::Char('c'), KeyModifiers::CONTROL))),
            vec![Key::Ctrl('c')]
        );
    }

    #[test]
    fn ctrl_j_is_a_newline() {
        assert_eq!(
            keys(translate(key(KeyCode::Char('j'), KeyModifiers::CONTROL))),
            vec![Key::Newline]
        );
    }

    #[test]
    fn the_wheel_is_a_scroll_and_other_mouse_events_are_ignored() {
        assert_eq!(
            keys(translate(mouse(MouseEventKind::ScrollUp))),
            vec![Key::ScrollUp]
        );
        assert_eq!(
            keys(translate(mouse(MouseEventKind::ScrollDown))),
            vec![Key::ScrollDown]
        );
        // No click targets yet: a click must not reach the app at all.
        assert!(translate(mouse(MouseEventKind::Moved)).is_empty());
    }

    #[test]
    fn paste_and_unknown_events() {
        assert!(matches!(
            translate(Event::Paste("hi".into())).as_slice(),
            [AppEvent::Paste(t)] if t == "hi"
        ));
        assert!(matches!(
            translate(Event::Resize(1, 1)).as_slice(),
            [AppEvent::Resize]
        ));
        assert!(translate(Event::FocusGained).is_empty());
    }

    #[test]
    fn tab_accepts_completion_and_backtab_is_the_previous_surface_chord() {
        assert_eq!(
            keys(translate(key(KeyCode::Tab, KeyModifiers::NONE))),
            vec![Key::Tab]
        );
        // BackTab (Shift-Tab) is now mapped: the previous-surface chord. It only
        // switches surface while the completion popup is closed — the popup's
        // handler (`App::on_completion_key`) consumes it first (tested in app.rs).
        assert_eq!(
            keys(translate(key(KeyCode::BackTab, KeyModifiers::SHIFT))),
            vec![Key::BackTab]
        );
    }

    #[test]
    fn alt_chords_and_function_keys_are_mapped_and_never_insert_a_char() {
        assert_eq!(
            keys(translate(key(KeyCode::Char('3'), KeyModifiers::ALT))),
            vec![Key::Alt('3')]
        );
        assert_eq!(
            keys(translate(key(KeyCode::F(1), KeyModifiers::NONE))),
            vec![Key::F(1)]
        );
    }
}

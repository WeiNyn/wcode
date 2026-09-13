//! Translate terminal events into [`AppEvent`]s.
//!
//! The only module that knows crossterm's input types; the app sees our own
//! [`Key`] instead, so it stays terminal-free.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::{AppEvent, Key};

/// Translate one crossterm event into zero or more app events.
pub fn translate(event: Event) -> Vec<AppEvent> {
    match event {
        // Ignore key *release* so we act on press only.
        Event::Key(k) if k.kind != KeyEventKind::Release => {
            translate_key(k).map(AppEvent::Key).into_iter().collect()
        }
        Event::Paste(text) => vec![AppEvent::Paste(text)],
        _ => Vec::new(),
    }
}

fn translate_key(k: KeyEvent) -> Option<Key> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    Some(match k.code {
        KeyCode::Char(c) if ctrl => Key::Ctrl(c),
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
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
    fn paste_and_unknown_events() {
        assert!(matches!(
            translate(Event::Paste("hi".into())).as_slice(),
            [AppEvent::Paste(t)] if t == "hi"
        ));
        assert!(translate(Event::Resize(1, 1)).is_empty());
    }
}

//! Input handling utilities

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Convert crossterm KeyEvent to nvim input string
pub fn key_to_nvim_input(key: &KeyEvent) -> String {
    let mut modifiers = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        modifiers.push_str("C-");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        modifiers.push_str("A-");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        modifiers.push_str("S-");
    }

    let key_str = match key.code {
        KeyCode::Char(c) => {
            if modifiers.is_empty() {
                return c.to_string();
            }
            c.to_string()
        }
        KeyCode::Enter => "CR".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Backspace => "BS".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Delete => "Del".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::F(n) => format!("F{}", n),
        _ => return String::new(),
    };

    if modifiers.is_empty() && key_str.len() == 1 {
        key_str
    } else {
        format!("<{}{}>", modifiers, key_str)
    }
}

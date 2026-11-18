use std::path::PathBuf;

use alacritty_terminal::term::TermMode;
use eframe::egui::{self, Key, Modifiers};

use crate::agents::images;

pub(super) fn mouse_bytes(
    button: u8,
    modifiers: Modifiers,
    (column, row): (usize, usize),
    pressed: bool,
    mode: TermMode,
) -> Vec<u8> {
    let modifiers = 4 * modifiers.shift as u8 + 8 * modifiers.alt as u8 + 16 * modifiers.ctrl as u8;
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return format!(
            "\x1b[<{};{};{}{end}",
            button + modifiers,
            column + 1,
            row + 1
        )
        .into_bytes();
    }
    let button = if pressed { button } else { 3 };
    if column > 222 || row > 222 {
        return Vec::new();
    }
    vec![
        0x1b,
        b'[',
        b'M',
        32 + button + modifiers,
        33 + column as u8,
        33 + row as u8,
    ]
}

pub(super) fn dropped_images(ui: &egui::Ui, rect: egui::Rect) -> Vec<PathBuf> {
    let over = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pos| rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id()));
    if !over {
        return Vec::new();
    }
    ui.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_path_buf())
            .filter(|path| images::is_image(path))
            .collect()
    })
}

pub(super) fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", "")).into_bytes()
    } else {
        text.into_bytes()
    }
}

pub(super) fn key_bytes(key: Key, modifiers: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let mac = cfg!(target_os = "macos");
    let code = 1 + modifiers.shift as u8 + 2 * modifiers.alt as u8 + 4 * modifiers.ctrl as u8;
    let csi = |end: &str| {
        if code > 1 {
            format!("\x1b[1;{code}{end}")
        } else {
            format!("\x1b[{end}")
        }
    };
    let cursor = |end: &str| {
        if code == 1 && mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{end}")
        } else {
            csi(end)
        }
    };
    let tilde = |number: u8| {
        if code > 1 {
            format!("\x1b[{number};{code}~")
        } else {
            format!("\x1b[{number}~")
        }
    };
    let only_alt = modifiers.alt && !modifiers.ctrl && !modifiers.shift && !modifiers.command;
    let text = match key {
        Key::ArrowLeft if mac && modifiers.mac_cmd => "\x01".to_string(),
        Key::ArrowRight if mac && modifiers.mac_cmd => "\x05".to_string(),
        Key::Backspace if mac && modifiers.mac_cmd => "\x15".to_string(),
        _ if modifiers.command && !modifiers.ctrl => return None,
        Key::ArrowLeft if only_alt => "\x1bb".to_string(),
        Key::ArrowRight if only_alt => "\x1bf".to_string(),
        Key::ArrowUp => cursor("A"),
        Key::ArrowDown => cursor("B"),
        Key::ArrowRight => cursor("C"),
        Key::ArrowLeft => cursor("D"),
        Key::Home => cursor("H"),
        Key::End => cursor("F"),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => "\x1bOP".to_string(),
        Key::F2 => "\x1bOQ".to_string(),
        Key::F3 => "\x1bOR".to_string(),
        Key::F4 => "\x1bOS".to_string(),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Enter if modifiers.shift || modifiers.alt => "\x1b\r".to_string(),
        Key::Enter => "\r".to_string(),
        Key::Tab if modifiers.shift => "\x1b[Z".to_string(),
        Key::Tab => "\t".to_string(),
        Key::Backspace if modifiers.ctrl => "\x08".to_string(),
        Key::Backspace if modifiers.alt => "\x1b\x7f".to_string(),
        Key::Backspace => "\x7f".to_string(),
        Key::Escape => "\x1b".to_string(),
        Key::Space if modifiers.ctrl => "\0".to_string(),
        _ if modifiers.ctrl => {
            let letter = key
                .name()
                .chars()
                .next()
                .filter(|c| key.name().len() == 1 && c.is_ascii_alphabetic())?;
            let control = (letter.to_ascii_uppercase() as u8) & 0x1f;
            if modifiers.alt {
                format!("\x1b{}", control as char)
            } else {
                (control as char).to_string()
            }
        }
        _ => return None,
    };
    Some(text.into_bytes())
}

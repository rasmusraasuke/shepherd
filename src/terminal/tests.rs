use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::NamedColor;
use eframe::egui::{Color32, Event, Key, Modifiers, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use super::input::{key_bytes, mouse_bytes, paste_bytes};
use super::paint::{DARK_COLORS, Palette, rgb};
use super::*;
use crate::testing::{Dropped, TempDir};

fn sent(key: Key, modifiers: Modifiers) -> String {
    String::from_utf8(key_bytes(key, modifiers, TermMode::default()).unwrap_or_default()).unwrap()
}

#[test]
fn keys_send_what_xterm_sends() {
    assert_eq!(sent(Key::Enter, Modifiers::NONE), "\r");
    assert_eq!(sent(Key::Enter, Modifiers::SHIFT), "\x1b\r");
    assert_eq!(sent(Key::Backspace, Modifiers::NONE), "\x7f");
    assert_eq!(sent(Key::Backspace, Modifiers::ALT), "\x1b\x7f");
    assert_eq!(sent(Key::Tab, Modifiers::SHIFT), "\x1b[Z");
    assert_eq!(sent(Key::Escape, Modifiers::NONE), "\x1b");
    assert_eq!(sent(Key::ArrowUp, Modifiers::NONE), "\x1b[A");
    assert_eq!(sent(Key::ArrowLeft, Modifiers::CTRL), "\x1b[1;5D");
    assert_eq!(sent(Key::ArrowRight, Modifiers::SHIFT), "\x1b[1;2C");
    assert_eq!(sent(Key::ArrowLeft, Modifiers::ALT), "\x1bb");
    assert_eq!(sent(Key::Delete, Modifiers::NONE), "\x1b[3~");
    assert_eq!(sent(Key::F5, Modifiers::NONE), "\x1b[15~");
    assert_eq!(sent(Key::C, Modifiers::CTRL), "\x03");
    assert_eq!(sent(Key::D, Modifiers::CTRL), "\x04");
    assert_eq!(sent(Key::A, Modifiers::NONE), "");
    assert_eq!(sent(Key::P, Modifiers::COMMAND), "");
}

#[test]
fn keys_follow_the_modes_the_program_asked_for() {
    assert_eq!(
        key_bytes(Key::ArrowUp, Modifiers::NONE, TermMode::APP_CURSOR).unwrap(),
        b"\x1bOA"
    );
}

#[test]
fn mouse_buttons_send_what_xterm_sends() {
    let sgr = TermMode::SGR_MOUSE;
    assert_eq!(
        mouse_bytes(0, Modifiers::NONE, (0, 0), true, sgr),
        b"\x1b[<0;1;1M"
    );
    assert_eq!(
        mouse_bytes(2, Modifiers::CTRL, (9, 4), false, sgr),
        b"\x1b[<18;10;5m"
    );
    assert_eq!(
        mouse_bytes(65, Modifiers::NONE, (1, 2), true, sgr),
        b"\x1b[<65;2;3M"
    );
    assert_eq!(
        mouse_bytes(32, Modifiers::SHIFT, (3, 3), true, TermMode::default()),
        [0x1b, b'[', b'M', 32 + 36, 36, 36]
    );
    assert_eq!(
        mouse_bytes(0, Modifiers::NONE, (3, 3), false, TermMode::default()),
        [0x1b, b'[', b'M', 35, 36, 36]
    );
    assert!(mouse_bytes(0, Modifiers::NONE, (300, 3), true, TermMode::default()).is_empty());
}

#[test]
fn pastes_line_breaks_as_enter_and_marks_the_paste_when_asked() {
    assert_eq!(paste_bytes("a\nb\r\nc", TermMode::default()), b"a\rb\rc");
    assert_eq!(
        paste_bytes("ls\x1b[201~rm", TermMode::BRACKETED_PASTE),
        b"\x1b[200~lsrm\x1b[201~"
    );
}

#[test]
fn has_the_standard_256_colors() {
    let palette = Palette {
        dark: true,
        foreground: Color32::WHITE,
        background: Color32::BLACK,
    };
    assert_eq!(palette.standard(1), rgb(DARK_COLORS[1]));
    assert_eq!(palette.standard(16), Color32::from_rgb(0, 0, 0));
    assert_eq!(palette.standard(21), Color32::from_rgb(0, 0, 255));
    assert_eq!(palette.standard(196), Color32::from_rgb(255, 0, 0));
    assert_eq!(palette.standard(232), Color32::from_gray(8));
    assert_eq!(
        palette.standard(NamedColor::Background as usize),
        Color32::BLACK
    );
}

fn harness(name: &str, script: &str) -> (Harness<'static, Terminals>, TempDir) {
    let dir = TempDir::new(name);
    let mut terminals = Terminals::new(dir.path().to_path_buf());
    terminals.image_dir = dir.path().join("images");
    terminals.run("/bin/sh", &["-c", script]);
    let harness = Harness::builder()
        .with_size(Vec2::new(600.0, 400.0))
        .with_max_steps(8)
        .build_ui_state(
            |ui, terminals: &mut Terminals| terminals.ui(ui, 1),
            terminals,
        );
    (harness, dir)
}

fn agent_harness(name: &str, script: &str, fresh: bool) -> (Harness<'static, Terminals>, TempDir) {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = TempDir::new(name);
    let program = dir.write("agent", script);
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminals = Terminals::new(dir.path().to_path_buf());
    terminals.image_dir = dir.path().join("images");
    terminals.run_agents_with(&program);
    if fresh {
        terminals.start(1, Prompt::default());
    }
    let harness = Harness::builder()
        .with_size(Vec2::new(600.0, 400.0))
        .with_max_steps(8)
        .build_ui_state(
            |ui, terminals: &mut Terminals| {
                terminals.agent_ui(ui, 1, Agent::Claude, "s1", &Tuning::default());
            },
            terminals,
        );
    (harness, dir)
}

fn wait(
    harness: &mut Harness<'_, Terminals>,
    what: &str,
    done: impl Fn(&Harness<'_, Terminals>) -> bool,
) {
    let start = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "no {what} on the screen:\n{}",
            harness.state().screen(1).unwrap()
        );
        harness.run();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_screen(harness: &mut Harness<'_, Terminals>, text: &str) {
    wait(harness, text, |harness| {
        harness.state().screen(1).unwrap().contains(text)
    });
}

const SHOW_INPUT: &str = "stty -icanon -echo; dd bs=1 count=COUNT 2>/dev/null | od -An -c; sleep 5";

#[test]
fn runs_a_program_in_the_project_and_shows_its_output() {
    let (mut harness, dir) = harness(
        "terminal",
        "pwd; printf '\\033]0;working\\007\\033[31mred\\033[0m'; read line; echo got $line",
    );
    wait_for_screen(&mut harness, "red");
    assert!(
        harness
            .state()
            .screen(1)
            .unwrap()
            .replace('\n', "")
            .starts_with(&dir.path().canonicalize().unwrap().display().to_string())
    );
    wait(&mut harness, "title", |harness| {
        harness.state().title(1).as_deref() == Some("working")
    });

    harness.state().terminals[&1]
        .as_ref()
        .unwrap()
        .write("hello\r");
    wait_for_screen(&mut harness, "got hello");
    wait(&mut harness, "end", |harness| {
        harness.state().title(1).as_deref() == Some("working (exited)")
    });
}

#[test]
fn tells_when_the_program_works_and_when_it_stopped() {
    let (mut harness, _dir) = harness(
        "terminal-activity",
        "sleep 0.5; for i in 1 2 3 4 5 6; do echo $i; sleep 0.2; done; sleep 10",
    );
    wait(&mut harness, "work", |harness| {
        harness.state().activity(1) == Some(Activity::Working)
    });
    wait(&mut harness, "stop", |harness| {
        harness.state().activity(1) == Some(Activity::Waiting)
    });
}

#[test]
fn sends_text_to_agent_tabs_as_if_typed_with_enter() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = TempDir::new("terminal-send");
    let script = dir.write("agent", "#!/bin/sh\nread line\necho hi $line\nsleep 5\n");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminals = Terminals::new(dir.path().to_path_buf());
    terminals.run_agents_with(&script);
    let mut harness = Harness::builder()
        .with_size(Vec2::new(600.0, 400.0))
        .with_max_steps(8)
        .build_ui_state(
            |ui, terminals: &mut Terminals| {
                terminals.agent_ui(ui, 1, Agent::Claude, "s1", &Tuning::default());
            },
            terminals,
        );
    harness.run();
    assert_eq!(harness.state().agents(), [(1, "Claude Code".to_string())]);
    harness.state_mut().send(1, "hello");
    wait_for_screen(&mut harness, "hi hello");
}

#[test]
fn starts_an_agent_fresh_or_continues_its_session() {
    let script = "#!/bin/sh\necho \"args: $@\"\nsleep 5\n";
    let (mut fresh, _dir) = agent_harness("agent-new", script, true);
    fresh.run();
    wait_for_screen(&mut fresh, "args: --session-id s1");

    let (mut resumed, _dir) = agent_harness("agent-resume", script, false);
    resumed.run();
    wait_for_screen(&mut resumed, "args: --resume s1");
}

#[test]
fn never_sends_text_into_a_plain_terminal() {
    let (mut harness, _dir) = harness("terminal-no-send", "sleep 5");
    harness.state_mut().send(1, "hello");
    harness.run_steps(4);
    assert_eq!(harness.state().screen(1).unwrap(), "");
}

#[test]
fn shutdown_ends_the_program_of_every_terminal() {
    let (mut harness, _dir) = harness(
        "terminal-shutdown",
        "trap '' HUP; echo ready; while true; do sleep 0.1; done",
    );
    wait_for_screen(&mut harness, "ready");
    let pid = harness.state().terminals[&1].as_ref().unwrap().child as libc::pid_t;

    harness.state_mut().shutdown();

    assert!(!harness.state().is_open(1));
    let start = Instant::now();
    while unsafe { libc::kill(pid, 0) } != -1 {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the program is still there"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn restarts_the_program_after_it_ended() {
    let (mut harness, _dir) = harness("terminal-restart", "echo started-$$");
    wait(&mut harness, "end", |harness| {
        harness
            .state()
            .title(1)
            .is_some_and(|title| title.ends_with("(exited)"))
    });
    let first = harness.state().screen(1).unwrap();
    harness.get_by_label("Restart").click();
    wait(&mut harness, "new start", |harness| {
        harness.state().screen(1).unwrap().starts_with("started-")
            && harness.state().screen(1).unwrap() != first
    });

    wait(&mut harness, "end", |harness| {
        harness
            .state()
            .title(1)
            .is_some_and(|title| title.ends_with("(exited)"))
    });
    let second = harness.state().screen(1).unwrap();
    harness.key_press(Key::Enter);
    wait(&mut harness, "new start", |harness| {
        harness.state().screen(1).unwrap().starts_with("started-")
            && harness.state().screen(1).unwrap() != second
    });
}

#[test]
fn programs_read_the_clipboard_only_when_allowed() {
    let script = format!(
        "printf '\\033]52;c;?\\007'; {}",
        SHOW_INPUT.replace("COUNT", "12")
    );
    let (mut harness, _dir) = harness("terminal-clipboard", &script);
    harness.state_mut().read_clipboard = || Some("hi".to_string());
    wait(&mut harness, "question", |harness| {
        harness.query_by_label("Allow").is_some()
    });
    harness.get_by_label("Allow").click();
    wait_for_screen(&mut harness, "a   G   k   =");
    assert!(harness.query_by_label("Allow").is_none());

    let (mut denied, _dir) = self::harness("terminal-clipboard-denied", &script);
    wait(&mut denied, "question", |harness| {
        harness.query_by_label("Deny").is_some()
    });
    denied.get_by_label("Deny").click();
    denied.run();
    assert!(denied.query_by_label("Deny").is_none());
    assert_eq!(denied.state().screen(1).unwrap(), "");
}

#[test]
fn mouse_clicks_go_to_programs_that_asked_for_them() {
    let script = format!(
        "printf '\\033[?1000h\\033[?1006h'; {}",
        SHOW_INPUT.replace("COUNT", "18")
    );
    let (mut harness, _dir) = harness("terminal-mouse", &script);
    let start = Instant::now();
    while !harness.state().terminals[&1]
        .as_ref()
        .unwrap()
        .term
        .lock()
        .mode()
        .contains(TermMode::SGR_MOUSE)
    {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the program didn't ask for the mouse"
        );
        harness.run();
    }
    let cell = harness.get_by_role(egui::Role::Terminal).rect().min + Vec2::splat(PADDING + 2.0);
    harness.event(Event::PointerMoved(cell));
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos: cell,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    let sent = "033[<0;1;1M033[<0;1;1m";
    wait(&mut harness, sent, |harness| {
        harness
            .state()
            .screen(1)
            .unwrap()
            .split_whitespace()
            .collect::<String>()
            .contains(sent)
    });
}

#[test]
fn mouse_clicks_on_a_popup_over_the_terminal_stay_on_the_popup() {
    let script = format!(
        "printf '\\033[?1000h\\033[?1006h'; {}",
        SHOW_INPUT.replace("COUNT", "18")
    );
    let dir = TempDir::new("terminal-mouse-popup");
    let mut terminals = Terminals::new(dir.path().to_path_buf());
    terminals.image_dir = dir.path().join("images");
    terminals.run("/bin/sh", &["-c", &script]);
    let mut harness = Harness::builder()
        .with_size(Vec2::new(600.0, 400.0))
        .with_max_steps(8)
        .build_ui_state(
            |ui, terminals: &mut Terminals| {
                terminals.ui(ui, 1);
                egui::Area::new(egui::Id::unique("popup"))
                    .fixed_pos(egui::pos2(0.0, 0.0))
                    .show(ui.ctx(), |ui| {
                        ui.allocate_exact_size(Vec2::new(40.0, 20.0), egui::Sense::hover());
                    });
            },
            terminals,
        );
    let start = Instant::now();
    while !harness.state().terminals[&1]
        .as_ref()
        .unwrap()
        .term
        .lock()
        .mode()
        .contains(TermMode::SGR_MOUSE)
    {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the program didn't ask for the mouse"
        );
        harness.run();
    }
    let terminal = harness.get_by_role(egui::Role::Terminal).rect().min;
    let covered = terminal + Vec2::splat(PADDING + 2.0);
    let free = covered + Vec2::new(0.0, 40.0);
    for cell in [covered, free] {
        harness.event(Event::PointerMoved(cell));
        harness.run();
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos: cell,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        harness.run();
    }
    let screen = |harness: &Harness<'_, Terminals>| {
        harness
            .state()
            .screen(1)
            .unwrap()
            .split_whitespace()
            .collect::<String>()
    };
    wait(&mut harness, "the click beside the popup", |harness| {
        screen(harness).contains("033[<0;1;")
    });
    assert!(
        !screen(&harness).contains("033[<0;1;1M"),
        "{}",
        screen(&harness)
    );
}

fn picture() -> Arc<egui::ColorImage> {
    Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]))
}

fn image_harness(name: &str) -> (Harness<'static, Terminals>, TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = TempDir::new(name);
    let capture = dir.path().join("input");
    let program = dir.write(
        "agent",
        format!(
            "#!/bin/sh\nstty -icanon -echo\nprintf ready\ncat > '{}'\n",
            capture.display()
        ),
    );
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut terminals = Terminals::new(dir.path().to_path_buf());
    terminals.image_dir = dir.path().join("images");
    terminals.run_agents_with(&program);
    terminals.start(1, Prompt::default());
    let harness = Harness::builder()
        .with_size(Vec2::new(600.0, 400.0))
        .with_max_steps(8)
        .build_ui_state(
            |ui, terminals: &mut Terminals| {
                terminals.agent_ui(ui, 1, Agent::Claude, "s1", &Tuning::default());
            },
            terminals,
        );
    (harness, dir, capture)
}

fn click_terminal(harness: &mut Harness<'_, Terminals>) {
    let cell = harness.get_by_role(egui::Role::Terminal).rect().min + Vec2::splat(PADDING + 2.0);
    harness.event(Event::PointerMoved(cell));
    for pressed in [true, false] {
        harness.event(Event::PointerButton {
            pos: cell,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.run();
}

fn wait_for_input(path: &Path, expected: &str) {
    let start = Instant::now();
    while !fs::read_to_string(path).is_ok_and(|text| text == expected) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the program didn't get {expected:?}, it has {:?}",
            fs::read_to_string(path).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn image_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

#[test]
fn pastes_a_picture_into_an_agent_tab_as_the_path_of_a_file() {
    let (mut harness, dir, capture) = image_harness("agent-image-paste");
    wait_for_screen(&mut harness, "ready");
    click_terminal(&mut harness);
    harness.event(Event::PasteImage(picture()));
    harness.run();

    let images = image_files(&dir.path().join("images"));
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].extension().unwrap(), "png");
    wait_for_input(&capture, &images[0].display().to_string());
    drop(harness);
    assert!(!images[0].exists());
}

#[test]
fn pastes_text_as_text_and_writes_no_picture() {
    let (mut harness, dir, capture) = image_harness("agent-image-text");
    wait_for_screen(&mut harness, "ready");
    click_terminal(&mut harness);
    harness.event(Event::Paste("hello".to_string()));
    harness.run();
    wait_for_input(&capture, "hello");
    assert!(image_files(&dir.path().join("images")).is_empty());
}

#[test]
fn drops_pictures_on_an_agent_tab_as_their_paths() {
    let (mut harness, dir, capture) = image_harness("agent-image-drop");
    wait_for_screen(&mut harness, "ready");
    let picture = dir.write("shot.png", "picture");
    let notes = dir.write("notes.txt", "text");
    let cell = harness.get_by_role(egui::Role::Terminal).rect().center();
    harness.event(Event::PointerMoved(cell));
    harness.step();
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(picture.clone())));
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(notes)));
    harness.step();
    wait_for_input(&capture, &picture.display().to_string());
    assert!(image_files(&dir.path().join("images")).is_empty());
    drop(harness);
    assert!(picture.exists());
}

#[test]
fn leaves_a_plain_terminal_alone_when_pictures_are_pasted_or_dropped() {
    let (mut harness, dir) = harness(
        "terminal-no-image",
        "stty -icanon -echo; printf ready; cat > input",
    );
    wait_for_screen(&mut harness, "ready");
    click_terminal(&mut harness);
    harness.event(Event::PasteImage(picture()));
    harness.run_steps(2);

    let picture = dir.write("shot.png", "picture");
    let cell = harness.get_by_role(egui::Role::Terminal).rect().center();
    harness.event(Event::PointerMoved(cell));
    harness.step();
    harness
        .input_mut()
        .dropped_files
        .push(Arc::new(Dropped(picture)));
    harness.run_steps(2);

    let input = fs::read_to_string(dir.path().join("input")).unwrap_or_default();
    assert!(input.is_empty(), "{input:?}");
    assert!(image_files(&dir.path().join("images")).is_empty());
}

use eframe::egui::{self, RichText};

const DOUBLE_SHIFT: f64 = 0.4;

const SECTIONS: [(&str, &[(&str, &str)]); 4] = [
    (
        "Workspace",
        &[
            ("Open a file", "⌘P"),
            ("Search in files", "⇧⇧  ⌘⇧F"),
            ("New terminal", "⌘T"),
            ("Next tab under the pointer", "⌃Tab"),
            ("Save now", "⌘S"),
            ("Keyboard shortcuts", "⌘/"),
        ],
    ),
    (
        "Editing",
        &[
            ("Find and replace", "⌘F"),
            ("Undo / redo", "⌘Z  ⌘⇧Z"),
            ("Previous / next word", "⌥←  ⌥→"),
            ("Start / end of the line", "⌘←  ⌘→"),
            ("Start / end of the file", "⌘↑  ⌘↓"),
            ("Delete the word / the line before", "⌥⌫  ⌘⌫"),
            ("Indent / outdent", "Tab  ⇧Tab"),
            ("Show the changes of the line", "⌘⇧R"),
        ],
    ),
    (
        "Review",
        &[
            ("Next / previous hunk", "j  k"),
            ("Mark the hunk reviewed", "Space"),
            ("Next unreviewed hunk", "n"),
            ("Open the file at the hunk", "o"),
            ("Give the keys back to the review", "Esc"),
            ("Refresh now", "r"),
            ("Send the comment", "⌘↩"),
        ],
    ),
    (
        "Terminal",
        &[
            ("Copy / paste", "⌘C  ⌘V"),
            ("Clear the scrollback", "⌘K"),
            ("New line for an agent", "⇧↩"),
            ("Select in programs using the mouse", "⇧ drag"),
        ],
    ),
];

pub fn ui(ui: &mut egui::Ui) {
    ui.set_width(640.0);
    ui.label(RichText::new("Keyboard shortcuts").strong().size(16.0));
    ui.add_space(12.0);
    ui.spacing_mut().item_spacing.x = 40.0;
    ui.columns(2, |columns| {
        for (index, (section, keys)) in SECTIONS.into_iter().enumerate() {
            let ui = &mut columns[index % 2];
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(section).color(ui.visuals().weak_text_color()));
            ui.add_space(2.0);
            for (action, key) in keys {
                ui.horizontal(|ui| {
                    ui.label(*action);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(*key).color(ui.visuals().strong_text_color()));
                    });
                });
            }
            ui.add_space(14.0);
        }
    });
}

#[derive(Default)]
pub struct DoubleShift {
    held: Option<Hold>,
    tapped: Option<f64>,
}

struct Hold {
    clean: bool,
    second: bool,
}

impl DoubleShift {
    pub fn follow(&mut self, events: &[egui::Event], time: f64) -> bool {
        let mut shift_alone = self.held.is_some();
        let mut fired = false;
        for event in events {
            let other = match event {
                egui::Event::ModifiersChanged(modifiers) => {
                    shift_alone = *modifiers == egui::Modifiers::SHIFT;
                    false
                }
                egui::Event::Key {
                    key: egui::Key::ShiftLeft | egui::Key::ShiftRight,
                    ..
                } => continue,
                egui::Event::Key { .. }
                | egui::Event::Text(_)
                | egui::Event::PointerButton { pressed: true, .. } => true,
                _ => continue,
            };
            fired |= self.update(shift_alone, other, time);
        }
        fired
    }

    fn update(&mut self, shift_alone: bool, other_input: bool, time: f64) -> bool {
        if other_input {
            self.tapped = None;
            if let Some(hold) = &mut self.held {
                hold.clean = false;
            }
        }
        match (self.held.take(), shift_alone) {
            (None, true) => {
                self.held = Some(Hold {
                    clean: !other_input,
                    second: self
                        .tapped
                        .is_some_and(|tapped| time - tapped <= DOUBLE_SHIFT),
                });
            }
            (Some(hold), true) => self.held = Some(hold),
            (Some(hold), false) if hold.clean && hold.second => {
                self.tapped = None;
                return true;
            }
            (Some(hold), false) => self.tapped = hold.clean.then_some(time),
            (None, false) => {}
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taps(steps: &[(bool, bool, f64)]) -> Vec<bool> {
        let mut double_shift = DoubleShift::default();
        steps
            .iter()
            .map(|&(shift, other, time)| double_shift.update(shift, other, time))
            .collect()
    }

    #[test]
    fn follows_the_shift_key_through_the_events_of_one_frame() {
        let modifiers = |modifiers| egui::Event::ModifiersChanged(modifiers);
        let shift_key = |pressed| egui::Event::Key {
            key: egui::Key::ShiftLeft,
            physical_key: Some(egui::Key::ShiftLeft),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let tap = [
            shift_key(true),
            modifiers(egui::Modifiers::SHIFT),
            shift_key(false),
            modifiers(egui::Modifiers::NONE),
        ];
        let mut double_shift = DoubleShift::default();
        assert!(!double_shift.follow(&tap, 0.0));
        assert!(double_shift.follow(&tap, 0.2));

        let mut double_shift = DoubleShift::default();
        let typed = [
            modifiers(egui::Modifiers::SHIFT),
            egui::Event::Text("A".to_string()),
            modifiers(egui::Modifiers::NONE),
        ];
        assert!(!double_shift.follow(&typed, 0.0));
        assert!(!double_shift.follow(&tap, 0.2));
        assert!(double_shift.follow(&tap, 0.4));
        assert!(!double_shift.follow(&[modifiers(egui::Modifiers::COMMAND)], 0.5));
    }

    #[test]
    fn notices_shift_pressed_twice_on_its_own() {
        let fired = |steps: &[(bool, bool, f64)]| taps(steps).contains(&true);
        let twice = [
            (true, false, 0.0),
            (false, false, 0.1),
            (true, false, 0.3),
            (false, false, 0.4),
        ];
        assert_eq!(taps(&twice), [false, false, false, true]);
        assert!(!fired(&[
            (true, false, 0.0),
            (false, false, 0.1),
            (true, false, 0.6),
            (false, false, 0.7),
        ]));
        assert!(!fired(&[
            (true, false, 0.0),
            (true, true, 0.05),
            (false, false, 0.1),
            (true, false, 0.2),
            (false, false, 0.3),
        ]));
        assert!(!fired(&[
            (true, false, 0.0),
            (false, false, 0.1),
            (false, true, 0.15),
            (true, false, 0.2),
            (false, false, 0.3),
        ]));
        assert!(!fired(&[
            (true, false, 0.0),
            (false, false, 0.1),
            (true, false, 0.2),
            (true, true, 0.25),
            (false, false, 0.3),
        ]));
    }
}

use eframe::egui::{self, CursorIcon};

pub struct Pointer;

impl egui::Plugin for Pointer {
    fn debug_name(&self) -> &'static str {
        "pointer cursor"
    }

    fn on_end_pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx();
        if ctx.output(|o| o.cursor_icon) != CursorIcon::Default {
            return;
        }
        let hovered = ctx.interaction_snapshot(|i| i.hovered.clone());
        let clickable = hovered
            .into_iter()
            .filter_map(|id| ctx.read_response(id))
            .any(|response| {
                response.enabled() && response.sense.senses_click() && !response.sense.senses_drag()
            });
        if clickable {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
    }
}

pub fn tab(response: &egui::Response) {
    if !response.contains_pointer() && !response.dragged() {
        return;
    }
    let held = response.dragged()
        || (response.is_pointer_button_down_on()
            && response.ctx.input(|i| i.pointer.primary_down()));
    response.ctx.set_cursor_icon(if held {
        CursorIcon::Grabbing
    } else {
        CursorIcon::PointingHand
    });
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::*;

    fn cursor_over(label: &str, add: impl FnMut(&mut egui::Ui) + 'static) -> CursorIcon {
        let mut harness = Harness::builder()
            .with_context_setup(|ctx| ctx.add_plugin(Pointer))
            .build_ui(add);
        harness.get_by_label(label).hover();
        harness.run();
        harness.output().platform_output.cursor_icon
    }

    #[test]
    fn points_at_whatever_can_be_clicked() {
        assert_eq!(
            cursor_over("Button", |ui| {
                let _ = ui.button("Button");
            }),
            CursorIcon::PointingHand
        );
        assert_eq!(
            cursor_over("Selectable", |ui| {
                let _ = ui.selectable_label(false, "Selectable");
            }),
            CursorIcon::PointingHand
        );
        let mut checked = false;
        assert_eq!(
            cursor_over("Check", move |ui| {
                ui.checkbox(&mut checked, "Check");
            }),
            CursorIcon::PointingHand
        );
    }

    #[test]
    fn leaves_what_cannot_be_clicked_and_what_is_disabled_alone() {
        assert_eq!(
            cursor_over("Label", |ui| {
                ui.add(egui::Label::new("Label").selectable(false));
            }),
            CursorIcon::Default
        );
        assert_eq!(
            cursor_over("Disabled", |ui| {
                ui.add_enabled(false, egui::Button::new("Disabled"));
            }),
            CursorIcon::Default
        );
    }
}

use eframe::egui::{self, Color32, RichText, Stroke};

use crate::ui::theme;

const HEIGHT: f32 = 22.0;
const ICON: f32 = 12.0;
const GAP: f32 = 6.0;
const PADDING: f32 = 8.0;
const POPUP_WIDTH: f32 = 380.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Busy,
    Done,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub text: String,
    pub dismissable: bool,
}

impl Item {
    fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            dismissable: false,
        }
    }

    pub fn busy(text: impl Into<String>) -> Self {
        Self::new(Kind::Busy, text)
    }

    pub fn done(text: impl Into<String>) -> Self {
        Self::new(Kind::Done, text)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(Kind::Error, text)
    }

    pub fn dismissable(self) -> Self {
        Self {
            dismissable: true,
            ..self
        }
    }

    fn color(&self, ui: &egui::Ui) -> Color32 {
        match self.kind {
            Kind::Error => ui.visuals().error_fg_color,
            Kind::Busy | Kind::Done => ui.visuals().weak_text_color(),
        }
    }
}

pub fn ui(ui: &mut egui::Ui, items: &[Item], area: egui::Rect) -> Option<usize> {
    let first = items.first()?;
    let more = items.len() - 1;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let painter = ui.painter().clone();
    let badge = (more > 0).then(|| {
        painter.layout_no_wrap(format!("+{more}"), font.clone(), ui.visuals().text_color())
    });
    let badge_width = badge
        .as_ref()
        .map_or(0.0, |badge| badge.size().x + 10.0 + GAP);
    let room = area.width() - PADDING * 2.0 - ICON - GAP - badge_width;
    if room < 24.0 {
        return None;
    }
    let mut job =
        egui::text::LayoutJob::simple_singleline(first.text.clone(), font, first.color(ui));
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let galley = painter.layout_job(job);
    let width = PADDING * 2.0 + ICON + GAP + galley.size().x + badge_width;
    let rect = egui::Rect::from_center_size(area.center(), egui::vec2(width, HEIGHT));
    let response = ui
        .interact(rect, ui.scope_id().with("status"), egui::Sense::click())
        .on_hover_text(if more > 0 {
            format!("{more} more going on")
        } else {
            "What is going on".to_string()
        });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &first.text));
    let popup = egui::Popup::menu(&response)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(POPUP_WIDTH);
    let palette = theme::palette(ui.visuals().dark_mode);
    if response.hovered() || popup.is_open() {
        painter.rect_filled(rect, 5.0, palette.hover);
    }
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + PADDING + ICON / 2.0, rect.center().y),
        egui::vec2(ICON, ICON),
    );
    paint_icon(ui, icon, first.kind);
    let text = egui::pos2(icon.right() + GAP, rect.center().y - galley.size().y / 2.0);
    painter.galley(text, galley, first.color(ui));
    if let Some(badge) = badge {
        let size = badge.size() + egui::vec2(10.0, 2.0);
        let pill = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - PADDING - size.x,
                rect.center().y - size.y / 2.0,
            ),
            size,
        );
        painter.rect_filled(pill, size.y / 2.0, palette.line);
        painter.galley(
            pill.center() - badge.size() / 2.0,
            badge,
            ui.visuals().text_color(),
        );
    }
    popup
        .show(|ui| {
            ui.set_max_width(POPUP_WIDTH);
            let mut dismissed = None;
            for (index, item) in items.iter().enumerate() {
                if row(ui, item) {
                    dismissed = Some(index);
                }
            }
            dismissed
        })
        .and_then(|shown| shown.inner)
}

fn row(ui: &mut egui::Ui, item: &Item) -> bool {
    ui.horizontal_top(|ui| {
        let (icon, _) = ui.allocate_exact_size(egui::vec2(ICON, ICON), egui::Sense::hover());
        paint_icon(ui, icon, item.kind);
        ui.vertical(|ui| {
            ui.add(egui::Label::new(RichText::new(&item.text).color(item.color(ui))).wrap());
            item.dismissable && ui.add(theme::quiet("Dismiss")).clicked()
        })
        .inner
    })
    .inner
}

pub fn paint_icon(ui: &egui::Ui, rect: egui::Rect, kind: Kind) {
    let painter = ui.painter();
    match kind {
        Kind::Busy => theme::spinner(ui, rect),
        Kind::Done => {
            let color = theme::palette(ui.visuals().dark_mode).added;
            let at = |x: f32, y: f32| rect.left_top() + egui::vec2(x, y) * rect.width();
            painter.line(
                vec![at(0.15, 0.55), at(0.4, 0.8), at(0.85, 0.25)],
                Stroke::new(1.5, color),
            );
        }
        Kind::Error => {
            painter.circle_filled(rect.center(), 4.0, ui.visuals().error_fg_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::*;

    struct Shown {
        items: Vec<Item>,
        dismissed: Option<usize>,
    }

    fn show(items: Vec<Item>) -> Harness<'static, Shown> {
        let shown = Shown {
            items,
            dismissed: None,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 300.0))
            .with_context_setup(|ctx| theme::apply(ctx, false))
            .build_ui_state(
                |ui, shown: &mut Shown| {
                    let area =
                        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 30.0));
                    if let Some(index) = super::ui(ui, &shown.items, area) {
                        shown.dismissed = Some(index);
                    }
                },
                shown,
            );
        harness.run_steps(2);
        harness
    }

    #[test]
    fn shows_one_thing_and_how_many_more_are_going_on() {
        let mut harness = show(vec![
            Item::busy("Pushing…"),
            Item::busy("Kotlin: Indexing"),
            Item::error("The Go language server stopped"),
        ]);
        harness.get_by_label("Pushing…");
        assert!(harness.query_by_label("Kotlin: Indexing").is_none());
        harness.get_by_label("Pushing…").click();
        harness.run_steps(2);
        harness.get_by_label("Kotlin: Indexing");
        harness.get_by_label("The Go language server stopped");
    }

    #[test]
    fn dismisses_an_error_from_the_popup() {
        let mut harness = show(vec![
            Item::busy("Kotlin: Indexing"),
            Item::error("Pushing failed").dismissable(),
        ]);
        harness.get_by_label("Kotlin: Indexing").click();
        harness.run_steps(2);
        harness.get_by_label("Dismiss").click();
        harness.run_steps(2);
        assert_eq!(harness.state().dismissed, Some(1));
    }

    #[test]
    fn shows_nothing_when_nothing_is_going_on() {
        let harness = show(Vec::new());
        assert!(harness.query_by_role(egui::Role::Button).is_none());
    }
}

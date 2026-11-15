use eframe::egui::{self, Align2, Color32, FontId, Painter, TextStyle, Ui};

/// The shared look of code rows in diffs and files: monospace text next to a gutter of line numbers.
pub struct CodeStyle {
    pub font: FontId,
    pub row_height: f32,
    pub digit_width: f32,
    pub text_color: Color32,
    pub weak_color: Color32,
}

impl CodeStyle {
    pub fn new(ui: &Ui) -> Self {
        Self::with_font(ui, TextStyle::Monospace.resolve(ui.style()))
    }

    pub fn with_font(ui: &Ui, font: FontId) -> Self {
        let text_color = ui.visuals().text_color();
        let digit_width = ui.painter().layout_no_wrap("0".into(), font.clone(), text_color).size().x;
        Self {
            row_height: ui.fonts_mut(|fonts| fonts.row_height(&font)) + 2.0,
            weak_color: ui.visuals().weak_text_color(),
            font,
            digit_width,
            text_color,
        }
    }

    /// Paints a line number so that it ends at `right`.
    pub fn number(&self, painter: &Painter, right: f32, top: f32, number: Option<u32>) {
        if let Some(number) = number {
            painter.text(egui::pos2(right, top), Align2::RIGHT_TOP, number, self.font.clone(), self.weak_color);
        }
    }

    pub fn text(&self, painter: &Painter, left: f32, top: f32, text: impl ToString, color: Color32) {
        painter.text(egui::pos2(left, top), Align2::LEFT_TOP, text, self.font.clone(), color);
    }
}

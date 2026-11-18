use std::fs;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Shadow, Stroke, Theme,
    Visuals,
};

const UI_FONT: &str = "/System/Library/Fonts/SFNS.ttf";
const CODE_FONT: &str = "/System/Library/Fonts/SFNSMono.ttf";
const UI_BUNDLED: &[u8] = include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
const CODE_BUNDLED: &[u8] = include_bytes!("../assets/fonts/lilex/Lilex-Regular.ttf");
const SYMBOLS: [(&str, &[u8], f32); 2] = [
    (
        "symbols",
        include_bytes!("../assets/fonts/juliamono/JuliaMono-Regular.ttf"),
        1.0,
    ),
    (
        "emoji",
        include_bytes!("../assets/fonts/noto-emoji/NotoEmoji-Regular.ttf"),
        0.81,
    ),
];

pub struct Palette {
    pub dark: bool,
    pub background: Color32,
    pub raised: Color32,
    pub line: Color32,
    pub hover: Color32,
    pub selection: Color32,
    pub text: Color32,
    pub strong: Color32,
    pub weak: Color32,
    pub accent: Color32,
    pub found: Color32,
    pub added: Color32,
    pub removed: Color32,
    pub modified: Color32,
}

pub const DARK: Palette = Palette {
    dark: true,
    background: Color32::from_rgb(30, 31, 34),
    raised: Color32::from_rgb(39, 41, 45),
    line: Color32::from_rgb(45, 47, 51),
    hover: Color32::from_rgb(44, 46, 50),
    selection: Color32::from_rgb(43, 58, 84),
    text: Color32::from_rgb(188, 190, 196),
    strong: Color32::from_rgb(228, 229, 232),
    weak: Color32::from_rgb(111, 115, 122),
    accent: Color32::from_rgb(122, 161, 232),
    found: Color32::from_rgb(196, 152, 84),
    added: Color32::from_rgb(120, 172, 128),
    removed: Color32::from_rgb(212, 124, 118),
    modified: Color32::from_rgb(104, 151, 187),
};

pub const LIGHT: Palette = Palette {
    dark: false,
    background: Color32::from_rgb(250, 250, 249),
    raised: Color32::from_rgb(255, 255, 255),
    line: Color32::from_rgb(229, 229, 226),
    hover: Color32::from_rgb(238, 238, 236),
    selection: Color32::from_rgb(214, 226, 248),
    text: Color32::from_rgb(56, 58, 63),
    strong: Color32::from_rgb(20, 21, 24),
    weak: Color32::from_rgb(138, 141, 147),
    accent: Color32::from_rgb(52, 104, 210),
    found: Color32::from_rgb(214, 160, 40),
    added: Color32::from_rgb(40, 130, 62),
    removed: Color32::from_rgb(196, 58, 50),
    modified: Color32::from_rgb(36, 99, 160),
};

pub fn palette(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

pub fn apply(ctx: &egui::Context, system_fonts: bool) {
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT));
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(10);
        style.spacing.interact_size.y = 22.0;
    });
    ctx.set_fonts(fonts(system_fonts));
}

pub fn primary(ui: &egui::Ui, label: &str) -> egui::Button<'static> {
    let palette = palette(ui.visuals().dark_mode);
    let text = if palette.dark {
        palette.background
    } else {
        Color32::WHITE
    };
    egui::Button::new(egui::RichText::new(label).color(text))
        .fill(palette.accent)
        .stroke(Stroke::NONE)
}

pub fn quiet(label: impl egui::IntoAtoms<'static>) -> egui::Button<'static> {
    egui::Button::new(label).frame_when_inactive(false)
}

pub fn dropdown(label: impl egui::IntoAtoms<'static>) -> egui::Button<'static> {
    quiet(label).right_text(egui::Atom::custom(
        egui::IdSalt::new("caret"),
        egui::vec2(7.0, 7.0),
    ))
}

pub fn caret(ui: &egui::Ui, response: &egui::Response) {
    let rect = response.rect;
    let center = egui::pos2(
        rect.right() - ui.spacing().button_padding.x - 3.5,
        rect.center().y,
    );
    let color = ui.style().interact(response).fg_stroke.color;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            center + egui::vec2(-3.5, -1.5),
            center + egui::vec2(3.5, -1.5),
            center + egui::vec2(0.0, 2.5),
        ],
        color,
        Stroke::NONE,
    ));
}

fn fonts(system_fonts: bool) -> FontDefinitions {
    let mut definitions = FontDefinitions::empty();
    for (name, font, family) in [
        ("bundled-ui", UI_BUNDLED, FontFamily::Proportional),
        ("bundled-code", CODE_BUNDLED, FontFamily::Monospace),
    ] {
        add(
            &mut definitions,
            name,
            Arc::new(FontData::from_static(font)),
            family,
        );
    }
    if system_fonts {
        for (name, path, family) in [
            ("ui", UI_FONT, FontFamily::Proportional),
            ("code", CODE_FONT, FontFamily::Monospace),
        ] {
            let Ok(font) = fs::read(path) else {
                continue;
            };
            add(
                &mut definitions,
                name,
                Arc::new(FontData::from_owned(font)),
                family,
            );
        }
        #[cfg(target_os = "linux")]
        {
            use fontique::GenericFamily;
            if let Some(font) = crate::system_fonts::font(
                &[GenericFamily::SystemUi, GenericFamily::SansSerif],
                false,
                false,
            ) {
                add(
                    &mut definitions,
                    "ui",
                    Arc::new(font),
                    FontFamily::Proportional,
                );
            }
            if let Some(font) = crate::system_fonts::font(&[GenericFamily::Monospace], false, false)
            {
                add(
                    &mut definitions,
                    "code",
                    Arc::new(font),
                    FontFamily::Monospace,
                );
            }
        }
    }
    let symbols = add_symbols(&mut definitions);
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(fonts) = definitions.families.get_mut(&family) {
            fonts.extend(symbols.iter().cloned());
        }
    }
    definitions
}

pub fn add_symbols(definitions: &mut FontDefinitions) -> Vec<String> {
    SYMBOLS
        .iter()
        .map(|(name, font, scale)| {
            let tweak = egui::FontTweak {
                scale: *scale,
                ..Default::default()
            };
            definitions.font_data.insert(
                name.to_string(),
                Arc::new(FontData::from_static(font).tweak(tweak)),
            );
            name.to_string()
        })
        .collect()
}

fn add(definitions: &mut FontDefinitions, name: &str, font: Arc<FontData>, family: FontFamily) {
    definitions.font_data.insert(name.to_string(), font);
    definitions
        .families
        .entry(family)
        .or_default()
        .insert(0, name.to_string());
}

fn visuals(palette: &Palette) -> Visuals {
    let mut visuals = if palette.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let radius = CornerRadius::same(5);
    let shadow = |blur: u8, alpha: u8| Shadow {
        offset: [0, 6],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    };
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.raised;
    visuals.extreme_bg_color = palette.background;
    visuals.text_edit_bg_color = Some(palette.background);
    visuals.faint_bg_color = palette.hover;
    visuals.code_bg_color = palette.raised;
    visuals.window_stroke = Stroke::new(1.0, palette.line);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.window_shadow = shadow(28, if palette.dark { 90 } else { 28 });
    visuals.popup_shadow = shadow(16, if palette.dark { 70 } else { 20 });
    visuals.selection.bg_fill = palette.selection;
    visuals.selection.stroke = Stroke::new(1.0, palette.accent);
    visuals.hyperlink_color = palette.accent;
    visuals.warn_fg_color = palette.found;
    visuals.weak_text_color = Some(palette.weak);
    visuals.text_cursor.stroke = Stroke::new(2.0, palette.accent);
    visuals.indent_has_left_vline = false;
    visuals.striped = false;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = palette.background;
    widgets.noninteractive.weak_bg_fill = palette.background;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.line);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    widgets.inactive.bg_fill = palette.hover;
    widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    widgets.inactive.bg_stroke = Stroke::new(1.0, palette.line);
    widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    for state in [&mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        state.bg_fill = palette.hover;
        state.weak_bg_fill = palette.hover;
        state.bg_stroke = Stroke::new(1.0, palette.line);
        state.fg_stroke = Stroke::new(1.0, palette.strong);
        state.expansion = 0.0;
    }
    for state in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.corner_radius = radius;
    }
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_the_same_fonts_unless_the_system_fonts_are_chosen() {
        let bundled = ctx(false).fonts(|fonts| fonts.definitions().clone());
        assert_eq!(
            bundled.families[&FontFamily::Proportional],
            ["bundled-ui", "symbols", "emoji"]
        );
        assert_eq!(
            bundled.families[&FontFamily::Monospace],
            ["bundled-code", "symbols", "emoji"]
        );
        assert_eq!(bundled.font_data.len(), 4);

        let system = ctx(true).fonts(|fonts| fonts.definitions().families.clone());
        assert_ne!(system[&FontFamily::Proportional][0], "bundled-ui");
        assert_ne!(system[&FontFamily::Monospace][0], "bundled-code");
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert!(system[&family].contains(&"symbols".to_string()));
        }
    }

    fn ctx(system_fonts: bool) -> egui::Context {
        let ctx = egui::Context::default();
        apply(&ctx, system_fonts);
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    #[test]
    fn text_stands_out_from_the_background_in_both_themes() {
        let luminance = |color: Color32| egui::Rgba::from(color).intensity();
        for palette in [&DARK, &LIGHT] {
            let background = luminance(palette.background);
            let contrast = |color: Color32| (luminance(color) - background).abs();
            assert!(contrast(palette.strong) > contrast(palette.text));
            assert!(contrast(palette.text) > contrast(palette.weak));
            assert!(contrast(palette.weak) > contrast(palette.line));
        }
    }
}

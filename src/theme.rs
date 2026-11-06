//! Design system: palette, typography (embedded Inter + Phosphor icons) and egui visuals.

use crate::model::ThemeChoice;
use eframe::egui::{
    self, style::ScrollStyle, Color32, FontData, FontDefinitions, FontFamily, FontId, Rounding, Shadow, Stroke, TextStyle, Theme, Visuals,
};

pub const FONT_MEDIUM: &str = "inter-medium";
pub const FONT_SEMIBOLD: &str = "inter-semibold";

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_MEDIUM.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_SEMIBOLD.into()))
}
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Semantic colors used for custom painting.
#[derive(Clone, Copy, Debug)]
pub struct Pal {
    pub dark: bool,
    /// Main content background.
    pub bg: Color32,
    /// Top bar, sidebar, tab strip.
    pub panel: Color32,
    /// Cards, popups, selected segment.
    pub elevated: Color32,
    /// Text inputs.
    pub input: Color32,
    /// Code / response viewer.
    pub code: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_weak: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

impl Pal {
    pub fn of(ui: &egui::Ui) -> Self {
        Self::new(ui.visuals().dark_mode)
    }

    pub fn new(dark: bool) -> Self {
        if dark {
            Pal {
                dark,
                bg: rgb(0x0f1218),
                panel: rgb(0x131720),
                elevated: rgb(0x1a1f2a),
                input: rgb(0x161b24),
                code: rgb(0x0b0e13),
                hover: rgb(0x1e2430),
                border: rgb(0x252c39),
                border_strong: rgb(0x343c4d),
                text: rgb(0xd5dae3),
                text_strong: rgb(0xf3f5f9),
                text_weak: rgb(0x8b94a7),
                text_faint: rgb(0x5d6679),
                accent: rgb(0x5b8cff),
                accent2: rgb(0x8b5cf6),
                success: rgb(0x3fb950),
                warning: rgb(0xe3a008),
                danger: rgb(0xf8615a),
            }
        } else {
            Pal {
                dark,
                bg: rgb(0xf5f6f8),
                panel: rgb(0xffffff),
                elevated: rgb(0xffffff),
                input: rgb(0xffffff),
                code: rgb(0xf9fafb),
                hover: rgb(0xeef1f5),
                border: rgb(0xe1e5eb),
                border_strong: rgb(0xc9d0da),
                text: rgb(0x2b323d),
                text_strong: rgb(0x0f141b),
                text_weak: rgb(0x667085),
                text_faint: rgb(0x9aa3b2),
                accent: rgb(0x3d6bff),
                accent2: rgb(0x7c4dff),
                success: rgb(0x1a7f37),
                warning: rgb(0x9a6700),
                danger: rgb(0xcf222e),
            }
        }
    }

    pub fn soft(&self, c: Color32) -> Color32 {
        c.gamma_multiply(if self.dark { 0.16 } else { 0.11 })
    }
}

pub fn method_color(method: &str, dark: bool) -> Color32 {
    let (d, l) = match method {
        "GET" => (0x3fb950, 0x1a7f37),
        "POST" => (0xf0b429, 0xa16207),
        "PUT" => (0x5b8cff, 0x2459e0),
        "PATCH" => (0xb083f0, 0x8250df),
        "DELETE" => (0xff6b6b, 0xcf222e),
        "HEAD" => (0x39c5bb, 0x0e7c86),
        "OPTIONS" => (0xf778ba, 0xbf3989),
        _ => (0xaaaaaa, 0x57606a),
    };
    rgb(if dark { d } else { l })
}

/// Short label for narrow places (sidebar, history).
pub fn short_method(method: &str) -> &str {
    match method {
        "DELETE" => "DEL",
        "OPTIONS" => "OPT",
        "PATCH" => "PTCH",
        m => m,
    }
}

pub fn status_color(status: u16, dark: bool) -> Color32 {
    let p = Pal::new(dark);
    match status {
        200..=299 => p.success,
        300..=399 => p.accent,
        400..=499 => rgb(if dark { 0xf0883e } else { 0xbc4c00 }),
        _ => p.danger,
    }
}

/// Syntax highlighting palette.
#[derive(Clone, Copy)]
pub struct Palette {
    pub text: Color32,
    pub key: Color32,
    pub string: Color32,
    pub number: Color32,
    pub keyword: Color32,
    pub punct: Color32,
    pub tag: Color32,
    pub comment: Color32,
    pub var_ok: Color32,
    pub var_missing: Color32,
    pub search_bg: Color32,
    pub search_current_bg: Color32,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Palette {
                text: rgb(0xd5dae3),
                key: rgb(0x8ab4ff),
                string: rgb(0x9ece6a),
                number: rgb(0xff9e64),
                keyword: rgb(0xbb9af7),
                punct: rgb(0x7a8295),
                tag: rgb(0x7dcfff),
                comment: rgb(0x6a7389),
                var_ok: rgb(0x3fb950),
                var_missing: rgb(0xff6b6b),
                search_bg: Color32::from_rgba_unmultiplied(0xe3, 0xa0, 0x08, 80),
                search_current_bg: Color32::from_rgba_unmultiplied(0xf0, 0x88, 0x3e, 200),
            }
        } else {
            Palette {
                text: rgb(0x24292f),
                key: rgb(0x0550ae),
                string: rgb(0x116329),
                number: rgb(0xbc4c00),
                keyword: rgb(0x8250df),
                punct: rgb(0x6e7781),
                tag: rgb(0x0a3069),
                comment: rgb(0x6e7781),
                var_ok: rgb(0x1a7f37),
                var_missing: rgb(0xcf222e),
                search_bg: Color32::from_rgba_unmultiplied(0xff, 0xd3, 0x3d, 140),
                search_current_bg: Color32::from_rgba_unmultiplied(0xff, 0x96, 0x32, 220),
            }
        }
    }
}

pub fn setup(ctx: &egui::Context) {
    setup_fonts(ctx);
    ctx.set_visuals_of(Theme::Dark, visuals(&Pal::new(true)));
    ctx.set_visuals_of(Theme::Light, visuals(&Pal::new(false)));
    ctx.all_styles_mut(tweak_style);
}

pub fn apply_theme(ctx: &egui::Context, theme: ThemeChoice) {
    ctx.set_theme(match theme {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    });
}

fn visuals(p: &Pal) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.elevated;
    v.window_stroke = Stroke::new(1.0_f32, p.border);
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.hover;
    v.code_bg_color = p.code;
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(10.0);
    let shadow_alpha = if p.dark { 150 } else { 38 };
    v.window_shadow = Shadow { offset: egui::vec2(0.0, 12.0), blur: 36.0, spread: 0.0, color: Color32::from_black_alpha(shadow_alpha) };
    v.popup_shadow = Shadow { offset: egui::vec2(0.0, 6.0), blur: 20.0, spread: 0.0, color: Color32::from_black_alpha(shadow_alpha) };
    v.selection.bg_fill = p.accent.gamma_multiply(if p.dark { 0.38 } else { 0.24 });
    v.selection.stroke = Stroke::new(1.0_f32, p.accent);
    v.hyperlink_color = p.accent;
    v.text_cursor.stroke = Stroke::new(2.0_f32, p.accent);
    v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    v.striped = false;

    let r = Rounding::same(7.0);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg;
    w.noninteractive.weak_bg_fill = p.elevated;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.rounding = r;

    w.inactive.bg_fill = p.hover;
    w.inactive.weak_bg_fill = p.elevated;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.inactive.rounding = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    w.hovered.fg_stroke = Stroke::new(1.5_f32, p.text_strong);
    w.hovered.rounding = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.soft(p.accent);
    w.active.weak_bg_fill = p.soft(p.accent);
    w.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    w.active.fg_stroke = Stroke::new(1.5_f32, p.text_strong);
    w.active.rounding = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0_f32, p.border_strong);
    w.open.fg_stroke = Stroke::new(1.0_f32, p.text_strong);
    w.open.rounding = r;
    v
}

fn tweak_style(style: &mut egui::Style) {
    style.text_styles = [
        (TextStyle::Heading, semibold(18.0)),
        (TextStyle::Body, regular(13.5)),
        (TextStyle::Button, medium(13.0)),
        (TextStyle::Monospace, mono(12.5)),
        (TextStyle::Small, regular(11.5)),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(10.0, 5.0);
    s.interact_size.y = 28.0;
    s.combo_height = 320.0;
    s.menu_margin = egui::Margin::same(6.0);
    s.window_margin = egui::Margin::same(18.0);
    s.scroll = ScrollStyle::floating();
    s.scroll.floating_width = 8.0;
}

/// Inter (embedded, OFL) in three weights + Phosphor icons, with system fallbacks.
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.otf")));
    fonts.font_data.insert(FONT_MEDIUM.into(), FontData::from_static(include_bytes!("../assets/fonts/Inter-Medium.otf")));
    fonts.font_data.insert(FONT_SEMIBOLD.into(), FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.otf")));
    if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
        prop.insert(0, "inter".into());
    }
    // Phosphor goes right after the primary font of the Proportional family.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(data) = fonts.font_data.get_mut("phosphor") {
        data.tweak.y_offset_factor = 0.08;
    }

    const SYMBOL_FALLBACKS: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
    ];
    if let Some(bytes) = SYMBOL_FALLBACKS.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("symbols".into(), FontData::from_owned(bytes));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push("symbols".into());
            }
        }
    }

    let fallbacks: Vec<String> = fonts.families.get(&FontFamily::Proportional).map(|l| l[1..].to_vec()).unwrap_or_default();
    for name in [FONT_MEDIUM, FONT_SEMIBOLD] {
        let mut list = vec![name.to_string()];
        list.extend(fallbacks.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), list);
    }
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("phosphor".into());
    }
    ctx.set_fonts(fonts);
}

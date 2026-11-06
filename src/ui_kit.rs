//! Custom-painted building blocks for a consistent, polished look.

use crate::theme::{self, Pal};
use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular as ph;

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

fn rounded_points(rect: Rect, r: f32) -> Vec<Pos2> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (rect.right() - r, rect.top() + r, -90.0_f32),
        (rect.right() - r, rect.bottom() - r, 0.0),
        (rect.left() + r, rect.bottom() - r, 90.0),
        (rect.left() + r, rect.top() + r, 180.0),
    ];
    let mut pts = Vec::with_capacity(28);
    for (cx, cy, start) in corners {
        for i in 0..=6 {
            let a = (start + 90.0 * i as f32 / 6.0).to_radians();
            pts.push(egui::pos2(cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    pts
}

/// Rounded rectangle with a horizontal gradient.
pub fn gradient_rect(painter: &egui::Painter, rect: Rect, rounding: f32, left: Color32, right: Color32) {
    let pts = rounded_points(rect, rounding);
    let w = rect.width().max(1.0);
    let col = |p: Pos2| lerp_color(left, right, ((p.x - rect.left()) / w).clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), col(rect.center()));
    for p in &pts {
        mesh.colored_vertex(*p, col(*p));
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
    // anti-aliased edge
    painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0_f32, lerp_color(left, right, 0.5).gamma_multiply(0.7))));
}

/// Gradient call-to-action button.
pub fn primary_button(ui: &mut Ui, icon: Option<&str>, text: &str, min_size: Vec2) -> Response {
    let p = Pal::of(ui);
    let label = match icon {
        Some(i) => format!("{i}  {text}"),
        None => text.to_string(),
    };
    let galley = ui.painter().layout_no_wrap(label, theme::semibold(13.5), Color32::WHITE);
    let size = egui::vec2((galley.size().x + 30.0).max(min_size.x), min_size.y.max(30.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let (mut a, mut b) = (p.accent, p.accent2);
        if !ui.is_enabled() {
            a = lerp_color(a, p.bg, 0.55);
            b = lerp_color(b, p.bg, 0.55);
        } else if resp.is_pointer_button_down_on() {
            a = lerp_color(a, Color32::BLACK, 0.15);
            b = lerp_color(b, Color32::BLACK, 0.15);
        } else if resp.hovered() {
            a = lerp_color(a, Color32::WHITE, 0.12);
            b = lerp_color(b, Color32::WHITE, 0.12);
        }
        gradient_rect(ui.painter(), rect, 8.0, a, b);
        let text_color = if ui.is_enabled() { Color32::WHITE } else { Color32::from_white_alpha(170) };
        ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, text_color);
    }
    resp
}

/// Standard bordered button with an optional leading icon.
pub fn button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    let label = if icon.is_empty() {
        text.to_string()
    } else if text.is_empty() {
        icon.to_string()
    } else {
        format!("{icon}  {text}")
    };
    ui.add(egui::Button::new(label).min_size(egui::vec2(0.0, 30.0)))
}

/// Frameless icon button with a hover background.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    icon_toggle(ui, false, icon, tooltip)
}

pub fn icon_toggle(ui: &mut Ui, selected: bool, icon: &str, tooltip: &str) -> Response {
    let p = Pal::of(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 7.0, p.soft(p.accent));
        } else if resp.hovered() {
            painter.rect_filled(rect, 7.0, p.hover);
        }
        let color = if selected {
            p.accent
        } else if resp.hovered() {
            p.text_strong
        } else {
            p.text_weak
        };
        painter.text(rect.center(), Align2::CENTER_CENTER, icon, theme::regular(16.0), color);
    }
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
}

/// Icon button at an exact rect without touching the layout cursor.
pub fn icon_button_at(ui: &mut Ui, rect: Rect, id: Id, icon: &str) -> Response {
    let p = Pal::of(ui);
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, p.border);
    }
    let color = if resp.hovered() { p.text_strong } else { p.text_weak };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::regular(16.0), color);
    resp
}

/// Small pill with tinted background.
pub fn chip(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    let p = Pal::of(ui);
    let galley = ui.painter().layout_no_wrap(text.into(), theme::medium(11.5), color);
    let size = galley.size() + egui::vec2(16.0, 7.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, rect.height() / 2.0, p.soft(color));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// Method label in a tinted pill, painted at `left_center`. Returns its rect.
pub fn paint_method_badge(ui: &Ui, left_center: Pos2, method: &str, width: f32) -> Rect {
    let p = Pal::of(ui);
    let color = theme::method_color(method, p.dark);
    let rect = Rect::from_min_size(egui::pos2(left_center.x, left_center.y - 9.0), egui::vec2(width, 18.0));
    ui.painter().rect_filled(rect, 5.0, p.soft(color));
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, theme::short_method(method), theme::semibold(10.0), color);
    rect
}

/// Segmented control. Returns true when the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, id: Id, value: &mut T, options: &[(T, &str)], full_width: bool) -> bool {
    let p = Pal::of(ui);
    let font = theme::medium(12.5);
    let galleys: Vec<_> = options.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), p.text)).collect();
    let (pad, h, inset) = (12.0, 30.0, 3.0);
    let natural: Vec<f32> = galleys.iter().map(|g| g.size().x + pad * 2.0).collect();
    let total = if full_width { ui.available_width() } else { natural.iter().sum::<f32>() + inset * 2.0 };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(total, h), Sense::hover());
    let painter = ui.painter().clone();
    let container = if p.dark { p.bg } else { p.hover };
    painter.rect(rect, 9.0, container, Stroke::new(1.0_f32, p.border));
    let each = (total - inset * 2.0) / options.len().max(1) as f32;
    let mut x = rect.left() + inset;
    let mut changed = false;
    for (i, ((val, _), galley)) in options.iter().zip(galleys).enumerate() {
        let w = if full_width { each } else { natural[i] };
        let seg = Rect::from_min_size(egui::pos2(x, rect.top() + inset), egui::vec2(w, h - inset * 2.0));
        let resp = ui.interact(seg, id.with(i), Sense::click());
        let selected = *value == *val;
        if selected {
            let fill = if p.dark { p.border } else { p.elevated };
            painter.rect(seg, 7.0, fill, Stroke::new(1.0_f32, if p.dark { p.border_strong } else { p.border }));
        } else if resp.hovered() {
            painter.rect_filled(seg, 7.0, p.hover);
        }
        let color = if selected { p.text_strong } else { p.text_weak };
        let pos = egui::pos2((seg.center().x - galley.size().x / 2.0).max(seg.left() + 4.0), seg.center().y - galley.size().y / 2.0);
        painter.with_clip_rect(seg.shrink(2.0)).galley_with_override_text_color(pos, galley, color);
        if resp.clicked() && !selected {
            *value = *val;
            changed = true;
        }
        x += w;
    }
    changed
}

/// iOS-style switch with a clickable label.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str, hint: &str) -> bool {
    let p = Pal::of(ui);
    let mut changed = false;
    ui.horizontal(|ui| {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(36.0, 20.0), Sense::click());
        let label_resp = ui.add(egui::Label::new(egui::RichText::new(label).color(p.text_strong)).sense(Sense::click()));
        if !hint.is_empty() {
            ui.label(egui::RichText::new(hint).small().color(p.text_weak));
        }
        if resp.clicked() || label_resp.clicked() {
            *on = !*on;
            changed = true;
        }
        let t = ui.ctx().animate_bool_with_time(resp.id, *on, 0.12);
        let bg = lerp_color(p.border_strong, p.accent, t);
        ui.painter().rect_filled(rect, 10.0, bg);
        let cx = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
        ui.painter().circle_filled(egui::pos2(cx, rect.center().y), 7.5, Color32::WHITE);
    });
    changed
}

pub fn initials(name: &str) -> String {
    let mut out: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    if out.is_empty() {
        out.push('?');
    }
    out.to_uppercase()
}

pub fn avatar_color(name: &str) -> Color32 {
    const COLORS: [u32; 8] = [0x5b8cff, 0x8b5cf6, 0xe0598b, 0xf0883e, 0x2ea98f, 0x3fa7d6, 0xc678dd, 0x58b368];
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    let c = COLORS[(h % COLORS.len() as u32) as usize];
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

pub fn avatar(ui: &mut Ui, name: &str, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let c = avatar_color(name);
    ui.painter().circle_filled(rect.center(), size / 2.0, c);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, initials(name), theme::semibold(size * 0.38), Color32::WHITE);
    resp.on_hover_text(name)
}

/// Keyboard key cap, e.g. `Ctrl`.
pub fn keycap(ui: &mut Ui, key: &str) {
    let p = Pal::of(ui);
    let galley = ui.painter().layout_no_wrap(key.to_string(), theme::medium(11.0), p.text);
    let (rect, _) = ui.allocate_exact_size(galley.size() + egui::vec2(12.0, 6.0), Sense::hover());
    ui.painter().rect(rect, 5.0, p.elevated, Stroke::new(1.0_f32, p.border_strong));
    ui.painter().hline(rect.shrink2(egui::vec2(3.0, 0.0)).x_range(), rect.bottom() - 0.5, Stroke::new(1.5_f32, p.border_strong));
    ui.painter().galley(rect.center() - galley.size() / 2.0 - egui::vec2(0.0, 0.5), galley, p.text);
}

pub fn card_frame(p: &Pal) -> egui::Frame {
    egui::Frame::none().fill(p.elevated).stroke(Stroke::new(1.0_f32, p.border)).rounding(10.0).inner_margin(egui::Margin::same(14.0))
}

/// App logo: gradient tile with a paper plane.
pub fn logo(ui: &mut Ui, size: f32) -> Response {
    let p = Pal::of(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    gradient_rect(ui.painter(), rect, size * 0.28, p.accent, p.accent2);
    ui.painter().text(
        rect.center() + egui::vec2(-0.5, 0.5),
        Align2::CENTER_CENTER,
        ph::PAPER_PLANE_TILT,
        theme::regular(size * 0.62),
        Color32::WHITE,
    );
    resp
}

/// Large centered placeholder with an icon in a soft halo.
pub fn hero(ui: &mut Ui, icon: &str, title: &str, subtitle: &str) {
    let p = Pal::of(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.22).max(8.0));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(76.0, 76.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 38.0, p.soft(p.accent).gamma_multiply(0.6));
        ui.painter().circle_filled(rect.center(), 27.0, p.soft(p.accent));
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::regular(28.0), p.accent);
        ui.add_space(12.0);
        ui.label(egui::RichText::new(title).font(theme::semibold(16.0)).color(p.text_strong));
        if !subtitle.is_empty() {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(subtitle).color(p.text_weak));
        }
    });
}

/// Search box with a leading magnifier icon and a clear button.
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, id: Id) -> Response {
    let p = Pal::of(ui);
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = Stroke::new(1.0_f32, if focused { p.accent } else { p.border });
    let inner = egui::Frame::none()
        .fill(p.input)
        .stroke(stroke)
        .rounding(8.0)
        .inner_margin(egui::Margin { left: 28.0, right: 4.0, top: 1.0, bottom: 1.0 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !text.is_empty() && icon_button(ui, ph::X, "Clear").clicked() {
                        text.clear();
                    }
                    ui.add(egui::TextEdit::singleline(text).id(id).frame(false).hint_text(hint).desired_width(f32::INFINITY))
                })
                .inner
            })
            .inner
        });
    let r = inner.response.rect;
    ui.painter().text(
        egui::pos2(r.left() + 15.0, r.center().y),
        Align2::CENTER_CENTER,
        ph::MAGNIFYING_GLASS,
        theme::regular(14.0),
        p.text_weak,
    );
    inner.inner
}

/// Text tab with an accent underline when selected and an optional count chip.
pub fn underline_tab(ui: &mut Ui, selected: bool, label: &str, badge: Option<usize>, dot: bool) -> Response {
    let p = Pal::of(ui);
    let color = if selected { p.text_strong } else { p.text_weak };
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::medium(13.0), color);
    let badge_g = badge.filter(|n| *n > 0).map(|n| ui.painter().layout_no_wrap(n.to_string(), theme::semibold(10.5), p.accent));
    let badge_w = badge_g.as_ref().map(|g| g.size().x + 12.0 + 6.0).unwrap_or(0.0) + if dot { 10.0 } else { 0.0 };
    let pad = egui::vec2(8.0, 7.0);
    let size = egui::vec2(galley.size().x + badge_w, galley.size().y) + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    if resp.hovered() && !selected {
        painter.rect_filled(rect.shrink2(egui::vec2(0.0, 2.0)), 6.0, p.hover);
    }
    let text_pos = rect.min + pad;
    let gw = galley.size().x;
    painter.galley(text_pos, galley, color);
    let mut x = text_pos.x + gw + 6.0;
    if let Some(g) = badge_g {
        let br = Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), egui::vec2(g.size().x + 12.0, 16.0));
        painter.rect_filled(br, 8.0, p.soft(p.accent));
        painter.galley(br.center() - g.size() / 2.0, g, p.accent);
        x = br.right() + 6.0;
    }
    if dot {
        painter.circle_filled(egui::pos2(x + 2.0, rect.center().y), 3.0, p.success);
    }
    if selected {
        let line = Rect::from_min_max(egui::pos2(rect.left() + 4.0, rect.bottom() - 2.0), egui::pos2(rect.right() - 4.0, rect.bottom()));
        gradient_rect(painter, line, 1.0, p.accent, p.accent2);
    }
    resp
}

/// Thin horizontal divider in the border color.
pub fn divider(ui: &mut Ui) {
    let p = Pal::of(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0_f32, p.border));
}

/// Clickable card with an icon, title and subtitle (welcome screen).
pub fn action_card(ui: &mut Ui, icon: &str, title: &str, subtitle: &str, width: f32) -> Response {
    let p = Pal::of(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 112.0), Sense::click());
    let hovered = resp.hovered();
    let lift = ui.ctx().animate_bool_with_time(resp.id, hovered, 0.12);
    let r = rect.translate(egui::vec2(0.0, -2.0 * lift));
    let painter = ui.painter();
    painter.rect(r, 12.0, p.elevated, Stroke::new(1.0_f32, lerp_color(p.border, p.accent, lift)));
    let c = egui::pos2(r.left() + 30.0, r.top() + 32.0);
    painter.circle_filled(c, 16.0, p.soft(p.accent));
    painter.text(c, Align2::CENTER_CENTER, icon, theme::regular(17.0), p.accent);
    painter.text(egui::pos2(r.left() + 16.0, r.top() + 64.0), Align2::LEFT_CENTER, title, theme::semibold(14.0), p.text_strong);
    painter.text(egui::pos2(r.left() + 16.0, r.top() + 86.0), Align2::LEFT_CENTER, subtitle, theme::regular(12.0), p.text_weak);
    resp
}

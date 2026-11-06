//! Response panel: status line, body viewer (virtualised, highlighted, searchable), headers.

use crate::app::{Action, BodyView, RequestTab, RespSection, RespState, RespUi};
use crate::response::{find_matches, format_duration, format_size, ContentKind, Match, Row};
use crate::theme::{self, Pal, Palette};
use crate::ui_request::EditorEnv;
use crate::{highlight, ui_kit};
use eframe::egui::{self, Color32, Key, RichText, Sense, Stroke, TextStyle};
use egui_phosphor::regular as ph;
use std::time::Duration;

pub fn show(ui: &mut egui::Ui, tab: &mut RequestTab, env: &EditorEnv, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    let tab_id = tab.id;
    match &mut tab.resp {
        RespState::Idle => idle(ui, &p),
        RespState::Loading { started, .. } => {
            let elapsed = started.elapsed();
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.28).max(10.0));
                ui.add(egui::Spinner::new().size(30.0).color(p.accent));
                ui.add_space(10.0);
                ui.label(RichText::new("Sending request…").font(theme::semibold(15.0)).color(p.text_strong));
                ui.label(RichText::new(format_duration(elapsed)).color(p.text_weak));
                ui.add_space(8.0);
                if ui_kit::button(ui, ph::X_CIRCLE, "Cancel").clicked() {
                    actions.push(Action::Cancel(tab_id));
                }
            });
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        RespState::Failed(msg) => {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.15).max(10.0));
                let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), Sense::hover());
                ui.painter().circle_filled(rect.center(), 32.0, p.soft(p.danger));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, ph::WARNING, theme::regular(28.0), p.danger);
                ui.add_space(10.0);
                ui.label(RichText::new("Could not get a response").font(theme::semibold(16.0)).color(p.text_strong));
                ui.add_space(8.0);
                ui.set_max_width(640.0);
                egui::Frame::none()
                    .fill(p.code)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(8.0)
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(msg.as_str()).font(theme::mono(12.5)).color(p.text)).selectable(true).wrap());
                    });
            });
        }
        RespState::Done(r) => response_view(ui, tab_id, r, env, actions),
    }
}

fn idle(ui: &mut egui::Ui, p: &Pal) {
    ui_kit::hero(ui, ph::PAPER_PLANE_TILT, "Ready when you are", "Enter a URL and hit Send to see the response here.");
    ui.add_space(10.0);
    ui.vertical_centered(|ui| {
        ui.horizontal(|ui| {
            let total = 230.0;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            ui_kit::keycap(ui, ui_kit::CMD);
            ui_kit::keycap(ui, "Enter");
            ui.label(RichText::new("send   ·").color(p.text_weak));
            ui_kit::keycap(ui, ui_kit::CMD);
            ui_kit::keycap(ui, "S");
            ui.label(RichText::new("save").color(p.text_weak));
        });
    });
}

fn meta_chip(ui: &mut egui::Ui, icon: &str, text: String, color: Color32) -> egui::Response {
    ui_kit::chip(ui, format!("{icon} {text}"), color)
}

fn response_view(ui: &mut egui::Ui, tab_id: u64, r: &mut RespUi, env: &EditorEnv, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let n_headers = r.data.resp.headers.len();
        for (sec, label, badge) in
            [(RespSection::Body, "Body", None), (RespSection::Headers, "Headers", Some(n_headers)), (RespSection::Request, "Request", None)]
        {
            if ui_kit::underline_tab(ui, r.section == sec, label, badge, false).clicked() {
                r.section = sec;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let resp = &r.data.resp;
            meta_chip(ui, ph::FILE_TEXT, format_size(resp.body.len()), p.text_weak).on_hover_text("Body size (decompressed)");
            meta_chip(ui, ph::TIMER, format_duration(resp.elapsed), p.text_weak).on_hover_text("Total time");
            let c = theme::status_color(resp.status, p.dark);
            meta_chip(ui, ph::CIRCLE, format!("{} {}", resp.status, resp.reason), c)
                .on_hover_text(format!("{} · {}", resp.version, resp.url));
        });
    });
    ui_kit::divider(ui);
    ui.add_space(8.0);
    match r.section {
        RespSection::Body => body_view(ui, tab_id, r, env, actions),
        RespSection::Headers => headers_list(ui, &r.data.resp.headers, ("resp_headers", tab_id)),
        RespSection::Request => request_view(ui, tab_id, r),
    }
}

fn body_view(ui: &mut egui::Ui, tab_id: u64, r: &mut RespUi, env: &EditorEnv, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    let kind = r.data.kind;
    let has_image = r.texture.is_some();
    ui.horizontal(|ui| {
        let mut options: Vec<(BodyView, &str)> = Vec::new();
        if kind != ContentKind::Binary {
            options.push((BodyView::Pretty, "Pretty"));
            options.push((BodyView::Raw, "Raw"));
        }
        if has_image {
            options.push((BodyView::Preview, "Preview"));
        }
        if !options.is_empty() {
            ui_kit::segmented(ui, egui::Id::new(("body_view", tab_id)), &mut r.view, &options, false);
        }
        ui_kit::chip(ui, kind.label(), p.accent);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui_kit::icon_button(ui, ph::DOWNLOAD_SIMPLE, "Save response to a file").clicked() {
                actions.push(Action::SaveResponse(tab_id));
            }
            if ui_kit::icon_button(ui, ph::COPY, "Copy response body").clicked() {
                let (text, _) = r.text_rows();
                actions.push(Action::CopyText(text.to_string(), "Response body"));
            }
            if ui_kit::icon_toggle(ui, r.search_open, ph::MAGNIFYING_GLASS, &format!("Find ({})", ui_kit::shortcut(&[ui_kit::CMD, "F"])))
                .clicked()
            {
                r.search_open = !r.search_open;
                r.focus_search = r.search_open;
            }
        });
    });
    ui.add_space(6.0);
    let showing_text = !(r.view == BodyView::Preview && has_image) && !matches!(kind, ContentKind::Binary | ContentKind::Image);
    if r.search_open && showing_text {
        search_bar(ui, r);
        ui.add_space(6.0);
    }
    if r.view == BodyView::Preview && has_image {
        image_view(ui, r);
    } else if matches!(kind, ContentKind::Binary | ContentKind::Image) {
        binary_info(ui, tab_id, r, actions);
    } else {
        text_view(ui, tab_id, r, env);
    }
}

fn search_bar(ui: &mut egui::Ui, r: &mut RespUi) {
    let p = Pal::of(ui);
    ui.horizontal(|ui| {
        let id = egui::Id::new("response_search");
        let field = ui
            .allocate_ui_with_layout(egui::vec2(300.0, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui_kit::search_field(ui, &mut r.search, "Find in response", id)
            })
            .inner;
        if r.focus_search {
            ui.memory_mut(|m| m.request_focus(id));
            r.focus_search = false;
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let shift = ui.input(|i| i.modifiers.shift);
        let key = (r.search.clone(), r.view);
        if r.searched.as_ref() != Some(&key) {
            let matches = {
                let (text, rows) = r.text_rows();
                find_matches(text, rows, &r.search, 20_000)
            };
            r.matches = matches;
            r.current = 0;
            r.scroll_to = r.matches.first().map(|m| m.row);
            r.searched = Some(key);
        }
        let n = r.matches.len();
        let prev = ui_kit::icon_button(ui, ph::CARET_UP, &format!("Previous ({})", ui_kit::shortcut(&[ui_kit::SHIFT, "Enter"]))).clicked();
        let next = ui_kit::icon_button(ui, ph::CARET_DOWN, "Next (Enter)").clicked();
        if n > 0 && (next || prev || enter) {
            r.current = if prev || (enter && shift) { (r.current + n - 1) % n } else { (r.current + 1) % n };
            r.scroll_to = Some(r.matches[r.current].row);
        }
        if enter {
            field.request_focus();
        }
        if !r.search.is_empty() {
            let (text, color) =
                if n == 0 { ("No results".to_string(), p.danger) } else { (format!("{} of {}", r.current + 1, n), p.text_weak) };
            ui.label(RichText::new(text).color(color));
        }
        if ui_kit::icon_button(ui, ph::X, "Close search").clicked() {
            r.search_open = false;
        }
    });
}

/// Background highlights for the search matches that intersect `row`.
fn row_highlights(matches: &[Match], row: Row, current: usize, pal: &Palette) -> Vec<(usize, usize, Color32)> {
    let mut out = Vec::new();
    let mut i = matches.partition_point(|m| m.end <= row.start);
    while i < matches.len() && matches[i].start < row.end {
        let m = matches[i];
        let bg = if i == current { pal.search_current_bg } else { pal.search_bg };
        out.push((m.start.max(row.start) - row.start, m.end.min(row.end) - row.start, bg));
        i += 1;
    }
    out
}

/// Virtualised viewer: only visible rows are laid out, so huge bodies stay fast.
fn text_view(ui: &mut egui::Ui, tab_id: u64, r: &mut RespUi, env: &EditorEnv) {
    let p = Pal::of(ui);
    let scroll_to = r.scroll_to.take();
    let pal = Palette::new(env.dark);
    let font = TextStyle::Monospace.resolve(ui.style());
    let row_h = ui.fonts(|f| f.row_height(&font)) + 4.0;
    let kind = if r.view == BodyView::Raw { ContentKind::Text } else { r.data.kind };
    let highlight_search = r.search_open && !r.search.is_empty();
    let (text, rows) = r.text_rows();
    if text.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label(RichText::new(format!("{}  Response body is empty", ph::TRAY)).color(p.text_weak));
        });
        return;
    }
    let max_line = rows.iter().rev().find_map(|r| r.line).unwrap_or(1);
    let char_w = ui.fonts(|f| f.glyph_width(&font, '0'));
    let gutter = max_line.to_string().len() as f32 * char_w + 20.0;

    let frame = egui::Frame::none()
        .fill(p.code)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(0.0, 6.0));
    frame.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.spacing_mut().interact_size.y = row_h;
        let gutter_x = ui.max_rect().left() + gutter;
        ui.painter().vline(gutter_x, ui.max_rect().y_range(), Stroke::new(1.0_f32, p.border));
        let mut area = egui::ScrollArea::both().id_salt(("resp_body", tab_id, r.view as u8)).auto_shrink([false, false]);
        if let Some(row) = scroll_to {
            area = area.vertical_scroll_offset(((row as f32) - 4.0).max(0.0) * row_h);
        }
        area.show_rows(ui, row_h, rows.len(), |ui, range| {
            for idx in range {
                let row = rows[idx];
                let line = &text[row.start..row.end];
                let hl = if highlight_search { row_highlights(&r.matches, row, r.current, &pal) } else { Vec::new() };
                ui.horizontal(|ui| {
                    let (g, _) = ui.allocate_exact_size(egui::vec2(gutter, row_h), Sense::hover());
                    if let Some(n) = row.line {
                        ui.painter().text(
                            g.right_center() - egui::vec2(10.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            n.to_string(),
                            font.clone(),
                            p.text_faint,
                        );
                    }
                    ui.add_space(4.0);
                    let job = highlight::line_job(line, kind, &pal, &font, &hl);
                    ui.add(egui::Label::new(job).selectable(true).extend());
                });
            }
        });
    });
}

fn image_view(ui: &mut egui::Ui, r: &RespUi) {
    let p = Pal::of(ui);
    let Some(tex) = &r.texture else { return };
    if let Some([w, h]) = r.data.image_size {
        ui.label(
            RichText::new(format!("{}  {w} × {h} px · {}", ph::IMAGE, format_size(r.data.resp.body.len()))).small().color(p.text_weak),
        );
    }
    egui::Frame::none().fill(p.code).stroke(Stroke::new(1.0_f32, p.border)).rounding(10.0).inner_margin(egui::Margin::same(12.0)).show(
        ui,
        |ui| {
            egui::ScrollArea::both().id_salt("image_view").auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::Image::new(egui::load::SizedTexture::from_handle(tex)).max_size(ui.available_size()).shrink_to_fit());
            });
        },
    );
}

fn binary_info(ui: &mut egui::Ui, tab_id: u64, r: &RespUi, actions: &mut Vec<Action>) {
    let ct = crate::http::header_value(&r.data.resp.headers, "content-type").unwrap_or("unknown type").to_string();
    ui_kit::hero(ui, ph::FILE_ARCHIVE, "Binary response", &format!("{} · {}", format_size(r.data.resp.body.len()), ct));
    ui.vertical_centered(|ui| {
        ui.add_space(8.0);
        if ui_kit::primary_button(ui, Some(ph::DOWNLOAD_SIMPLE), "Save to file…", egui::vec2(0.0, 32.0)).clicked() {
            actions.push(Action::SaveResponse(tab_id));
        }
    });
}

fn headers_list(ui: &mut egui::Ui, headers: &[(String, String)], id: impl std::hash::Hash) {
    let p = Pal::of(ui);
    if headers.is_empty() {
        ui.label(RichText::new("No headers").color(p.text_weak));
        return;
    }
    egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::none().fill(p.input).stroke(Stroke::new(1.0_f32, p.border)).rounding(8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let key_w = (ui.available_width() * 0.32).clamp(140.0, 280.0);
            for (i, (k, v)) in headers.iter().enumerate() {
                if i > 0 {
                    let y = ui.cursor().top();
                    ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0_f32, p.border));
                }
                egui::Frame::none().inner_margin(egui::Margin::symmetric(12.0, 7.0)).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui_with_layout(egui::vec2(key_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ui.set_width(key_w);
                            ui.add(
                                egui::Label::new(RichText::new(k).font(theme::medium(13.0)).color(p.text_strong)).selectable(true).wrap(),
                            );
                        });
                        ui.add(egui::Label::new(RichText::new(v).font(theme::mono(12.5)).color(p.text)).selectable(true).wrap());
                    });
                });
            }
        });
    });
}

fn request_view(ui: &mut egui::Ui, tab_id: u64, r: &RespUi) {
    let p = Pal::of(ui);
    let d = &r.data;
    ui_kit::card_frame(&p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&d.request_method).font(theme::semibold(13.0)).color(theme::method_color(&d.request_method, p.dark)));
            ui.add(egui::Label::new(RichText::new(&d.request_url).font(theme::mono(12.5)).color(p.text_strong)).selectable(true).wrap());
        });
        if d.resp.url != d.request_url {
            ui.label(RichText::new(format!("{}  Redirected to {}", ph::ARROW_BEND_DOWN_RIGHT, d.resp.url)).color(p.text_weak));
        }
        ui.label(RichText::new(format!("Protocol: {}", d.resp.version)).small().color(p.text_weak));
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Request headers").font(theme::semibold(13.0)).color(p.text_strong));
    ui.label(RichText::new("Host, Content-Length, Accept-Encoding and Cookie are added automatically.").small().color(p.text_weak));
    ui.add_space(4.0);
    headers_list(ui, &d.request_headers, ("req_headers", tab_id));
}

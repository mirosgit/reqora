//! Reusable composite widgets.

use crate::model::{FieldKind, KeyValue};
use crate::theme::{self, Pal};
use crate::ui_kit;
use eframe::egui::{self, Color32, FontId, Galley, Rect, RichText, Stroke, Ui};
use egui_phosphor::regular as ph;
use std::sync::Arc;

/// Single-line galley elided with "…" to `max_width`.
pub fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap =
        egui::text::TextWrapping { max_width: max_width.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.fonts(|f| f.layout_job(job))
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

pub struct KvOptions<'a> {
    pub id: egui::Id,
    pub key_hint: &'a str,
    pub value_hint: &'a str,
    pub allow_files: bool,
}

#[derive(Default)]
pub struct KvResult {
    pub changed: bool,
    /// Row whose "Select file" button was clicked.
    pub pick_file: Option<usize>,
}

const ROW_H: f32 = 34.0;
const HEAD_H: f32 = 28.0;

/// Editable key/value table. Always keeps one blank row at the end for adding.
pub fn kv_table(ui: &mut Ui, rows: &mut Vec<KeyValue>, opts: KvOptions) -> KvResult {
    let mut res = KvResult::default();
    if rows.last().is_none_or(|r| !r.is_blank()) {
        rows.push(KeyValue::default());
    }
    let p = Pal::of(ui);
    let mut remove = None;
    egui::ScrollArea::vertical().id_salt(opts.id).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        let width = ui.available_width() - 1.0;
        let check_w = 36.0;
        let del_w = 36.0;
        let type_w = if opts.allow_files { 84.0 } else { 0.0 };
        let text_w = (width - check_w - del_w - type_w).max(160.0);
        let key_w = (text_w * 0.38).floor();
        let val_w = text_w - key_w;
        let left = ui.cursor().left();
        let top = ui.cursor().top();
        let xs = [left + check_w, left + check_w + key_w, left + check_w + key_w + type_w, left + check_w + key_w + type_w + val_w];

        let bg_idx = ui.painter().add(egui::Shape::Noop);
        let grid_idx = ui.painter().add(egui::Shape::Noop);

        // header
        ui.horizontal(|ui| {
            ui.set_height(HEAD_H);
            let head = |ui: &mut Ui, w: f32, t: &str| {
                ui.allocate_ui_with_layout(egui::vec2(w, HEAD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(w);
                    ui.add_space(10.0);
                    ui.label(RichText::new(t).font(theme::semibold(10.5)).color(p.text_weak));
                });
            };
            ui.add_space(check_w);
            head(ui, key_w, "KEY");
            if opts.allow_files {
                head(ui, type_w, "TYPE");
            }
            head(ui, val_w, "VALUE");
        });
        let used = ui.cursor().top() - top;
        if used < HEAD_H {
            ui.add_space(HEAD_H - used);
        }

        let n = rows.len();
        for (i, row) in rows.iter_mut().enumerate() {
            let last = i + 1 == n;
            let row_top = ui.cursor().top();
            let row_rect = Rect::from_min_size(egui::pos2(left, row_top), egui::vec2(width, ROW_H));
            let hovered = ui.rect_contains_pointer(row_rect);
            if hovered && !last {
                ui.painter().rect_filled(row_rect.shrink(1.0), 0.0, p.hover.gamma_multiply(0.6));
            }
            ui.horizontal(|ui| {
                ui.set_height(ROW_H);
                if last {
                    ui.allocate_exact_size(egui::vec2(check_w, ROW_H), egui::Sense::hover());
                    ui.painter().text(
                        egui::pos2(left + check_w / 2.0, row_top + ROW_H / 2.0),
                        egui::Align2::CENTER_CENTER,
                        ph::PLUS,
                        theme::regular(13.0),
                        p.text_faint,
                    );
                } else {
                    let r = ui.add_sized([check_w, ROW_H], egui::Checkbox::without_text(&mut row.enabled));
                    res.changed |= r.on_hover_text("Enable / disable").changed();
                }
                let text_color = (!row.enabled).then_some(p.text_faint);
                let r = cell_edit(ui, &mut row.key, opts.id.with(("k", i)), opts.key_hint, key_w, text_color, &p);
                res.changed |= r.changed();

                if opts.allow_files {
                    let before = row.kind;
                    ui.allocate_ui_with_layout(egui::vec2(type_w, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_width(type_w);
                        ui.add_space(6.0);
                        egui::ComboBox::from_id_salt(opts.id.with(("t", i)))
                            .width(type_w - 16.0)
                            .selected_text(if row.kind == FieldKind::File { "File" } else { "Text" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut row.kind, FieldKind::Text, "Text");
                                ui.selectable_value(&mut row.kind, FieldKind::File, "File");
                            });
                    });
                    if row.kind != before {
                        row.value.clear();
                        res.changed = true;
                    }
                }

                if opts.allow_files && row.kind == FieldKind::File {
                    let label = if row.value.is_empty() {
                        format!("{}  Select file…", ph::FOLDER_OPEN)
                    } else {
                        let name =
                            std::path::Path::new(&row.value).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                        format!("{}  {}", ph::FILE, truncate_chars(&name, 40))
                    };
                    ui.allocate_ui_with_layout(egui::vec2(val_w, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_width(val_w);
                        ui.add_space(6.0);
                        if ui.button(label).on_hover_text(&row.value).clicked() {
                            res.pick_file = Some(i);
                        }
                    });
                } else {
                    let r = cell_edit(ui, &mut row.value, opts.id.with(("v", i)), opts.value_hint, val_w, text_color, &p);
                    res.changed |= r.changed();
                }

                if !last && hovered {
                    ui.allocate_ui_with_layout(egui::vec2(del_w, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        if ui_kit::icon_button(ui, ph::TRASH, "Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
            });
            // keep rows on a fixed grid regardless of widget heights
            let used = ui.cursor().top() - row_top;
            if used < ROW_H {
                ui.add_space(ROW_H - used);
            }
        }

        let bottom = ui.cursor().top();
        let table = Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + width, bottom));
        ui.painter().set(bg_idx, egui::Shape::rect_filled(table, 8.0, p.input));
        let header_bottom = top + HEAD_H;
        let mut grid = vec![egui::Shape::rect_filled(
            Rect::from_min_max(table.min, egui::pos2(table.right(), header_bottom)),
            egui::Rounding { nw: 8.0, ne: 8.0, sw: 0.0, se: 0.0 },
            if p.dark { p.elevated } else { p.hover },
        )];
        let line = Stroke::new(1.0_f32, p.border);
        for k in 0..n {
            grid.push(egui::Shape::hline(table.x_range(), header_bottom + k as f32 * ROW_H, line));
        }
        let cols: &[f32] = if opts.allow_files { &xs } else { &[xs[0], xs[1], xs[3]] };
        for &x in cols {
            grid.push(egui::Shape::vline(x, table.y_range(), line));
        }
        grid.push(egui::Shape::rect_stroke(table, 8.0, Stroke::new(1.0_f32, p.border)));
        ui.painter().set(grid_idx, egui::Shape::Vec(grid));
        ui.add_space(4.0);
    });
    if let Some(i) = remove {
        rows.remove(i);
        res.changed = true;
    }
    if res.changed {
        ui.ctx().request_repaint();
    }
    res
}

/// Frameless text cell; shows an accent outline while focused.
fn cell_edit(ui: &mut Ui, text: &mut String, id: egui::Id, hint: &str, width: f32, color: Option<Color32>, p: &Pal) -> egui::Response {
    let rect = Rect::from_min_size(ui.cursor().min, egui::vec2(width, ROW_H));
    if ui.memory(|m| m.has_focus(id)) {
        ui.painter().rect(rect.shrink(3.0), 5.0, p.soft(p.accent).gamma_multiply(0.5), Stroke::new(1.0_f32, p.accent));
    }
    ui.allocate_ui_with_layout(egui::vec2(width, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(width);
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(text).id(id).frame(false).hint_text(hint).desired_width(width - 20.0).text_color_opt(color))
    })
    .inner
}

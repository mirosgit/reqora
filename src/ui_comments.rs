//! Comment threads (requests and collections).

use crate::model::{now_ms, Comment};
use crate::theme::{self, Pal};
use crate::ui_kit;
use eframe::egui::{self, Id, Key, KeyboardShortcut, Modifiers, RichText, Stroke, Ui};
use egui_phosphor::regular as ph;

/// Per-thread UI state (draft text, inline editing, delete confirmation).
#[derive(Default)]
pub struct CommentState {
    pub draft: String,
    pub editing: Option<(String, String)>,
    pub confirm_delete: Option<String>,
}

pub fn composer_id(salt: impl std::hash::Hash) -> Id {
    Id::new(("comment_composer", salt))
}

pub fn edit_id(salt: impl std::hash::Hash) -> Id {
    Id::new(("comment_edit", salt))
}

/// Is a comment text field focused? (Ctrl+Enter then posts instead of sending the request.)
pub fn is_typing_comment(ctx: &egui::Context, salt: impl std::hash::Hash + Copy) -> bool {
    let focused = ctx.memory(|m| m.focused());
    focused.is_some_and(|f| f == composer_id(salt) || f == edit_id(salt))
}

pub fn relative_time(ms: i64) -> String {
    let secs = (now_ms() - ms).max(0) / 1000;
    match secs {
        0..=44 => "just now".into(),
        45..=89 => "1 min ago".into(),
        90..=3599 => format!("{} min ago", (secs + 30) / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => chrono::DateTime::from_timestamp_millis(ms)
            .map(|d| d.with_timezone(&chrono::Local).format("%d %b %Y").to_string())
            .unwrap_or_default(),
    }
}

/// Full thread: list + composer. `salt` makes widget ids unique. Returns true if comments changed.
pub fn thread(
    ui: &mut Ui,
    salt: impl std::hash::Hash + Copy,
    comments: &mut Vec<Comment>,
    state: &mut CommentState,
    author: &str,
    note: &str,
) -> bool {
    let p = Pal::of(ui);
    let mut changed = false;
    let ctrl_enter = KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter);

    egui::TopBottomPanel::bottom(Id::new(("comment_composer_panel", salt)))
        .frame(egui::Frame::none().inner_margin(egui::Margin { top: 10.0, ..Default::default() }))
        .show_separator_line(false)
        .show_inside(ui, |ui| {
            if !note.is_empty() {
                ui.label(RichText::new(format!("{}  {note}", ph::INFO)).small().color(p.text_weak));
                ui.add_space(4.0);
            }
            let id = composer_id(salt);
            let focused = ui.memory(|m| m.has_focus(id));
            let mut post = focused && ui.input_mut(|i| i.consume_shortcut(&ctrl_enter));
            let frame_stroke = Stroke::new(1.0_f32, if focused { p.accent } else { p.border });
            egui::Frame::none().fill(p.input).stroke(frame_stroke).rounding(10.0).inner_margin(egui::Margin::same(10.0)).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui_kit::avatar(ui, author, 26.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut state.draft)
                            .id(id)
                            .frame(false)
                            .hint_text("Write a comment…")
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(34.0);
                    ui_kit::keycap(ui, ui_kit::CMD);
                    ui_kit::keycap(ui, "Enter");
                    ui.label(RichText::new("to post").small().color(p.text_faint));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let enabled = !state.draft.trim().is_empty();
                        let b = ui.add_enabled_ui(enabled, |ui| {
                            ui_kit::primary_button(ui, Some(ph::PAPER_PLANE_RIGHT), "Comment", egui::vec2(0.0, 30.0))
                        });
                        post |= b.inner.clicked();
                    });
                });
            });
            if post && !state.draft.trim().is_empty() {
                comments.push(Comment::new(author, &state.draft));
                state.draft.clear();
                changed = true;
            }
        });

    egui::CentralPanel::default().frame(egui::Frame::none()).show_inside(ui, |ui| {
        if comments.is_empty() {
            ui_kit::hero(ui, ph::CHAT_CIRCLE_DOTS, "No comments yet", "Leave notes for yourself or your team.");
            return;
        }
        let mut op: Option<Op> = None;
        egui::ScrollArea::vertical().id_salt(("comments_scroll", salt)).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            for c in comments.iter() {
                comment_card(ui, salt, c, state, &p, &mut op);
                ui.add_space(8.0);
            }
        });
        if let Some(op) = op {
            match op {
                Op::Delete(id) => {
                    comments.retain(|c| c.id != id);
                    changed = true;
                }
                Op::Save(id, text) => {
                    if let Some(c) = comments.iter_mut().find(|c| c.id == id) {
                        if c.text != text.trim() && !text.trim().is_empty() {
                            c.text = text.trim().to_string();
                            c.edited_ms = Some(now_ms());
                            changed = true;
                        }
                    }
                }
            }
        }
    });
    changed
}

enum Op {
    Delete(String),
    Save(String, String),
}

fn comment_card(ui: &mut Ui, salt: impl std::hash::Hash + Copy, c: &Comment, state: &mut CommentState, p: &Pal, op: &mut Option<Op>) {
    let editing = state.editing.as_ref().is_some_and(|(id, _)| id == &c.id);
    let confirming = state.confirm_delete.as_deref() == Some(c.id.as_str());
    let card =
        egui::Frame::none().fill(p.elevated).stroke(Stroke::new(1.0_f32, p.border)).rounding(10.0).inner_margin(egui::Margin::same(12.0));
    card.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui_kit::avatar(ui, &c.author, 30.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&c.author).font(theme::semibold(13.0)).color(p.text_strong));
                    let when = relative_time(c.created_ms);
                    let full = chrono::DateTime::from_timestamp_millis(c.created_ms)
                        .map(|d| d.with_timezone(&chrono::Local).format("%d %b %Y, %H:%M").to_string())
                        .unwrap_or_default();
                    ui.label(RichText::new(when).small().color(p.text_weak)).on_hover_text(full);
                    if c.edited_ms.is_some() {
                        ui.label(RichText::new("· edited").small().color(p.text_faint));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !editing && !confirming {
                            if ui_kit::icon_button(ui, ph::TRASH, "Delete").clicked() {
                                state.confirm_delete = Some(c.id.clone());
                            }
                            if ui_kit::icon_button(ui, ph::PENCIL_SIMPLE, "Edit").clicked() {
                                state.editing = Some((c.id.clone(), c.text.clone()));
                                ui.memory_mut(|m| m.request_focus(edit_id(salt)));
                            }
                        }
                    });
                });
                if editing {
                    let mut save = false;
                    let mut cancel = false;
                    if let Some((_, buf)) = &mut state.editing {
                        let id = edit_id(salt);
                        if ui.memory(|m| m.has_focus(id))
                            && ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Enter)))
                        {
                            save = true;
                        }
                        ui.add(egui::TextEdit::multiline(buf).id(id).desired_rows(2).desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {
                            save |= ui_kit::primary_button(ui, Some(ph::CHECK), "Save", egui::vec2(0.0, 28.0)).clicked();
                            cancel = ui_kit::button(ui, "", "Cancel").clicked();
                        });
                        if save {
                            *op = Some(Op::Save(c.id.clone(), buf.clone()));
                        }
                    }
                    if save || cancel {
                        state.editing = None;
                    }
                } else {
                    ui.add(egui::Label::new(RichText::new(&c.text).color(p.text)).wrap().selectable(true));
                }
                if confirming {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Delete this comment?").color(p.danger));
                        if ui.add(egui::Button::new(RichText::new("Delete").color(egui::Color32::WHITE)).fill(p.danger)).clicked() {
                            *op = Some(Op::Delete(c.id.clone()));
                            state.confirm_delete = None;
                        }
                        if ui_kit::button(ui, "", "Cancel").clicked() {
                            state.confirm_delete = None;
                        }
                    });
                }
            });
        });
    });
}

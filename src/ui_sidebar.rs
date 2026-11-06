//! Left sidebar: collections tree, environments and request history.

use crate::app::{Action, DeleteTarget, RenameTarget, SidebarSection, SidebarState};
use crate::model::Request;
use crate::storage::Store;
use crate::theme::{self, Pal};
use crate::ui_kit;
use crate::widgets::elided;
use eframe::egui::{self, Align2, Key, Rect, Response, RichText, Sense, Ui};
use egui_phosphor::regular as ph;
use std::cell::RefCell;

/// What the active tab points at (for highlighting).
#[derive(Default)]
pub struct ActiveRefs<'a> {
    pub request: Option<(&'a str, &'a str)>,
    pub environment: Option<&'a str>,
    pub collection: Option<&'a str>,
}

pub fn show(ui: &mut Ui, st: &mut SidebarState, store: &Store, active: ActiveRefs, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    let collections = format!("{} Collections", ph::FOLDERS);
    let envs = format!("{} Envs", ph::STACK);
    let history = format!("{} History", ph::CLOCK_COUNTER_CLOCKWISE);
    ui_kit::segmented(
        ui,
        egui::Id::new("sidebar_sections"),
        &mut st.section,
        &[(SidebarSection::Collections, &collections), (SidebarSection::Environments, &envs), (SidebarSection::History, &history)],
        true,
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match st.section {
                SidebarSection::Collections => {
                    if ui_kit::icon_button(ui, ph::PLUS, "New collection").clicked() {
                        actions.push(Action::NewCollection);
                    }
                }
                SidebarSection::Environments => {
                    if ui_kit::icon_button(ui, ph::PLUS, "New environment").clicked() {
                        actions.push(Action::NewEnvironment);
                    }
                }
                SidebarSection::History => {
                    if ui
                        .add_enabled_ui(!store.history.is_empty(), |ui| ui_kit::icon_button(ui, ph::TRASH, "Clear history"))
                        .inner
                        .clicked()
                    {
                        actions.push(Action::AskDelete(DeleteTarget::History));
                    }
                }
            }
            ui_kit::search_field(ui, &mut st.filter, "Filter", egui::Id::new("sidebar_filter"));
        });
    });
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("sidebar_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        match st.section {
            SidebarSection::Collections => collections_list(ui, st, store, &active, actions),
            SidebarSection::Environments => environments(ui, st, store, active.environment, actions),
            SidebarSection::History => history_list(ui, st, store, actions),
        }
    });
}

/// Full-width clickable row with hover/selection background.
fn row(ui: &mut Ui, id: egui::Id, height: f32, selected: bool) -> (Rect, Response) {
    let p = Pal::of(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 7.0, p.soft(p.accent));
        let bar = Rect::from_min_size(egui::pos2(rect.left(), rect.top() + 6.0), egui::vec2(3.0, rect.height() - 12.0));
        ui_kit::gradient_rect(ui.painter(), bar, 1.5, p.accent, p.accent2);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 7.0, p.hover);
    }
    (rect, resp)
}

/// "⋯" button shown while the row is hovered (or its menu is open), plus right-click menu.
fn row_menu(ui: &mut Ui, rect: Rect, resp: &Response, id: egui::Id, add: impl Fn(&mut Ui)) -> bool {
    let popup_id = id.with("menu");
    let open = ui.memory(|m| m.is_popup_open(popup_id));
    let show = open || ui.rect_contains_pointer(rect);
    if show {
        let b_rect = Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), egui::vec2(26.0, 24.0));
        let b = ui_kit::icon_button_at(ui, b_rect, id.with("dots"), ph::DOTS_THREE);
        if b.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup_id));
        }
        egui::popup_below_widget(ui, popup_id, &b, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(190.0);
            add(ui);
        });
    }
    resp.context_menu(|ui| {
        ui.set_min_width(190.0);
        add(ui);
    });
    show
}

fn menu_item(ui: &mut Ui, icon: &str, label: &str, push: impl FnOnce()) {
    if ui.button(format!("{icon}  {label}")).clicked() {
        push();
        ui.close_menu();
    }
}

fn rename_field(ui: &mut Ui, st: &mut SidebarState, indent: f32, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let r = ui.add(egui::TextEdit::singleline(&mut st.rename_buf).desired_width(ui.available_width() - 12.0));
        if st.rename_focus {
            r.request_focus();
            st.rename_focus = false;
        } else if r.lost_focus() {
            let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
            if let Some(target) = st.renaming.take() {
                if !cancelled {
                    actions.push(Action::Rename(target, st.rename_buf.clone()));
                }
            }
        }
    });
}

fn count_label(ui: &Ui, rect: Rect, right_pad: f32, comments: usize, extra: Option<String>) {
    let p = Pal::of(ui);
    let mut x = rect.right() - right_pad;
    if let Some(t) = extra {
        let g = ui.painter().layout_no_wrap(t, theme::medium(11.0), p.text_faint);
        x -= g.size().x;
        ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, p.text_faint);
        x -= 10.0;
    }
    if comments > 0 {
        let g = ui.painter().layout_no_wrap(format!("{} {comments}", ph::CHAT_CIRCLE), theme::medium(11.0), p.text_weak);
        x -= g.size().x;
        ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, p.text_weak);
    }
}

fn collections_list(ui: &mut Ui, st: &mut SidebarState, store: &Store, active: &ActiveRefs, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    if store.ws.collections.is_empty() {
        ui_kit::hero(ui, ph::FOLDERS, "No collections yet", "Group and save your requests.");
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            if ui_kit::primary_button(ui, Some(ph::PLUS), "Create collection", egui::vec2(0.0, 30.0)).clicked() {
                actions.push(Action::NewCollection);
            }
            ui.add_space(4.0);
            if ui_kit::button(ui, ph::DOWNLOAD_SIMPLE, "Import from Postman").clicked() {
                actions.push(Action::Import);
            }
        });
        return;
    }
    let f = st.filter.trim().to_lowercase();
    for coll in &store.ws.collections {
        let name_match = f.is_empty() || coll.name.to_lowercase().contains(&f);
        let reqs: Vec<&Request> = coll
            .requests
            .iter()
            .filter(|r| name_match || r.name.to_lowercase().contains(&f) || r.url.to_lowercase().contains(&f))
            .collect();
        if !name_match && reqs.is_empty() {
            continue;
        }
        let expanded = st.expanded.contains(&coll.id) || !f.is_empty();
        let target = RenameTarget::Collection(coll.id.clone());
        if st.renaming.as_ref() == Some(&target) {
            rename_field(ui, st, 4.0, actions);
        } else {
            let id = egui::Id::new(("coll", &coll.id));
            let (rect, resp) = row(ui, id, 32.0, active.collection == Some(coll.id.as_str()));
            let y = rect.center().y;
            let painter = ui.painter();
            painter.text(
                egui::pos2(rect.left() + 12.0, y),
                Align2::CENTER_CENTER,
                if expanded { ph::CARET_DOWN } else { ph::CARET_RIGHT },
                theme::regular(12.0),
                p.text_weak,
            );
            painter.text(
                egui::pos2(rect.left() + 31.0, y),
                Align2::CENTER_CENTER,
                if expanded { ph::FOLDER_OPEN } else { ph::FOLDER },
                theme::regular(16.0),
                p.accent,
            );
            let g = elided(ui, &coll.name, theme::medium(13.5), p.text_strong, rect.width() - 110.0);
            ui.painter().galley(egui::pos2(rect.left() + 46.0, y - g.size().y / 2.0), g, p.text_strong);
            if resp.double_clicked() {
                actions.push(Action::OpenCollection(coll.id.clone()));
            } else if resp.clicked() {
                if expanded {
                    st.expanded.remove(&coll.id);
                } else {
                    st.expanded.insert(coll.id.clone());
                }
            }
            let (cid, name) = (coll.id.clone(), coll.name.clone());
            let cell = RefCell::new(&mut *actions);
            let menu_shown = row_menu(ui, rect, &resp, id, |ui| {
                menu_item(ui, ph::PLUS, "Add request", || cell.borrow_mut().push(Action::AddRequest(cid.clone())));
                menu_item(ui, ph::CHAT_CIRCLE_TEXT, "Overview & comments", || cell.borrow_mut().push(Action::OpenCollection(cid.clone())));
                menu_item(ui, ph::PENCIL_SIMPLE, "Rename", || {
                    cell.borrow_mut().push(Action::StartRename(RenameTarget::Collection(cid.clone()), name.clone()))
                });
                menu_item(ui, ph::UPLOAD_SIMPLE, "Export (Postman v2.1)", || cell.borrow_mut().push(Action::ExportCollection(cid.clone())));
                ui.separator();
                menu_item(ui, ph::TRASH, "Delete", || cell.borrow_mut().push(Action::AskDelete(DeleteTarget::Collection(cid.clone()))));
            });
            if !menu_shown {
                count_label(ui, rect, 10.0, coll.comments.len(), Some(coll.requests.len().to_string()));
            }
        }
        if !expanded {
            continue;
        }
        if reqs.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(46.0);
                if ui.link(RichText::new(format!("{} Add a request", ph::PLUS)).small()).clicked() {
                    actions.push(Action::AddRequest(coll.id.clone()));
                }
            });
        }
        for req in reqs {
            let target = RenameTarget::Request(coll.id.clone(), req.id.clone());
            if st.renaming.as_ref() == Some(&target) {
                rename_field(ui, st, 26.0, actions);
                continue;
            }
            let selected = active.request == Some((coll.id.as_str(), req.id.as_str()));
            let id = egui::Id::new(("req", &coll.id, &req.id));
            let (rect, resp) = row(ui, id, 30.0, selected);
            ui_kit::paint_method_badge(ui, egui::pos2(rect.left() + 26.0, rect.center().y), &req.method, 40.0);
            let color = if selected { p.text_strong } else { p.text };
            let g = elided(ui, req.display_name(), theme::regular(13.5), color, rect.width() - 120.0);
            ui.painter().galley(egui::pos2(rect.left() + 74.0, rect.center().y - g.size().y / 2.0), g, color);
            if resp.double_clicked() {
                actions.push(Action::StartRename(target.clone(), req.name.clone()));
            } else if resp.clicked() {
                actions.push(Action::OpenRequest(coll.id.clone(), req.id.clone()));
            }
            let (cid, rid, name) = (coll.id.clone(), req.id.clone(), req.name.clone());
            let resp = resp.on_hover_text(format!("{} {}", req.method, req.url));
            let cell = RefCell::new(&mut *actions);
            let menu_shown = row_menu(ui, rect, &resp, id, |ui| {
                menu_item(ui, ph::ARROW_SQUARE_OUT, "Open", || cell.borrow_mut().push(Action::OpenRequest(cid.clone(), rid.clone())));
                menu_item(ui, ph::PENCIL_SIMPLE, "Rename", || {
                    cell.borrow_mut().push(Action::StartRename(RenameTarget::Request(cid.clone(), rid.clone()), name.clone()))
                });
                menu_item(ui, ph::COPY_SIMPLE, "Duplicate", || cell.borrow_mut().push(Action::DuplicateRequest(cid.clone(), rid.clone())));
                ui.separator();
                menu_item(ui, ph::TRASH, "Delete", || {
                    cell.borrow_mut().push(Action::AskDelete(DeleteTarget::Request(cid.clone(), rid.clone())))
                });
            });
            if !menu_shown {
                count_label(ui, rect, 10.0, req.comments.len(), None);
            }
        }
        ui.add_space(2.0);
    }
}

fn environments(ui: &mut Ui, st: &mut SidebarState, store: &Store, active_env_tab: Option<&str>, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    if store.ws.environments.is_empty() {
        ui_kit::hero(ui, ph::STACK, "No environments yet", "Keep variables like {{base_url}} per stage.");
        ui.vertical_centered(|ui| {
            ui.add_space(8.0);
            if ui_kit::primary_button(ui, Some(ph::PLUS), "Create environment", egui::vec2(0.0, 30.0)).clicked() {
                actions.push(Action::NewEnvironment);
            }
        });
        return;
    }
    let f = st.filter.trim().to_lowercase();
    for env in &store.ws.environments {
        if !f.is_empty() && !env.name.to_lowercase().contains(&f) {
            continue;
        }
        let target = RenameTarget::Environment(env.id.clone());
        if st.renaming.as_ref() == Some(&target) {
            rename_field(ui, st, 4.0, actions);
            continue;
        }
        let is_active = store.ws.active_environment.as_deref() == Some(env.id.as_str());
        let id = egui::Id::new(("env", &env.id));
        let (rect, resp) = row(ui, id, 32.0, active_env_tab == Some(env.id.as_str()));
        let icon_color = if is_active { p.success } else { p.text_weak };
        ui.painter().text(
            egui::pos2(rect.left() + 16.0, rect.center().y),
            Align2::CENTER_CENTER,
            ph::STACK,
            theme::regular(15.0),
            icon_color,
        );
        let g = elided(ui, &env.name, theme::medium(13.5), p.text_strong, rect.width() - 110.0);
        ui.painter().galley(egui::pos2(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, p.text_strong);
        if resp.clicked() {
            actions.push(Action::OpenEnvironment(env.id.clone()));
        }
        let (eid, name) = (env.id.clone(), env.name.clone());
        let cell = RefCell::new(&mut *actions);
        let menu_shown = row_menu(ui, rect, &resp, id, |ui| {
            if is_active {
                menu_item(ui, ph::CIRCLE, "Deactivate", || cell.borrow_mut().push(Action::SetActiveEnv(None)));
            } else {
                menu_item(ui, ph::CHECK_CIRCLE, "Set active", || cell.borrow_mut().push(Action::SetActiveEnv(Some(eid.clone()))));
            }
            menu_item(ui, ph::PENCIL_SIMPLE, "Rename", || {
                cell.borrow_mut().push(Action::StartRename(RenameTarget::Environment(eid.clone()), name.clone()))
            });
            menu_item(ui, ph::UPLOAD_SIMPLE, "Export", || cell.borrow_mut().push(Action::ExportEnvironment(eid.clone())));
            ui.separator();
            menu_item(ui, ph::TRASH, "Delete", || cell.borrow_mut().push(Action::AskDelete(DeleteTarget::Environment(eid.clone()))));
        });
        if is_active && !menu_shown {
            let g = ui.painter().layout_no_wrap("ACTIVE".into(), theme::semibold(9.5), p.success);
            let r = Rect::from_min_size(
                egui::pos2(rect.right() - g.size().x - 22.0, rect.center().y - 9.0),
                egui::vec2(g.size().x + 12.0, 18.0),
            );
            ui.painter().rect_filled(r, 9.0, p.soft(p.success));
            ui.painter().galley(r.center() - g.size() / 2.0, g, p.success);
        }
    }
}

fn history_list(ui: &mut Ui, st: &mut SidebarState, store: &Store, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    if store.history.is_empty() {
        ui_kit::hero(ui, ph::CLOCK_COUNTER_CLOCKWISE, "No history yet", "Requests you send will appear here.");
        return;
    }
    let f = st.filter.trim().to_lowercase();
    let today = chrono::Local::now().date_naive();
    let mut last_day = None;
    for (idx, h) in store.history.iter().enumerate() {
        if !f.is_empty() && !h.request.url.to_lowercase().contains(&f) && !h.request.name.to_lowercase().contains(&f) {
            continue;
        }
        let local = chrono::DateTime::from_timestamp_millis(h.timestamp_ms).map(|d| d.with_timezone(&chrono::Local));
        let day = local.map(|d| d.date_naive());
        if day != last_day {
            last_day = day;
            let label = match day {
                Some(d) if d == today => "TODAY".to_string(),
                Some(d) if today.pred_opt() == Some(d) => "YESTERDAY".to_string(),
                Some(d) => d.format("%d %b %Y").to_string().to_uppercase(),
                None => "UNKNOWN DATE".to_string(),
            };
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(label).font(theme::semibold(10.5)).color(p.text_faint));
            });
            ui.add_space(2.0);
        }
        let id = egui::Id::new(("hist", &h.id));
        let (rect, resp) = row(ui, id, 44.0, false);
        let top = rect.top() + 14.0;
        let bottom = rect.bottom() - 12.0;
        ui_kit::paint_method_badge(ui, egui::pos2(rect.left() + 6.0, top), &h.request.method, 40.0);
        let url = if h.request.url.is_empty() { "(no URL)" } else { h.request.url.as_str() };
        let g = elided(ui, url, theme::regular(13.0), p.text, rect.width() - 60.0);
        ui.painter().galley(egui::pos2(rect.left() + 54.0, top - g.size().y / 2.0), g, p.text);
        let (status, color) = match (h.status, &h.error) {
            (Some(s), _) => (s.to_string(), theme::status_color(s, p.dark)),
            (None, Some(_)) => ("Error".to_string(), p.danger),
            _ => ("—".to_string(), p.text_weak),
        };
        ui.painter().circle_filled(egui::pos2(rect.left() + 58.0, bottom), 3.0, color);
        let sg = ui.painter().layout_no_wrap(status, theme::semibold(11.0), color);
        let sw = sg.size().x;
        ui.painter().galley(egui::pos2(rect.left() + 65.0, bottom - sg.size().y / 2.0), sg, color);
        let mut meta = local.map(|d| d.format("%H:%M:%S").to_string()).unwrap_or_default();
        if let Some(ms) = h.elapsed_ms {
            meta = format!("{meta} · {ms} ms");
        }
        ui.painter().text(egui::pos2(rect.left() + 72.0 + sw, bottom), Align2::LEFT_CENTER, meta, theme::regular(11.0), p.text_weak);
        let resp = match &h.error {
            Some(e) => resp.on_hover_text(e),
            None => resp.on_hover_text(&h.request.url),
        };
        if resp.clicked() {
            actions.push(Action::OpenHistory(idx));
        }
    }
}

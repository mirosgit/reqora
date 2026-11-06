//! Request editor: breadcrumb + actions, method/URL bar, config sections, comments drawer.

use crate::app::{Action, FilePurpose, ReqSection, RequestTab};
use crate::model::*;
use crate::response::ContentKind;
use crate::theme::{self, Pal, Palette};
use crate::vars::{resolve, resolve_single, Vars};
use crate::widgets::{self, KvOptions};
use crate::{curl, highlight, ui_comments, ui_kit, ui_response};
use eframe::egui::{self, Color32, Key, RichText, Stroke, TextStyle};
use egui_phosphor::regular as ph;

pub struct EditorEnv<'a> {
    pub vars: &'a Vars,
    pub collection_name: Option<&'a str>,
    pub dark: bool,
    pub layout: PaneLayout,
    pub focus_url: bool,
    pub author: &'a str,
}

pub fn show(ui: &mut egui::Ui, tab: &mut RequestTab, env: &EditorEnv, actions: &mut Vec<Action>) {
    header_row(ui, tab, env, actions);
    ui.add_space(8.0);
    url_bar(ui, tab, env, actions);
    ui.add_space(10.0);

    if tab.comments_open {
        egui::SidePanel::right("comments_drawer")
            .resizable(true)
            .default_width(340.0)
            .width_range(280.0..=560.0)
            .frame(egui::Frame::none().inner_margin(egui::Margin { left: 14.0, ..Default::default() }))
            .show_inside(ui, |ui| comments_drawer(ui, tab, env, actions));
    }

    let panel_frame = egui::Frame::none();
    match env.layout {
        PaneLayout::Vertical => {
            let max_h = (ui.available_height() - 140.0).max(130.0);
            egui::TopBottomPanel::top("request_config")
                .resizable(true)
                .default_height(250.0)
                .height_range(120.0..=max_h)
                .frame(panel_frame.inner_margin(egui::Margin { bottom: 8.0, ..Default::default() }))
                .show_inside(ui, |ui| config(ui, tab, env, actions));
            egui::CentralPanel::default()
                .frame(panel_frame.inner_margin(egui::Margin { top: 8.0, ..Default::default() }))
                .show_inside(ui, |ui| ui_response::show(ui, tab, env, actions));
        }
        PaneLayout::Horizontal => {
            let max_w = (ui.available_width() - 320.0).max(300.0);
            egui::SidePanel::left("request_config_side")
                .resizable(true)
                .default_width(ui.available_width() * 0.48)
                .width_range(280.0..=max_w)
                .frame(panel_frame.inner_margin(egui::Margin { right: 10.0, ..Default::default() }))
                .show_inside(ui, |ui| config(ui, tab, env, actions));
            egui::CentralPanel::default()
                .frame(panel_frame.inner_margin(egui::Margin { left: 10.0, ..Default::default() }))
                .show_inside(ui, |ui| ui_response::show(ui, tab, env, actions));
        }
    }
}

fn header_row(ui: &mut egui::Ui, tab: &mut RequestTab, env: &EditorEnv, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    let tab_id = tab.id;
    ui.horizontal(|ui| {
        ui.set_height(32.0);
        match env.collection_name {
            Some(c) => {
                ui.label(RichText::new(format!("{}  {c}", ph::FOLDER)).color(p.text_weak));
                ui.label(RichText::new(ph::CARET_RIGHT).color(p.text_faint));
            }
            None => {
                ui_kit::chip(ui, "Draft", p.text_weak).on_hover_text("Not saved in a collection yet — press Ctrl+S");
            }
        }
        if tab.editing_name {
            let r = ui.add(egui::TextEdit::singleline(&mut tab.req.name).font(theme::semibold(15.0)).desired_width(320.0));
            if tab.focus_name {
                r.request_focus();
                tab.focus_name = false;
            }
            if r.lost_focus() {
                tab.editing_name = false;
            }
        } else {
            let r = ui
                .add(
                    egui::Label::new(RichText::new(tab.req.display_name()).font(theme::semibold(16.0)).color(p.text_strong))
                        .sense(egui::Sense::click()),
                )
                .on_hover_text("Click to rename");
            if r.clicked() {
                tab.editing_name = true;
                tab.focus_name = true;
            }
        }
        if tab.is_dirty() {
            ui_kit::chip(ui, "Unsaved", p.warning);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(RichText::new(ph::DOTS_THREE).size(16.0), |ui| {
                ui.set_min_width(190.0);
                if ui.button(format!("{}  Save as…", ph::FLOPPY_DISK_BACK)).clicked() {
                    actions.push(Action::SaveAs(tab_id));
                    ui.close_menu();
                }
                if ui.button(format!("{}  Copy as cURL", ph::TERMINAL_WINDOW)).clicked() {
                    actions.push(Action::CopyCurl(tab_id));
                    ui.close_menu();
                }
                if ui.button(format!("{}  Duplicate tab", ph::COPY_SIMPLE)).clicked() {
                    actions.push(Action::DuplicateTab(tab_id));
                    ui.close_menu();
                }
            });
            if ui_kit::button(ui, ph::FLOPPY_DISK, "Save").on_hover_text("Save to collection (Ctrl+S)").clicked() {
                actions.push(Action::Save(tab_id));
            }
            if ui_kit::button(ui, ph::TERMINAL_WINDOW, "cURL").on_hover_text("Copy as cURL command").clicked() {
                actions.push(Action::CopyCurl(tab_id));
            }
            let n = tab.req.comments.len();
            let label = if n > 0 { format!("{}  {n}", ph::CHAT_CIRCLE_TEXT) } else { format!("{}  Comment", ph::CHAT_CIRCLE_TEXT) };
            let r = ui.add(egui::Button::new(label).selected(tab.comments_open).min_size(egui::vec2(0.0, 30.0)));
            if r.on_hover_text("Comments").clicked() {
                tab.comments_open = !tab.comments_open;
                if tab.comments_open {
                    ui.memory_mut(|m| m.request_focus(ui_comments::composer_id(("req", tab_id))));
                }
            }
        });
    });
}

fn url_bar(ui: &mut egui::Ui, tab: &mut RequestTab, env: &EditorEnv, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    let pal = Palette::new(p.dark);
    let tab_id = tab.id;
    let url_id = egui::Id::new(("url", tab_id));
    let focused = ui.memory(|m| m.has_focus(url_id));
    let loading = tab.is_loading();
    let (send_w, h) = (116.0, 40.0);
    ui.horizontal(|ui| {
        let bar_w = (ui.available_width() - send_w - ui.spacing().item_spacing.x).max(200.0);
        let frame = egui::Frame::none()
            .fill(p.input)
            .stroke(Stroke::new(1.0_f32, if focused { p.accent } else { p.border }))
            .rounding(10.0)
            .inner_margin(egui::Margin::symmetric(4.0, 0.0));
        ui.allocate_ui_with_layout(egui::vec2(bar_w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            frame.show(ui, |ui| {
                ui.set_width(bar_w - 10.0);
                ui.set_height(h - 2.0);
                ui.horizontal_centered(|ui| {
                    let m = tab.req.method.clone();
                    ui.scope(|ui| {
                        let v = ui.visuals_mut();
                        for w in [&mut v.widgets.inactive, &mut v.widgets.active, &mut v.widgets.open] {
                            w.weak_bg_fill = Color32::TRANSPARENT;
                            w.bg_stroke = Stroke::NONE;
                        }
                        v.widgets.hovered.bg_stroke = Stroke::NONE;
                        egui::ComboBox::from_id_salt(("method", tab_id))
                            .width(88.0)
                            .selected_text(RichText::new(&m).font(theme::semibold(13.0)).color(theme::method_color(&m, p.dark)))
                            .show_ui(ui, |ui| {
                                for mm in METHODS {
                                    ui.selectable_value(
                                        &mut tab.req.method,
                                        mm.to_string(),
                                        RichText::new(*mm).font(theme::semibold(13.0)).color(theme::method_color(mm, p.dark)),
                                    );
                                }
                            });
                    });
                    let (sep, _) = ui.allocate_exact_size(egui::vec2(1.0, 22.0), egui::Sense::hover());
                    ui.painter().vline(sep.center().x, sep.y_range(), Stroke::new(1.0_f32, p.border));

                    let font = TextStyle::Monospace.resolve(ui.style());
                    let text_color = p.text_strong;
                    let vars = env.vars;
                    let mut layouter = |ui: &egui::Ui, text: &str, _wrap: f32| {
                        let job = highlight::vars_job(text, vars, &pal, &font, text_color);
                        ui.fonts(|f| f.layout_job(job))
                    };
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut tab.req.url)
                            .id(url_id)
                            .frame(false)
                            .hint_text("Enter URL or paste a cURL command — {{base_url}}/users")
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter),
                    );
                    if env.focus_url {
                        r.request_focus();
                    }
                    if r.changed() {
                        if curl::looks_like_curl(&tab.req.url) {
                            actions.push(Action::CurlPasted(tab_id));
                        } else {
                            params_from_url(&mut tab.req);
                        }
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        actions.push(Action::Send(tab_id));
                    }
                    if tab.req.url.contains('{') {
                        r.on_hover_text(format!("Resolved: {}", resolve_single(&resolve(&tab.req.url, vars), vars)));
                    }
                });
            });
        });
        if loading {
            if ui.add_sized([send_w, h], egui::Button::new(format!("{}  Cancel", ph::X_CIRCLE))).clicked() {
                actions.push(Action::Cancel(tab_id));
            }
        } else if ui_kit::primary_button(ui, Some(ph::PAPER_PLANE_RIGHT), "Send", egui::vec2(send_w, h))
            .on_hover_text("Ctrl+Enter")
            .clicked()
        {
            actions.push(Action::Send(tab_id));
        }
    });
}

fn comments_drawer(ui: &mut egui::Ui, tab: &mut RequestTab, env: &EditorEnv, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{}  Comments", ph::CHAT_CIRCLE_TEXT)).font(theme::semibold(14.0)).color(p.text_strong));
        if !tab.req.comments.is_empty() {
            ui_kit::chip(ui, tab.req.comments.len().to_string(), p.accent);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui_kit::icon_button(ui, ph::X, "Close comments").clicked() {
                tab.comments_open = false;
            }
        });
    });
    ui.add_space(8.0);
    let note = if tab.collection_id.is_none() { "Comments are kept with this draft and saved with the request." } else { "" };
    if ui_comments::thread(ui, ("req", tab.id), &mut tab.req.comments, &mut tab.comment_state, env.author, note) {
        actions.push(Action::SyncComments(tab.id));
    }
}

fn config(ui: &mut egui::Ui, tab: &mut RequestTab, env: &EditorEnv, actions: &mut Vec<Action>) {
    // egui panels remember the size of their content: fill the panel so a dragged size sticks.
    ui.set_min_size(ui.available_size());
    let tab_id = tab.id;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let params = tab.req.params.iter().filter(|p| p.is_active()).count();
        let headers = tab.req.headers.iter().filter(|p| p.is_active()).count();
        let auth_on = tab.req.auth.kind != AuthKind::None;
        let body_on = tab.req.body.mode != BodyMode::None;
        for (sec, label, badge, dot) in [
            (ReqSection::Params, "Params", Some(params), false),
            (ReqSection::Auth, "Authorization", None, auth_on),
            (ReqSection::Headers, "Headers", Some(headers), false),
            (ReqSection::Body, "Body", None, body_on),
            (ReqSection::Settings, "Settings", None, false),
        ] {
            if ui_kit::underline_tab(ui, tab.section == sec, label, badge, dot).clicked() {
                tab.section = sec;
            }
        }
    });
    ui_kit::divider(ui);
    ui.add_space(10.0);
    match tab.section {
        ReqSection::Params => {
            let r = widgets::kv_table(
                ui,
                &mut tab.req.params,
                KvOptions { id: egui::Id::new(("params", tab_id)), key_hint: "Add parameter", value_hint: "Value", allow_files: false },
            );
            if r.changed {
                url_from_params(&mut tab.req);
            }
        }
        ReqSection::Headers => {
            widgets::kv_table(
                ui,
                &mut tab.req.headers,
                KvOptions { id: egui::Id::new(("headers", tab_id)), key_hint: "Add header", value_hint: "Value", allow_files: false },
            );
        }
        ReqSection::Auth => auth_section(ui, &mut tab.req.auth, tab_id),
        ReqSection::Body => body_section(ui, tab, env, actions),
        ReqSection::Settings => settings_section(ui, &mut tab.req),
    }
}

fn body_section(ui: &mut egui::Ui, tab: &mut RequestTab, env: &EditorEnv, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    let tab_id = tab.id;
    let body = &mut tab.req.body;
    ui.horizontal(|ui| {
        let options: Vec<(BodyMode, &str)> = BodyMode::ALL.iter().map(|m| (*m, m.label())).collect();
        ui_kit::segmented(ui, egui::Id::new(("body_mode", tab_id)), &mut body.mode, &options, false);
        if body.mode == BodyMode::Raw {
            ui.add_space(6.0);
            egui::ComboBox::from_id_salt(("raw_type", tab_id)).width(110.0).selected_text(body.raw_type.label()).show_ui(ui, |ui| {
                for t in RawType::ALL {
                    ui.selectable_value(&mut body.raw_type, t, t.label());
                }
            });
            if body.raw_type == RawType::Json && ui_kit::button(ui, ph::MAGIC_WAND, "Beautify").on_hover_text("Format JSON").clicked() {
                match serde_json::from_str::<serde_json::Value>(&body.raw) {
                    Ok(v) => body.raw = serde_json::to_string_pretty(&v).unwrap_or_default(),
                    Err(e) => actions.push(Action::Toast(format!("Invalid JSON: {e}"))),
                }
            }
        }
    });
    ui.add_space(10.0);
    match body.mode {
        BodyMode::None => {
            ui.vertical_centered(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new(ph::PROHIBIT).size(22.0).color(p.text_faint));
                ui.label(RichText::new("This request does not have a body").color(p.text_weak));
            });
        }
        BodyMode::Raw => {
            let kind = match body.raw_type {
                RawType::Json => ContentKind::Json,
                RawType::Xml | RawType::Html => ContentKind::Xml,
                _ => ContentKind::Text,
            };
            let pal = Palette::new(env.dark);
            let font = TextStyle::Monospace.resolve(ui.style());
            let mut layouter = |ui: &egui::Ui, text: &str, wrap: f32| {
                let job = highlight::code_job(text, kind, &pal, &font, wrap);
                ui.fonts(|f| f.layout_job(job))
            };
            let hint = if kind == ContentKind::Json { "{\n  \"key\": \"value\"\n}" } else { "Request body" };
            let avail = ui.available_size();
            egui::Frame::none()
                .fill(p.code)
                .stroke(Stroke::new(1.0_f32, p.border))
                .rounding(8.0)
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    let min = egui::vec2(ui.available_width(), (avail.y - 22.0).max(70.0));
                    egui::ScrollArea::vertical().id_salt(("raw_body", tab_id)).auto_shrink([false, false]).max_height(min.y).show(
                        ui,
                        |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut body.raw)
                                    .id(egui::Id::new(("raw_body_edit", tab_id)))
                                    .code_editor()
                                    .frame(false)
                                    .hint_text(hint)
                                    .desired_width(f32::INFINITY)
                                    .min_size(min)
                                    .layouter(&mut layouter),
                            );
                        },
                    );
                });
        }
        BodyMode::UrlEncoded => {
            widgets::kv_table(
                ui,
                &mut body.urlencoded,
                KvOptions { id: egui::Id::new(("urlenc", tab_id)), key_hint: "Add field", value_hint: "Value", allow_files: false },
            );
        }
        BodyMode::FormData => {
            let r = widgets::kv_table(
                ui,
                &mut body.form_data,
                KvOptions { id: egui::Id::new(("formdata", tab_id)), key_hint: "Add field", value_hint: "Value", allow_files: true },
            );
            if let Some(row) = r.pick_file {
                actions.push(Action::PickFile(FilePurpose::FormFile { tab_id, row }));
            }
        }
        BodyMode::Binary => {
            ui_kit::card_frame(&p).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui_kit::button(ui, ph::FOLDER_OPEN, "Select file…").clicked() {
                        actions.push(Action::PickFile(FilePurpose::BinaryFile { tab_id }));
                    }
                    ui.add(egui::TextEdit::singleline(&mut body.binary_path).hint_text("/path/to/file").desired_width(f32::INFINITY));
                });
                ui.label(RichText::new("The file contents are sent as the raw request body.").small().color(p.text_weak));
            });
        }
    }
}

fn auth_section(ui: &mut egui::Ui, auth: &mut Auth, tab_id: u64) {
    let p = Pal::of(ui);
    let w = 420.0;
    ui_kit::card_frame(&p).show(ui, |ui| {
        egui::Grid::new(("auth_grid", tab_id)).num_columns(2).spacing([18.0, 12.0]).min_col_width(90.0).show(ui, |ui| {
            ui.label(RichText::new("Type").color(p.text_weak));
            egui::ComboBox::from_id_salt(("auth_kind", tab_id)).width(200.0).selected_text(auth.kind.label()).show_ui(ui, |ui| {
                for k in AuthKind::ALL {
                    ui.selectable_value(&mut auth.kind, k, k.label());
                }
            });
            ui.end_row();
            let field = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, password: bool| {
                ui.label(RichText::new(label).color(p.text_weak));
                ui.add(egui::TextEdit::singleline(value).hint_text(hint).password(password).desired_width(w).margin(egui::vec2(8.0, 5.0)));
                ui.end_row();
            };
            match auth.kind {
                AuthKind::None => {
                    ui.label("");
                    ui.label(RichText::new(format!("{}  This request does not use any authorization.", ph::LOCK_OPEN)).color(p.text_weak));
                    ui.end_row();
                }
                AuthKind::Bearer => field(ui, "Token", &mut auth.token, "Token or {{variable}}", false),
                AuthKind::Basic => {
                    field(ui, "Username", &mut auth.username, "", false);
                    field(ui, "Password", &mut auth.password, "", true);
                }
                AuthKind::ApiKey => {
                    field(ui, "Key", &mut auth.key, "X-API-Key", false);
                    field(ui, "Value", &mut auth.value, "", false);
                    ui.label(RichText::new("Add to").color(p.text_weak));
                    egui::ComboBox::from_id_salt(("apikey_in", tab_id))
                        .width(200.0)
                        .selected_text(if auth.location == ApiKeyIn::Header { "Header" } else { "Query Params" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut auth.location, ApiKeyIn::Header, "Header");
                            ui.selectable_value(&mut auth.location, ApiKeyIn::Query, "Query Params");
                        });
                    ui.end_row();
                }
            }
        });
    });
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!("{}  Variables like {{{{token}}}} come from the collection and the active environment.", ph::INFO))
            .small()
            .color(p.text_weak),
    );
}

fn settings_section(ui: &mut egui::Ui, req: &mut Request) {
    let p = Pal::of(ui);
    ui_kit::card_frame(&p).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        ui_kit::toggle(ui, &mut req.follow_redirects, "Follow redirects", "Automatically follow 3xx responses");
        ui_kit::toggle(ui, &mut req.verify_ssl, "Verify SSL certificates", "Turn off for self-signed certificates");
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}  Request timeout", ph::TIMER)).color(p.text_strong));
            ui.add(egui::DragValue::new(&mut req.timeout_secs).range(1..=3600).suffix(" s"));
        });
    });
}

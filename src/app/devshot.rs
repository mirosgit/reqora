//! Developer-only scripted run that captures window screenshots
//! (compiled only with `--features dev-screenshot`; set REQORA_SHOT_DIR=/some/dir).
//! With REQORA_SHOT_SYNC="<server> <email> <password>" (an existing account) it also
//! signs in for real and captures the result of the first sync. With REQORA_SHOT_INVITE=<code>
//! it creates that account with the invite code instead of signing in.

use super::*;

pub struct DevShot {
    dir: PathBuf,
    step: usize,
    frames: u32,
    pending: Option<&'static str>,
    sync_login: Option<(String, String, String)>,
    sync_invite: Option<String>,
}

impl DevShot {
    pub fn from_env() -> Option<Self> {
        let sync_login = std::env::var("REQORA_SHOT_SYNC").ok().and_then(|v| {
            // The password is everything after the email, so it may contain spaces.
            let (server, rest) = v.trim().split_once(' ')?;
            let (email, password) = rest.trim_start().split_once(' ')?;
            Some((server.to_string(), email.to_string(), password.trim_start().to_string()))
        });
        let sync_invite = std::env::var("REQORA_SHOT_INVITE").ok();
        std::env::var_os("REQORA_SHOT_DIR").map(|d| Self { dir: d.into(), step: 0, frames: 0, pending: None, sync_login, sync_invite })
    }
}

impl App {
    fn devshot_set_theme(&mut self, ctx: &egui::Context, t: ThemeChoice) {
        self.store.ws.settings.theme = t;
        theme::apply_theme(ctx, t);
    }

    pub(super) fn devshot_tick(&mut self, ctx: &egui::Context) {
        let Some(mut ds) = self.devshot.take() else { return };
        ctx.request_repaint();
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let (Some(name), Some(img)) = (ds.pending, shot) {
            let path = ds.dir.join(format!("{name}.png"));
            let [w, h] = img.size;
            let _ = image::save_buffer(&path, img.as_raw(), w as u32, h as u32, image::ColorType::Rgba8);
            eprintln!("devshot: saved {}", path.display());
            ds.pending = None;
            ds.step += 1;
            ds.frames = 0;
        }
        if ds.pending.is_some() {
            self.devshot = Some(ds);
            return;
        }
        ds.frames += 1;
        let first = ds.frames == 1;
        let busy = self.tabs.iter().any(|t| matches!(t, Tab::Request(r) if r.is_loading()))
            || self.sync_ui.busy
            || self.sync_status.in_flight
            || self.sync_status.due.is_some()
            || self.sync_ui.devices.loading
            || !self.sync_ui.devices.revoking.is_empty()
            || self.sync_ui.change.busy;
        let name = match ds.step {
            0 => "01-start",
            1 => {
                if first {
                    if let Some(id) = self.tabs.get(self.active).map(Tab::id) {
                        self.send(id);
                    }
                }
                "02-response"
            }
            2 => {
                if first {
                    if let Some(Tab::Request(t)) = self.tabs.get_mut(self.active) {
                        t.comments_open = true;
                        t.comment_state.draft = "Looks good — ship it after QA 🚀".into();
                    }
                }
                "03-comments"
            }
            3 => {
                if first {
                    if let Some(Tab::Request(t)) = self.tabs.get_mut(self.active) {
                        t.comments_open = false;
                        if let RespState::Done(r) = &mut t.resp {
                            r.search_open = true;
                            r.search = "email".into();
                        }
                    }
                    self.sidebar.section = SidebarSection::History;
                }
                "04-search-history"
            }
            4 => {
                if first {
                    self.devshot_set_theme(ctx, ThemeChoice::Light);
                    self.sidebar.section = SidebarSection::Collections;
                    if let Some(c) = self.store.ws.collections.first() {
                        self.sidebar.expanded.insert(c.id.clone());
                        let id = c.id.clone();
                        self.handle(Action::OpenCollection(id));
                    }
                }
                "05-collection-light"
            }
            5 => {
                if first {
                    if let Some(i) = self.tabs.iter().position(|t| matches!(t, Tab::Request(r) if r.req.method == "POST")) {
                        self.active = i;
                        if let Tab::Request(t) = &mut self.tabs[i] {
                            t.section = ReqSection::Body;
                        }
                        let id = self.tabs[i].id();
                        self.send(id);
                    }
                }
                "06-body-light"
            }
            6 => {
                if first {
                    self.devshot_set_theme(ctx, ThemeChoice::Dark);
                    if let Some(e) = self.store.ws.environments.first() {
                        let id = e.id.clone();
                        self.open_env_tab(id);
                    }
                    self.sidebar.section = SidebarSection::Environments;
                }
                "07-env"
            }
            7 => {
                if first {
                    self.active = 0;
                    if let Some(Tab::Request(t)) = self.tabs.get_mut(0) {
                        t.section = ReqSection::Settings;
                    }
                    self.dialog = Some(Dialog::SaveAs {
                        tab_id: self.tabs[0].id(),
                        name: "List users".into(),
                        collection: None,
                        new_collection: "My Collection".into(),
                        close_after: false,
                        focus: true,
                    });
                }
                "08-dialog"
            }
            8 => {
                if first {
                    let url = "https://sync.example.com".to_string();
                    let info = crate::sync::ServerInfo {
                        name: "reqora-sync".into(),
                        version: "0.1.0".into(),
                        registration: "invite".into(),
                        max_document_bytes: 1 << 20,
                        max_changes_per_push: 500,
                        max_request_bytes: 32 << 20,
                    };
                    let f = &mut self.sync_ui;
                    f.server = url.clone();
                    f.email = "ada@example.com".into();
                    f.password = "correct horse".into();
                    f.display_name = "Ada Lovelace".into();
                    f.invite_code = "SC3N-3XSH-MS7E".into();
                    f.mode = crate::ui_sync::AuthMode::Register;
                    f.check = Some((url, Ok(info)));
                    self.dialog = Some(Dialog::Account);
                }
                "09-sync-sign-in"
            }
            9 => {
                if first {
                    // Signed in, with the last attempt just now so no real sync starts.
                    self.sync.account = Some(crate::sync::Account {
                        server_url: "https://sync.example.com".into(),
                        email: "ada@example.com".into(),
                        display_name: "Ada Lovelace".into(),
                        token: "devshot".into(),
                        ..Default::default()
                    });
                    self.sync_status.last_attempt = Some(Instant::now());
                    self.sync_status.last_ok = Instant::now().checked_sub(Duration::from_secs(120));
                    self.sync_status.due = None;
                }
                "10-sync-account"
            }
            10 => {
                if first {
                    let at = |mins: i64| (chrono::Utc::now() - chrono::Duration::minutes(mins)).to_rfc3339();
                    let device = |id: &str, name: &str, used: i64, since: i64, current: bool| crate::sync::SessionInfo {
                        id: id.into(),
                        device_name: name.into(),
                        created_at: at(since),
                        last_used_at: at(used),
                        current,
                    };
                    let d = &mut self.sync_ui.devices;
                    d.list = Some(Ok(vec![
                        device("a", "Reqora on Linux (ada-laptop)", 0, 60 * 24 * 12, true),
                        device("b", "Reqora on macOS (studio)", 40, 60 * 24 * 30, false),
                        device("c", "Reqora on Windows (OFFICE-PC)", 60 * 24 * 3, 60 * 24 * 70, false),
                    ]));
                    d.loaded_at = Some(Instant::now());
                    d.revoking.clear();
                    self.sync_ui.tab = crate::ui_sync::AccountTab::Devices;
                }
                "11-sync-devices"
            }
            11 => {
                if first {
                    let c = &mut self.sync_ui.change;
                    c.current = "correct horse".into();
                    c.new = "battery staple".into();
                    c.repeat = "battery stapl".into();
                    self.sync_ui.tab = crate::ui_sync::AccountTab::Password;
                    self.on_account_action(crate::ui_sync::FormAction::ChangePassword);
                }
                "12-sync-password"
            }
            12 if ds.sync_login.is_some() => {
                if first {
                    let (server, email, password) = ds.sync_login.clone().unwrap_or_default();
                    self.sync.account = None;
                    self.sync_status = Default::default();
                    let f = &mut self.sync_ui;
                    f.check = None;
                    f.server = server;
                    f.email = email;
                    f.password = password;
                    f.mode = crate::ui_sync::AuthMode::SignIn;
                    if let Some(code) = &ds.sync_invite {
                        f.mode = crate::ui_sync::AuthMode::Register;
                        f.invite_code = code.clone();
                        f.display_name = "Ada".into();
                    }
                    f.merge_local = true;
                    f.merge_touched = true;
                    self.dialog = Some(Dialog::Account);
                    self.on_account_action(crate::ui_sync::FormAction::Submit);
                    self.sidebar.section = SidebarSection::Collections;
                    self.store.ws.settings.show_sidebar = true;
                }
                // Once signed in, show the Devices tab, which loads the real list.
                if self.sync.signed_in() {
                    self.sync_ui.tab = crate::ui_sync::AccountTab::Devices;
                }
                "13-sync-live"
            }
            13 if ds.sync_login.is_some() => {
                if first {
                    let other = match &self.sync_ui.devices.list {
                        Some(Ok(list)) => list.iter().find(|s| !s.current).map(|s| s.id.clone()),
                        _ => None,
                    };
                    if let Some(id) = other {
                        self.on_account_action(crate::ui_sync::FormAction::RevokeDevice(id));
                    }
                }
                "14-sync-live-revoked"
            }
            14 if ds.sync_login.is_some() => {
                if first {
                    // Same password again: exercises the whole flow and keeps the test account usable.
                    let password = ds.sync_login.as_ref().map(|l| l.2.clone()).unwrap_or_default();
                    let c = &mut self.sync_ui.change;
                    c.current = password.clone();
                    c.new = password.clone();
                    c.repeat = password;
                    self.sync_ui.tab = crate::ui_sync::AccountTab::Password;
                    self.on_account_action(crate::ui_sync::FormAction::ChangePassword);
                }
                "15-sync-live-password"
            }
            12..=15 => {
                if first {
                    self.sync.account = None;
                    self.dialog = None;
                    self.tabs.clear();
                    self.active = 0;
                }
                ds.step = 15;
                "16-welcome"
            }
            _ => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.devshot = Some(ds);
                return;
            }
        };
        if !busy && ds.frames > 24 {
            ds.pending = Some(name);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        self.devshot = Some(ds);
    }
}

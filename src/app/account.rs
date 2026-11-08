//! Glue between the app and the sync engine: sign-in, sync rounds, refreshing tabs.

use super::*;
use crate::storage;
use crate::sync::{self, Account, Api, ApiError, LoginOk, RoundOutcome, ServerInfo, SessionInfo};
use crate::ui_sync::{AccountTab, AuthMode, FormAction, SYNC_INTERVAL, SYNC_RETRY};

impl App {
    /// Sync soon (debounced): called whenever the workspace or history is saved.
    pub(super) fn schedule_sync(&mut self, delay: Duration) {
        if self.sync.signed_in() {
            let at = Instant::now() + delay;
            self.sync_status.due = Some(self.sync_status.due.map_or(at, |d| d.min(at)));
        }
    }

    /// Called every frame: starts a round when one is due, otherwise wakes the UI up in time for the next.
    pub(super) fn sync_tick(&mut self) {
        if !self.sync.signed_in() || self.sync_status.in_flight {
            return;
        }
        let now = Instant::now();
        let s = &self.sync_status;
        let periodic = s.last_attempt.map_or(now, |t| t + SYNC_INTERVAL);
        let mut next = s.due.map_or(periodic, |d| d.min(periodic));
        if s.error.is_some() {
            if let Some(t) = s.last_attempt {
                next = next.max(t + SYNC_RETRY);
            }
        }
        if now >= next {
            self.start_sync();
        } else {
            self.ctx.request_repaint_after(next - now);
        }
    }

    pub(super) fn start_sync(&mut self) {
        let Some(account) = self.sync.account.clone().filter(Account::signed_in) else { return };
        if self.sync_status.in_flight {
            return;
        }
        if sync::ensure_unique_ids(&mut self.store.ws, &mut self.store.history) {
            self.ws_dirty = true;
            self.refresh_tabs_after_sync();
        }
        let api = match Api::new(&account.server_url) {
            Ok(api) => api,
            Err(e) => {
                self.sync_status.error = Some(e);
                return;
            }
        };
        let local = self.sync.local_docs(&self.store.ws, &self.store.history);
        let changes = self.sync.outgoing(&local, &account.limits);
        let cursor = self.sync.state.cursor;
        let owner = account.owner();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = sync::run_round(&api, &account.token, changes, cursor, &account.limits).await;
            let _ = tx.send(AppEvent::SyncDone { owner, result: Box::new(result) });
            ctx.request_repaint();
        });
        let s = &mut self.sync_status;
        s.in_flight = true;
        s.due = None;
        s.last_attempt = Some(Instant::now());
    }

    pub(super) fn on_sync_done(&mut self, owner: String, result: Result<RoundOutcome, ApiError>) {
        self.sync_status.in_flight = false;
        // Signed out or switched accounts while the round was running: its results don't apply.
        if self.sync.account.as_ref().filter(|a| a.signed_in()).map(Account::owner) != Some(owner) {
            return;
        }
        match result {
            Ok(outcome) => {
                let report = self.sync.apply(&mut self.store.ws, &mut self.store.history, outcome);
                if report.workspace_changed {
                    self.ws_dirty = true;
                    self.refresh_tabs_after_sync();
                }
                if report.history_changed {
                    if let Err(e) = self.store.save_history() {
                        eprintln!("reqora: cannot save history: {e}");
                    }
                }
                if let Err(e) = self.sync.save_state() {
                    eprintln!("reqora: cannot save sync state: {e}");
                }
                let s = &mut self.sync_status;
                s.last_ok = Some(Instant::now());
                s.error = None;
                s.too_large = self.sync.too_large_count();
                if report.needs_another_round {
                    s.due = Some(Instant::now() + Duration::from_millis(300));
                }
            }
            Err(ApiError::Unauthorized(_)) => self.session_lost(),
            Err(ApiError::CursorAhead) => {
                self.sync.reset_state();
                let _ = self.sync.save_state();
                self.sync_status.due = Some(Instant::now());
            }
            Err(e) => self.sync_status.error = Some(e.to_string()),
        }
    }

    /// Brings open tabs in line with the workspace after the sync changed it.
    pub(super) fn refresh_tabs_after_sync(&mut self) {
        let mut gone = Vec::new();
        for tab in &mut self.tabs {
            match tab {
                Tab::Request(t) if t.collection_id.is_some() => {
                    let found = self
                        .store
                        .ws
                        .collections
                        .iter()
                        .find_map(|c| c.requests.iter().find(|r| r.id == t.req.id).map(|r| (c.id.clone(), r.clone())));
                    match found {
                        Some((cid, r)) => {
                            if t.saved != r {
                                // Unsaved edits in the tab stay; the comment thread always follows the server.
                                if !t.is_dirty() {
                                    t.req = r.clone();
                                }
                                t.req.comments = r.comments.clone();
                                t.saved = r;
                            }
                            t.collection_id = Some(cid);
                        }
                        // Deleted on another device: the tab keeps its content as an unsaved request.
                        None => t.collection_id = None,
                    }
                }
                Tab::Env(e) if !self.store.ws.environments.iter().any(|x| x.id == e.env_id) => gone.push(e.id),
                Tab::Collection(c) if self.store.collection(&c.coll_id).is_none() => gone.push(c.id),
                _ => {}
            }
        }
        for id in gone {
            self.remove_tab(id);
        }
    }

    pub(super) fn open_account_dialog(&mut self) {
        let f = &mut self.sync_ui;
        if let Some(a) = &self.sync.account {
            if f.server.is_empty() {
                f.server = a.server_url.clone();
            }
            if f.email.is_empty() {
                f.email = a.email.clone();
            }
        }
        if f.display_name.is_empty() {
            f.display_name = self.store.ws.settings.author_name.clone();
        }
        f.error = None;
        f.focus = f.server.is_empty();
        f.tab = AccountTab::Sync;
        f.devices.loaded_at = None; // fresh list when the Devices tab is opened
        f.change = Default::default();
        self.dialog = Some(Dialog::Account);
    }

    pub(super) fn on_account_action(&mut self, action: FormAction) {
        match action {
            FormAction::Check => self.start_server_check(),
            FormAction::Submit => self.start_auth(),
            FormAction::SyncNow => {
                self.sync_status.error = None;
                self.start_sync();
            }
            FormAction::SignOut => self.sign_out(),
            FormAction::Close => self.dialog = None,
            FormAction::LoadDevices => self.load_devices(),
            FormAction::RevokeDevice(id) => self.revoke_device(id),
            FormAction::ChangePassword => self.change_password(),
        }
    }

    /// The server no longer accepts our token (it expired, or the device was signed out
    /// from another one): stop syncing and ask to sign in again.
    fn session_lost(&mut self) {
        self.sync.sign_out();
        let _ = self.sync.save_account();
        self.reset_account_views();
        self.sync_status.error = Some("Signed out: the session expired or was revoked. Sign in again to keep syncing.".into());
        self.toast("Sync stopped: sign in again");
    }

    fn reset_account_views(&mut self) {
        self.sync_status = Default::default();
        let f = &mut self.sync_ui;
        f.tab = AccountTab::Sync;
        f.devices = Default::default();
        f.change = Default::default();
    }

    /// Account calls need the signed-in account; results are dropped if it changed meanwhile.
    fn account_api(&self) -> Option<(Api, String, String)> {
        let a = self.sync.account.as_ref().filter(|a| a.signed_in())?;
        Some((Api::new(&a.server_url).ok()?, a.token.clone(), a.owner()))
    }

    fn is_current_owner(&self, owner: &str) -> bool {
        self.sync.account.as_ref().filter(|a| a.signed_in()).is_some_and(|a| a.owner() == owner)
    }

    fn load_devices(&mut self) {
        let Some((api, token, owner)) = self.account_api() else { return };
        self.sync_ui.devices.loading = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = api.sessions(&token).await;
            let _ = tx.send(AppEvent::DevicesLoaded { owner, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn on_devices_loaded(&mut self, owner: String, result: Result<Vec<SessionInfo>, ApiError>) {
        if !self.is_current_owner(&owner) {
            return;
        }
        let d = &mut self.sync_ui.devices;
        d.loading = false;
        d.loaded_at = Some(Instant::now());
        match result {
            Ok(mut list) => {
                list.sort_by_key(|s| !s.current); // this device first, the rest most recently used first
                d.list = Some(Ok(list));
            }
            Err(ApiError::Unauthorized(_)) => self.session_lost(),
            Err(e) => d.list = Some(Err(e.to_string())),
        }
    }

    fn revoke_device(&mut self, id: String) {
        let Some((api, token, owner)) = self.account_api() else { return };
        if !self.sync_ui.devices.revoking.insert(id.clone()) {
            return;
        }
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = api.revoke_session(&token, &id).await;
            let _ = tx.send(AppEvent::DeviceRevoked { owner, id, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn on_device_revoked(&mut self, owner: String, id: String, result: Result<(), ApiError>) {
        if !self.is_current_owner(&owner) {
            return;
        }
        self.sync_ui.devices.revoking.remove(&id);
        match result {
            // 404: it was already signed out.
            Ok(()) | Err(ApiError::Status(404, _)) => {
                if let Some(Ok(list)) = &mut self.sync_ui.devices.list {
                    list.retain(|s| s.id != id);
                }
                self.toast("Device signed out");
            }
            Err(ApiError::Unauthorized(_)) => return self.session_lost(),
            Err(e) => self.toast(format!("Could not sign the device out: {e}")),
        }
        self.load_devices();
    }

    fn change_password(&mut self) {
        let c = &mut self.sync_ui.change;
        c.done = false;
        c.error = if c.current.is_empty() {
            Some("Enter your current password".into())
        } else if c.new.chars().count() < 8 {
            Some("Use a new password of at least 8 characters".into())
        } else if c.new != c.repeat {
            Some("The new passwords don't match".into())
        } else {
            None
        };
        if c.error.is_some() {
            return;
        }
        let (current, new) = (c.current.clone(), c.new.clone());
        let Some((api, token, owner)) = self.account_api() else { return };
        self.sync_ui.change.busy = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = api.change_password(&token, &current, &new).await;
            let _ = tx.send(AppEvent::PasswordChanged { owner, result });
            ctx.request_repaint();
        });
    }

    pub(super) fn on_password_changed(&mut self, owner: String, result: Result<(), ApiError>) {
        if !self.is_current_owner(&owner) {
            return;
        }
        self.sync_ui.change.busy = false;
        match result {
            Ok(()) => {
                self.sync_ui.change = ui_sync::PasswordChange { done: true, ..Default::default() };
                self.sync_ui.devices.loaded_at = None; // the other devices are gone now
                self.toast("Password changed");
            }
            Err(ApiError::Status(403, _)) => self.sync_ui.change.error = Some("The current password is wrong".into()),
            Err(ApiError::Unauthorized(_)) => self.session_lost(),
            Err(e) => self.sync_ui.change.error = Some(e.to_string()),
        }
    }

    fn start_server_check(&mut self) {
        let url = match sync::normalize_server_url(&self.sync_ui.server) {
            Ok(u) => u,
            Err(e) => {
                self.sync_ui.check = Some((self.sync_ui.server.clone(), Err(e)));
                return;
            }
        };
        let api = match Api::new(&url) {
            Ok(a) => a,
            Err(e) => {
                self.sync_ui.check = Some((url, Err(e)));
                return;
            }
        };
        self.sync_ui.checking = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = api.server_info().await.map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::ServerChecked { url, result });
            ctx.request_repaint();
        });
    }

    fn start_auth(&mut self) {
        let f = &mut self.sync_ui;
        f.error = None;
        let url = match sync::normalize_server_url(&f.server) {
            Ok(u) => u,
            Err(e) => {
                f.error = Some(e);
                return;
            }
        };
        let email = f.email.trim().to_string();
        let password = f.password.clone();
        let register = f.mode == AuthMode::Register;
        if email.is_empty() || password.is_empty() {
            f.error = Some("Enter your email and password".into());
            return;
        }
        if register && password.chars().count() < 8 {
            f.error = Some("Use a password of at least 8 characters".into());
            return;
        }
        let api = match Api::new(&url) {
            Ok(a) => a,
            Err(e) => {
                f.error = Some(e);
                return;
            }
        };
        f.busy = true;
        let display_name = f.display_name.trim().to_string();
        let invite_code = f.invite_code.trim().to_string();
        let merge_local = f.merge_local;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let result = async {
                let info = api.server_info().await.map_err(|e| e.to_string())?;
                let login = if register {
                    api.register(&email, &password, &display_name, &invite_code).await
                } else {
                    api.sign_in(&email, &password).await
                };
                login.map(|l| (l, info)).map_err(|e| e.to_string())
            }
            .await;
            let _ = tx.send(AppEvent::AuthDone { url, merge_local, result: Box::new(result) });
            ctx.request_repaint();
        });
    }

    pub(super) fn on_server_checked(&mut self, url: String, result: Result<ServerInfo, String>) {
        self.sync_ui.checking = false;
        self.sync_ui.check = Some((url, result));
    }

    pub(super) fn on_auth_done(&mut self, url: String, merge_local: bool, result: Result<(LoginOk, ServerInfo), String>) {
        self.sync_ui.busy = false;
        let (login, info) = match result {
            Ok(ok) => ok,
            Err(e) => {
                self.sync_ui.error = Some(e);
                return;
            }
        };
        if !merge_local {
            self.move_local_data_aside();
        }
        let account = Account {
            server_url: url,
            email: login.user.email,
            display_name: login.user.display_name,
            user_id: login.user.id,
            session_id: login.session_id,
            token: login.token,
            limits: info.limits(),
        };
        let who = if account.display_name.is_empty() { account.email.clone() } else { account.display_name.clone() };
        self.sync.begin_session(account, merge_local);
        if let Err(e) = self.sync.save_account().and_then(|_| self.sync.save_state()) {
            self.toast(format!("Could not save the account: {e}"));
        }
        self.sync_ui.password.clear();
        self.sync_ui.invite_code.clear();
        self.sync_ui.error = None;
        self.sync_ui.merge_touched = false;
        self.reset_account_views();
        self.start_sync();
        self.toast(format!("Signed in as {who}"));
    }

    fn sign_out(&mut self) {
        if let Some(account) = self.sync.account.clone().filter(Account::signed_in) {
            // Revoke the token on the server too; best effort, the local sign-out happens regardless.
            if let Ok(api) = Api::new(&account.server_url) {
                self.rt.spawn(async move {
                    let _ = api.sign_out(&account.token).await;
                });
            }
        }
        self.sync.sign_out();
        if let Err(e) = self.sync.save_account() {
            self.toast(format!("Could not save the account: {e}"));
        }
        self.reset_account_views();
        self.toast("Signed out. Your data stays on this device");
    }

    /// Signing in without merging: keep a backup of this device's data, then start empty
    /// so the account's data comes down as it is.
    fn move_local_data_aside(&mut self) {
        let dir = self.store.dir().to_path_buf();
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let ws_file = dir.join(format!("workspace.before-sign-in-{stamp}.json"));
        let backup = serde_json::to_string_pretty(&self.store.ws)
            .map_err(|e| e.to_string())
            .and_then(|t| storage::write_atomic(&ws_file, t.as_bytes()))
            .and_then(|_| serde_json::to_string_pretty(&self.store.history).map_err(|e| e.to_string()))
            .and_then(|t| storage::write_atomic(&dir.join(format!("history.before-sign-in-{stamp}.json")), t.as_bytes()));
        if let Err(e) = backup {
            // Never drop data that couldn't be backed up: merge it instead.
            self.toast(format!("Could not back up local data ({e}); merging it into the account"));
            return;
        }
        let ws = &mut self.store.ws;
        ws.collections.clear();
        ws.environments.clear();
        ws.active_environment = None;
        self.store.history.clear();
        let _ = self.store.save_history();
        self.ws_dirty = true;
        self.refresh_tabs_after_sync();
        self.toast(format!("Previous data saved to {}", ws_file.display()));
    }

    pub(super) fn sync_indicator(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = Pal::of(ui);
        let (icon, text, color, tip) = ui_sync::indicator(&p, self.sync.account.as_ref(), &self.sync_status);
        let label = RichText::new(format!("{icon}  {text}")).font(theme::regular(11.5)).color(color);
        let r = ui.add(egui::Label::new(label).sense(Sense::click()));
        if r.on_hover_text(format!("{tip}\nClick for sync settings")).clicked() {
            actions.push(Action::OpenAccount);
        }
    }
}

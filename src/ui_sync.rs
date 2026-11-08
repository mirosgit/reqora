//! "Sync & account" dialog: pick a sync server, sign in or create an account, see sync status.

use crate::sync::{self, Account, ServerInfo, SessionInfo};
use crate::theme::{self, Pal};
use crate::ui_kit;
use eframe::egui::{self, RichText};
use egui_phosphor::regular as ph;
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    #[default]
    SignIn,
    Register,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum AccountTab {
    #[default]
    Sync,
    Devices,
    Password,
}

/// The account's devices, as last loaded from the server.
#[derive(Default)]
pub struct DevicesState {
    pub loading: bool,
    pub list: Option<Result<Vec<SessionInfo>, String>>,
    pub loaded_at: Option<Instant>,
    /// Devices being signed out right now.
    pub revoking: HashSet<String>,
}

#[derive(Default)]
pub struct PasswordChange {
    pub current: String,
    pub new: String,
    pub repeat: String,
    pub busy: bool,
    pub error: Option<String>,
    pub done: bool,
}

/// What's typed into the dialog. Kept while the dialog is closed, except the password.
#[derive(Default)]
pub struct SyncForm {
    pub server: String,
    pub email: String,
    pub password: String,
    pub display_name: String,
    /// Needed to create an account unless the server has open sign-up.
    pub invite_code: String,
    pub mode: AuthMode,
    /// Upload this device's data to the account (otherwise it is backed up and replaced).
    pub merge_local: bool,
    /// The user flipped `merge_local` themselves, so it's no longer derived from the inputs.
    pub merge_touched: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub checking: bool,
    /// Result of the last server check, and the address it was for.
    pub check: Option<(String, Result<ServerInfo, String>)>,
    pub focus: bool,
    /// Signed-in view.
    pub tab: AccountTab,
    pub devices: DevicesState,
    pub change: PasswordChange,
}

/// Sync status shown in the status bar and the dialog.
#[derive(Default)]
pub struct SyncStatus {
    pub in_flight: bool,
    pub last_ok: Option<Instant>,
    pub last_attempt: Option<Instant>,
    pub error: Option<String>,
    /// A sync is wanted at this time (debounced after local changes).
    pub due: Option<Instant>,
    pub too_large: usize,
}

pub enum FormAction {
    Check,
    Submit,
    SyncNow,
    SignOut,
    Close,
    LoadDevices,
    RevokeDevice(String),
    ChangePassword,
}

/// Reload the device list when it's this old and the Devices tab is open.
const DEVICES_REFRESH: Duration = Duration::from_secs(30);

/// Everything the dialog shows besides the form.
pub struct DialogView<'a> {
    pub account: Option<&'a Account>,
    pub status: &'a SyncStatus,
    /// Collections, environments and history entries on this device.
    pub local: (usize, usize, usize),
}

/// "just now", "5 min ago", "3 days ago", or a date for anything older than a month.
pub fn relative_time(t: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> String {
    let mins = (now - t).num_minutes().max(0);
    match mins {
        0..=9 => "just now".into(),
        10..=59 => format!("{mins} min ago"),
        60..=1439 => format!("{} h ago", mins / 60),
        1440..=2879 => "yesterday".into(),
        2880..=43199 => format!("{} days ago", mins / 1440),
        _ => format!("on {}", short_date(t)),
    }
}

fn short_date(t: chrono::DateTime<chrono::Utc>) -> String {
    t.with_timezone(&chrono::Local).format("%b %-d, %Y").to_string()
}

pub fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        _ => format!("{} h ago", s / 3600),
    }
}

/// Icon, short text, color and tooltip for the status bar.
pub fn indicator(p: &Pal, account: Option<&Account>, status: &SyncStatus) -> (&'static str, String, egui::Color32, String) {
    match account.filter(|a| a.signed_in()) {
        None => (ph::CLOUD_SLASH, "Sync off".into(), p.text_faint, "Sign in to a sync server to sync between devices".into()),
        Some(_) if status.in_flight => (ph::ARROWS_CLOCKWISE, "Syncing…".into(), p.accent, "Syncing with the server".into()),
        Some(a) => match (&status.error, status.last_ok) {
            (Some(e), _) => (ph::CLOUD_WARNING, "Sync error".into(), p.danger, e.clone()),
            (None, Some(t)) => (ph::CLOUD_CHECK, "Synced".into(), p.success, format!("Synced {} · {}", ago(t), a.server_url)),
            (None, None) => (ph::CLOUD, "Sync pending".into(), p.text_weak, a.server_url.clone()),
        },
    }
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    let p = Pal::of(ui);
    ui.add_space(6.0);
    ui.label(RichText::new(text).color(p.text_weak));
}

fn text_field(ui: &mut egui::Ui, value: &mut String, hint: &str, password: bool, width: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(value).hint_text(hint).password(password).desired_width(width).margin(egui::vec2(8.0, 6.0)))
}

pub fn account_dialog(ui: &mut egui::Ui, form: &mut SyncForm, view: &DialogView, enter: bool) -> Option<FormAction> {
    ui.set_max_width(400.0);
    match view.account.filter(|a| a.signed_in()) {
        Some(account) => signed_in(ui, form, account, view, enter),
        None => sign_in_form(ui, form, view, enter),
    }
}

fn signed_in(ui: &mut egui::Ui, form: &mut SyncForm, account: &Account, view: &DialogView, enter: bool) -> Option<FormAction> {
    let p = Pal::of(ui);
    let mut action = None;
    let name = if account.display_name.trim().is_empty() { &account.email } else { &account.display_name };
    ui.horizontal(|ui| {
        ui_kit::avatar(ui, name, 38.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(name.as_str()).font(theme::semibold(14.5)).color(p.text_strong));
            ui.label(RichText::new(&account.email).color(p.text_weak));
        });
    });
    ui.add_space(12.0);
    let tabs = [(AccountTab::Sync, "Sync"), (AccountTab::Devices, "Devices"), (AccountTab::Password, "Password")];
    ui_kit::segmented(ui, egui::Id::new("sync_account_tab"), &mut form.tab, &tabs, true);
    ui.add_space(12.0);
    match form.tab {
        AccountTab::Sync => sync_tab(ui, account, view),
        AccountTab::Devices => devices_tab(ui, form, &mut action),
        AccountTab::Password => password_tab(ui, form),
    }

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        match form.tab {
            AccountTab::Sync => {
                let r = ui.add_enabled_ui(!view.status.in_flight, |ui| {
                    ui_kit::primary_button(ui, Some(ph::ARROWS_CLOCKWISE), "Sync now", egui::vec2(110.0, 32.0))
                });
                if r.inner.clicked() {
                    action = Some(FormAction::SyncNow);
                }
            }
            AccountTab::Password => {
                let busy = form.change.busy;
                let r = ui.add_enabled_ui(!busy, |ui| {
                    ui_kit::primary_button(ui, Some(ph::KEY), if busy { "Changing…" } else { "Change password" }, egui::vec2(150.0, 32.0))
                });
                if r.inner.clicked() || (enter && !busy) {
                    action = Some(FormAction::ChangePassword);
                }
            }
            AccountTab::Devices => {}
        }
        if ui_kit::button(ui, ph::SIGN_OUT, "Sign out here").on_hover_text("Sign out on this device. Your data stays here").clicked() {
            action = Some(FormAction::SignOut);
        }
        if ui_kit::button(ui, "", "Close").clicked() {
            action = Some(FormAction::Close);
        }
    });
    action
}

fn sync_tab(ui: &mut egui::Ui, account: &Account, view: &DialogView) {
    let p = Pal::of(ui);
    let status = view.status;
    let (status_text, status_color) = if status.in_flight {
        ("Syncing…".to_string(), p.accent)
    } else if let Some(e) = &status.error {
        (e.clone(), p.danger)
    } else if let Some(t) = status.last_ok {
        (format!("Up to date · synced {}", ago(t)), p.success)
    } else {
        ("Waiting for the first sync".to_string(), p.text_weak)
    };
    let (c, e, h) = view.local;
    ui_kit::card_frame(&p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::Grid::new("sync_account_grid").num_columns(2).spacing(egui::vec2(14.0, 8.0)).show(ui, |ui| {
            ui.label(RichText::new("Server").color(p.text_weak));
            ui.label(RichText::new(&account.server_url).color(p.text));
            ui.end_row();
            ui.label(RichText::new("Status").color(p.text_weak));
            ui.add(egui::Label::new(RichText::new(status_text).color(status_color)).wrap());
            ui.end_row();
            ui.label(RichText::new("Data").color(p.text_weak));
            ui.label(RichText::new(format!("{c} collections · {e} environments · {h} history entries")).color(p.text));
            ui.end_row();
        });
    });
    if status.too_large > 0 {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "{} {} too large for the server and not synced (over {} KB).",
                ph::WARNING,
                if status.too_large == 1 { "1 item is".to_string() } else { format!("{} items are", status.too_large) },
                account.limits.max_document_bytes / 1024
            ))
            .color(p.warning),
        );
    }
}

fn devices_tab(ui: &mut egui::Ui, form: &mut SyncForm, action: &mut Option<FormAction>) {
    let p = Pal::of(ui);
    let d = &form.devices;
    if !d.loading && d.loaded_at.is_none_or(|t| t.elapsed() > DEVICES_REFRESH) {
        *action = Some(FormAction::LoadDevices);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Signed in on these devices").color(p.text_weak));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if d.loading {
                ui.add(egui::Spinner::new().size(14.0));
            } else if ui_kit::icon_button(ui, ph::ARROW_CLOCKWISE, "Refresh").clicked() {
                *action = Some(FormAction::LoadDevices);
            }
        });
    });
    ui.add_space(6.0);
    match &d.list {
        None => {
            ui.label(RichText::new("Loading…").color(p.text_weak));
        }
        Some(Err(e)) => {
            ui.add(egui::Label::new(RichText::new(format!("{}  {e}", ph::WARNING_CIRCLE)).color(p.danger)).wrap());
        }
        Some(Ok(list)) => {
            let now = chrono::Utc::now();
            ui_kit::card_frame(&p).inner_margin(egui::Margin::symmetric(12.0, 6.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, s) in list.iter().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        device_row(ui, s, now, d.revoking.contains(&s.id), action);
                    }
                });
            });
        }
    }
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            RichText::new("A device that's signed out stops syncing; its data stays on it. Don't recognize one? Change your password.")
                .small()
                .color(p.text_weak),
        )
        .wrap(),
    );
}

fn device_row(ui: &mut egui::Ui, s: &SessionInfo, now: chrono::DateTime<chrono::Utc>, revoking: bool, action: &mut Option<FormAction>) {
    let p = Pal::of(ui);
    let since = sync::parse_time(&s.created_at).map(|t| format!("signed in {}", short_date(t))).unwrap_or_default();
    let detail = if s.current {
        since
    } else {
        let active = sync::parse_time(&s.last_used_at).map(|t| format!("Active {}", relative_time(t, now))).unwrap_or_default();
        [active, since].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")
    };
    let name = if s.device_name.trim().is_empty() { "Unknown device" } else { s.device_name.as_str() };
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        let icon = if s.current { ph::LAPTOP } else { ph::DESKTOP };
        ui.label(RichText::new(icon).size(20.0).color(if s.current { p.accent } else { p.text_weak }));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new(crate::widgets::truncate_chars(name, 34)).font(theme::medium(13.0)).color(p.text_strong))
                .on_hover_text(name);
            ui.label(RichText::new(detail).small().color(p.text_weak));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if s.current {
                ui_kit::chip(ui, "This device", p.accent);
            } else if revoking {
                ui.add(egui::Spinner::new().size(14.0));
            } else if ui_kit::button(ui, ph::SIGN_OUT, "Sign out").on_hover_text("Sign this device out").clicked() {
                *action = Some(FormAction::RevokeDevice(s.id.clone()));
            }
        });
    });
}

fn password_tab(ui: &mut egui::Ui, form: &mut SyncForm) {
    let p = Pal::of(ui);
    let width = 380.0;
    let c = &mut form.change;
    ui.add(
        egui::Label::new(
            RichText::new("Changing the password signs you out on all your other devices. Sign in there again with the new one.")
                .color(p.text_weak),
        )
        .wrap(),
    );
    let mut changed = false;
    field_label(ui, "Current password");
    changed |= text_field(ui, &mut c.current, "", true, width).changed();
    field_label(ui, "New password");
    changed |= text_field(ui, &mut c.new, "At least 8 characters", true, width).changed();
    field_label(ui, "Repeat the new password");
    changed |= text_field(ui, &mut c.repeat, "", true, width).changed();
    if changed {
        c.done = false;
        c.error = None;
    }
    if let Some(err) = &c.error {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(format!("{}  {err}", ph::WARNING_CIRCLE)).color(p.danger)).wrap());
    } else if c.done {
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                RichText::new(format!("{}  Password changed. Your other devices were signed out.", ph::CHECK_CIRCLE)).color(p.success),
            )
            .wrap(),
        );
    }
}

fn sign_in_form(ui: &mut egui::Ui, form: &mut SyncForm, view: &DialogView, enter: bool) -> Option<FormAction> {
    let p = Pal::of(ui);
    let mut action = None;
    let width = 380.0;
    ui.add(
        egui::Label::new(
            RichText::new(
                "Sync collections, environments and history between your devices through a Reqora sync server. \
                 Use your team's server or run your own.",
            )
            .color(p.text_weak),
        )
        .wrap(),
    );
    if let Some(e) = view.status.error.as_ref().filter(|_| form.error.is_none()) {
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{}  {e}", ph::WARNING_CIRCLE)).color(p.warning));
    }

    field_label(ui, "Server");
    ui.horizontal(|ui| {
        let r = text_field(ui, &mut form.server, "https://sync.example.com", false, width - 84.0);
        if std::mem::take(&mut form.focus) {
            r.request_focus();
        }
        let check =
            ui.add_enabled_ui(!form.checking && !form.server.trim().is_empty(), |ui| ui_kit::button(ui, ph::PLUGS_CONNECTED, "Check"));
        if check.inner.on_hover_text("Check that a Reqora sync server answers at this address").clicked() {
            action = Some(FormAction::Check);
        }
    });
    let normalized = sync::normalize_server_url(&form.server).ok();
    if form.checking {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(13.0));
            ui.label(RichText::new("Checking…").color(p.text_weak));
        });
    } else if let Some((url, result)) = form.check.as_ref().filter(|(url, _)| Some(url) == normalized.as_ref()) {
        let (text, color) = match result {
            Ok(info) => (
                format!(
                    "{}  Reqora sync server {} · {}",
                    ph::CHECK_CIRCLE,
                    info.version,
                    if info.invite_only() { "sign-up by invite" } else { "open sign-up" }
                ),
                p.success,
            ),
            Err(e) => (format!("{}  {e}", ph::WARNING_CIRCLE), p.danger),
        };
        ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
        if sync::is_insecure(url) {
            ui.label(
                RichText::new(format!("{}  Not encrypted: use https:// unless the server is on your own network", ph::LOCK_SIMPLE_OPEN))
                    .color(p.warning),
            );
        }
    }

    ui.add_space(10.0);
    // Known only after a check; until then assume the (default) invite-only setup.
    let open_sign_up = matches!(&form.check, Some((url, Ok(info))) if Some(url) == normalized.as_ref() && !info.invite_only());
    let modes = [(AuthMode::SignIn, "Sign in"), (AuthMode::Register, "Create account")];
    if ui_kit::segmented(ui, egui::Id::new("sync_auth_mode"), &mut form.mode, &modes, true) {
        form.error = None;
    }

    field_label(ui, "Email");
    text_field(ui, &mut form.email, "you@example.com", false, width);
    field_label(ui, "Password");
    let hint = if form.mode == AuthMode::Register { "At least 8 characters" } else { "" };
    text_field(ui, &mut form.password, hint, true, width);
    if form.mode == AuthMode::Register {
        field_label(ui, "Your name");
        text_field(ui, &mut form.display_name, "Shown on your comments", false, width);
        if !open_sign_up {
            field_label(ui, "Invite code");
            text_field(ui, &mut form.invite_code, "XXXX-XXXX-XXXX", false, width);
            ui.label(RichText::new("Ask the server's administrator for one.").small().color(p.text_weak));
        }
    }

    let (c, e, h) = view.local;
    if c + e + h > 0 {
        // Default: upload when this is the first account on this device or the same one as before.
        if !form.merge_touched {
            form.merge_local = match view.account {
                None => true,
                Some(prev) => Some(&prev.server_url) == normalized.as_ref() && prev.email.eq_ignore_ascii_case(form.email.trim()),
            };
        }
        ui.add_space(10.0);
        let hint = format!("{c} collections, {e} environments, {h} history entries");
        if ui_kit::toggle(ui, &mut form.merge_local, "Upload this device's data", "") {
            form.merge_touched = true;
        }
        let note = if form.merge_local {
            format!("{hint} will be merged into the account.")
        } else {
            format!("{hint} will be saved to a backup file and replaced with the account's data.")
        };
        ui.add(egui::Label::new(RichText::new(note).small().color(p.text_weak)).wrap());
    } else {
        form.merge_local = true;
    }

    if let Some(err) = &form.error {
        ui.add_space(8.0);
        ui.add(egui::Label::new(RichText::new(format!("{}  {err}", ph::WARNING_CIRCLE)).color(p.danger)).wrap());
    }

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let label = match (form.mode, form.busy) {
            (AuthMode::SignIn, false) => "Sign in",
            (AuthMode::SignIn, true) => "Signing in…",
            (AuthMode::Register, false) => "Create account",
            (AuthMode::Register, true) => "Creating…",
        };
        let icon = if form.mode == AuthMode::SignIn { ph::SIGN_IN } else { ph::USER_CIRCLE_PLUS };
        let r = ui.add_enabled_ui(!form.busy, |ui| ui_kit::primary_button(ui, Some(icon), label, egui::vec2(140.0, 32.0)));
        if r.inner.clicked() || (enter && !form.busy) {
            action = Some(FormAction::Submit);
        }
        if form.busy {
            ui.add(egui::Spinner::new().size(16.0));
        }
        if ui_kit::button(ui, "", "Cancel").clicked() {
            action = Some(FormAction::Close);
        }
    });
    action
}

/// How long to wait after a local change before syncing it.
pub const SYNC_DEBOUNCE: Duration = Duration::from_secs(2);
/// How often to check the server for changes made on other devices.
pub const SYNC_INTERVAL: Duration = Duration::from_secs(30);
/// Minimum time between attempts while the server is failing.
pub const SYNC_RETRY: Duration = Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn relative_times() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
        let ago = |mins: i64| relative_time(now - chrono::Duration::minutes(mins), now);
        assert_eq!(ago(3), "just now");
        assert_eq!(ago(-5), "just now"); // clock skew
        assert_eq!(ago(25), "25 min ago");
        assert_eq!(ago(150), "2 h ago");
        assert_eq!(ago(60 * 30), "yesterday");
        assert_eq!(ago(60 * 24 * 5), "5 days ago");
        assert!(ago(60 * 24 * 90).starts_with("on Aug"));
    }
}

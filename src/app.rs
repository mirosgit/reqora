//! Application state, event loop glue, actions and dialogs.

use crate::http::{self, HttpEngine};
use crate::model::*;
use crate::response::{self, Processed, Row};
use crate::storage::{Session, SessionTab, Store};
use crate::theme::Pal;
use crate::ui_comments::{self, CommentState};
use crate::widgets::{self, KvOptions};
use crate::{curl, postman, theme, ui_kit, ui_request, ui_sidebar};
use eframe::egui::{self, Color32, Key, KeyboardShortcut, Modifiers, RichText, Sense, Stroke};
use egui_phosphor::regular as ph;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[cfg(feature = "dev-screenshot")]
mod devshot;

// ---------------------------------------------------------------------------
// Tab state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReqSection {
    Params,
    Auth,
    Headers,
    Body,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RespSection {
    Body,
    Headers,
    Request,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BodyView {
    Pretty,
    Raw,
    Preview,
}

pub struct RespUi {
    pub data: Processed,
    pub texture: Option<egui::TextureHandle>,
    pub section: RespSection,
    pub view: BodyView,
    pub search_open: bool,
    pub focus_search: bool,
    pub search: String,
    pub searched: Option<(String, BodyView)>,
    pub matches: Vec<response::Match>,
    pub current: usize,
    pub scroll_to: Option<usize>,
}

impl RespUi {
    fn new(data: Processed, texture: Option<egui::TextureHandle>) -> Self {
        let view = if texture.is_some() { BodyView::Preview } else { BodyView::Pretty };
        Self {
            data,
            texture,
            section: RespSection::Body,
            view,
            search_open: false,
            focus_search: false,
            search: String::new(),
            searched: None,
            matches: Vec::new(),
            current: 0,
            scroll_to: None,
        }
    }

    pub fn text_rows(&self) -> (&str, &[Row]) {
        if self.view == BodyView::Raw {
            (&self.data.raw, &self.data.raw_rows)
        } else {
            (&self.data.pretty, &self.data.pretty_rows)
        }
    }
}

pub enum RespState {
    Idle,
    Loading { seq: u64, started: Instant, handle: tokio::task::JoinHandle<()> },
    Done(Box<RespUi>),
    Failed(String),
}

pub struct RequestTab {
    pub id: u64,
    pub req: Request,
    /// Snapshot of the last saved/loaded state (for the "unsaved" indicator).
    pub saved: Request,
    /// Collection the request is saved in (None = not saved yet).
    pub collection_id: Option<String>,
    pub section: ReqSection,
    pub resp: RespState,
    pub seq: u64,
    pub editing_name: bool,
    pub focus_name: bool,
    pub comments_open: bool,
    pub comment_state: CommentState,
}

impl RequestTab {
    pub fn new(id: u64, req: Request, collection_id: Option<String>) -> Self {
        Self {
            id,
            saved: req.clone(),
            req,
            collection_id,
            section: ReqSection::Params,
            resp: RespState::Idle,
            seq: 0,
            editing_name: false,
            focus_name: false,
            comments_open: false,
            comment_state: CommentState::default(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.req.content_eq(&self.saved)
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.resp, RespState::Loading { .. })
    }

    fn is_pristine(&self) -> bool {
        self.collection_id.is_none() && !self.is_dirty() && self.req.url.is_empty() && matches!(self.resp, RespState::Idle)
    }
}

pub struct EnvTab {
    pub id: u64,
    pub env_id: String,
}

/// Collection overview: description, variables, requests and comments.
pub struct CollTab {
    pub id: u64,
    pub coll_id: String,
    pub comment_state: CommentState,
}

#[allow(clippy::large_enum_variant)] // a handful of tabs, not worth boxing
pub enum Tab {
    Request(RequestTab),
    Env(EnvTab),
    Collection(CollTab),
}

impl Tab {
    pub fn id(&self) -> u64 {
        match self {
            Tab::Request(t) => t.id,
            Tab::Env(e) => e.id,
            Tab::Collection(c) => c.id,
        }
    }
}

fn find_req(tabs: &mut [Tab], id: u64) -> Option<&mut RequestTab> {
    tabs.iter_mut().find_map(|t| match t {
        Tab::Request(r) if r.id == id => Some(r),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// Actions, events, dialogs
// ---------------------------------------------------------------------------

pub enum FilePurpose {
    Import,
    FormFile { tab_id: u64, row: usize },
    BinaryFile { tab_id: u64 },
}

#[allow(clippy::large_enum_variant)] // short-lived channel messages
pub enum AppEvent {
    Response { tab_id: u64, seq: u64, result: Result<Processed, String>, request: Box<Request> },
    FilePicked { purpose: FilePurpose, path: Option<PathBuf> },
    Notify(String),
}

#[derive(Clone, PartialEq, Eq)]
pub enum RenameTarget {
    Collection(String),
    Request(String, String),
    Environment(String),
}

#[derive(Clone)]
pub enum DeleteTarget {
    Collection(String),
    Request(String, String),
    Environment(String),
    History,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidebarSection {
    Collections,
    Environments,
    History,
}

pub struct SidebarState {
    pub section: SidebarSection,
    pub filter: String,
    pub expanded: HashSet<String>,
    pub renaming: Option<RenameTarget>,
    pub rename_buf: String,
    pub rename_focus: bool,
}

/// Everything the UI can ask for. UI code only pushes actions; they are applied
/// after the frame is laid out, which keeps borrowing simple.
pub enum Action {
    Send(u64),
    Cancel(u64),
    Save(u64),
    SaveAs(u64),
    CopyCurl(u64),
    DuplicateTab(u64),
    CurlPasted(u64),
    PickFile(FilePurpose),
    SaveResponse(u64),
    Toast(String),
    CopyText(String, &'static str),
    NewTab,
    ActivateTab(usize),
    CloseTab(u64),
    CloseOtherTabs(u64),
    OpenRequest(String, String),
    OpenHistory(usize),
    NewCollection,
    AddRequest(String),
    StartRename(RenameTarget, String),
    Rename(RenameTarget, String),
    DuplicateRequest(String, String),
    AskDelete(DeleteTarget),
    ExportCollection(String),
    NewEnvironment,
    OpenEnvironment(String),
    SetActiveEnv(Option<String>),
    ExportEnvironment(String),
    Import,
    ClearCookies,
    About,
    SyncComments(u64),
    OpenCollection(String),
}

enum Dialog {
    SaveAs { tab_id: u64, name: String, collection: Option<String>, new_collection: String, close_after: bool, focus: bool },
    ConfirmClose { tab_id: u64 },
    ConfirmDelete { target: DeleteTarget, message: String },
    About,
}

enum DialogResult {
    SaveAs { tab_id: u64, name: String, collection: Option<String>, new_collection: String, close_after: bool },
    Discard(u64),
    SaveAndClose(u64),
    Delete(DeleteTarget),
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub store: Store,
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_tab_id: u64,
    pub sidebar: SidebarState,
    rt: tokio::runtime::Runtime,
    engine: Arc<HttpEngine>,
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
    dialog: Option<Dialog>,
    toasts: Vec<(String, Instant)>,
    ws_dirty: bool,
    ws_saved_at: Instant,
    session_checked_at: Instant,
    last_session: String,
    ctx: egui::Context,
    focus_url: bool,
    #[cfg(feature = "dev-screenshot")]
    devshot: Option<devshot::DevShot>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::setup(&cc.egui_ctx);
        let store = Store::open();
        theme::apply_theme(&cc.egui_ctx, store.ws.settings.theme);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("reqora-net")
            .enable_all()
            .build()
            .expect("failed to start the async runtime");
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            store,
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 0,
            sidebar: SidebarState {
                section: SidebarSection::Collections,
                filter: String::new(),
                expanded: HashSet::new(),
                renaming: None,
                rename_buf: String::new(),
                rename_focus: false,
            },
            rt,
            engine: Arc::new(HttpEngine::default()),
            tx,
            rx,
            dialog: None,
            toasts: Vec::new(),
            ws_dirty: false,
            ws_saved_at: Instant::now(),
            session_checked_at: Instant::now(),
            last_session: String::new(),
            ctx: cc.egui_ctx.clone(),
            focus_url: false,
            #[cfg(feature = "dev-screenshot")]
            devshot: devshot::DevShot::from_env(),
        };
        app.restore_session();
        if app.tabs.is_empty() {
            app.new_tab();
        }
        app
    }

    fn alloc_id(&mut self) -> u64 {
        self.next_tab_id += 1;
        self.next_tab_id
    }

    fn restore_session(&mut self) {
        let Some(session) = self.store.load_session() else { return };
        for t in session.tabs {
            let cid = t.collection_id.filter(|cid| self.store.find_request(cid, &t.request.id).is_some());
            let id = self.alloc_id();
            let mut tab = RequestTab::new(id, t.request, cid);
            tab.saved = t.saved;
            self.tabs.push(Tab::Request(tab));
        }
        self.active = session.active.min(self.tabs.len().saturating_sub(1));
        self.last_session = self.session_json();
    }

    fn session_json(&self) -> String {
        let mut active = 0;
        let mut tabs = Vec::new();
        for (i, t) in self.tabs.iter().enumerate() {
            if let Tab::Request(t) = t {
                if i <= self.active {
                    active = tabs.len();
                }
                tabs.push(SessionTab { request: t.req.normalized(), saved: t.saved.clone(), collection_id: t.collection_id.clone() });
            }
        }
        serde_json::to_string(&Session { tabs, active }).unwrap_or_default()
    }

    /// Debounced persistence of the workspace and the open tabs.
    fn persist(&mut self, force: bool) {
        if self.ws_dirty {
            if force || self.ws_saved_at.elapsed() > Duration::from_millis(700) {
                if let Err(e) = self.store.save_workspace() {
                    self.toast(format!("Could not save workspace: {e}"));
                }
                self.ws_dirty = false;
                self.ws_saved_at = Instant::now();
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(800));
            }
        }
        if force || self.session_checked_at.elapsed() > Duration::from_secs(3) {
            self.session_checked_at = Instant::now();
            let s = self.session_json();
            if s != self.last_session && self.store.save_session_text(&s).is_ok() {
                self.last_session = s;
            }
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), Instant::now()));
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn new_tab(&mut self) {
        let id = self.alloc_id();
        self.tabs.push(Tab::Request(RequestTab::new(id, Request::default(), None)));
        self.active = self.tabs.len() - 1;
        self.focus_url = true;
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id() == id)
    }

    /// Opens a request in a new tab, reusing the current tab if it is an untouched blank one.
    fn open_request_tab(&mut self, req: Request, cid: Option<String>) {
        let id = self.alloc_id();
        let tab = Tab::Request(RequestTab::new(id, req, cid));
        if let Some(Tab::Request(t)) = self.tabs.get(self.active) {
            if t.is_pristine() {
                self.tabs[self.active] = tab;
                return;
            }
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    fn open_env_tab(&mut self, env_id: String) {
        if let Some(i) = self.tabs.iter().position(|t| matches!(t, Tab::Env(e) if e.env_id == env_id)) {
            self.active = i;
            return;
        }
        let id = self.alloc_id();
        self.tabs.push(Tab::Env(EnvTab { id, env_id }));
        self.active = self.tabs.len() - 1;
    }

    fn request_close(&mut self, tab_id: u64) {
        let dirty = matches!(self.tabs.iter().find(|t| t.id() == tab_id), Some(Tab::Request(t)) if t.is_dirty());
        if dirty {
            self.dialog = Some(Dialog::ConfirmClose { tab_id });
        } else {
            self.remove_tab(tab_id);
        }
    }

    fn remove_tab(&mut self, tab_id: u64) {
        let Some(i) = self.tab_index(tab_id) else { return };
        if let Tab::Request(RequestTab { resp: RespState::Loading { handle, .. }, .. }) = &self.tabs[i] {
            handle.abort();
        }
        self.tabs.remove(i);
        if i < self.active {
            self.active -= 1;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
    }

    fn send(&mut self, tab_id: u64) {
        let cid = match self.tabs.iter().find(|t| t.id() == tab_id) {
            Some(Tab::Request(t)) => t.collection_id.clone(),
            _ => None,
        };
        let vars = self.store.vars_for(cid.as_deref());
        let Some(t) = find_req(&mut self.tabs, tab_id) else { return };
        if t.is_loading() {
            return;
        }
        let prepared = match http::prepare(&t.req, &vars, true) {
            Ok(p) => p,
            Err(e) => {
                t.resp = RespState::Failed(e);
                return;
            }
        };
        t.seq += 1;
        let seq = t.seq;
        let mut snapshot = Box::new(t.req.normalized());
        snapshot.comments.clear(); // history keeps the request, not the discussion
        let engine = self.engine.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let handle = self.rt.spawn(async move {
            let result = match http::execute(&engine, &prepared).await {
                Ok(resp) => tokio::task::spawn_blocking(move || response::process(resp, &prepared))
                    .await
                    .map_err(|e| format!("Internal error: {e}")),
                Err(e) => Err(e),
            };
            let _ = tx.send(AppEvent::Response { tab_id, seq, result, request: snapshot });
            ctx.request_repaint();
        });
        t.resp = RespState::Loading { seq, started: Instant::now(), handle };
    }

    fn poll_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                AppEvent::Response { tab_id, seq, result, request } => {
                    let (status, elapsed_ms, error) = match &result {
                        Ok(p) => (Some(p.resp.status), Some(p.resp.elapsed.as_millis() as u64), None),
                        Err(e) => (None, None, Some(e.clone())),
                    };
                    self.store.add_history(HistoryEntry {
                        id: new_id(),
                        timestamp_ms: now_ms(),
                        request: *request,
                        status,
                        elapsed_ms,
                        error,
                    });
                    if let Err(e) = self.store.save_history() {
                        eprintln!("reqora: cannot save history: {e}");
                    }
                    let ctx = self.ctx.clone();
                    if let Some(t) = find_req(&mut self.tabs, tab_id) {
                        if matches!(t.resp, RespState::Loading { seq: s, .. } if s == seq) {
                            t.resp = match result {
                                Ok(mut p) => {
                                    let tex = p
                                        .image
                                        .take()
                                        .map(|img| ctx.load_texture(format!("response-{tab_id}-{seq}"), img, egui::TextureOptions::LINEAR));
                                    RespState::Done(Box::new(RespUi::new(p, tex)))
                                }
                                Err(e) => RespState::Failed(e),
                            };
                        }
                    }
                }
                AppEvent::FilePicked { purpose, path: Some(path) } => self.on_file_picked(purpose, path),
                AppEvent::FilePicked { path: None, .. } => {}
                AppEvent::Notify(msg) => self.toast(msg),
            }
        }
    }

    fn on_file_picked(&mut self, purpose: FilePurpose, path: PathBuf) {
        let p = path.to_string_lossy().into_owned();
        match purpose {
            FilePurpose::Import => self.import_file(&path),
            FilePurpose::FormFile { tab_id, row } => {
                if let Some(r) = find_req(&mut self.tabs, tab_id).and_then(|t| t.req.body.form_data.get_mut(row)) {
                    r.value = p;
                    r.kind = FieldKind::File;
                }
            }
            FilePurpose::BinaryFile { tab_id } => {
                if let Some(t) = find_req(&mut self.tabs, tab_id) {
                    t.req.body.binary_path = p;
                }
            }
        }
    }

    fn import_file(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return self.toast(format!("Cannot read {}: {e}", path.display())),
        };
        match postman::import(&text) {
            Ok(postman::Imported::Collection(collection)) => {
                let msg = format!("Imported collection \"{}\" ({} requests)", collection.name, collection.requests.len());
                self.sidebar.expanded.insert(collection.id.clone());
                self.store.ws.collections.push(collection);
                self.sidebar.section = SidebarSection::Collections;
                self.ws_dirty = true;
                self.toast(msg);
            }
            Ok(postman::Imported::Environment(env)) => {
                let msg = format!("Imported environment \"{}\"", env.name);
                self.store.ws.environments.push(env);
                self.sidebar.section = SidebarSection::Environments;
                self.ws_dirty = true;
                self.toast(msg);
            }
            Err(e) => self.toast(e),
        }
    }

    /// Native file dialogs run on a helper thread so the UI never blocks.
    fn pick_file(&self, purpose: FilePurpose) {
        let (title, filter): (&str, Option<(&str, &[&str])>) = match purpose {
            FilePurpose::Import => ("Import Postman collection or environment", Some(("JSON", &["json"]))),
            _ => ("Select file", None),
        };
        let (title, filter) =
            (title.to_string(), filter.map(|(n, e)| (n.to_string(), e.iter().map(|s| s.to_string()).collect::<Vec<_>>())));
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut dlg = rfd::FileDialog::new().set_title(&title);
            if let Some((name, exts)) = &filter {
                dlg = dlg.add_filter(name, exts);
            }
            let path = dlg.pick_file();
            let _ = tx.send(AppEvent::FilePicked { purpose, path });
            ctx.request_repaint();
        });
    }

    fn save_file_dialog(&self, title: &str, file_name: String, data: Vec<u8>) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let title = title.to_string();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new().set_title(&title).set_file_name(&file_name).save_file() {
                let msg = match std::fs::write(&path, &data) {
                    Ok(()) => format!("Saved to {}", path.display()),
                    Err(e) => format!("Could not save {}: {e}", path.display()),
                };
                let _ = tx.send(AppEvent::Notify(msg));
                ctx.request_repaint();
            }
        });
    }

    fn save_tab(&mut self, tab_id: u64, close_after: bool) {
        let first_collection = self.store.ws.collections.first().map(|c| c.id.clone());
        let Some(t) = find_req(&mut self.tabs, tab_id) else { return };
        let bound = t.collection_id.clone().filter(|cid| self.store.collection(cid).is_some());
        match bound {
            Some(cid) => {
                let req = t.req.normalized();
                t.saved = req.clone();
                self.store.upsert_request(&cid, req);
                self.ws_dirty = true;
                self.toast("Saved");
                if close_after {
                    self.remove_tab(tab_id);
                }
            }
            None => {
                let name = t.req.name.clone();
                let new_collection = if first_collection.is_none() { "My Collection".to_string() } else { String::new() };
                self.dialog = Some(Dialog::SaveAs { tab_id, name, collection: first_collection, new_collection, close_after, focus: true });
            }
        }
    }

    fn commit_save_as(&mut self, tab_id: u64, name: String, collection: Option<String>, new_collection: String, close_after: bool) {
        let cid = match collection.filter(|c| self.store.collection(c).is_some()) {
            Some(c) => c,
            None => {
                let n = new_collection.trim();
                let c = Collection { name: if n.is_empty() { "New Collection".into() } else { n.into() }, ..Default::default() };
                let id = c.id.clone();
                self.store.ws.collections.push(c);
                id
            }
        };
        let Some(t) = find_req(&mut self.tabs, tab_id) else { return };
        t.req.id = new_id();
        t.req.name = if name.trim().is_empty() { "New Request".into() } else { name.trim().into() };
        t.collection_id = Some(cid.clone());
        let req = t.req.normalized();
        t.saved = req.clone();
        self.store.upsert_request(&cid, req);
        self.sidebar.expanded.insert(cid);
        self.ws_dirty = true;
        self.toast("Saved");
        if close_after {
            self.remove_tab(tab_id);
        }
    }

    fn rename(&mut self, target: RenameTarget, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        match target {
            RenameTarget::Collection(cid) => {
                if let Some(c) = self.store.collection_mut(&cid) {
                    c.name = name;
                }
            }
            RenameTarget::Request(cid, rid) => {
                if let Some(r) = self.store.collection_mut(&cid).and_then(|c| c.requests.iter_mut().find(|r| r.id == rid)) {
                    r.name = name.clone();
                }
                for tab in &mut self.tabs {
                    if let Tab::Request(t) = tab {
                        if t.collection_id.as_deref() == Some(cid.as_str()) && t.req.id == rid {
                            t.req.name = name.clone();
                            t.saved.name = name.clone();
                        }
                    }
                }
            }
            RenameTarget::Environment(id) => {
                if let Some(e) = self.store.environment_mut(&id) {
                    e.name = name;
                }
            }
        }
        self.ws_dirty = true;
    }

    fn delete(&mut self, target: DeleteTarget) {
        let unbind = |tabs: &mut [Tab], cid: &str, rid: Option<&str>| {
            for tab in tabs.iter_mut() {
                if let Tab::Request(t) = tab {
                    if t.collection_id.as_deref() == Some(cid) && rid.is_none_or(|r| t.req.id == r) {
                        t.collection_id = None;
                    }
                }
            }
        };
        match target {
            DeleteTarget::Collection(cid) => {
                self.store.ws.collections.retain(|c| c.id != cid);
                unbind(&mut self.tabs, &cid, None);
                let ids: Vec<u64> = self.tabs.iter().filter(|t| matches!(t, Tab::Collection(c) if c.coll_id == cid)).map(Tab::id).collect();
                for tid in ids {
                    self.remove_tab(tid);
                }
            }
            DeleteTarget::Request(cid, rid) => {
                if let Some(c) = self.store.collection_mut(&cid) {
                    c.requests.retain(|r| r.id != rid);
                }
                unbind(&mut self.tabs, &cid, Some(&rid));
            }
            DeleteTarget::Environment(id) => {
                self.store.ws.environments.retain(|e| e.id != id);
                if self.store.ws.active_environment.as_deref() == Some(id.as_str()) {
                    self.store.ws.active_environment = None;
                }
                let ids: Vec<u64> = self.tabs.iter().filter(|t| matches!(t, Tab::Env(e) if e.env_id == id)).map(Tab::id).collect();
                for tid in ids {
                    self.remove_tab(tid);
                }
            }
            DeleteTarget::History => {
                self.store.history.clear();
                let _ = self.store.save_history();
                return;
            }
        }
        self.ws_dirty = true;
    }

    fn handle(&mut self, action: Action) {
        match action {
            Action::Send(id) => self.send(id),
            Action::Cancel(id) => {
                if let Some(t) = find_req(&mut self.tabs, id) {
                    if let RespState::Loading { handle, .. } = &t.resp {
                        handle.abort();
                        t.resp = RespState::Failed("Request cancelled.".into());
                    }
                }
            }
            Action::Save(id) => self.save_tab(id, false),
            Action::SaveAs(id) => {
                let first = self.store.ws.collections.first().map(|c| c.id.clone());
                if let Some(t) = find_req(&mut self.tabs, id) {
                    let collection = t.collection_id.clone().or(first);
                    let new_collection = if collection.is_none() { "My Collection".to_string() } else { String::new() };
                    self.dialog = Some(Dialog::SaveAs {
                        tab_id: id,
                        name: t.req.name.clone(),
                        collection,
                        new_collection,
                        close_after: false,
                        focus: true,
                    });
                }
            }
            Action::CopyCurl(id) => {
                let cid = match self.tabs.iter().find(|t| t.id() == id) {
                    Some(Tab::Request(t)) => t.collection_id.clone(),
                    _ => None,
                };
                let vars = self.store.vars_for(cid.as_deref());
                if let Some(t) = find_req(&mut self.tabs, id) {
                    match http::prepare(&t.req, &vars, false) {
                        Ok(p) => {
                            self.ctx.copy_text(curl::to_curl(&p));
                            self.toast("cURL command copied to clipboard");
                        }
                        Err(e) => self.toast(e),
                    }
                }
            }
            Action::DuplicateTab(id) => {
                if let Some(t) = find_req(&mut self.tabs, id) {
                    let mut r = t.req.clone();
                    r.id = new_id();
                    r.name = format!("{} Copy", r.name);
                    r.comments.clear();
                    let nid = self.alloc_id();
                    let mut tab = RequestTab::new(nid, r, None);
                    tab.saved = Request::default();
                    self.tabs.push(Tab::Request(tab));
                    self.active = self.tabs.len() - 1;
                }
            }
            Action::CurlPasted(id) => {
                if let Some(t) = find_req(&mut self.tabs, id) {
                    match curl::parse(&t.req.url) {
                        Ok(mut parsed) => {
                            parsed.id = t.req.id.clone();
                            parsed.name = t.req.name.clone();
                            t.req = parsed;
                            self.toast("Imported cURL command");
                        }
                        Err(e) => self.toast(format!("cURL import failed: {e}")),
                    }
                }
            }
            Action::PickFile(purpose) => self.pick_file(purpose),
            Action::SaveResponse(id) => {
                if let Some(Tab::Request(t)) = self.tabs.iter().find(|t| t.id() == id) {
                    if let RespState::Done(r) = &t.resp {
                        let ext = match r.data.kind {
                            response::ContentKind::Json => "json",
                            response::ContentKind::Xml => "xml",
                            response::ContentKind::Html => "html",
                            response::ContentKind::Text => "txt",
                            _ => "bin",
                        };
                        let data = r.data.resp.body.clone();
                        self.save_file_dialog("Save response", format!("response.{ext}"), data);
                    }
                }
            }
            Action::Toast(m) => self.toast(m),
            Action::CopyText(text, what) => {
                self.ctx.copy_text(text);
                self.toast(format!("{what} copied to clipboard"));
            }
            Action::NewTab => self.new_tab(),
            Action::ActivateTab(i) => {
                if i < self.tabs.len() {
                    self.active = i;
                }
            }
            Action::CloseTab(id) => self.request_close(id),
            Action::CloseOtherTabs(id) => {
                let others: Vec<u64> =
                    self.tabs.iter().filter(|t| t.id() != id && !matches!(t, Tab::Request(r) if r.is_dirty())).map(Tab::id).collect();
                for o in others {
                    self.remove_tab(o);
                }
                if let Some(i) = self.tab_index(id) {
                    self.active = i;
                }
                if self.tabs.len() > 1 {
                    self.toast("Tabs with unsaved changes were kept open");
                }
            }
            Action::OpenRequest(cid, rid) => {
                let existing = self
                    .tabs
                    .iter()
                    .position(|t| matches!(t, Tab::Request(r) if r.collection_id.as_deref() == Some(cid.as_str()) && r.req.id == rid));
                if let Some(i) = existing {
                    self.active = i;
                } else if let Some(req) = self.store.find_request(&cid, &rid).cloned() {
                    self.open_request_tab(req, Some(cid));
                }
            }
            Action::OpenHistory(idx) => {
                if let Some(h) = self.store.history.get(idx) {
                    let mut r = h.request.clone();
                    r.id = new_id();
                    self.open_request_tab(r, None);
                }
            }
            Action::NewCollection => {
                let c = Collection::default();
                let id = c.id.clone();
                self.store.ws.collections.push(c);
                self.sidebar.section = SidebarSection::Collections;
                self.sidebar.renaming = Some(RenameTarget::Collection(id));
                self.sidebar.rename_buf = "New Collection".into();
                self.sidebar.rename_focus = true;
                self.store.ws.settings.show_sidebar = true;
                self.ws_dirty = true;
            }
            Action::AddRequest(cid) => {
                let req = Request::default();
                if self.store.upsert_request(&cid, req.clone()) {
                    self.sidebar.expanded.insert(cid.clone());
                    let id = self.alloc_id();
                    self.tabs.push(Tab::Request(RequestTab::new(id, req, Some(cid))));
                    self.active = self.tabs.len() - 1;
                    self.focus_url = true;
                    self.ws_dirty = true;
                }
            }
            Action::StartRename(target, current) => {
                self.sidebar.renaming = Some(target);
                self.sidebar.rename_buf = current;
                self.sidebar.rename_focus = true;
            }
            Action::Rename(target, name) => self.rename(target, name),
            Action::DuplicateRequest(cid, rid) => {
                if let Some(c) = self.store.collection_mut(&cid) {
                    if let Some(pos) = c.requests.iter().position(|r| r.id == rid) {
                        let mut r = c.requests[pos].clone();
                        r.id = new_id();
                        r.name = format!("{} Copy", r.name);
                        r.comments.clear();
                        c.requests.insert(pos + 1, r);
                        self.ws_dirty = true;
                    }
                }
            }
            Action::AskDelete(target) => {
                let message = match &target {
                    DeleteTarget::Collection(cid) => {
                        let c = self.store.collection(cid);
                        format!(
                            "Delete collection \"{}\" and its {} request(s)?",
                            c.map(|c| c.name.as_str()).unwrap_or("?"),
                            c.map(|c| c.requests.len()).unwrap_or(0)
                        )
                    }
                    DeleteTarget::Request(cid, rid) => {
                        format!("Delete request \"{}\"?", self.store.find_request(cid, rid).map(|r| r.name.as_str()).unwrap_or("?"))
                    }
                    DeleteTarget::Environment(id) => format!(
                        "Delete environment \"{}\"?",
                        self.store.ws.environments.iter().find(|e| &e.id == id).map(|e| e.name.as_str()).unwrap_or("?")
                    ),
                    DeleteTarget::History => format!("Clear all {} history entries?", self.store.history.len()),
                };
                self.dialog = Some(Dialog::ConfirmDelete { target, message });
            }
            Action::ExportCollection(cid) => {
                if let Some(c) = self.store.collection(&cid) {
                    let text = postman::export_collection(c);
                    let name = format!("{}.postman_collection.json", sanitize_file_name(&c.name));
                    self.save_file_dialog("Export collection (Postman v2.1)", name, text.into_bytes());
                }
            }
            Action::NewEnvironment => {
                let e = Environment::default();
                let id = e.id.clone();
                self.store.ws.environments.push(e);
                self.sidebar.section = SidebarSection::Environments;
                self.open_env_tab(id);
                self.ws_dirty = true;
            }
            Action::OpenEnvironment(id) => self.open_env_tab(id),
            Action::SetActiveEnv(id) => {
                self.store.ws.active_environment = id;
                self.ws_dirty = true;
            }
            Action::ExportEnvironment(id) => {
                if let Some(e) = self.store.ws.environments.iter().find(|e| e.id == id) {
                    let text = postman::export_environment(e);
                    let name = format!("{}.postman_environment.json", sanitize_file_name(&e.name));
                    self.save_file_dialog("Export environment", name, text.into_bytes());
                }
            }
            Action::Import => self.pick_file(FilePurpose::Import),
            Action::ClearCookies => {
                self.engine.clear_cookies();
                self.toast("Cookies cleared");
            }
            Action::About => self.dialog = Some(Dialog::About),
            Action::SyncComments(id) => {
                if let Some(t) = find_req(&mut self.tabs, id) {
                    t.saved.comments = t.req.comments.clone();
                    if let Some(cid) = t.collection_id.clone() {
                        let comments = t.req.comments.clone();
                        let rid = t.req.id.clone();
                        if let Some(r) = self.store.collection_mut(&cid).and_then(|c| c.requests.iter_mut().find(|r| r.id == rid)) {
                            r.comments = comments;
                            self.ws_dirty = true;
                        }
                    }
                }
            }
            Action::OpenCollection(cid) => {
                if let Some(i) = self.tabs.iter().position(|t| matches!(t, Tab::Collection(c) if c.coll_id == cid)) {
                    self.active = i;
                } else if self.store.collection(&cid).is_some() {
                    let id = self.alloc_id();
                    self.tabs.push(Tab::Collection(CollTab { id, coll_id: cid, comment_state: CommentState::default() }));
                    self.active = self.tabs.len() - 1;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // UI pieces
    // -----------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let sc = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)));
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers { shift: true, ..Modifiers::COMMAND };
        let ctrl_shift = Modifiers { shift: true, ..Modifiers::CTRL };
        let active_req = match self.tabs.get(self.active) {
            Some(Tab::Request(t)) => Some(t.id),
            _ => None,
        };
        let typing_comment = match self.tabs.get(self.active) {
            Some(Tab::Request(t)) => ui_comments::is_typing_comment(ctx, ("req", t.id)),
            Some(Tab::Collection(c)) => ui_comments::is_typing_comment(ctx, ("coll", c.id)),
            _ => false,
        };
        if !typing_comment && sc(cmd, Key::Enter) {
            if let Some(id) = active_req {
                actions.push(Action::Send(id));
            }
        }
        if sc(cmd_shift, Key::S) {
            if let Some(id) = active_req {
                actions.push(Action::SaveAs(id));
            }
        } else if sc(cmd, Key::S) {
            if let Some(id) = active_req {
                actions.push(Action::Save(id));
            }
        }
        if sc(cmd, Key::T) || sc(cmd, Key::N) {
            actions.push(Action::NewTab);
        }
        if sc(cmd, Key::W) {
            if let Some(t) = self.tabs.get(self.active) {
                actions.push(Action::CloseTab(t.id()));
            }
        }
        if sc(cmd, Key::L) {
            self.focus_url = true;
        }
        if sc(cmd, Key::B) {
            let s = &mut self.store.ws.settings.show_sidebar;
            *s = !*s;
            self.ws_dirty = true;
        }
        if !self.tabs.is_empty() {
            if sc(ctrl_shift, Key::Tab) {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            } else if sc(Modifiers::CTRL, Key::Tab) {
                self.active = (self.active + 1) % self.tabs.len();
            }
        }
        if sc(cmd, Key::F) {
            if let Some(Tab::Request(RequestTab { resp: RespState::Done(r), .. })) = self.tabs.get_mut(self.active) {
                r.section = RespSection::Body;
                r.search_open = true;
                r.focus_search = true;
            }
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let p = Pal::new(ctx.style().visuals.dark_mode);
        let frame = egui::Frame::none().fill(p.panel).inner_margin(egui::Margin::symmetric(10.0, 8.0));
        egui::TopBottomPanel::top("top_bar").frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let show = self.store.ws.settings.show_sidebar;
                if ui_kit::icon_toggle(ui, show, ph::SIDEBAR_SIMPLE, "Toggle sidebar (Ctrl+B)").clicked() {
                    self.store.ws.settings.show_sidebar = !show;
                    self.ws_dirty = true;
                }
                ui.add_space(4.0);
                ui_kit::logo(ui, 26.0);
                ui.label(RichText::new("Reqora").font(theme::semibold(15.0)).color(p.text_strong));
                ui.add_space(14.0);
                if ui_kit::button(ui, ph::PLUS, "New").on_hover_text("New request tab (Ctrl+T)").clicked() {
                    actions.push(Action::NewTab);
                }
                if ui_kit::button(ui, ph::DOWNLOAD_SIMPLE, "Import")
                    .on_hover_text("Import a Postman collection or environment (JSON)")
                    .clicked()
                {
                    actions.push(Action::Import);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new(ph::GEAR_SIX).size(17.0), |ui| self.settings_menu(ui, actions))
                        .response
                        .on_hover_text("Settings");

                    let envs = &self.store.ws.environments;
                    let current = self.store.ws.active_environment.clone();
                    let current_name = current
                        .as_ref()
                        .and_then(|id| envs.iter().find(|e| &e.id == id))
                        .map(|e| e.name.clone())
                        .unwrap_or_else(|| "No Environment".into());
                    let color = if current.is_some() { p.text_strong } else { p.text_weak };
                    egui::ComboBox::from_id_salt("env_select")
                        .width(210.0)
                        .selected_text(RichText::new(format!("{}  {}", ph::STACK, widgets::truncate_chars(&current_name, 24))).color(color))
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_none(), "No Environment").clicked() {
                                actions.push(Action::SetActiveEnv(None));
                            }
                            for e in envs {
                                if ui
                                    .selectable_label(current.as_deref() == Some(e.id.as_str()), format!("{}  {}", ph::STACK, e.name))
                                    .clicked()
                                {
                                    actions.push(Action::SetActiveEnv(Some(e.id.clone())));
                                }
                            }
                            ui.separator();
                            if ui.selectable_label(false, format!("{}  New environment…", ph::PLUS)).clicked() {
                                actions.push(Action::NewEnvironment);
                            }
                        });
                    if let Some(id) = &current {
                        if ui_kit::icon_button(ui, ph::PENCIL_SIMPLE, "Edit active environment").clicked() {
                            actions.push(Action::OpenEnvironment(id.clone()));
                        }
                    }
                });
            });
        });
    }

    fn settings_menu(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = Pal::of(ui);
        ui.set_min_width(260.0);
        ui.add_space(4.0);
        ui.label(RichText::new("Your name").font(theme::semibold(12.5)).color(p.text_strong));
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.store.ws.settings.author_name).hint_text("Shown on your comments").desired_width(240.0),
        );
        if r.changed() {
            self.ws_dirty = true;
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Theme").font(theme::semibold(12.5)).color(p.text_strong));
        let mut t = self.store.ws.settings.theme;
        let opts = [(ThemeChoice::System, "Auto"), (ThemeChoice::Light, "Light"), (ThemeChoice::Dark, "Dark")];
        if ui_kit::segmented(ui, egui::Id::new("theme_segmented"), &mut t, &opts, true) {
            self.store.ws.settings.theme = t;
            theme::apply_theme(ui.ctx(), t);
            self.ws_dirty = true;
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Response position").font(theme::semibold(12.5)).color(p.text_strong));
        let mut l = self.store.ws.settings.layout;
        if ui_kit::segmented(
            ui,
            egui::Id::new("layout_segmented"),
            &mut l,
            &[(PaneLayout::Vertical, "Below"), (PaneLayout::Horizontal, "Right")],
            true,
        ) {
            self.store.ws.settings.layout = l;
            self.ws_dirty = true;
        }
        ui.add_space(6.0);
        ui.separator();
        if ui.button(format!("{}  Clear cookies", ph::COOKIE)).clicked() {
            actions.push(Action::ClearCookies);
            ui.close_menu();
        }
        if ui.button(format!("{}  About & shortcuts", ph::INFO)).clicked() {
            actions.push(Action::About);
            ui.close_menu();
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let p = Pal::new(ctx.style().visuals.dark_mode);
        let frame = egui::Frame::none().fill(p.panel).inner_margin(egui::Margin::symmetric(12.0, 2.0));
        egui::TopBottomPanel::bottom("status_bar").frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let small = |t: String, c: Color32| RichText::new(t).font(theme::regular(11.5)).color(c);
                match self.store.active_environment() {
                    Some(e) => {
                        let id = e.id.clone();
                        let r = ui.add(egui::Label::new(small(format!("{}  {}", ph::STACK, e.name), p.success)).sense(Sense::click()));
                        if r.on_hover_text("Active environment — click to edit").clicked() {
                            actions.push(Action::OpenEnvironment(id));
                        }
                    }
                    None => {
                        ui.label(small(format!("{}  No environment", ph::STACK), p.text_faint));
                    }
                }
                ui.label(small(format!("{}  {} in history", ph::CLOCK_COUNTER_CLOCKWISE, self.store.history.len()), p.text_weak));
                let r = ui.add(egui::Label::new(small(format!("{}  Clear cookies", ph::COOKIE), p.text_weak)).sense(Sense::click()));
                if r.clicked() {
                    actions.push(Action::ClearCookies);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.label(small(format!("v{}", env!("CARGO_PKG_VERSION")), p.text_faint));
                    ui.add_space(8.0);
                    let dark = ui.visuals().dark_mode;
                    if ui_kit::icon_button(ui, if dark { ph::SUN } else { ph::MOON }, "Toggle light / dark").clicked() {
                        let t = if dark { ThemeChoice::Light } else { ThemeChoice::Dark };
                        self.store.ws.settings.theme = t;
                        theme::apply_theme(ui.ctx(), t);
                        self.ws_dirty = true;
                    }
                    let vertical = self.store.ws.settings.layout == PaneLayout::Vertical;
                    let (icon, tip) =
                        if vertical { (ph::COLUMNS, "Show response on the right") } else { (ph::ROWS, "Show response below") };
                    if ui_kit::icon_button(ui, icon, tip).clicked() {
                        self.store.ws.settings.layout = if vertical { PaneLayout::Horizontal } else { PaneLayout::Vertical };
                        self.ws_dirty = true;
                    }
                });
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = Pal::of(ui);
        let strip = egui::Frame::none().fill(p.panel).inner_margin(egui::Margin { left: 8.0, right: 8.0, top: 6.0, bottom: 0.0 });
        let mut selected_rect = None;
        let resp = strip.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let max_w = (ui.available_width() - 40.0).max(50.0);
                egui::ScrollArea::horizontal()
                    .id_salt("tab_scroll")
                    .max_width(max_w)
                    .auto_shrink([true, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for i in 0..self.tabs.len() {
                                if let Some(r) = self.draw_tab(ui, i, actions) {
                                    selected_rect = Some(r);
                                }
                            }
                        });
                    });
                if ui_kit::icon_button(ui, ph::PLUS, "New tab (Ctrl+T)").clicked() {
                    actions.push(Action::NewTab);
                }
            });
        });
        let r = resp.response.rect;
        let y = r.bottom() - 0.5;
        let stroke = Stroke::new(1.0_f32, p.border);
        match selected_rect {
            Some(s) => {
                ui.painter().hline(r.left()..=s.left(), y, stroke);
                ui.painter().hline(s.right()..=r.right(), y, stroke);
            }
            None => {
                ui.painter().hline(r.x_range(), y, stroke);
            }
        }
    }

    /// Draws one tab; returns its rect when it is the selected one.
    fn draw_tab(&self, ui: &mut egui::Ui, i: usize, actions: &mut Vec<Action>) -> Option<egui::Rect> {
        let p = Pal::of(ui);
        let tab = &self.tabs[i];
        let selected = i == self.active;
        let text_color = if selected { p.text_strong } else { p.text_weak };
        let (badge, badge_font, badge_color, name, dirty, loading, tooltip) = match tab {
            Tab::Request(t) => (
                theme::short_method(&t.req.method).to_string(),
                theme::semibold(10.5),
                theme::method_color(&t.req.method, p.dark),
                t.req.display_name().to_string(),
                t.is_dirty(),
                t.is_loading(),
                format!("{} {}", t.req.method, t.req.url),
            ),
            Tab::Env(e) => {
                let name = self
                    .store
                    .ws
                    .environments
                    .iter()
                    .find(|x| x.id == e.env_id)
                    .map(|x| x.name.clone())
                    .unwrap_or_else(|| "Environment".into());
                (ph::STACK.to_string(), theme::regular(14.0), p.success, name.clone(), false, false, format!("Environment: {name}"))
            }
            Tab::Collection(c) => {
                let name = self.store.collection(&c.coll_id).map(|x| x.name.clone()).unwrap_or_else(|| "Collection".into());
                (ph::FOLDER_OPEN.to_string(), theme::regular(14.0), p.accent, name.clone(), false, false, format!("Collection: {name}"))
            }
        };
        let badge_g = ui.painter().layout_no_wrap(badge, badge_font, badge_color);
        let name_g = widgets::elided(ui, &name, theme::medium(13.0), text_color, 170.0);
        let (pad, gap, close_w, h) = (12.0, 7.0, 16.0, 34.0);
        let w = pad + badge_g.size().x + gap + name_g.size().x + gap + close_w + pad - 4.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), Sense::click());
        let rounding = egui::Rounding { nw: 9.0, ne: 9.0, sw: 0.0, se: 0.0 };
        {
            let painter = ui.painter();
            if selected {
                painter.rect_filled(rect, rounding, p.bg);
                let tall = rect.with_max_y(rect.bottom() + 12.0);
                painter.with_clip_rect(rect.expand2(egui::vec2(1.0, 0.0))).rect_stroke(tall, 9.0, Stroke::new(1.0_f32, p.border));
                let line =
                    egui::Rect::from_min_max(egui::pos2(rect.left() + 9.0, rect.top()), egui::pos2(rect.right() - 9.0, rect.top() + 2.0));
                ui_kit::gradient_rect(painter, line, 1.0, p.accent, p.accent2);
            } else if resp.hovered() {
                painter.rect_filled(rect.shrink2(egui::vec2(0.0, 2.0)), 8.0, p.hover);
            }
            let mut x = rect.left() + pad;
            painter.galley(egui::pos2(x, rect.center().y - badge_g.size().y / 2.0), badge_g.clone(), badge_color);
            x += badge_g.size().x + gap;
            painter.galley(egui::pos2(x, rect.center().y - name_g.size().y / 2.0), name_g, text_color);
        }
        let close_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - pad + 2.0 - close_w / 2.0, rect.center().y),
            egui::vec2(close_w + 4.0, close_w + 4.0),
        );
        let close = ui.interact(close_rect, egui::Id::new(("tab_close", tab.id())), Sense::click());
        let painter = ui.painter();
        if loading && !close.hovered() {
            let t = ui.input(|i| i.time) as f32;
            let a = (t * 6.0).sin() * 0.5 + 0.5;
            painter.circle_filled(close_rect.center(), 3.5, p.accent.gamma_multiply(0.4 + 0.6 * a));
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        } else if dirty && !close.hovered() {
            painter.circle_filled(close_rect.center(), 4.0, p.warning);
        } else if selected || ui.rect_contains_pointer(rect) {
            if close.hovered() {
                painter.rect_filled(close_rect, 5.0, p.border);
            }
            painter.text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                ph::X,
                theme::regular(12.0),
                if close.hovered() { p.text_strong } else { p.text_weak },
            );
        }

        let id = tab.id();
        if close.clicked() || resp.middle_clicked() {
            actions.push(Action::CloseTab(id));
        } else if resp.clicked() {
            actions.push(Action::ActivateTab(i));
        }
        let is_request = matches!(tab, Tab::Request(_));
        resp.context_menu(|ui| {
            if ui.button(format!("{}  Close", ph::X)).clicked() {
                actions.push(Action::CloseTab(id));
                ui.close_menu();
            }
            if ui.button(format!("{}  Close other tabs", ph::X_SQUARE)).clicked() {
                actions.push(Action::CloseOtherTabs(id));
                ui.close_menu();
            }
            if is_request && ui.button(format!("{}  Duplicate", ph::COPY_SIMPLE)).clicked() {
                actions.push(Action::DuplicateTab(id));
                ui.close_menu();
            }
        });
        resp.on_hover_text(tooltip);
        selected.then_some(rect)
    }

    fn tab_content(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        if self.tabs.is_empty() {
            welcome(ui, actions);
            return;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        let dark = ui.visuals().dark_mode;
        let layout = self.store.ws.settings.layout;
        let focus_url = std::mem::take(&mut self.focus_url);
        let author = self.store.ws.settings.author_name.clone();
        let author = if author.trim().is_empty() { "Me".to_string() } else { author };
        let margin = egui::Margin { left: 16.0, right: 16.0, top: 12.0, bottom: 10.0 };
        match &mut self.tabs[self.active] {
            Tab::Request(t) => {
                let vars = self.store.vars_for(t.collection_id.as_deref());
                let collection_name = t.collection_id.as_ref().and_then(|c| self.store.collection(c)).map(|c| c.name.clone());
                let env = ui_request::EditorEnv {
                    vars: &vars,
                    collection_name: collection_name.as_deref(),
                    dark,
                    layout,
                    focus_url,
                    author: &author,
                };
                egui::Frame::none().inner_margin(margin).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui_request::show(ui, t, &env, actions);
                });
            }
            Tab::Env(e) => {
                let env_id = e.env_id.clone();
                egui::Frame::none().inner_margin(margin).show(ui, |ui| {
                    if env_editor(ui, &mut self.store, &env_id, actions) {
                        self.ws_dirty = true;
                    }
                });
            }
            Tab::Collection(c) => {
                let coll_id = c.coll_id.clone();
                let tab_id = c.id;
                let state = &mut c.comment_state;
                egui::Frame::none().inner_margin(margin).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    if collection_overview(ui, &mut self.store, &coll_id, tab_id, state, &author, actions) {
                        self.ws_dirty = true;
                    }
                });
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else { return };
        let mut close = false;
        let mut result: Option<DialogResult> = None;
        let p = Pal::new(ctx.style().visuals.dark_mode);
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        match dialog {
            Dialog::SaveAs { tab_id, name, collection, new_collection, close_after, focus } => {
                let colls = &self.store.ws.collections;
                modal(ctx, ph::FLOPPY_DISK, "Save request", |ui| {
                    ui.label(RichText::new("Request name").color(p.text_weak));
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(360.0).margin(egui::vec2(8.0, 6.0)));
                    if std::mem::take(focus) {
                        r.request_focus();
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("Collection").color(p.text_weak));
                    let selected = collection
                        .as_ref()
                        .and_then(|c| colls.iter().find(|x| &x.id == c))
                        .map(|c| format!("{}  {}", ph::FOLDER, c.name))
                        .unwrap_or_else(|| format!("{}  New collection", ph::PLUS));
                    egui::ComboBox::from_id_salt("save_as_collection").width(360.0).selected_text(selected).show_ui(ui, |ui| {
                        for c in colls {
                            ui.selectable_value(collection, Some(c.id.clone()), format!("{}  {}", ph::FOLDER, c.name));
                        }
                        ui.selectable_value(collection, None, format!("{}  New collection", ph::PLUS));
                    });
                    if collection.is_none() {
                        ui.add(
                            egui::TextEdit::singleline(new_collection)
                                .hint_text("New collection name")
                                .desired_width(360.0)
                                .margin(egui::vec2(8.0, 6.0)),
                        );
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui_kit::primary_button(ui, Some(ph::CHECK), "Save", egui::vec2(96.0, 32.0)).clicked() || enter {
                            result = Some(DialogResult::SaveAs {
                                tab_id: *tab_id,
                                name: name.clone(),
                                collection: collection.clone(),
                                new_collection: new_collection.clone(),
                                close_after: *close_after,
                            });
                        }
                        if ui_kit::button(ui, "", "Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::ConfirmClose { tab_id } => {
                let tab_id = *tab_id;
                modal(ctx, ph::WARNING_CIRCLE, "Unsaved changes", |ui| {
                    ui.label(RichText::new("This request has unsaved changes.\nSave them before closing?").color(p.text));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui_kit::primary_button(ui, Some(ph::FLOPPY_DISK), "Save", egui::vec2(96.0, 32.0)).clicked() {
                            result = Some(DialogResult::SaveAndClose(tab_id));
                        }
                        if ui_kit::button(ui, "", "Don't save").clicked() {
                            result = Some(DialogResult::Discard(tab_id));
                        }
                        if ui_kit::button(ui, "", "Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::ConfirmDelete { target, message } => {
                modal(ctx, ph::TRASH, "Delete", |ui| {
                    ui.label(RichText::new(message.as_str()).color(p.text));
                    ui.label(RichText::new("This cannot be undone.").small().color(p.text_weak));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        let b = egui::Button::new(RichText::new(format!("{}  Delete", ph::TRASH)).color(Color32::WHITE))
                            .fill(p.danger)
                            .min_size(egui::vec2(96.0, 32.0));
                        if ui.add(b).clicked() {
                            result = Some(DialogResult::Delete(target.clone()));
                        }
                        if ui_kit::button(ui, "", "Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::About => {
                let dir = self.store.dir().display().to_string();
                modal(ctx, ph::INFO, "About", |ui| {
                    ui.vertical_centered(|ui| {
                        ui_kit::logo(ui, 52.0);
                        ui.add_space(6.0);
                        ui.label(RichText::new("Reqora").font(theme::semibold(20.0)).color(p.text_strong));
                        ui.label(
                            RichText::new(format!("Version {} · fast, lightweight API client", env!("CARGO_PKG_VERSION")))
                                .color(p.text_weak),
                        );
                    });
                    ui.add_space(12.0);
                    ui.label(RichText::new("Keyboard shortcuts").font(theme::semibold(13.0)).color(p.text_strong));
                    ui.add_space(4.0);
                    for (keys, v) in [
                        (&["Ctrl", "Enter"][..], "Send request / post comment"),
                        (&["Ctrl", "S"][..], "Save"),
                        (&["Ctrl", "Shift", "S"][..], "Save as"),
                        (&["Ctrl", "T"][..], "New tab"),
                        (&["Ctrl", "W"][..], "Close tab"),
                        (&["Ctrl", "Tab"][..], "Next tab"),
                        (&["Ctrl", "L"][..], "Focus URL"),
                        (&["Ctrl", "F"][..], "Find in response"),
                        (&["Ctrl", "B"][..], "Toggle sidebar"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(egui::vec2(150.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for k in keys {
                                    ui_kit::keycap(ui, k);
                                }
                            });
                            ui.label(RichText::new(v).color(p.text));
                        });
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("{}  Data: {dir}", ph::FOLDER)).small().color(p.text_weak));
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        if ui_kit::primary_button(ui, None, "Close", egui::vec2(110.0, 32.0)).clicked() {
                            close = true;
                        }
                    });
                });
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            close = true;
        }
        if let Some(r) = result {
            self.dialog = None;
            match r {
                DialogResult::SaveAs { tab_id, name, collection, new_collection, close_after } => {
                    self.commit_save_as(tab_id, name, collection, new_collection, close_after)
                }
                DialogResult::Discard(id) => self.remove_tab(id),
                DialogResult::SaveAndClose(id) => self.save_tab(id, true),
                DialogResult::Delete(t) => self.delete(t),
            }
        } else if close {
            self.dialog = None;
        }
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        self.toasts.retain(|(_, t)| t.elapsed() < Duration::from_millis(2800));
        if self.toasts.is_empty() {
            return;
        }
        let p = Pal::new(ctx.style().visuals.dark_mode);
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -46.0))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                for (msg, t) in &self.toasts {
                    let fade = (1.0 - (t.elapsed().as_secs_f32() - 2.4).max(0.0) / 0.4).clamp(0.0, 1.0);
                    let lower = msg.to_lowercase();
                    let (icon, color) =
                        if lower.contains("fail") || lower.contains("invalid") || lower.contains("could not") || lower.contains("cannot") {
                            (ph::WARNING_CIRCLE, p.danger)
                        } else {
                            (ph::CHECK_CIRCLE, p.success)
                        };
                    egui::Frame::popup(ui.style())
                        .fill(p.elevated)
                        .rounding(10.0)
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .multiply_with_opacity(fade)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(icon).size(16.0).color(color.gamma_multiply(fade)));
                                ui.label(RichText::new(msg.as_str()).color(p.text_strong.gamma_multiply(fade)));
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(60));
    }

    fn active_refs(&self) -> (Option<(String, String)>, Option<String>, Option<String>) {
        match self.tabs.get(self.active) {
            Some(Tab::Request(t)) => (t.collection_id.clone().map(|c| (c, t.req.id.clone())), None, None),
            Some(Tab::Env(e)) => (None, Some(e.env_id.clone()), None),
            Some(Tab::Collection(c)) => (None, None, Some(c.coll_id.clone())),
            None => (None, None, None),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        let mut actions = Vec::new();
        if self.dialog.is_none() {
            self.shortcuts(ctx, &mut actions);
        }
        let p = Pal::new(ctx.style().visuals.dark_mode);
        self.top_bar(ctx, &mut actions);
        self.status_bar(ctx, &mut actions);
        if self.store.ws.settings.show_sidebar {
            let (req_ref, env_ref, coll_ref) = self.active_refs();
            let frame = egui::Frame::none().fill(p.panel).inner_margin(egui::Margin::symmetric(10.0, 8.0));
            egui::SidePanel::left("sidebar").resizable(true).default_width(300.0).width_range(220.0..=520.0).frame(frame).show(ctx, |ui| {
                let refs = ui_sidebar::ActiveRefs {
                    request: req_ref.as_ref().map(|(c, r)| (c.as_str(), r.as_str())),
                    environment: env_ref.as_deref(),
                    collection: coll_ref.as_deref(),
                };
                ui_sidebar::show(ui, &mut self.sidebar, &self.store, refs, &mut actions);
            });
        }
        let central = egui::Frame::none().fill(p.bg);
        egui::CentralPanel::default().frame(central).show(ctx, |ui| {
            if self.dialog.is_some() {
                ui.disable();
            }
            self.tab_bar(ui, &mut actions);
            self.tab_content(ui, &mut actions);
        });
        self.dialogs(ctx);
        self.show_toasts(ctx);
        for a in actions {
            self.handle(a);
        }
        self.persist(false);
        #[cfg(feature = "dev-screenshot")]
        self.devshot_tick(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist(true);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sanitize_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "export".into()
    } else {
        s
    }
}

/// Centered window above a dimmed backdrop that swallows clicks.
fn modal(ctx: &egui::Context, icon: &str, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let p = Pal::new(ctx.style().visuals.dark_mode);
    let screen = ctx.screen_rect();
    egui::Area::new(egui::Id::new("modal_backdrop")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(if p.dark { 140 } else { 90 }));
    });
    egui::Window::new(title)
        .id(egui::Id::new("modal_window"))
        .order(egui::Order::Foreground)
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), Sense::hover());
                ui.painter().rect_filled(rect, 9.0, p.soft(p.accent));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, theme::regular(18.0), p.accent);
                ui.label(RichText::new(title).font(theme::semibold(17.0)).color(p.text_strong));
            });
            ui.add_space(12.0);
            add(ui);
        });
}

fn section_title(ui: &mut egui::Ui, icon: &str, title: &str, hint: &str) {
    let p = Pal::of(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{icon}  {title}")).font(theme::semibold(14.0)).color(p.text_strong));
        if !hint.is_empty() {
            ui.label(RichText::new(hint).small().color(p.text_weak));
        }
    });
    ui.add_space(6.0);
}

fn welcome(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let p = Pal::of(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.14).max(20.0));
        ui_kit::logo(ui, 68.0);
        ui.add_space(14.0);
        ui.label(RichText::new("Reqora").font(theme::semibold(28.0)).color(p.text_strong));
        ui.label(RichText::new("A fast, lightweight API client").size(15.0).color(p.text_weak));
        ui.add_space(28.0);
        let (card_w, gap) = (210.0, 14.0);
        let total = card_w * 3.0 + gap * 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            if ui_kit::action_card(ui, ph::PAPER_PLANE_TILT, "New request", "Start from a blank tab", card_w).clicked() {
                actions.push(Action::NewTab);
            }
            if ui_kit::action_card(ui, ph::DOWNLOAD_SIMPLE, "Import", "Postman collections & envs", card_w).clicked() {
                actions.push(Action::Import);
            }
            if ui_kit::action_card(ui, ph::STACK, "Environment", "Variables like {{base_url}}", card_w).clicked() {
                actions.push(Action::NewEnvironment);
            }
        });
        ui.add_space(26.0);
        ui.horizontal(|ui| {
            let w = 330.0;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            ui.spacing_mut().item_spacing.x = 4.0;
            ui_kit::keycap(ui, "Ctrl");
            ui_kit::keycap(ui, "T");
            ui.label(RichText::new(" new tab    ").color(p.text_weak));
            ui_kit::keycap(ui, "Ctrl");
            ui_kit::keycap(ui, "Enter");
            ui.label(RichText::new(" send").color(p.text_weak));
        });
    });
}

/// Environment editor tab. Returns true when something changed.
fn env_editor(ui: &mut egui::Ui, store: &mut Store, env_id: &str, actions: &mut Vec<Action>) -> bool {
    let p = Pal::of(ui);
    let is_active = store.ws.active_environment.as_deref() == Some(env_id);
    let Some(env) = store.environment_mut(env_id) else {
        ui_kit::hero(ui, ph::STACK, "Environment not found", "It may have been deleted.");
        return false;
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), Sense::hover());
        let c = if is_active { p.success } else { p.accent };
        ui.painter().rect_filled(rect, 11.0, p.soft(c));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, ph::STACK, theme::regular(22.0), c);
        ui.vertical(|ui| {
            ui.label(RichText::new("ENVIRONMENT").font(theme::semibold(10.5)).color(p.text_faint));
            changed |=
                ui.add(egui::TextEdit::singleline(&mut env.name).font(theme::semibold(19.0)).frame(false).desired_width(360.0)).changed();
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui_kit::button(ui, ph::TRASH, "Delete").clicked() {
                actions.push(Action::AskDelete(DeleteTarget::Environment(env_id.to_string())));
            }
            if ui_kit::button(ui, ph::UPLOAD_SIMPLE, "Export").clicked() {
                actions.push(Action::ExportEnvironment(env_id.to_string()));
            }
            if is_active {
                if ui_kit::button(ui, ph::CIRCLE, "Deactivate").clicked() {
                    actions.push(Action::SetActiveEnv(None));
                }
                ui_kit::chip(ui, format!("{} Active", ph::CHECK_CIRCLE), p.success);
            } else if ui_kit::primary_button(ui, Some(ph::CHECK_CIRCLE), "Set active", egui::vec2(0.0, 30.0)).clicked() {
                actions.push(Action::SetActiveEnv(Some(env_id.to_string())));
            }
        });
    });
    ui.add_space(14.0);
    ui_kit::card_frame(&p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(ph::LIGHTBULB).size(16.0).color(p.warning));
            ui.label(RichText::new("Use variables as").color(p.text));
            ui_kit::chip(ui, "{{base_url}}", p.accent);
            ui.label(RichText::new("anywhere, or").color(p.text));
            ui_kit::chip(ui, "{base_url}", p.accent);
            ui.label(RichText::new("in the URL. Built-ins:").color(p.text));
            for b in ["{{$guid}}", "{{$timestamp}}", "{{$isoTimestamp}}", "{{$randomInt}}"] {
                ui_kit::chip(ui, b, p.text_weak);
            }
        });
    });
    ui.add_space(14.0);
    section_title(ui, ph::BRACKETS_CURLY, "Variables", "");
    let r = widgets::kv_table(
        ui,
        &mut env.variables,
        KvOptions {
            id: egui::Id::new(("env_vars", env_id)),
            key_hint: "Add variable, e.g. base_url",
            value_hint: "Value",
            allow_files: false,
        },
    );
    changed | r.changed
}

/// Collection overview tab. Returns true when the collection changed.
fn collection_overview(
    ui: &mut egui::Ui,
    store: &mut Store,
    coll_id: &str,
    tab_id: u64,
    state: &mut CommentState,
    author: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let p = Pal::of(ui);
    let Some(coll) = store.collection_mut(coll_id) else {
        ui_kit::hero(ui, ph::FOLDER, "Collection not found", "It may have been deleted.");
        return false;
    };
    let mut changed = false;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), Sense::hover());
        ui.painter().rect_filled(rect, 11.0, p.soft(p.accent));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, ph::FOLDER_OPEN, theme::regular(22.0), p.accent);
        ui.vertical(|ui| {
            ui.label(RichText::new("COLLECTION").font(theme::semibold(10.5)).color(p.text_faint));
            changed |=
                ui.add(egui::TextEdit::singleline(&mut coll.name).font(theme::semibold(19.0)).frame(false).desired_width(360.0)).changed();
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui_kit::button(ui, ph::TRASH, "").on_hover_text("Delete collection").clicked() {
                actions.push(Action::AskDelete(DeleteTarget::Collection(coll_id.to_string())));
            }
            if ui_kit::button(ui, ph::UPLOAD_SIMPLE, "Export").clicked() {
                actions.push(Action::ExportCollection(coll_id.to_string()));
            }
            if ui_kit::primary_button(ui, Some(ph::PLUS), "Add request", egui::vec2(0.0, 30.0)).clicked() {
                actions.push(Action::AddRequest(coll_id.to_string()));
            }
        });
    });
    ui.add_space(14.0);

    egui::SidePanel::right(egui::Id::new(("coll_comments", tab_id)))
        .resizable(true)
        .default_width(340.0)
        .width_range(280.0..=560.0)
        .frame(egui::Frame::none().inner_margin(egui::Margin { left: 14.0, ..Default::default() }))
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}  Comments", ph::CHAT_CIRCLE_TEXT)).font(theme::semibold(14.0)).color(p.text_strong));
                if !coll.comments.is_empty() {
                    ui_kit::chip(ui, coll.comments.len().to_string(), p.accent);
                }
            });
            ui.add_space(8.0);
            if ui_comments::thread(ui, ("coll", tab_id), &mut coll.comments, state, author, "") {
                changed = true;
            }
        });

    egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin { right: 14.0, ..Default::default() })).show_inside(
        ui,
        |ui| {
            egui::ScrollArea::vertical().id_salt(("coll_overview", tab_id)).auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui_kit::chip(ui, format!("{} {} requests", ph::LIST_BULLETS, coll.requests.len()), p.accent);
                    for m in METHODS {
                        let n = coll.requests.iter().filter(|r| r.method == *m).count();
                        if n > 0 {
                            ui_kit::chip(ui, format!("{m} {n}"), theme::method_color(m, p.dark));
                        }
                    }
                    let vars = coll.variables.iter().filter(|v| !v.is_blank()).count();
                    if vars > 0 {
                        ui_kit::chip(ui, format!("{} {vars} variables", ph::BRACKETS_CURLY), p.text_weak);
                    }
                });
                ui.add_space(16.0);

                section_title(ui, ph::TEXT_ALIGN_LEFT, "Description", "");
                egui::Frame::none()
                    .fill(p.input)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(10.0)
                    .inner_margin(egui::Margin::same(12.0))
                    .show(ui, |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::multiline(&mut coll.description)
                                    .frame(false)
                                    .hint_text("Describe what this collection is for, auth notes, links to docs…")
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                    });
                ui.add_space(18.0);

                section_title(ui, ph::BRACKETS_CURLY, "Variables", "{{name}} or {name} in requests · environment wins");
                let r = widgets::kv_table(
                    ui,
                    &mut coll.variables,
                    KvOptions {
                        id: egui::Id::new(("coll_vars", tab_id)),
                        key_hint: "Add variable, e.g. base_url",
                        value_hint: "Value",
                        allow_files: false,
                    },
                );
                changed |= r.changed;
                ui.add_space(18.0);

                section_title(ui, ph::LIST_BULLETS, "Requests", "");
                egui::Frame::none()
                    .fill(p.input)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(10.0)
                    .inner_margin(egui::Margin::same(4.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if coll.requests.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(10.0);
                                ui.label(RichText::new("No requests yet").color(p.text_weak));
                                ui.add_space(10.0);
                            });
                        }
                        for r in &coll.requests {
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 7.0, p.hover);
                            }
                            ui_kit::paint_method_badge(ui, egui::pos2(rect.left() + 10.0, rect.center().y), &r.method, 44.0);
                            let name = widgets::elided(ui, r.display_name(), theme::medium(13.5), p.text_strong, rect.width() * 0.35);
                            let name_w = name.size().x;
                            ui.painter().galley(egui::pos2(rect.left() + 64.0, rect.center().y - name.size().y / 2.0), name, p.text_strong);
                            let url = widgets::elided(ui, &r.url, theme::mono(12.0), p.text_weak, rect.width() - name_w - 140.0);
                            ui.painter().galley(
                                egui::pos2(rect.left() + 80.0 + name_w, rect.center().y - url.size().y / 2.0),
                                url,
                                p.text_weak,
                            );
                            if !r.comments.is_empty() {
                                ui.painter().text(
                                    egui::pos2(rect.right() - 12.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    format!("{} {}", ph::CHAT_CIRCLE, r.comments.len()),
                                    theme::medium(11.5),
                                    p.text_weak,
                                );
                            }
                            if resp.clicked() {
                                actions.push(Action::OpenRequest(coll_id.to_string(), r.id.clone()));
                            }
                        }
                    });
            });
        },
    );
    changed
}

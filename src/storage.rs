//! Persistence: workspace (collections, environments, settings), history and
//! the open-tab session, stored as JSON in `$XDG_DATA_HOME/reqora`.

use crate::model::*;
use crate::vars::Vars;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const HISTORY_LIMIT: usize = 300;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionTab {
    pub request: Request,
    pub saved: Request,
    pub collection_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub tabs: Vec<SessionTab>,
    pub active: usize,
}

pub struct Store {
    dir: PathBuf,
    pub ws: Workspace,
    pub history: Vec<HistoryEntry>,
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("REQORA_DATA_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("reqora");
    // The app used to be called "Free Postman": move its data over once.
    let legacy = base.join("free-postman");
    if !dir.exists() && legacy.is_dir() {
        if let Err(e) = std::fs::rename(&legacy, &dir) {
            eprintln!("reqora: could not migrate {} -> {}: {e}", legacy.display(), dir.display());
            return legacy;
        }
    }
    dir
}

/// Reads JSON; a corrupt file is moved aside (never silently overwritten).
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            let backup = path.with_extension(format!("corrupt-{}.json", now_ms()));
            eprintln!("reqora: cannot parse {}: {e}; moved to {}", path.display(), backup.display());
            let _ = std::fs::rename(path, &backup);
            None
        }
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_atomic(path, text.as_bytes())
}

pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

impl Store {
    pub fn open() -> Self {
        Self::open_at(data_dir())
    }

    pub fn open_at(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let ws = read_json(&dir.join("workspace.json")).unwrap_or_default();
        let history = read_json(&dir.join("history.json")).unwrap_or_default();
        Self { dir, ws, history }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save_workspace(&self) -> Result<(), String> {
        write_json(&self.dir.join("workspace.json"), &self.ws)
    }

    pub fn save_history(&self) -> Result<(), String> {
        write_json(&self.dir.join("history.json"), &self.history)
    }

    pub fn load_session(&self) -> Option<Session> {
        read_json(&self.dir.join("session.json"))
    }

    pub fn save_session_text(&self, text: &str) -> Result<(), String> {
        write_atomic(&self.dir.join("session.json"), text.as_bytes())
    }

    pub fn add_history(&mut self, entry: HistoryEntry) {
        self.history.insert(0, entry);
        self.history.truncate(HISTORY_LIMIT);
    }

    /// Variables visible to a request: collection variables, overridden by the active environment.
    pub fn vars_for(&self, collection_id: Option<&str>) -> Vars {
        let mut vars = Vars::new();
        if let Some(c) = collection_id.and_then(|id| self.collection(id)) {
            for v in c.variables.iter().filter(|v| v.is_active()) {
                vars.insert(v.key.trim().to_string(), v.value.clone());
            }
        }
        vars.extend(self.active_vars());
        vars
    }

    pub fn active_vars(&self) -> Vars {
        let mut vars = Vars::new();
        if let Some(env) = self.active_environment() {
            for v in env.variables.iter().filter(|v| v.is_active()) {
                vars.insert(v.key.trim().to_string(), v.value.clone());
            }
        }
        vars
    }

    pub fn active_environment(&self) -> Option<&Environment> {
        let id = self.ws.active_environment.as_deref()?;
        self.ws.environments.iter().find(|e| e.id == id)
    }

    pub fn collection(&self, id: &str) -> Option<&Collection> {
        self.ws.collections.iter().find(|c| c.id == id)
    }

    pub fn collection_mut(&mut self, id: &str) -> Option<&mut Collection> {
        self.ws.collections.iter_mut().find(|c| c.id == id)
    }

    pub fn find_request(&self, cid: &str, rid: &str) -> Option<&Request> {
        self.collection(cid)?.requests.iter().find(|r| r.id == rid)
    }

    /// Insert or replace a request (by id) in a collection.
    pub fn upsert_request(&mut self, cid: &str, req: Request) -> bool {
        let Some(c) = self.collection_mut(cid) else { return false };
        match c.requests.iter_mut().find(|r| r.id == req.id) {
            Some(slot) => *slot = req,
            None => c.requests.push(req),
        }
        true
    }

    pub fn environment_mut(&mut self, id: &str) -> Option<&mut Environment> {
        self.ws.environments.iter_mut().find(|e| e.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fp-test-{name}-{}", new_id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn roundtrip_and_corruption() {
        let dir = tmp_dir("store");
        let mut s = Store::open_at(dir.clone());
        let mut c = Collection::default();
        let r = Request { url: "http://x".into(), ..Default::default() };
        let rid = r.id.clone();
        c.requests.push(r);
        let cid = c.id.clone();
        s.ws.collections.push(c);
        let env = Environment {
            variables: vec![KeyValue::new("a", "1"), KeyValue { enabled: false, ..KeyValue::new("b", "2") }],
            ..Default::default()
        };
        s.ws.active_environment = Some(env.id.clone());
        s.ws.environments.push(env);
        s.save_workspace().unwrap();
        for i in 0..(HISTORY_LIMIT + 5) {
            s.add_history(HistoryEntry { id: i.to_string(), ..Default::default() });
        }
        s.save_history().unwrap();

        let s2 = Store::open_at(dir.clone());
        assert!(s2.find_request(&cid, &rid).is_some());
        assert_eq!(s2.history.len(), HISTORY_LIMIT);
        assert_eq!(s2.history[0].id, (HISTORY_LIMIT + 4).to_string());
        let vars = s2.active_vars();
        assert_eq!(vars.get("a").map(String::as_str), Some("1"));
        assert!(!vars.contains_key("b"));
        let mut s2 = s2;
        s2.ws.collections[0].variables = vec![KeyValue::new("a", "coll"), KeyValue::new("base_url", "http://c")];
        let v = s2.vars_for(Some(&cid));
        assert_eq!(v.get("a").map(String::as_str), Some("1")); // environment wins
        assert_eq!(v.get("base_url").map(String::as_str), Some("http://c"));
        assert!(!s2.vars_for(None).contains_key("base_url"));

        std::fs::write(dir.join("workspace.json"), "{ broken").unwrap();
        let s3 = Store::open_at(dir.clone());
        assert!(s3.ws.collections.is_empty());
        let backups =
            std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("corrupt")).count();
        assert_eq!(backups, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

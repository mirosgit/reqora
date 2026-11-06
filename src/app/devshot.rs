//! Developer-only scripted run that captures window screenshots
//! (compiled only with `--features dev-screenshot`; set REQORA_SHOT_DIR=/some/dir).

use super::*;

pub struct DevShot {
    dir: PathBuf,
    step: usize,
    frames: u32,
    pending: Option<&'static str>,
}

impl DevShot {
    pub fn from_env() -> Option<Self> {
        std::env::var_os("REQORA_SHOT_DIR").map(|d| Self { dir: d.into(), step: 0, frames: 0, pending: None })
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
        let busy = self.tabs.iter().any(|t| matches!(t, Tab::Request(r) if r.is_loading()));
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
                    self.dialog = None;
                    self.tabs.clear();
                    self.active = 0;
                }
                "09-welcome"
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

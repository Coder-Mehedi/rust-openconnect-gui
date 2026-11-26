//! egui front-end: profile form, status, certificate banner, prompt answers,
//! diagnostics, and a live log pane fed by the manager thread.

use std::sync::mpsc::{Receiver, Sender};

use eframe::egui::{self, Align, Color32, Layout, RichText, TextEdit};

use crate::{config, preflight, secrets, vpn};
use vpn::{Cmd, Event, Status};

const GREEN: Color32 = Color32::from_rgb(124, 222, 116);
const RED: Color32 = Color32::from_rgb(237, 135, 150);
const YELLOW: Color32 = Color32::from_rgb(235, 203, 139);
const MUTED: Color32 = Color32::from_rgb(166, 173, 200);
const BLUE: Color32 = Color32::from_rgb(137, 180, 250);

const MAX_LOG_LINES: usize = 4000;

pub struct App {
    cfg: config::Config,
    password: String,
    has_saved_password: bool,
    status: Status,
    log: Vec<String>,
    cert_prompt: bool,
    response_input: String,
    cmd_tx: Sender<Cmd>,
    events: Receiver<Event>,
    checks: Option<Vec<preflight::Check>>,
    open_setup: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let cfg = config::load();
        let (password, has_saved_password) = match secrets::load() {
            Ok(Some(p)) => (p, true),
            Ok(None) => (String::new(), false),
            Err(e) => {
                eprintln!("keyring: {e}");
                (String::new(), false)
            }
        };

        let (ev_tx, events) = std::sync::mpsc::channel();
        let cmd_tx = vpn::spawn(cc.egui_ctx.clone(), ev_tx);

        let mut app = Self {
            cfg,
            password,
            has_saved_password,
            status: Status::Disconnected,
            log: Vec::new(),
            cert_prompt: false,
            response_input: String::new(),
            cmd_tx,
            events,
            checks: None,
            open_setup: false,
        };

        // Zero-touch launch when everything needed is already saved.
        if app.cfg.auto_connect && app.cfg.save_password && !app.cfg.server.trim().is_empty() {
            app.connect();
        }
        app
    }

    fn connect(&mut self) {
        let mut password = self.password.clone();
        if password.is_empty() {
            match secrets::load() {
                Ok(Some(p)) => password = p,
                Ok(None) => {
                    self.push_note("no password available — type one first");
                    return;
                }
                Err(e) => {
                    self.push_note(&format!("keyring: {e}"));
                    return;
                }
            }
        }
        if self.cfg.save_password && !self.password.is_empty() {
            match secrets::store(&self.password) {
                Ok(()) => {
                    self.has_saved_password = true;
                    self.push_note("password saved to keyring");
                }
                Err(e) => self.push_note(&format!("could not save password: {e}")),
            }
        }
        if let Err(e) = config::save(&self.cfg) {
            self.push_note(&format!("could not save settings: {e}"));
        }
        self.cert_prompt = false;
        let params = vpn::ConnectParams {
            server: self.cfg.server.clone(),
            username: self.cfg.username.clone(),
            servercert: self.cfg.servercert.clone(),
            password: Some(password),
        };
        let _ = self.cmd_tx.send(Cmd::Connect(params));
    }

    fn forget(&mut self) {
        if let Err(e) = secrets::forget() {
            self.push_note(&format!("could not remove saved password: {e}"));
            return;
        }
        self.has_saved_password = false;
        self.password.clear();
        self.cfg.save_password = false;
        let _ = config::save(&self.cfg);
        self.push_note("saved password removed");
    }

    fn push_note(&mut self, s: &str) {
        self.log.push(format!("◈ {s}"));
        if self.log.len() > MAX_LOG_LINES {
            let drop = self.log.len() - MAX_LOG_LINES;
            self.log.drain(..drop);
        }
    }
}

impl eframe::App for App {
    /// Runs even while the window is hidden — keeps state fresh while the
    /// manager thread streams events, without waking the renderer.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut any = false;
        while let Ok(ev) = self.events.try_recv() {
            any = true;
            match ev {
                Event::Status(s) => self.status = s,
                Event::Log(line) => {
                    self.log.push(line);
                    if self.log.len() > MAX_LOG_LINES {
                        let drop = self.log.len() - MAX_LOG_LINES;
                        self.log.drain(..drop);
                    }
                }
                Event::CertPrompt => self.cert_prompt = true,
                Event::CertPinned(fp) => {
                    if self.cfg.servercert != fp {
                        self.cfg.servercert = fp;
                        match config::save(&self.cfg) {
                            Ok(()) => self.push_note("server certificate pinned — future connections won't ask"),
                            Err(e) => self.push_note(&format!("could not save pinned certificate: {e}")),
                        }
                    }
                    self.cert_prompt = false;
                }
                Event::Note(n) => {
                    if n.contains("sudoers rule") {
                        self.open_setup = true;
                    }
                    self.push_note(&n);
                }
            }
        }
        if any {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let running = matches!(self.status, Status::Connecting | Status::Connected | Status::Disconnecting);

        // ---- header -------------------------------------------------------
        ui.horizontal(|ui| {
            ui.heading("OpenConnect GUI");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (color, label) = match self.status {
                    Status::Connected => (GREEN, "connected"),
                    Status::Connecting => (YELLOW, "connecting…"),
                    Status::Disconnecting => (MUTED, "disconnecting…"),
                    Status::Disconnected => (RED, "disconnected"),
                };
                ui.label(RichText::new(label).color(color).strong());
                ui.label(RichText::new("●").color(color).size(14.0));
            });
        });
        ui.separator();

        // ---- profile form -------------------------------------------------
        field_row(ui, "Server", &mut self.cfg.server, false, "VPN gateway (IP or hostname)");
        field_row(ui, "Username", &mut self.cfg.username, false, "");
        let hint = if self.has_saved_password { "" } else { "password" };
        field_row(ui, "Password", &mut self.password, true, hint);

        checkbox_row(ui, &mut self.cfg.save_password, "Save password in keyring");
        checkbox_row(ui, &mut self.cfg.auto_connect, "Connect automatically at launch");

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("Connect")).clicked() {
                self.connect();
            }
            if ui.add_enabled(running, egui::Button::new("Disconnect")).clicked() {
                self.cert_prompt = false;
                let _ = self.cmd_tx.send(Cmd::Disconnect);
            }
            if ui.button("Forget password").clicked() {
                self.forget();
            }
            if ui.button("Preflight").clicked() {
                self.checks = Some(preflight::run_checks());
            }
        });

        // ---- certificate banner -------------------------------------------
        if self.cert_prompt && running {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("⚠").color(YELLOW));
                ui.label("The server's certificate isn't trusted yet. Accept it?");
                if ui.button("Accept & save").clicked() {
                    let _ = self.cmd_tx.send(Cmd::Respond("yes".into()));
                    self.cert_prompt = false;
                }
                if ui.button("Reject").clicked() {
                    let _ = self.cmd_tx.send(Cmd::Respond("no".into()));
                    self.cert_prompt = false;
                }
            });
        }

        // ---- extra prompt answer (2FA etc.) --------------------------------
        if running {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Answer").weak());
                let te = TextEdit::singleline(&mut self.response_input)
                    .hint_text("for extra prompts (e.g. 2FA)")
                    .desired_width(ui.available_width() - 64.0);
                let resp = ui.add(te);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (enter || ui.button("Send").clicked()) && !self.response_input.trim().is_empty() {
                    let _ = self
                        .cmd_tx
                        .send(Cmd::Respond(self.response_input.trim().to_string()));
                    self.response_input.clear();
                }
            });
        }

        ui.add_space(6.0);

        // ---- diagnostics ---------------------------------------------------
        if let Some(checks) = &self.checks {
            let all_ok = checks.iter().all(|c| c.ok);
            let header = RichText::new(if all_ok {
                "Diagnostics — all checks passed"
            } else {
                "Diagnostics — some checks failed"
            })
            .color(if all_ok { GREEN } else { RED });
            egui::CollapsingHeader::new(header)
                .default_open(true)
                .show(ui, |ui| {
                    for c in checks {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if c.ok { "✓" } else { "✗" })
                                    .color(if c.ok { GREEN } else { RED })
                                    .strong(),
                            );
                            ui.label(c.name);
                            ui.label(RichText::new(&c.detail).weak());
                        });
                    }
                });
            ui.add_space(4.0);
        }

        // ---- setup ---------------------------------------------------------
        let user = std::env::var("USER").unwrap_or_else(|_| "your-username".into());
        let sudoers_line = format!(
            "{user} ALL=(root) NOPASSWD: /usr/bin/openconnect *, /usr/bin/pkill -INT -x openconnect"
        );
        let setup_cmd = format!(
            "echo '{sudoers_line}' | sudo install -m440 /dev/stdin /etc/sudoers.d/openconnect-gui && sudo visudo -cf /etc/sudoers.d/openconnect-gui"
        );
        egui::CollapsingHeader::new(RichText::new("Setup — passwordless sudo for openconnect").color(BLUE))
            .default_open(self.open_setup)
            .show(ui, |ui| {
                ui.label("The GUI runs as your user and starts openconnect through sudo. Create this file once so no password prompt is needed:");
                ui.label(RichText::new(&sudoers_line).monospace().size(12.0));
                if ui.button("Copy line").clicked() {
                    ui.ctx().copy_text(sudoers_line.clone());
                }
                ui.label("Then run in a terminal (creates and validates the file):");
                ui.label(RichText::new(&setup_cmd).monospace().size(12.0));
                if ui.button("Copy command").clicked() {
                    ui.ctx().copy_text(setup_cmd.clone());
                }
                ui.add_space(4.0);
                ui.small(
                    "Note: /usr/bin/openconnect * allows any openconnect command to run as root — \
                     the same power your sudo password already grants on this single-user machine. \
                     To tighten it, list the exact arguments your connection uses instead of *.",
                );
            });

        // ---- log -----------------------------------------------------------
        ui.add_space(4.0);
        ui.separator();
        ui.label(RichText::new("openconnect log").small().weak());
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if self.log.is_empty() {
                    ui.label(RichText::new("no output yet").weak().italics());
                }
                for line in &self.log {
                    ui.label(RichText::new(line).monospace().size(12.0));
                }
            });
    }
}

fn field_row(ui: &mut egui::Ui, label: &str, value: &mut String, secret: bool, hint: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([86.0, 19.0], egui::Label::new(RichText::new(label).weak()));
        let mut te = TextEdit::singleline(value).desired_width(ui.available_width() - 8.0);
        if secret {
            te = te.password(true);
        }
        if !hint.is_empty() {
            te = te.hint_text(hint);
        }
        ui.add(te);
    });
}

fn checkbox_row(ui: &mut egui::Ui, checked: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([86.0, 19.0], egui::Label::new(""));
        ui.checkbox(checked, label);
    });
}

//! egui front-end: status card with a big connect button, profile card,
//! certificate banner, prompt answers, diagnostics, and a live log pane —
//! styled with the Catppuccin Mocha theme (see [`crate::theme`]).

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Layout, Margin, RichText, ScrollArea, Sense, Stroke, TextEdit};

use crate::{config, preflight, secrets, theme, vpn};
use vpn::{Cmd, Event, Status};

const MAX_LOG_LINES: usize = 4000;

pub struct App {
    cfg: config::Config,
    password: String,
    has_saved_password: bool,
    status: Status,
    connected_at: Option<Instant>,
    log: Vec<String>,
    response_input: String,
    cmd_tx: Sender<Cmd>,
    events: Receiver<Event>,
    checks: Option<Vec<preflight::Check>>,
    /// The sudoers Setup card is hidden unless the NOPASSWD rule is missing.
    needs_setup: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

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
            connected_at: None,
            log: Vec::new(),
            response_input: String::new(),
            cmd_tx,
            events,
            checks: None,
            needs_setup: !preflight::sudoers_ok(),
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
        self.truncate_log();
    }

    fn truncate_log(&mut self) {
        if self.log.len() > MAX_LOG_LINES {
            let drop = self.log.len() - MAX_LOG_LINES;
            self.log.drain(..drop);
        }
    }

    fn status_appearance(&self) -> (&'static str, egui::Color32) {
        match self.status {
            Status::Connected => ("Connected", theme::GREEN),
            Status::Connecting => ("Connecting…", theme::YELLOW),
            Status::Disconnecting => ("Disconnecting…", theme::OVERLAY1),
            Status::Disconnected => ("Not connected", theme::OVERLAY1),
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
                Event::Status(s) => {
                    match s {
                        Status::Connected => self.connected_at = Some(Instant::now()),
                        Status::Disconnected => self.connected_at = None,
                        _ => {}
                    }
                    self.status = s;
                }
                Event::Log(line) => {
                    self.log.push(line);
                    self.truncate_log();
                }
                Event::CertPinned(fp) => {
                    if self.cfg.servercert != fp {
                        self.cfg.servercert = fp;
                        match config::save(&self.cfg) {
                            Ok(()) => self.push_note("server certificate pinned — future connections won't ask"),
                            Err(e) => self.push_note(&format!("could not save pinned certificate: {e}")),
                        }
                    }
                }
                Event::Note(n) => {
                    if n.contains("sudoers rule") {
                        self.needs_setup = true;
                    }
                    self.push_note(&n);
                }
            }
        }
        if any {
            ctx.request_repaint();
        }
        // Tick the uptime display while connected.
        if self.status == Status::Connected {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let running = matches!(self.status, Status::Connecting | Status::Connected | Status::Disconnecting);
        let profile_ready = !self.cfg.server.trim().is_empty() && !self.cfg.username.trim().is_empty();

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Page padding so the content column doesn't touch the window
                // edges; the header then aligns with the card borders.
                egui::Frame::NONE
                    .outer_margin(Margin {
                        left: 16,
                        right: 16,
                        top: 12,
                        bottom: 18,
                    })
                    .show(ui, |ui| {
                        self.header(ui);
                        ui.add_space(12.0);

                        self.status_card(ui, running);
                        ui.add_space(12.0);

                        self.profile_card(ui, profile_ready);
                        ui.add_space(12.0);

                        if let Some(checks) = self.checks.clone() {
                            self.diagnostics_card(ui, &checks);
                            ui.add_space(12.0);
                        }

                        if self.needs_setup {
                            self.setup_card(ui);
                            ui.add_space(12.0);
                        }

                        self.log_card(ui);
                    });
            });
    }
}

// ---- sections ---------------------------------------------------------------

impl App {
    fn header(&mut self, ui: &mut egui::Ui) {
        let (label, color) = self.status_appearance();
        ui.horizontal(|ui| {
            ui.label(RichText::new("OpenConnect").size(19.0).strong().color(theme::TEXT));
            ui.label(RichText::new("VPN").size(19.0).color(theme::OVERLAY0));
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                let (bg, stroke, text) = pill_colors(color);
                egui::Frame::NONE
                    .fill(bg)
                    .stroke(stroke)
                    .corner_radius(999.0)
                    .inner_margin(Margin::symmetric(10, 3))
                    .show(ui, |ui| {
                        ui.label(RichText::new(label).small().strong().color(text));
                    });
            });
        });
    }

    fn status_card(&mut self, ui: &mut egui::Ui, running: bool) {
        let (title, color) = self.status_appearance();

        card(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(RichText::new(title).size(16.0).strong().color(color));
                if self.status == Status::Connected {
                    if let Some(t) = self.connected_at {
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(uptime(t.elapsed())).small().color(theme::OVERLAY1));
                        });
                    }
                }
            });

            if !self.cfg.server.trim().is_empty() {
                ui.horizontal(|ui| {
                    ui.add_sized([52.0, 16.0], egui::Label::new(RichText::new("Server").small().color(theme::OVERLAY1)));
                    ui.label(RichText::new(self.cfg.server.trim()).small().color(theme::SUBTEXT1));
                });
                ui.horizontal(|ui| {
                    ui.add_sized([52.0, 16.0], egui::Label::new(RichText::new("User").small().color(theme::OVERLAY1)));
                    ui.label(RichText::new(self.cfg.username.trim()).small().color(theme::SUBTEXT1));
                });
            } else {
                ui.label(RichText::new("Fill in your profile below to get started").small().color(theme::SUBTEXT0));
            }

            ui.add_space(12.0);

            let (label, fill, enabled, is_connect) = match self.status {
                Status::Disconnected => ("Connect", theme::GREEN.gamma_multiply(0.80), true, true),
                Status::Connecting | Status::Connected => ("Disconnect", theme::RED.gamma_multiply(0.85), true, false),
                Status::Disconnecting => ("Disconnecting…", theme::SURFACE1, false, false),
            };
            let button = egui::Button::new(RichText::new(label).size(14.5).strong().color(theme::CRUST))
                .fill(fill)
                .corner_radius(8.0);
            let resp = ui.add_sized([ui.available_width(), 38.0], button);
            if enabled && resp.clicked() {
                if is_connect {
                    self.connect();
                } else {
                    let _ = self.cmd_tx.send(Cmd::Disconnect);
                }
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if small_button(ui, "Forget password") {
                    self.forget();
                }
                if small_button(ui, "Preflight") {
                    self.checks = Some(preflight::run_checks());
                }
            });

            // Extra prompt answer (2FA etc.) while the connection is up.
            if running {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Answer").small().color(theme::OVERLAY1));
                    let te = TextEdit::singleline(&mut self.response_input)
                        .hint_text("for extra prompts (e.g. 2FA)")
                        .desired_width(ui.available_width() - 70.0);
                    let resp = ui.add(te);
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (enter || small_button(ui, "Send"))
                        && !self.response_input.trim().is_empty()
                    {
                        let _ = self
                            .cmd_tx
                            .send(Cmd::Respond(self.response_input.trim().to_string()));
                        self.response_input.clear();
                    }
                });
            }
        });
    }

    fn profile_card(&mut self, ui: &mut egui::Ui, profile_ready: bool) {
        let incomplete = !profile_ready || !self.has_saved_password;
        card(ui, |ui| {
            let hint = if profile_ready {
                format!("{} · {}", self.cfg.server.trim(), self.cfg.username.trim())
            } else {
                "not configured yet".into()
            };
            egui::CollapsingHeader::new(
                RichText::new(format!("Profile   {hint}")).strong().color(theme::SUBTEXT1).small(),
            )
            .default_open(incomplete)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    field(ui, "Server", &mut self.cfg.server, false, "VPN gateway (IP or hostname)");
                    field(ui, "Username", &mut self.cfg.username, false, "");
                    let pw_hint = if self.has_saved_password {
                        "saved in keyring — type to replace"
                    } else {
                        "password"
                    };
                    field(ui, "Password", &mut self.password, true, pw_hint);

                    ui.add_space(4.0);
                    checkbox(ui, &mut self.cfg.save_password, "Save password in keyring");
                    checkbox(ui, &mut self.cfg.auto_connect, "Connect automatically at launch");
                });
        });
    }

    fn diagnostics_card(&mut self, ui: &mut egui::Ui, checks: &[preflight::Check]) {
        let all_ok = checks.iter().all(|c| c.ok);
        let (title, color) = if all_ok {
            ("Diagnostics — all checks passed", theme::GREEN)
        } else {
            ("Diagnostics — some checks failed", theme::PEACH)
        };
        card(ui, |ui| {
            egui::CollapsingHeader::new(RichText::new(title).strong().color(color).small())
                .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    for c in checks {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(if c.ok { "✓" } else { "✗" })
                                    .strong()
                                    .color(if c.ok { theme::GREEN } else { theme::RED }),
                            );
                            ui.label(RichText::new(c.name).small().color(theme::SUBTEXT1));
                        });
                        ui.label(RichText::new(&c.detail).small().color(theme::OVERLAY1));
                    }
                });
        });
    }

    fn setup_card(&mut self, ui: &mut egui::Ui) {
        let user = std::env::var("USER").unwrap_or_else(|_| "your-username".into());
        let sudoers_line = format!(
            "{user} ALL=(root) NOPASSWD: /usr/bin/openconnect *, /usr/bin/pkill -INT -x openconnect"
        );
        let setup_cmd = format!(
            "echo '{sudoers_line}' | sudo install -m440 /dev/stdin /etc/sudoers.d/openconnect-gui && sudo visudo -cf /etc/sudoers.d/openconnect-gui"
        );
        card(ui, |ui| {
            egui::CollapsingHeader::new(
                RichText::new("Setup · passwordless sudo   one-time").strong().color(theme::SUBTEXT1).small(),
            )
            .default_open(true)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "The GUI runs as your user and starts openconnect through sudo. \
                             Create this file once so no password prompt is needed:",
                        )
                        .small()
                        .color(theme::SUBTEXT1),
                    );
                    code_block(ui, &sudoers_line);
                    if ui.button(RichText::new("Copy line").small()).clicked() {
                        ui.ctx().copy_text(sudoers_line.clone());
                    }
                    ui.label(RichText::new("Then run in a terminal:").small().color(theme::SUBTEXT1));
                    code_block(ui, &setup_cmd);
                    if ui.button(RichText::new("Copy command").small()).clicked() {
                        ui.ctx().copy_text(setup_cmd.clone());
                    }
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "/usr/bin/openconnect * allows any openconnect command to run as root — \
                             the same power your sudo password already grants on this single-user machine. \
                             To tighten it, list the exact arguments your connection uses instead of *.",
                        )
                        .small()
                        .color(theme::OVERLAY1),
                    );
                });
        });
    }

    fn log_card(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Log").small().strong().color(theme::SUBTEXT1));
        egui::Frame::NONE
            .fill(theme::CRUST)
            .stroke(Stroke::new(1.0, theme::SURFACE0))
            .corner_radius(12.0)
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ScrollArea::vertical()
                    .max_height(220.0)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.log.is_empty() {
                            ui.set_min_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                ui.add_space(24.0);
                                ui.label(
                                    RichText::new("no output yet")
                                        .small()
                                        .italics()
                                        .color(theme::OVERLAY1),
                                );
                            });
                        }
                        for line in &self.log {
                            ui.label(RichText::new(line).monospace().size(11.5).color(log_color(line)));
                        }
                    });
            });
    }
}

// ---- helpers ----------------------------------------------------------------

/// A compact secondary button with a visible surface fill, so it doesn't
/// read as disabled next to the primary connect button.
fn small_button(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add(
        egui::Button::new(RichText::new(label).small().color(theme::SUBTEXT1))
            .fill(theme::SURFACE0)
            .stroke(Stroke::new(1.0, theme::SURFACE1))
            .corner_radius(8.0),
    )
    .clicked()
}

/// Tinted background for the header status pill: a dim wash of the status
/// color, or a neutral surface for the grey idle/transition states.
/// Returns (fill, stroke, text color).
fn pill_colors(color: egui::Color32) -> (egui::Color32, egui::Stroke, egui::Color32) {
    let neutral = color == theme::OVERLAY1;
    let bg = if neutral { theme::SURFACE1 } else { color.gamma_multiply(0.16) };
    let stroke_color = if neutral { theme::SURFACE2 } else { color.gamma_multiply(0.45) };
    let text = if neutral { theme::SUBTEXT1 } else { color };
    (bg, Stroke::new(1.0, stroke_color), text)
}

fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(theme::MANTLE)
        .stroke(Stroke::new(1.0, theme::SURFACE0))
        .corner_radius(12.0)
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });
}

fn code_block(ui: &mut egui::Ui, text: &str) {
    egui::Frame::NONE
        .fill(theme::CRUST)
        .corner_radius(8.0)
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(text).monospace().size(11.5).color(theme::TEXT));
        });
    ui.add_space(4.0);
}

fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, secret: bool, hint: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([78.0, 19.0], egui::Label::new(RichText::new(label).small().color(theme::OVERLAY1)));
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

fn checkbox(ui: &mut egui::Ui, checked: &mut bool, label: &str) {
    ui.checkbox(checked, RichText::new(label).small().color(theme::SUBTEXT0));
}

fn uptime(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("up {}h {:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("up {}m {:02}s", s / 60, s % 60)
    }
}

fn log_color(line: &str) -> egui::Color32 {
    if line.starts_with('◈') {
        theme::YELLOW
    } else if [
        "failed",
        "Failed",
        "error",
        "Error",
        "denied",
        "password is required",
        "aborted",
    ]
    .iter()
    .any(|k| line.contains(k))
    {
        theme::RED
    } else if line.contains("connected") || line.contains("Connected") || line.contains("Established") {
        theme::GREEN
    } else {
        theme::SUBTEXT0
    }
}

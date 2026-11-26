//! Manager thread that owns the openconnect child process.
//!
//! Design notes:
//! - The GUI never runs as root. It spawns `sudo -n /usr/bin/openconnect …`
//!   and relies on a sudoers NOPASSWD rule (see Setup in the app).
//! - The password travels via stdin (`--passwd-on-stdin`), never argv.
//! - openconnect's stdout/stderr are redirected to a log file which this
//!   thread tails. If the GUI exits, the child keeps running (a pipe would
//!   get SIGPIPE-killed on GUI close); a restarted GUI adopts the running
//!   process via `pgrep`.
//! - Disconnect uses `sudo -n pkill -INT -x openconnect`: SIGINT makes
//!   openconnect log out of the gateway cleanly, like pressing Ctrl-C.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::config;

pub const OPENCONNECT: &str = "/usr/bin/openconnect";
pub const SUDO: &str = "/usr/bin/sudo";
pub const PGREP: &str = "/usr/bin/pgrep";
pub const PKILL: &str = "/usr/bin/pkill";

const LOG_ROTATE_BYTES: u64 = 5 * 1024 * 1024;
const ADOPT_POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
}

pub struct ConnectParams {
    pub server: String,
    pub username: String,
    pub servercert: String,
    pub password: Option<String>,
}

pub enum Cmd {
    Connect(ConnectParams),
    Disconnect,
    /// Send a line to openconnect's stdin (2FA answers, cert "yes").
    Respond(String),
}

pub enum Event {
    Status(Status),
    Log(String),
    /// A `--servercert <fingerprint>` value was found in the output — pin it.
    CertPinned(String),
    Note(String),
}

/// Start the manager thread; returns the sending half the GUI uses.
pub fn spawn(ctx: egui::Context, events: Sender<Event>) -> Sender<Cmd> {
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let log_path = config::log_path().unwrap_or_else(|| PathBuf::from("/tmp/openconnect-gui.log"));
    if let Some(dir) = log_path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    std::thread::Builder::new()
        .name("vpn-manager".into())
        .spawn(move || {
            let mut m = Manager {
                ctx,
                events,
                cmd_rx,
                child: None,
                stdin: None,
                tail: None,
                tail_buf: Vec::new(),
                log_path,
                status: Status::Disconnected,
                cert_answered: false,
                last_adopt_check: Instant::now(),
            };
            m.adopt_existing();
            m.run();
        })
        .expect("failed to spawn vpn manager thread");
    cmd_tx
}

/// Is an openconnect process running right now (ours or adopted)?
pub fn is_running() -> bool {
    Command::new(PGREP)
        .arg("-x")
        .arg("openconnect")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct Manager {
    ctx: egui::Context,
    events: Sender<Event>,
    cmd_rx: Receiver<Cmd>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    tail: Option<fs::File>,
    tail_buf: Vec<u8>,
    log_path: PathBuf,
    status: Status,
    /// Whether we already auto-accepted the certificate question on this
    /// connection — prevents a stray second `yes` answering a later prompt.
    cert_answered: bool,
    last_adopt_check: Instant,
}

impl Manager {
    fn run(&mut self) {
        loop {
            match self.cmd_rx.recv_timeout(Duration::from_millis(150)) {
                Ok(Cmd::Connect(p)) => self.connect(p),
                Ok(Cmd::Disconnect) => self.disconnect(),
                Ok(Cmd::Respond(s)) => {
                    let _ = self.write_stdin(&s);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
        }
        // On GUI exit: leave the child running (detached), just stop watching.
    }

    /// GUI restarted while a connection is up: adopt it and show recent log.
    fn adopt_existing(&mut self) {
        if !is_running() {
            return;
        }
        self.set_status(Status::Connected);
        for line in recent_log_lines(200) {
            self.emit(Event::Log(line));
        }
        self.emit(Event::Note("attached to an openconnect process that is already running".into()));
        self.open_tail_at_end();
    }

    fn connect(&mut self, p: ConnectParams) {
        if self.child.is_some() || is_running() {
            self.emit(Event::Note("openconnect is already running — disconnect first".into()));
            return;
        }
        if p.server.trim().is_empty() {
            self.emit(Event::Note("no server configured".into()));
            return;
        }
        self.cert_answered = false;

        // Rotate an oversized log, then remember where this session starts.
        if fs::metadata(&self.log_path).map(|m| m.len() > LOG_ROTATE_BYTES).unwrap_or(false) {
            let _ = fs::remove_file(&self.log_path);
        }
        let tail_start = fs::metadata(&self.log_path).map(|m| m.len()).unwrap_or(0);

        let mut log = match OpenOptions::new().create(true).append(true).open(&self.log_path) {
            Ok(f) => f,
            Err(e) => {
                self.emit(Event::Note(format!("cannot open log file: {e}")));
                return;
            }
        };
        let log_err = match log.try_clone() {
            Ok(f) => f,
            Err(e) => {
                self.emit(Event::Note(format!("cannot duplicate log fd: {e}")));
                return;
            }
        };
        let _ = writeln!(log, "--- connecting to {} as {} ---", p.server.trim(), p.username.trim());

        let mut cmd = Command::new(SUDO);
        cmd.arg("-n")
            .arg(OPENCONNECT)
            .arg(p.server.trim())
            .arg(format!("--user={}", p.username.trim()))
            .arg("--passwd-on-stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));
        if !p.servercert.trim().is_empty() {
            cmd.arg(format!("--servercert={}", p.servercert.trim()));
        }

        self.tail = None;
        self.tail_buf.clear();
        match cmd.spawn() {
            Ok(mut child) => {
                self.stdin = child.stdin.take();
                self.child = Some(child);
                if let Some(pw) = p.password {
                    if !self.write_stdin(&pw) {
                        return;
                    }
                }
                self.open_tail_at_offset(tail_start);
                self.set_status(Status::Connecting);
            }
            Err(e) => self.emit(Event::Note(format!("failed to spawn openconnect: {e}"))),
        }
    }

    fn disconnect(&mut self) {
        if !is_running() && self.child.is_none() {
            self.emit(Event::Note("not connected".into()));
            return;
        }
        self.stdin.take();
        self.set_status(Status::Disconnecting);
        match Command::new(SUDO)
            .args(["-n", PKILL, "-INT", "-x", "openconnect"])
            .output()
        {
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                if err.contains("password is required") {
                    self.emit(Event::Note(
                        "sudo: password required — the pkill sudoers rule is missing (see Setup)".into(),
                    ));
                }
            }
            Err(e) => self.emit(Event::Note(format!("failed to run pkill: {e}"))),
        }
    }

    fn write_stdin(&mut self, s: &str) -> bool {
        let Some(stdin) = self.stdin.as_mut() else {
            self.emit(Event::Note("openconnect is not accepting input".into()));
            return false;
        };
        let res = stdin
            .write_all(s.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush());
        match res {
            Ok(()) => true,
            Err(e) => {
                self.emit(Event::Note(format!("failed to write to openconnect stdin: {e}")));
                false
            }
        }
    }

    fn tick(&mut self) {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    self.stdin = None;
                    self.drain_tail();
                    self.set_status(Status::Disconnected);
                }
                Ok(None) | Err(_) => {}
            }
        } else if self.last_adopt_check.elapsed() >= ADOPT_POLL {
            self.last_adopt_check = Instant::now();
            if is_running() {
                if self.status == Status::Disconnected {
                    self.set_status(Status::Connected);
                    self.open_tail_at_end();
                    self.emit(Event::Note("attached to a running openconnect process".into()));
                }
            } else if matches!(self.status, Status::Connected | Status::Disconnecting) {
                self.set_status(Status::Disconnected);
            }
        }
        self.drain_tail();
    }

    fn open_tail_at_offset(&mut self, offset: u64) {
        self.tail_buf.clear();
        match OpenOptions::new().read(true).open(&self.log_path) {
            Ok(mut f) => {
                let _ = f.seek(SeekFrom::Start(offset));
                self.tail = Some(f);
            }
            Err(_) => self.tail = None,
        }
    }

    fn open_tail_at_end(&mut self) {
        let end = fs::metadata(&self.log_path).map(|m| m.len()).unwrap_or(0);
        self.open_tail_at_offset(end);
    }

    fn drain_tail(&mut self) {
        let mut new_bytes = Vec::new();
        if let Some(tail) = self.tail.as_mut() {
            let mut buf = [0u8; 8192];
            loop {
                match tail.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        new_bytes.extend_from_slice(&buf[..n]);
                        if new_bytes.len() > (1 << 20) {
                            break;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        }
        self.tail_buf.extend_from_slice(&new_bytes);
        // openconnect occasionally uses \r to redraw progress; treat both as ends.
        while let Some(idx) = self.tail_buf.iter().position(|&b| b == b'\n' || b == b'\r') {
            let line: Vec<u8> = self.tail_buf.drain(..=idx).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            if !line.is_empty() {
                self.handle_line(line);
            }
        }
        // The certificate question arrives WITHOUT a trailing newline while
        // openconnect blocks waiting for the answer — flush and handle the
        // partial line so we can auto-accept it instead of hanging.
        if !self.tail_buf.is_empty() {
            let pending = String::from_utf8_lossy(&self.tail_buf).trim().to_string();
            if !pending.is_empty() && is_cert_question(&pending) && !self.cert_answered {
                self.tail_buf.clear();
                self.handle_line(pending);
            }
        }
    }

    fn handle_line(&mut self, line: String) {
        if is_connected_line(&line) && self.status != Status::Connected {
            self.set_status(Status::Connected);
        }
        if needs_sudoers_hint(&line) {
            self.emit(Event::Note(
                "sudo needs a password — the sudoers rule from Setup is missing or not applied".into(),
            ));
        }
        // Trust on first use: automatically answer the server-certificate
        // question the first time it appears; the fingerprint openconnect
        // prints next gets pinned (CertPinned) so it never asks again.
        if is_cert_question(&line) && !self.cert_answered && self.stdin.is_some() {
            self.cert_answered = true;
            if self.write_stdin("yes") {
                self.emit(Event::Note(
                    "server certificate not yet trusted — accepted; pinning fingerprint".into(),
                ));
            }
        }
        if let Some(fp) = extract_servercert(&line) {
            self.emit(Event::CertPinned(fp));
        }
        self.emit(Event::Log(line));
    }

    fn set_status(&mut self, s: Status) {
        if self.status != s {
            self.status = s;
            self.emit(Event::Status(s));
        }
    }

    fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
        self.ctx.request_repaint();
    }
}

fn is_connected_line(line: &str) -> bool {
    line.contains("CSTP connected") || line.contains("ESP connected")
}

fn needs_sudoers_hint(line: &str) -> bool {
    line.contains("a password is required")
}

/// The interactive certificate question openconnect prints on first contact.
fn is_cert_question(line: &str) -> bool {
    line.contains("to accept") && line.contains("to abort")
}

/// openconnect itself prints `--servercert <fingerprint>` after you accept a
/// certificate ("To trust this server in future, perhaps add this to your
/// command line"). Reuse that exact value for future connections.
fn extract_servercert(line: &str) -> Option<String> {
    let idx = line.find("--servercert ")?;
    let rest = line[idx + "--servercert ".len()..].trim();
    let fp = rest.split_whitespace().next()?;
    (fp.len() >= 8).then(|| fp.to_string())
}

/// Last `n` lines of the log file (for adopting a session on GUI restart).
fn recent_log_lines(n: usize) -> Vec<String> {
    let Some(path) = config::log_path() else { return Vec::new() };
    let Ok(mut f) = fs::File::open(path) else { return Vec::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(16 * 1024)));
    let mut s = String::new();
    let _ = f.read_to_string(&mut s);
    let lines: Vec<&str> = s.lines().collect();
    lines.iter().rev().take(n).rev().map(|l| l.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_connected_lines() {
        assert!(is_connected_line("CSTP connected. DPD 30, Keepalive 10"));
        assert!(!is_connected_line("Establishing DTLS connection"));
    }

    #[test]
    fn detects_cert_question() {
        assert!(is_cert_question(
            "Enter 'yes' to accept, 'no' to abort; anything else to view: "
        ));
        // The verification-failure notice alone is not the question itself.
        assert!(!is_cert_question(
            "Certificate from VPN server \"198.51.100.1\" failed verification."
        ));
        assert!(!is_cert_question("Established DTLS connection (using GnuTLS). Ciphersuite x."));
    }

    #[test]
    fn extracts_servercert() {
        let line = "\t--servercert 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b";
        assert_eq!(
            extract_servercert(line).as_deref(),
            Some("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b")
        );
        assert_eq!(extract_servercert("To trust this server in future, perhaps add this"), None);
        assert_eq!(extract_servercert("--servercert "), None);
    }

    #[test]
    fn sudoers_hint_line() {
        assert!(needs_sudoers_hint("sudo: a password is required"));
    }
}

mod app;
mod config;
mod preflight;
mod secrets;
mod vpn;

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--check") {
        let checks = preflight::run_checks();
        let all_ok = checks.iter().all(|c| c.ok);
        for c in &checks {
            println!("{} {} — {}", if c.ok { "✓" } else { "✗" }, c.name, c.detail);
        }
        if !all_ok {
            println!("\nFix the failing checks (see Setup in the app or the README), then re-run.");
        }
        std::process::exit(if all_ok { 0 } else { 1 });
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("OpenConnect GUI")
            .with_inner_size([560.0, 820.0]),
        ..Default::default()
    };

    eframe::run_native(
        "OpenConnect GUI",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)) as Box<dyn eframe::App>)),
    )
}

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod art;
mod shell;

use desktop_pet::{
    config,
    engine::{Engine, Interaction},
    safety,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Default)]
pub struct Options {
    pub settings: bool,
    pub exit_after: Option<f64>,
    pub screenshot: Option<PathBuf>,
    pub capture_ok: Arc<AtomicBool>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Mochi: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "--version" || argument == "-V")
    {
        println!("Mochi {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!(
            "Mochi desktop pet\n  --settings              Open the control panel\n  --restore-desktop       Restore this app's active desktop visibility lease\n  --simulate              Run deterministic behavior simulation (no GUI/OS changes)\n  --exit-after SECONDS    Exit automatically (development)\n  --screenshot PATH      Save a rendered control-panel screenshot (development)\n  --data-dir PATH         Use an isolated configuration directory (development)"
        );
        return Ok(());
    }
    if arguments.iter().any(|argument| argument == "--simulate") {
        let mut engine = Engine::new(config::Config::default(), 0.0, 42);
        for (time, action) in [
            (1.0, Interaction::Pet),
            (5.0, Interaction::Feed),
            (10.0, Interaction::Poke),
            (20.0, Interaction::Play),
            (35.0, Interaction::Busy),
        ] {
            engine.interact(action, time);
            println!(
                "{time:>5.0}s {action:?}: {:?} {}",
                engine.mood,
                engine.bubble.unwrap_or("")
            );
        }
        assert!(engine.quiet(40.0));
        println!(
            "Simulation complete: no desktop changes; affinity={}",
            engine.config.affinity
        );
        return Ok(());
    }
    let value = |flag: &str| -> Result<Option<String>, Box<dyn std::error::Error>> {
        match arguments.iter().position(|argument| argument == flag) {
            Some(index) => Ok(Some(
                arguments
                    .get(index + 1)
                    .filter(|text| !text.starts_with("--"))
                    .ok_or_else(|| format!("{flag} requires a value"))?
                    .clone(),
            )),
            None => Ok(None),
        }
    };
    let directory = if let Some(path) = value("--data-dir")? {
        PathBuf::from(path)
    } else if let Some(path) = std::env::var_os("MOCHI_DATA_DIR") {
        PathBuf::from(path)
    } else {
        config::data_dir()?
    };
    std::fs::create_dir_all(&directory)?;
    if arguments.first().map(String::as_str) == Some("--desktop-guard") {
        return safety::guard(
            &directory,
            arguments.get(1).ok_or("Missing watchdog lease identity")?,
        )
        .map_err(Into::into);
    }
    if arguments
        .iter()
        .any(|argument| argument == "--restore-desktop")
    {
        println!("Restored: {}", safety::restore(&directory)?);
        return Ok(());
    }
    let instance = safety::lock(&directory.join("instance.lock"))
        .map_err(|_| "Mochi is already running. Open its tray menu or press Ctrl+Alt+Shift+M.")?;
    let startup_recovery = safety::restore(&directory);
    let config_path = directory.join("config.json");
    let first_run = !config_path.exists();
    let (config, warning) = match config::load(&config_path) {
        Ok(config) => (config, None),
        Err(error) => {
            let backup = directory.join(format!("config-invalid-{}.json", safety::timestamp()));
            std::fs::copy(&config_path, &backup)?;
            (
                config::Config::default(),
                Some(format!("设置文件损坏，已备份并使用安全默认值：{error}")),
            )
        }
    };
    let warning = warning.or_else(|| {
        startup_recovery
            .err()
            .map(|error| format!("桌面恢复失败：{error}。请使用恢复按钮重试。"))
    });
    let options = Options {
        settings: first_run || arguments.iter().any(|argument| argument == "--settings"),
        exit_after: value("--exit-after")?
            .map(|text| text.parse::<f64>())
            .transpose()?
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0),
        screenshot: value("--screenshot")?.map(PathBuf::from),
        capture_ok: Arc::new(AtomicBool::new(false)),
    };
    let screenshot_requested = options.screenshot.is_some();
    let capture_ok = options.capture_ok.clone();
    let scale = config.scale;
    let native = eframe::NativeOptions {
        viewport: if screenshot_requested {
            eframe::egui::ViewportBuilder::default()
                .with_title("麻薯 Mochi · 控制面板")
                .with_inner_size([700.0, 720.0])
        } else {
            eframe::egui::ViewportBuilder::default()
                .with_title("麻薯 Mochi")
                .with_inner_size([260.0 * scale, 250.0 * scale])
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_active(false)
        },
        renderer: eframe::Renderer::Glow,
        vsync: false,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        run_and_return: true,
        ..Default::default()
    };
    let started = Instant::now();
    eframe::run_native(
        "Mochi",
        native,
        Box::new(move |creation| {
            Ok(Box::new(app::PetApp::new(
                creation, config, directory, instance, warning, options, started,
            )))
        }),
    )?;
    if screenshot_requested && !capture_ok.load(Ordering::Acquire) {
        return Err("Requested control-panel screenshot was not captured".into());
    }
    Ok(())
}

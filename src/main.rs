#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod art;
mod content_panel;
mod hover;
mod native_content;
mod panel;
mod shell;

use desktop_pet::{
    config,
    engine::{Engine, Interaction},
    safety, skin,
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
    pub settings_page: usize,
    pub exit_after: Option<f64>,
    pub screenshot: Option<PathBuf>,
    pub capture_ok: Arc<AtomicBool>,
    pub skin: Option<Arc<skin::Skin>>,
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
            "Mochi desktop pet\n  --settings              Open the control panel\n  --settings-page PAGE   Open home/preferences/journal/skin/clipboard/transfer\n  --skin PATH             Use a skin directory, .mochi package, or ZIP\n  --check-skin PATH       Validate a skin without starting the GUI\n  --pack-skin DIRECTORY   Pack a skin directory (requires --output FILE.mochi or FILE.zip)\n  --unpack-skin PACKAGE    Extract a skin into a new directory (requires --output)\n  --output PATH           Output file/directory for skin packaging\n  --restore-desktop       Restore this app's active desktop visibility lease\n  --simulate              Run deterministic behavior simulation (no GUI/OS changes)\n  --exit-after SECONDS    Exit automatically (development)\n  --screenshot PATH      Save a rendered control-panel screenshot (development)\n  --data-dir PATH         Use an isolated configuration directory (development)"
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
    if let Some(directory) = value("--pack-skin")? {
        let output = value("--output")?.ok_or("--pack-skin requires --output")?;
        skin::pack(
            std::path::Path::new(&directory),
            std::path::Path::new(&output),
        )?;
        println!("Skin package saved: {output}");
        return Ok(());
    }
    if let Some(path) = value("--unpack-skin")? {
        let output = value("--output")?.ok_or("--unpack-skin requires --output")?;
        skin::unpack(std::path::Path::new(&path), std::path::Path::new(&output))?;
        println!("Skin unpacked: {output}");
        return Ok(());
    }
    if let Some(path) = value("--check-skin")? {
        let skin = skin::load(std::path::Path::new(&path))?;
        println!("Valid skin: {} ({})", skin.manifest.name, skin.manifest.id);
        return Ok(());
    }
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
    let (mut config, warning) = match config::load(&config_path) {
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
    let mut warning = warning.or_else(|| {
        startup_recovery
            .err()
            .map(|error| format!("桌面恢复失败：{error}。请使用恢复按钮重试。"))
    });
    let active_skin = if let Some(path) = value("--skin")? {
        let path = PathBuf::from(path).canonicalize()?;
        let skin = skin::load(&path)?;
        config.skin = Some(path);
        skin
    } else if let Some(path) = &config.skin {
        match skin::load(path) {
            Ok(skin) => skin,
            Err(error) => {
                let message = format!("皮肤加载失败，使用默认皮肤：{error}");
                warning = Some(warning.map_or_else(
                    || message.clone(),
                    |warning| format!("{warning}\n{message}"),
                ));
                skin::Skin::builtin()
            }
        }
    } else {
        skin::Skin::builtin()
    };
    let canvas = active_skin.ui.canvas;
    let page = value("--settings-page")?;
    let settings_page =
        match page.as_deref() {
            None | Some("home") => 0,
            Some("preferences") => 1,
            Some("journal") => 2,
            Some("skin") => 3,
            Some("clipboard") => 4,
            Some("transfer") => 5,
            Some(_) => return Err(
                "Settings page must be home, preferences, journal, skin, clipboard, or transfer"
                    .into(),
            ),
        };
    let options = Options {
        settings: first_run
            || page.is_some()
            || arguments.iter().any(|argument| argument == "--settings"),
        settings_page,
        exit_after: value("--exit-after")?
            .map(|text| text.parse::<f64>())
            .transpose()?
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0),
        screenshot: value("--screenshot")?.map(PathBuf::from),
        capture_ok: Arc::new(AtomicBool::new(false)),
        skin: Some(active_skin),
    };
    let screenshot_requested = options.screenshot.is_some();
    let capture_ok = options.capture_ok.clone();
    let scale = config.scale;
    let native = eframe::NativeOptions {
        viewport: if screenshot_requested {
            eframe::egui::ViewportBuilder::default()
                .with_title("麻薯 Mochi · 控制面板")
                .with_inner_size([1000.0, 760.0])
                .with_drag_and_drop(false)
        } else {
            eframe::egui::ViewportBuilder::default()
                .with_title("麻薯 Mochi")
                .with_inner_size([canvas[0] * scale, canvas[1] * scale])
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_drag_and_drop(false)
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

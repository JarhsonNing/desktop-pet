//! Native authoring preview. It renders skins only; no behavior engine or desktop operations.
use desktop_pet::{
    engine::Mood,
    skin::{self, SkinRenderer},
};
use eframe::egui::{self, Color32, Rect, RichText, Vec2, ViewportCommand};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MOODS: [(&str, Mood); 10] = [
    ("idle", Mood::Idle),
    ("happy", Mood::Happy),
    ("grumpy", Mood::Grumpy),
    ("sleep", Mood::Sleep),
    ("dragged", Mood::Dragged),
    ("play", Mood::Play),
    ("chase", Mood::Chase),
    ("hiding", Mood::Hiding),
    ("dizzy", Mood::Dizzy),
    ("walk", Mood::Walk),
];

struct Preview {
    renderer: SkinRenderer,
    output: PathBuf,
    gallery: bool,
    mood: Mood,
    frames: usize,
    fps: f64,
    index: usize,
    pending: bool,
    started: Instant,
    success: Arc<AtomicBool>,
    error: Option<String>,
}

impl eframe::App for Preview {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let events = context.input(|input| input.events.clone());
        for event in events {
            if let egui::Event::Screenshot { image, .. } = event {
                let path = self.output.join(if self.gallery {
                    "gallery.png".to_owned()
                } else {
                    format!("frame-{:04}.png", self.index)
                });
                let bytes: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(Color32::to_srgba_unmultiplied)
                    .collect();
                match image::save_buffer(
                    &path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => {
                        self.index += 1;
                        self.pending = false;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        }
        if self.error.is_some() || self.started.elapsed() > Duration::from_secs(90) {
            eprintln!(
                "Preview capture failed: {}",
                self.error.as_deref().unwrap_or("timeout")
            );
            context.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        let complete = self.index >= if self.gallery { 1 } else { self.frames };
        if complete {
            self.success.store(true, Ordering::Release);
            context.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(context, |ui| {
                if self.gallery {
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(
                            RichText::new(&self.renderer.skin.manifest.name)
                                .size(25.0)
                                .strong(),
                        );
                    });
                    let canvas = Vec2::from(self.renderer.skin.ui.canvas);
                    let cell_size = Vec2::new(246.0, 210.0);
                    for (index, (name, mood)) in MOODS.iter().enumerate() {
                        let min = ui.max_rect().min
                            + Vec2::new(
                                24.0 + (index % 3) as f32 * 254.0,
                                66.0 + (index / 3) as f32 * 220.0,
                            );
                        let cell = Rect::from_min_size(min, cell_size);
                        ui.painter().rect_filled(cell, 14.0, Color32::WHITE);
                        ui.painter().text(
                            min + Vec2::new(16.0, 15.0),
                            egui::Align2::LEFT_TOP,
                            *name,
                            egui::FontId::proportional(15.0),
                            Color32::from_rgb(88, 77, 97),
                        );
                        let scale =
                            ((cell_size.x - 20.0) / canvas.x).min((cell_size.y - 38.0) / canvas.y);
                        let origin = min + Vec2::new((cell_size.x - canvas.x * scale) / 2.0, 34.0);
                        self.renderer.paint(
                            ui.painter(),
                            origin,
                            scale,
                            *mood,
                            0.75,
                            Vec2::new(0.15, -0.1),
                        );
                    }
                } else {
                    self.renderer.paint(
                        ui.painter(),
                        ui.max_rect().min,
                        1.0,
                        self.mood,
                        self.index as f64 / self.fps,
                        Vec2::new(0.15, -0.1),
                    );
                }
            });
        if !self.pending && self.started.elapsed() > Duration::from_millis(250) {
            self.pending = true;
            context.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        context.request_repaint_after(Duration::from_secs_f64(1.0 / self.fps));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(Color32::from_rgb(246, 244, 240)).to_array()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| -> Result<&str, Box<dyn std::error::Error>> {
        args.iter()
            .position(|argument| argument == name)
            .and_then(|index| args.get(index + 1))
            .map(String::as_str)
            .ok_or_else(|| format!("Missing {name}").into())
    };
    let skin = skin::load(std::path::Path::new(value("--skin")?))?;
    let output = PathBuf::from(value("--output")?);
    std::fs::create_dir_all(&output)?;
    let gallery = args.iter().any(|argument| argument == "--gallery");
    let mood = if gallery {
        Mood::Idle
    } else {
        MOODS
            .iter()
            .find(|(name, _)| *name == value("--mood").unwrap_or("happy"))
            .ok_or("Unknown mood")?
            .1
    };
    let fps: f64 = if let Ok(value) = value("--fps") {
        value.parse()?
    } else {
        12.0
    };
    let frames: usize = if let Ok(value) = value("--frames") {
        value.parse()?
    } else {
        36
    };
    if !fps.is_finite() || !(1.0..=30.0).contains(&fps) || !(1..=120).contains(&frames) {
        return Err("Preview requires 1–30 FPS and 1–120 frames".into());
    }
    let size = if gallery {
        [800.0, 958.0]
    } else {
        skin.ui.canvas
    };
    let success = Arc::new(AtomicBool::new(false));
    let finished = success.clone();
    eframe::run_native(
        "Mochi Skin Preview",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(size)
                .with_resizable(false)
                .with_active(false),
            renderer: eframe::Renderer::Glow,
            vsync: false,
            run_and_return: true,
            ..Default::default()
        },
        Box::new(move |creation| {
            install_fonts(&creation.egui_ctx);
            Ok(Box::new(Preview {
                renderer: SkinRenderer::new(skin, &creation.egui_ctx),
                output,
                gallery,
                mood,
                frames,
                fps,
                index: 0,
                pending: false,
                started: Instant::now(),
                success,
                error: None,
            }))
        }),
    )?;
    if !finished.load(Ordering::Acquire) {
        return Err("Native preview did not complete; inspect graphics/capture diagnostics".into());
    }
    Ok(())
}

fn install_fonts(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "mochi".into(),
        egui::FontData::from_static(include_bytes!("../assets/mochi-chinese.ttf")).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "mochi".into());
    context.set_fonts(fonts);
    context.set_visuals(egui::Visuals::light());
}

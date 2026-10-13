use crate::{
    Options, art,
    shell::{Shell, ShellAction},
};
use desktop_pet::{
    config::{self, MischiefLevel},
    engine::{Effect, Engine, Interaction, Mood},
    platform, safety,
};
use eframe::egui::{
    self, Color32, Pos2, Rect, RichText, Stroke, Vec2, ViewportCommand, ViewportId,
};
use std::{
    fs::File,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

enum DesktopCommand {
    Hide(u64),
    Restore,
    Stop,
}
struct DesktopWorker {
    sender: Sender<DesktopCommand>,
    receiver: Receiver<Result<String, String>>,
    handle: Option<JoinHandle<()>>,
}

impl DesktopWorker {
    fn new(directory: PathBuf, context: egui::Context) -> Self {
        let (sender, incoming) = mpsc::channel();
        let (outgoing, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let mut guards = Vec::new();
            loop {
                guards.retain_mut(|child: &mut std::process::Child| {
                    child.try_wait().ok().flatten().is_none()
                });
                let command = match incoming.recv_timeout(Duration::from_secs(1)) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => DesktopCommand::Stop,
                };
                let result = match command {
                    DesktopCommand::Hide(seconds) => {
                        safety::hide(&directory, seconds).map(|guard| {
                            guards.push(guard);
                            "图标暂时藏好，恢复守护进程已启动。".to_owned()
                        })
                    }
                    DesktopCommand::Restore | DesktopCommand::Stop => {
                        let mut result = safety::restore(&directory);
                        for _ in 0..10 {
                            if result.is_ok() {
                                break;
                            }
                            thread::sleep(Duration::from_millis(200));
                            result = safety::restore(&directory);
                        }
                        result.map(|_| "桌面已归还。".to_owned())
                    }
                };
                let _ = outgoing.send(result.map_err(|error| error.to_string()));
                context.request_repaint_of(ViewportId::ROOT);
                if matches!(command, DesktopCommand::Stop) {
                    break;
                }
            }
        });
        Self {
            sender,
            receiver,
            handle: Some(handle),
        }
    }
    fn stop(&mut self) {
        let _ = self.sender.send(DesktopCommand::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DesktopWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct PetApp {
    engine: Engine,
    directory: PathBuf,
    _instance: File,
    desktop: DesktopWorker,
    shell: Option<Shell>,
    shell_initialized: bool,
    settings: bool,
    tab: usize,
    confirm_icons: bool,
    acknowledge_icons: bool,
    status: Option<String>,
    options: Options,
    started: Instant,
    previous: f64,
    save_at: Option<f64>,
    position: Pos2,
    move_to: Option<Pos2>,
    home: Pos2,
    positioned: bool,
    passthrough: bool,
    was_chasing: bool,
    screenshot_requested: bool,
    screenshot_done: bool,
    next_cursor_poll: f64,
    cursor: Option<platform::Cursor>,
    work_areas: Vec<[f32; 4]>,
}

impl PetApp {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        config: config::Config,
        directory: PathBuf,
        instance: File,
        status: Option<String>,
        options: Options,
        started: Instant,
    ) -> Self {
        configure_style(&creation.egui_ctx);
        let desktop = DesktopWorker::new(directory.clone(), creation.egui_ctx.clone());
        let settings = options.settings || options.screenshot.is_some() || status.is_some();
        Self {
            engine: Engine::new(config, 0.0, safety::timestamp()),
            directory,
            _instance: instance,
            desktop,
            shell: None,
            shell_initialized: false,
            settings,
            tab: 0,
            confirm_icons: false,
            acknowledge_icons: false,
            status,
            options,
            started,
            previous: 0.0,
            save_at: None,
            position: Pos2::ZERO,
            move_to: None,
            home: Pos2::ZERO,
            positioned: false,
            passthrough: false,
            was_chasing: false,
            screenshot_requested: false,
            screenshot_done: false,
            next_cursor_poll: 0.0,
            cursor: None,
            work_areas: Vec::new(),
        }
    }

    fn action(&mut self, action: Interaction, now: f64) {
        if let Some(effect) = self.engine.interact(action, now) {
            self.effect(effect);
        }
    }

    fn effect(&mut self, effect: Effect) {
        let command = match effect {
            Effect::HideIcons => DesktopCommand::Hide(self.engine.config.icon_hide_seconds),
            Effect::RestoreIcons => DesktopCommand::Restore,
        };
        if self.desktop.sender.send(command).is_err() {
            self.engine.hidden = false;
            self.status = Some("桌面工作线程不可用，捣蛋已停止。".into());
        }
    }

    fn restore(&mut self, now: f64) {
        self.effect(Effect::RestoreIcons);
        self.engine.hidden = false;
        self.action(Interaction::Busy, now);
        self.move_to = Some(self.home);
    }

    fn shell_action(&mut self, context: &egui::Context, action: ShellAction, now: f64) {
        match action {
            ShellAction::Settings => {
                self.settings = true;
                context.send_viewport_cmd_to(
                    ViewportId::from_hash_of("settings"),
                    ViewportCommand::Focus,
                );
            }
            ShellAction::Feed => self.action(Interaction::Feed, now),
            ShellAction::Quiet => {
                self.engine.config.quiet = !self.engine.config.quiet;
                self.engine.dirty = true;
                if self.engine.config.quiet {
                    self.restore(now);
                } else {
                    self.action(Interaction::Wake, now);
                }
            }
            ShellAction::Home => {
                self.move_to = Some(self.home);
                self.action(Interaction::Busy, now);
            }
            ShellAction::Restore => self.restore(now),
            ShellAction::Quit => context.send_viewport_cmd(ViewportCommand::Close),
        }
    }

    fn control_panel(&mut self, context: &egui::Context, now: f64) {
        if !self.settings {
            return;
        }
        if self.options.screenshot.is_some() {
            self.panel_contents(context, now);
            return;
        }
        context.show_viewport_immediate(
            ViewportId::from_hash_of("settings"),
            egui::ViewportBuilder::default()
                .with_title("麻薯 Mochi · 控制面板")
                .with_inner_size([700.0, 720.0])
                .with_min_inner_size([600.0, 560.0]),
            |context, _class| self.panel_contents(context, now),
        );
    }

    fn panel_contents(&mut self, context: &egui::Context, now: f64) {
        if context.input(|input| input.viewport().close_requested()) {
            self.settings = false;
        }
        egui::TopBottomPanel::bottom("recovery-footer")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(245, 245, 240))
                    .inner_margin(16.0),
            )
            .show(context, |ui| {
                if self.engine.hidden {
                    let remaining = (self.engine.effect_until - now).max(0.0).ceil() as u64;
                    ui.colored_label(
                        art::BLUE,
                        format!("图标暂时隐藏 · 最多 {remaining} 秒后自动恢复"),
                    );
                }
                if let Some(status) = &self.status {
                    ui.label(
                        RichText::new(status)
                            .small()
                            .color(Color32::from_rgb(149, 92, 70)),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("立即恢复桌面 / 停止捣蛋").clicked() {
                        self.restore(now);
                    }
                    if ui.small_button("退出麻薯").clicked() {
                        context.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
                    }
                });
                ui.label(
                    RichText::new("Ctrl+Alt+Shift+R 恢复 · Ctrl+Alt+Shift+M 面板")
                        .small()
                        .color(Color32::GRAY),
                );
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(245, 245, 240))
                    .inner_margin(24.0),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("MOCHI").size(14.0).strong().color(art::BLUE));
                    ui.label(
                        RichText::new("/ 住在桌面的小麻烦")
                            .size(14.0)
                            .color(Color32::GRAY),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("收起面板").clicked() {
                            self.settings = false;
                        }
                    });
                });
                ui.add_space(16.0);
                ui.label(
                    RichText::new(format!("今天也要和{}好好相处。", self.engine.config.name))
                        .size(25.0)
                        .strong()
                        .color(art::INK),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("有点黏人，有点戏精，但知道什么时候该安静。")
                        .color(Color32::from_rgb(114, 117, 134)),
                );
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    for (index, label) in ["一起玩", "性格与边界", "小日记"].into_iter().enumerate()
                    {
                        if ui
                            .selectable_label(self.tab == index, RichText::new(label).size(15.0))
                            .clicked()
                        {
                            self.tab = index;
                        }
                    }
                });
                ui.add_space(14.0);
                egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                    0 => self.home_panel(ui, now),
                    1 => self.preferences(ui, now),
                    _ => self.journal(ui),
                });
            });
        if self.confirm_icons {
            self.consent_window(context);
        }
        if self.options.screenshot.is_some() && now > 2.0 && !self.screenshot_requested {
            self.screenshot_requested = true;
            context.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        for event in context.input(|input| input.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event
                && let Some(path) = &self.options.screenshot
            {
                let bytes: Vec<u8> = image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_array())
                    .collect();
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match image::save_buffer(
                    path,
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => {
                        self.screenshot_done = true;
                        self.options
                            .capture_ok
                            .store(true, std::sync::atomic::Ordering::Release);
                        eprintln!("Screenshot saved: {}", path.display());
                    }
                    Err(error) => self.status = Some(format!("截图失败：{error}")),
                }
            }
        }
    }

    fn home_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(252.0, 228.0), egui::Sense::hover());
                art::pet(
                    ui.painter(),
                    rect.min - Vec2::new(0.0, 13.0),
                    0.95,
                    self.engine.mood,
                    now,
                    Vec2::ZERO,
                );
                ui.vertical(|ui| {
                    ui.add_space(25.0);
                    ui.label(RichText::new(&self.engine.config.name).size(23.0).strong());
                    let mood = match self.engine.mood {
                        Mood::Sleep => "正在闭眼思考",
                        Mood::Grumpy => "有一点小脾气",
                        Mood::Hiding => "在策划大事",
                        Mood::Chase => "正在争宠",
                        _ => "今天也元气满满",
                    };
                    ui.colored_label(art::BLUE, mood);
                    ui.add_space(14.0);
                    ui.label(format!("亲密度  {} / 100", self.engine.config.affinity));
                    ui.add(
                        egui::ProgressBar::new(self.engine.config.affinity as f32 / 100.0)
                            .desired_width(220.0)
                            .fill(art::BLUE),
                    );
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!(
                            "摸摸与互动 {} 次 · 小饼干 {} 颗",
                            self.engine.config.interactions, self.engine.config.feeds
                        ))
                        .small()
                        .color(Color32::GRAY),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("左键摸摸 · 双击戳戳\n拖动拎起来 · 右键打开面板").small(),
                    );
                });
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("给生活加一点小插曲").strong().size(16.0));
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                for (label, action) in [
                    ("摸摸头", Interaction::Pet),
                    ("投喂饼干", Interaction::Feed),
                    ("玩小球", Interaction::Play),
                    ("我在忙 · 30 分钟", Interaction::Busy),
                    ("叫醒它", Interaction::Wake),
                ] {
                    if ui
                        .add_sized([108.0, 34.0], egui::Button::new(label))
                        .clicked()
                    {
                        self.action(action, now);
                    }
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.engine.config.allow_cursor_chase && !self.engine.quiet(now),
                        egui::Button::new("试试光标追逐"),
                    )
                    .clicked()
                {
                    self.engine.say("别看那里，看我！", Mood::Chase, now, 10.0);
                }
                if ui
                    .add_enabled(
                        self.engine.config.allow_hide_icons
                            && self.engine.config.level == MischiefLevel::Devil
                            && !self.engine.hidden
                            && !self.engine.quiet(now),
                        egui::Button::new("试试藏桌面图标"),
                    )
                    .clicked()
                {
                    self.engine.begin_hide(now);
                    self.effect(Effect::HideIcons);
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new("高级捣蛋默认关闭。请先在「性格与边界」里逐项开启。")
                    .small()
                    .color(Color32::GRAY),
            );
        });
    }

    fn preferences(&mut self, ui: &mut egui::Ui, now: f64) {
        let mut changed = false;
        card(ui, |ui| {
            ui.label(RichText::new("它的个性，由你决定").strong().size(16.0));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label("昵称");
                changed |= ui
                    .text_edit_singleline(&mut self.engine.config.name)
                    .changed();
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                for (level, label) in [
                    (MischiefLevel::Calm, "乖巧"),
                    (MischiefLevel::Playful, "调皮"),
                    (MischiefLevel::Devil, "小恶魔"),
                ] {
                    changed |= ui
                        .selectable_value(&mut self.engine.config.level, level, label)
                        .changed();
                }
            });
            ui.add_space(8.0);
            changed |= ui
                .checkbox(&mut self.engine.config.quiet, "安静陪伴 · 不主动捣蛋")
                .changed();
            changed |= ui
                .checkbox(&mut self.engine.config.roaming, "允许在当前屏幕工作区散步")
                .changed();
            changed |= ui
                .checkbox(
                    &mut self.engine.config.low_power,
                    "省电模式 · 活动动画限制为 4 FPS",
                )
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut self.engine.config.scale, 0.65..=1.5).text("宠物大小"))
                .changed();
            let mut minutes = self.engine.config.neglect_seconds / 60;
            if ui
                .add(egui::Slider::new(&mut minutes, 1..=120).text("多久没互动开始争宠（分钟）"))
                .changed()
            {
                self.engine.config.neglect_seconds = minutes * 60;
                changed = true;
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(RichText::new("捣蛋契约").strong().size(16.0));
            ui.add_space(8.0);
            changed |= ui
                .checkbox(
                    &mut self.engine.config.allow_cursor_chase,
                    "允许光标追逐与视觉遮挡（10 秒）",
                )
                .changed();
            ui.label(
                RichText::new("不移动你的鼠标，不拦截点击，不抢键盘焦点。")
                    .small()
                    .color(Color32::GRAY),
            );
            ui.add_space(12.0);
            let enabled = self.engine.config.allow_hide_icons;
            ui.horizontal(|ui| {
                ui.label(if enabled {
                    "桌面图标捉迷藏：已授权"
                } else {
                    "桌面图标捉迷藏：未授权"
                });
                if enabled {
                    if ui.button("撤销授权").clicked() {
                        self.engine.config.allow_hide_icons = false;
                        changed = true;
                        self.restore(now);
                    }
                } else if ui
                    .add_enabled(
                        platform::supported() && self.shell.is_some(),
                        egui::Button::new("查看风险并开启"),
                    )
                    .clicked()
                {
                    self.confirm_icons = true;
                    self.acknowledge_icons = false;
                }
            });
            ui.label(
                RichText::new("仅「小恶魔」模式触发；不删除、移动或修改任何桌面文件。")
                    .small()
                    .color(Color32::GRAY),
            );
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.engine.config.icon_hide_seconds, 5..=30)
                        .text("自动归还（秒）"),
                )
                .changed();
            ui.add_space(8.0);
            ui.label(
                RichText::new("恢复守护进程、托盘菜单、恢复快捷键和下次启动恢复共同兜底。")
                    .small()
                    .color(Color32::GRAY),
            );
        });
        if changed {
            self.engine.config.sanitize();
            self.engine.dirty = true;
            self.engine.last_interaction = now;
            self.engine.stage = 0;
        }
    }

    fn consent_window(&mut self, context: &egui::Context) {
        egui::Window::new("开启桌面图标捉迷藏？").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).default_width(440.0).show(context, |ui| {
            ui.label("这是会改变系统桌面可见性的功能，不只是宠物动画。");
            ui.add_space(10.0);
            if cfg!(target_os = "macos") {
                ui.colored_label(Color32::from_rgb(175, 93, 62), "macOS：临时修改 Finder 的 CreateDesktop 设置，并重启 Finder。隐藏和恢复时 Finder 窗口可能闪烁，正在进行的 Finder 操作可能受影响。");
            } else {
                ui.label("Windows：暂时隐藏 Explorer 的桌面图标窗口；不会修改桌面文件。");
            }
            ui.add_space(10.0);
            ui.label("最长 30 秒自动恢复。随时可以通过托盘或 Ctrl+Alt+Shift+R 停止；宠物异常退出时，独立守护进程尝试恢复。若所有进程都被终止，可重启应用或运行 --restore-desktop。");
            ui.add_space(10.0);
            ui.checkbox(&mut self.acknowledge_icons, "我了解系统影响，并希望开启这个功能");
            ui.horizontal(|ui| {
                if ui.add_enabled(self.acknowledge_icons, egui::Button::new("我了解，开启")).clicked() {
                    self.engine.config.allow_hide_icons = true; self.engine.config.hide_icons_consent_version = 1;
                    self.engine.dirty = true; self.confirm_icons = false;
                }
                if ui.button("先不要").clicked() { self.confirm_icons = false; }
            });
        });
    }

    fn journal(&self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("今天的桌面小事").size(16.0).strong());
            ui.label(
                RichText::new("最近 30 条，本次会话的小记忆。")
                    .small()
                    .color(Color32::GRAY),
            );
            ui.add_space(14.0);
            for text in self.engine.history.iter().rev() {
                ui.horizontal(|ui| {
                    ui.colored_label(art::BLUE, "·");
                    ui.label(text);
                });
                ui.add_space(6.0);
            }
        });
        ui.add_space(12.0);
        ui.label(
            RichText::new("完全本地运行 · 不截图读取屏幕 · 不上传数据 · 不需要账户")
                .small()
                .color(Color32::GRAY),
        );
        ui.label(
            RichText::new("Mochi 0.1.0 / Rust + egui / Windows & macOS")
                .small()
                .color(Color32::GRAY),
        );
    }

    fn draw_pet(&mut self, context: &egui::Context, now: f64) {
        let scale = self.engine.config.scale;
        let gaze = self
            .cursor
            .map(|cursor| {
                (Pos2::new(cursor.position[0], cursor.position[1])
                    - self.position
                    - Vec2::new(130.0, 160.0) * scale)
                    / 130.0
            })
            .unwrap_or(Vec2::ZERO);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(context, |ui| {
                let rect = ui.max_rect();
                art::pet(ui.painter(), rect.min, scale, self.engine.mood, now, gaze);
                let pet_rect = Rect::from_min_max(
                    rect.min + Vec2::new(54.0, 90.0) * scale,
                    rect.min + Vec2::new(210.0, 226.0) * scale,
                );
                let response =
                    ui.interact(pet_rect, ui.id().with("pet"), egui::Sense::click_and_drag());
                if response.secondary_clicked() {
                    self.settings = true;
                }
                if response.double_clicked() {
                    self.action(Interaction::Poke, now);
                } else if response.clicked() {
                    self.action(Interaction::Pet, now);
                }
                if response.drag_started() {
                    self.action(Interaction::Drag, now);
                    context.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if self.engine.mood == Mood::Dragged
                    && !context.input(|input| input.pointer.primary_down())
                {
                    self.home = self.position;
                    self.action(Interaction::Release, now);
                }
                if let Some(text) = self.engine.bubble {
                    let bubble_rect = Rect::from_min_size(
                        rect.min + Vec2::new(12.0, 12.0) * scale,
                        Vec2::new(236.0, if self.engine.hidden { 66.0 } else { 54.0 }) * scale,
                    );
                    ui.painter().rect_filled(
                        bubble_rect,
                        12.0 * scale,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 245),
                    );
                    ui.painter().rect_stroke(
                        bubble_rect,
                        12.0 * scale,
                        Stroke::new(1.0_f32, Color32::from_rgb(225, 226, 233)),
                        egui::StrokeKind::Inside,
                    );
                    let galley = ui.painter().layout(
                        text.to_owned(),
                        egui::FontId::proportional(12.0 * scale),
                        art::INK,
                        216.0 * scale,
                    );
                    ui.painter().galley(
                        bubble_rect.min + Vec2::new(10.0, 9.0) * scale,
                        galley,
                        art::INK,
                    );
                    if self.engine.hidden {
                        let button_rect = Rect::from_min_size(
                            bubble_rect.min + Vec2::new(60.0, 38.0) * scale,
                            Vec2::new(116.0, 23.0) * scale,
                        );
                        if ui
                            .put(
                                button_rect,
                                egui::Button::new(format!(
                                    "立即归还 · {}s",
                                    (self.engine.effect_until - now).max(0.0).ceil() as u64
                                )),
                            )
                            .clicked()
                        {
                            self.restore(now);
                        }
                    }
                }
            });
    }

    fn save(&mut self) {
        if let Err(error) =
            config::atomic_json(&self.directory.join("config.json"), &self.engine.config)
        {
            self.status = Some(format!("保存设置失败：{error}"));
        }
    }
}

impl eframe::App for PetApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let now = self.started.elapsed().as_secs_f64();
        if !self.shell_initialized {
            self.shell_initialized = true;
            match Shell::new(context.clone()) {
                Ok(shell) => self.shell = Some(shell),
                Err(error) => {
                    self.engine.config.allow_hide_icons = false;
                    self.settings = true;
                    self.status = Some(format!("托盘或快捷键不可用，高级捣蛋已禁用：{error}"));
                }
            }
        }
        let actions: Vec<ShellAction> = self
            .shell
            .as_ref()
            .map(|shell| shell.receiver.try_iter().collect())
            .unwrap_or_default();
        for action in actions {
            self.shell_action(context, action, now);
        }
        for result in self.desktop.receiver.try_iter() {
            match result {
                Ok(message) => self.status = Some(message),
                Err(error) => {
                    self.engine.hidden = false;
                    self.engine.config.allow_hide_icons = false;
                    self.engine.dirty = true;
                    self.status = Some(format!("捣蛋已停止：{error}"));
                }
            }
        }
        if let Some(effect) = self.engine.tick(now) {
            self.effect(effect);
        }
        let scale = self.engine.config.scale;
        if now >= self.next_cursor_poll {
            let native_scale =
                context.input(|input| input.viewport().native_pixels_per_point.unwrap_or(1.0));
            self.cursor = platform::cursor(native_scale);
            self.work_areas = platform::work_areas(native_scale);
            self.next_cursor_poll = now
                + if self.engine.mood == Mood::Chase {
                    1.0 / 12.0
                } else {
                    0.2
                };
        }
        if !self.positioned {
            let bounds = self
                .cursor
                .map(|cursor| cursor.work_area)
                .unwrap_or([0.0, 0.0, 1280.0, 800.0]);
            self.home = Pos2::new(
                bounds[0] + bounds[2] - 280.0 * scale,
                bounds[1] + bounds[3] - 270.0 * scale,
            );
            self.position = self.home;
            self.positioned = true;
        } else if let Some(rect) = context.input(|input| input.viewport().outer_rect) {
            self.position = rect.min;
        }
        if let Some(position) = self.move_to.take() {
            self.position = position;
        }
        let chasing = self.engine.mood == Mood::Chase && !self.engine.quiet(now);
        let delta = (now - self.previous).clamp(0.0, 0.25) as f32;
        if chasing {
            if let Some(cursor) = self.cursor {
                let target = Pos2::new(cursor.position[0], cursor.position[1])
                    - Vec2::new(124.0, 150.0) * scale;
                self.position += art::limit_length(target - self.position, 420.0 * delta);
            }
        } else if self.was_chasing {
            self.position = self.home;
        } else if self.engine.mood == Mood::Walk
            && self.engine.config.roaming
            && !self.engine.quiet(now)
        {
            self.position.x += (if (self.engine.next_antic as u64).is_multiple_of(2) {
                1.0
            } else {
                -1.0
            }) * 36.0
                * delta;
        }
        if let Some(cursor) = self.cursor {
            let bounds = cursor.work_area;
            if chasing || self.engine.mood == Mood::Walk || !self.position.is_finite() {
                self.position.x = self.position.x.clamp(
                    bounds[0],
                    (bounds[0] + bounds[2] - 260.0 * scale).max(bounds[0]),
                );
                self.position.y = self.position.y.clamp(
                    bounds[1],
                    (bounds[1] + bounds[3] - 250.0 * scale).max(bounds[1]),
                );
            }
        }
        self.was_chasing = chasing;
        let pet_bounds = Rect::from_min_size(
            self.position + Vec2::new(55.0, 90.0) * scale,
            Vec2::new(155.0, 136.0) * scale,
        );
        if !self.work_areas.is_empty()
            && !self.work_areas.iter().any(|area| {
                let bounds =
                    Rect::from_min_size(Pos2::new(area[0], area[1]), Vec2::new(area[2], area[3]));
                let overlap = bounds.intersect(pet_bounds);
                overlap.width() >= 24.0 && overlap.height() >= 24.0
            })
        {
            let area = self
                .cursor
                .map(|cursor| cursor.work_area)
                .unwrap_or(self.work_areas[0]);
            self.home = Pos2::new(
                area[0] + (area[2] - 280.0 * scale).max(0.0),
                area[1] + (area[3] - 270.0 * scale).max(0.0),
            );
            self.position = self.home;
        }
        if self.engine.mood != Mood::Dragged && self.options.screenshot.is_none() {
            let outer = context.input(|input| input.viewport().outer_rect);
            if outer.is_none_or(|rect| rect.min.distance(self.position) > 0.5) {
                context.send_viewport_cmd(ViewportCommand::OuterPosition(self.position));
            }
            let size = Vec2::new(260.0, 250.0) * scale;
            if context
                .input(|input| input.viewport().inner_rect)
                .is_none_or(|rect| (rect.size() - size).length() > 0.5)
            {
                context.send_viewport_cmd(ViewportCommand::InnerSize(size));
            }
        }
        let local = self.cursor.map(|cursor| {
            Pos2::new(cursor.position[0], cursor.position[1]) - self.position.to_vec2()
        });
        let over_pet = local.is_none_or(|point| art::hit_test(point, scale));
        let over_recovery = self.engine.hidden
            && local.is_some_and(|point| {
                point.y < 80.0 * scale && point.x > 12.0 * scale && point.x < 248.0 * scale
            });
        let passthrough = self.options.screenshot.is_none()
            && (chasing || !(over_pet || over_recovery) && self.engine.mood != Mood::Dragged);
        if self.passthrough != passthrough {
            self.passthrough = passthrough;
            context.send_viewport_cmd(ViewportCommand::MousePassthrough(passthrough));
        }
        if self.options.screenshot.is_none() {
            self.draw_pet(context, now);
        }
        self.control_panel(context, now);
        if self.engine.dirty {
            self.engine.dirty = false;
            self.save_at = Some(now + 0.75);
        }
        if self.save_at.is_some_and(|deadline| now >= deadline) {
            self.save();
            self.save_at = None;
        }
        if self
            .options
            .exit_after
            .is_some_and(|seconds| now >= seconds)
        {
            if self.options.screenshot.is_some() && !self.screenshot_done {
                eprintln!("Requested screenshot was not captured");
            }
            context.send_viewport_cmd(ViewportCommand::Close);
        }
        self.previous = now;
        let interval = self.engine.frame_interval(now);
        context.request_repaint_after(interval);
    }
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
        self.desktop.stop();
    }
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .corner_radius(16.0)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(230, 231, 235)))
        .inner_margin(18.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn configure_style(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font = include_bytes!("../assets/mochi-chinese.ttf");
    fonts.font_data.insert(
        "mochi-chinese".into(),
        egui::FontData::from_static(font).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "mochi-chinese".into());
    context.set_fonts(fonts);
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(art::INK);
    visuals.selection.bg_fill = art::BLUE;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.inactive.corner_radius = 8.0.into();
    visuals.widgets.hovered.corner_radius = 8.0.into();
    visuals.widgets.active.corner_radius = 8.0.into();
    context.set_visuals(visuals);
    context.style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(10.0, 9.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        style.animation_time = 0.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    });
}

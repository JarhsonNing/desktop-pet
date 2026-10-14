use crate::{
    Options, art,
    hover::{HoverSampler, Probe},
    panel::{self, card},
    shell::{Shell, ShellAction},
};
use desktop_pet::{
    config::{self, MischiefLevel},
    engine::{Effect, Engine, Interaction, Mood},
    platform, safety,
    skin::{self, Skin, SkinRenderer},
};
use eframe::egui::{
    self, Color32, Pos2, Rect, RichText, Stroke, Vec2, ViewportCommand, ViewportId,
};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
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
    handle: Option<JoinHandle<Result<(), String>>>,
    stopping: Arc<AtomicBool>,
}

impl DesktopWorker {
    fn new(directory: PathBuf, context: egui::Context) -> Self {
        Self::with_operations(directory, context, safety::hide, safety::restore)
    }

    fn with_operations(
        directory: PathBuf,
        context: egui::Context,
        mut hide: impl FnMut(&Path, u64) -> std::io::Result<std::process::Child> + Send + 'static,
        mut restore: impl FnMut(&Path) -> std::io::Result<bool> + Send + 'static,
    ) -> Self {
        let (sender, incoming) = mpsc::channel();
        let (outgoing, receiver) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let stop_requested = stopping.clone();
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
                // Pending pranks must not run after shutdown has started.
                let command = if stop_requested.load(Ordering::Acquire) {
                    DesktopCommand::Stop
                } else {
                    command
                };
                let result = match command {
                    DesktopCommand::Hide(seconds) => hide(&directory, seconds).map(|guard| {
                        guards.push(guard);
                        "图标暂时藏好，恢复守护进程已启动。".to_owned()
                    }),
                    DesktopCommand::Restore | DesktopCommand::Stop => {
                        let mut result = restore(&directory);
                        for _ in 0..10 {
                            if result.is_ok() {
                                break;
                            }
                            thread::sleep(Duration::from_millis(200));
                            result = restore(&directory);
                        }
                        result.map(|_| "桌面已归还。".to_owned())
                    }
                };
                let result = result.map_err(|error| error.to_string());
                let _ = outgoing.send(result.clone());
                context.request_repaint_of(ViewportId::ROOT);
                if matches!(command, DesktopCommand::Stop) {
                    return result.map(|_| ());
                }
            }
        });
        Self {
            sender,
            receiver,
            handle: Some(handle),
            stopping,
        }
    }
    fn stop(&mut self) -> Result<(), String> {
        if let Some(handle) = self.handle.take() {
            self.stopping.store(true, Ordering::Release);
            let _ = self.sender.send(DesktopCommand::Stop);
            return handle
                .join()
                .map_err(|_| "桌面恢复工作线程异常退出，恢复日志仍保留。".to_owned())?;
        }
        Ok(())
    }
}

impl Drop for DesktopWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct PetApp {
    engine: Engine,
    content: crate::content_panel::ContentPanel,
    skin_drop_area: Option<Rect>,
    skin: SkinRenderer,
    skin_path: String,
    nickname: String,
    directory: PathBuf,
    _instance: File,
    desktop: DesktopWorker,
    hover: HoverSampler,
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
    exiting: bool,
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
        let hover = HoverSampler::new(creation.egui_ctx.clone());
        let settings = options.settings || options.screenshot.is_some() || status.is_some();
        let skin_path = config
            .skin
            .as_ref()
            .map(|path| skin_path_text(path))
            .unwrap_or_default();
        let skin = SkinRenderer::new(
            options.skin.clone().unwrap_or_else(Skin::builtin),
            &creation.egui_ctx,
        );
        let nickname = config.name.clone();
        let content = crate::content_panel::ContentPanel::new(creation, directory.clone(), &config);
        Self {
            engine: Engine::new(config, 0.0, safety::timestamp()),
            content,
            skin_drop_area: None,
            skin,
            skin_path,
            nickname,
            directory,
            _instance: instance,
            desktop,
            hover,
            shell: None,
            shell_initialized: false,
            settings,
            tab: options.settings_page,
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
            exiting: false,
        }
    }

    fn action(&mut self, action: Interaction, now: f64) {
        if self.exiting {
            return;
        }
        if let Some(effect) = self.engine.interact(action, now) {
            self.effect(effect);
        }
    }

    fn effect(&mut self, effect: Effect) {
        if self.exiting {
            return;
        }
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
            ShellAction::Clipboard => {
                self.settings = true;
                self.tab = 4;
            }
            ShellAction::Transfer => {
                self.settings = true;
                self.tab = 5;
            }
            ShellAction::ToggleClipboard => {
                self.engine.config.clipboard_enabled = !self.engine.config.clipboard_enabled;
                self.engine.dirty = true;
                self.content.configure(&self.engine.config);
            }
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
            ShellAction::Quit => self.request_exit(context, now),
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
                .with_inner_size([1000.0, 760.0])
                .with_drag_and_drop(false)
                .with_min_inner_size([820.0, 620.0]),
            |context, _class| self.panel_contents(context, now),
        );
    }

    fn panel_contents(&mut self, context: &egui::Context, now: f64) {
        self.dropped_skin(context);
        if context.input(|input| input.viewport().close_requested()) {
            self.commit_nickname(now);
            self.settings = false;
        }
        egui::TopBottomPanel::bottom("recovery-footer")
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0_f32, panel::BORDER))
                    .inner_margin(egui::Margin::symmetric(24, 12)),
            )
            .show(context, |ui| {
                if self.engine.hidden {
                    let remaining = (self.engine.effect_until - now).max(0.0).ceil() as u64;
                    ui.colored_label(
                        art::BLUE,
                        format!("桌面图标暂时隐藏 · {remaining} 秒后自动恢复"),
                    );
                }
                if let Some(status) = &self.status {
                    ui.label(
                        RichText::new(status)
                            .size(12.0)
                            .color(Color32::from_rgb(149, 92, 70)),
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_sized([154.0, 36.0], egui::Button::new("恢复桌面 / 停止捣蛋"))
                        .clicked()
                    {
                        self.restore(now);
                    }
                    ui.label(
                        RichText::new("Ctrl+Alt+Shift+R")
                            .size(12.0)
                            .color(panel::MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized([88.0, 36.0], egui::Button::new("退出麻薯"))
                            .clicked()
                        {
                            self.request_exit(context, now);
                        }
                        ui.label(
                            RichText::new("更改保存在本机")
                                .size(12.0)
                                .color(panel::MUTED),
                        );
                    });
                });
            });
        if self.exiting {
            return;
        }
        egui::SidePanel::left("settings-navigation")
            .exact_width(184.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(16, 24)),
            )
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 12.0, art::BLUE);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "M",
                        egui::FontId::proportional(22.0),
                        Color32::WHITE,
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new("MOCHI").size(18.0).strong().color(art::INK));
                        ui.label(RichText::new("你的桌面伙伴").size(11.0).color(panel::MUTED));
                    });
                });
                ui.add_space(36.0);
                ui.label(RichText::new("陪伴控制台").size(11.0).color(panel::MUTED));
                ui.add_space(10.0);
                for (index, label) in [
                    "一起陪伴",
                    "偏好设置",
                    "互动日记",
                    "皮肤外观",
                    "剪切板",
                    "中转站",
                ]
                .into_iter()
                .enumerate()
                {
                    if panel::navigation(ui, index, self.tab == index, label).clicked() {
                        self.commit_nickname(now);
                        self.tab = index;
                    }
                    ui.add_space(5.0);
                }
                ui.add_space((ui.available_height() - 75.0).max(12.0));
                ui.separator();
                ui.add_space(12.0);
                ui.label(RichText::new("完全本地运行").size(12.0).color(art::INK));
                ui.label(
                    RichText::new("安心陪伴，不打扰。\nMochi 0.1.0")
                        .size(11.0)
                        .color(panel::MUTED),
                );
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(panel::BACKGROUND)
                    .inner_margin(26.0),
            )
            .show(context, |ui| {
                let (title, subtitle) = match self.tab {
                    0 => ("让桌面，多一点陪伴", "摸摸头、聊聊天，和它一起度过今天。"),
                    1 => ("偏好设置", "名字、性格与陪伴方式，都由你决定。"),
                    2 => ("互动日记", "收藏今天发生在桌面上的小瞬间。"),
                    3 => ("皮肤外观", "选择它的模样，让陪伴更像你。"),
                    4 => ("剪切板", "复制过的文字、图片与文件，随时找回来。"),
                    _ => ("中转站", "把东西交给桌宠，再带到需要它的地方。"),
                };
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).size(27.0).strong().color(art::INK));
                        ui.label(RichText::new(subtitle).size(13.0).color(panel::MUTED));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized([92.0, 34.0], egui::Button::new("收起面板"))
                            .clicked()
                        {
                            self.commit_nickname(now);
                            self.settings = false;
                        }
                    });
                });
                ui.add_space(22.0);
                egui::ScrollArea::vertical()
                    .id_salt(("settings-content", self.tab))
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        0 => self.home_panel(ui, now),
                        1 => self.preferences(ui, now),
                        2 => self.journal(ui),
                        3 => self.skin_panel(ui, now),
                        page => {
                            let enabled = self.engine.config.clipboard_enabled;
                            self.content.draw(
                                ui,
                                if page == 4 {
                                    desktop_pet::content::Source::Clipboard
                                } else {
                                    desktop_pet::content::Source::Transfer
                                },
                                &mut self.engine.config,
                            );
                            if enabled != self.engine.config.clipboard_enabled {
                                self.engine.dirty = true;
                                self.content.configure(&self.engine.config);
                            }
                        }
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

    fn paint_preview(&self, ui: &egui::Ui, rect: Rect, now: f64) {
        ui.painter().rect_filled(rect, 16.0, panel::SOFT_BLUE);
        let bounds = self.skin.skin.ui.interaction_bounds;
        let size = Vec2::new(bounds[2], bounds[3]);
        let scale = ((rect.width() - 24.0) / size.x).min((rect.height() - 24.0) / size.y);
        self.skin.paint(
            &ui.painter().with_clip_rect(rect),
            rect.center() - (Vec2::new(bounds[0], bounds[1]) + size / 2.0) * scale,
            scale,
            self.engine.mood,
            now,
            Vec2::ZERO,
        );
    }

    fn home_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() * 0.38).min(224.0);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(width, 224.0), egui::Sense::hover());
                self.paint_preview(ui, rect, now);
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width().max(180.0));
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.add_space(4.0);
                    panel::badge(ui, "你的专属伙伴");
                    ui.add_space(8.0);
                    ui.label(RichText::new(&self.engine.config.name).size(28.0).strong());
                    let mood = match self.engine.mood {
                        Mood::Sleep => "闭眼休息中",
                        Mood::Grumpy => "有一点小脾气",
                        Mood::Hiding => "在和你捉迷藏",
                        Mood::Chase => "正在追逐光标",
                        Mood::Dizzy => "有点晕乎乎",
                        Mood::Play => "玩得很开心",
                        Mood::Walk => "正在悠闲散步",
                        _ => "今天也想陪着你",
                    };
                    ui.label(RichText::new(mood).size(14.0).color(panel::MUTED));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("亲密度").size(12.0).color(panel::MUTED));
                        ui.label(
                            RichText::new(format!("{} / 100", self.engine.config.affinity))
                                .size(12.0)
                                .color(art::BLUE),
                        );
                    });
                    ui.scope(|ui| {
                        ui.spacing_mut().interact_size.y = 6.0;
                        ui.add(
                            egui::ProgressBar::new(self.engine.config.affinity as f32 / 100.0)
                                .desired_width(ui.available_width())
                                .desired_height(6.0)
                                .fill(art::BLUE),
                        );
                    });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "互动 {} 次   ·   饼干 {} 颗",
                            self.engine.config.interactions, self.engine.config.feeds
                        ))
                        .size(12.0)
                        .color(panel::MUTED),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("左键摸摸 · 双击戳戳\n拖动拎起来 · 右键打开面板")
                            .size(12.0)
                            .color(panel::MUTED),
                    );
                });
            });
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            panel::heading(ui, "今天想做点什么？", "小小的互动，也会让它开心一整天。");
            let width = (ui.available_width() - 20.0) / 3.0;
            ui.horizontal(|ui| {
                for (label, action) in [
                    ("摸摸头", Interaction::Pet),
                    ("投喂饼干", Interaction::Feed),
                    ("一起玩球", Interaction::Play),
                ] {
                    if ui
                        .add_sized(
                            [width, 44.0],
                            egui::Button::new(label)
                                .fill(panel::SOFT_BLUE)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        self.action(action, now);
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("我在忙 · 安静 30 分钟").clicked() {
                    self.action(Interaction::Busy, now);
                }
                if ui.button("叫醒它").clicked() {
                    self.action(Interaction::Wake, now);
                }
            });
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.engine.config.allow_cursor_chase && !self.engine.quiet(now),
                        egui::Button::new("体验光标追逐"),
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
                        egui::Button::new("体验图标捉迷藏"),
                    )
                    .clicked()
                {
                    self.engine.begin_hide(now);
                    self.effect(Effect::HideIcons);
                }
            });
            ui.label(
                RichText::new("高级互动需要先在偏好设置中逐项授权。")
                    .size(12.0)
                    .color(panel::MUTED),
            );
        });
    }

    fn commit_nickname(&mut self, now: f64) -> bool {
        let name: String = self.nickname.trim().chars().take(16).collect();
        if name.is_empty() {
            self.nickname = self.engine.config.name.clone();
            return false;
        }
        self.nickname = name.clone();
        if name == self.engine.config.name {
            return false;
        }
        self.engine.config.name = name;
        self.engine.dirty = true;
        self.engine.last_interaction = now;
        self.engine.stage = 0;
        true
    }

    fn preferences(&mut self, ui: &mut egui::Ui, now: f64) {
        let mut changed = false;
        card(ui, |ui| {
            panel::heading(ui, "基本信息", "给它一个你喜欢的名字。");
            ui.label(RichText::new("昵称").size(12.0).color(panel::MUTED));
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 102.0).max(120.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.nickname)
                        .id(egui::Id::new("nickname-field"))
                        .font(egui::FontId::proportional(16.0))
                        .desired_width(width)
                        .margin(Vec2::new(12.0, 10.0))
                        .hint_text("例如：曦玥、团团"),
                );
                if response.lost_focus() {
                    changed |= self.commit_nickname(now);
                }
                if ui.add_sized([68.0, 40.0], panel::primary("保存")).clicked() {
                    changed |= self.commit_nickname(now);
                }
            });
            ui.label(
                RichText::new("支持中文 · 最多 16 个字符 · 回车或结束编辑时保存")
                    .size(12.0)
                    .color(panel::MUTED),
            );
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            panel::heading(ui, "性格与陪伴", "选择适合你工作节奏的相处方式。");
            let width = (ui.available_width() - 20.0) / 3.0;
            ui.horizontal(|ui| {
                for (level, label) in [
                    (MischiefLevel::Calm, "乖巧\n安静守候"),
                    (MischiefLevel::Playful, "调皮\n偶尔撒娇"),
                    (MischiefLevel::Devil, "小恶魔\n更多小插曲"),
                ] {
                    let selected = self.engine.config.level == level;
                    if ui
                        .add_sized(
                            [width, 58.0],
                            egui::Button::new(label)
                                .fill(if selected {
                                    panel::SOFT_BLUE
                                } else {
                                    Color32::WHITE
                                })
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    if selected { art::BLUE } else { panel::BORDER },
                                )),
                        )
                        .clicked()
                        && !selected
                    {
                        self.engine.config.level = level;
                        changed = true;
                    }
                }
            });
            ui.add_space(16.0);
            changed |= panel::toggle(
                ui,
                &mut self.engine.config.quiet,
                "安静陪伴",
                "不主动捣蛋，专心陪你工作。",
            );
            changed |= panel::toggle(
                ui,
                &mut self.engine.config.roaming,
                "允许散步",
                "在当前屏幕的工作区内悠闲走动。",
            );
            changed |= panel::toggle(
                ui,
                &mut self.engine.config.low_power,
                "省电模式",
                "活动动画限制为 4 FPS，降低资源占用。",
            );
            ui.add_space(12.0);
            changed |= ui
                .add(egui::Slider::new(&mut self.engine.config.scale, 0.65..=1.5).text("宠物大小"))
                .changed();
            let mut minutes = self.engine.config.neglect_seconds / 60;
            if ui
                .add(
                    egui::Slider::new(&mut minutes, 1..=120)
                        .text("争宠等待")
                        .suffix(" 分钟"),
                )
                .changed()
            {
                self.engine.config.neglect_seconds = minutes * 60;
                changed = true;
            }
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            panel::heading(ui, "互动边界", "每一项高级互动，都需要你的选择。");
            changed |= panel::toggle(
                ui,
                &mut self.engine.config.allow_cursor_chase,
                "光标追逐",
                "最长 10 秒；不移动鼠标，不拦截点击。",
            );
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("桌面图标捉迷藏").size(14.0));
                panel::badge(
                    ui,
                    if self.engine.config.allow_hide_icons {
                        "已授权"
                    } else {
                        "未授权"
                    },
                );
                if self.engine.config.allow_hide_icons {
                    if ui.button("撤销授权").clicked() {
                        self.engine.config.allow_hide_icons = false;
                        changed = true;
                        self.restore(now);
                    }
                } else if ui
                    .add_enabled(
                        platform::supported() && self.shell.is_some(),
                        egui::Button::new("了解影响并开启"),
                    )
                    .clicked()
                {
                    self.confirm_icons = true;
                    self.acknowledge_icons = false;
                }
            });
            ui.label(
                RichText::new("只在小恶魔模式触发；暂时隐藏图标，不修改桌面文件。")
                    .size(12.0)
                    .color(panel::MUTED),
            );
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.engine.config.icon_hide_seconds, 5..=30)
                        .text("自动归还")
                        .suffix(" 秒"),
                )
                .changed();
            ui.add_space(8.0);
            ui.label(
                RichText::new("随时可用底部恢复按钮、托盘或快捷键停止。")
                    .size(12.0)
                    .color(panel::MUTED),
            );
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            panel::heading(
                ui,
                "剪切板与中转",
                "记录复制内容，并管理主动收纳的文件副本。",
            );
            changed |= panel::toggle(
                ui,
                &mut self.engine.config.clipboard_enabled,
                "自动记录剪切板",
                "桌宠运行期间记录；暂停期间不补录。",
            );
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.engine.config.clipboard_days, 1..=3650)
                        .text("历史保留")
                        .suffix(" 天"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.engine.config.clipboard_limit, 10..=100000)
                        .logarithmic(true)
                        .text("非收藏记录上限")
                        .suffix(" 条"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.engine.config.content_quota_gib, 1..=1024)
                        .logarithmic(true)
                        .text("内容库空间")
                        .suffix(" GB"),
                )
                .changed();
            changed |= panel::toggle(
                ui,
                &mut self.engine.config.download_dropped_resources,
                "下载拖入的网络资源",
                "明确的图片或文件自动下载；普通网页链接仅记录。",
            );
            ui.label(
                RichText::new("收藏与中转内容不会自动删除。所有内容保存在本机，不上传。")
                    .small()
                    .color(panel::MUTED),
            );
        });
        if changed {
            self.engine.config.sanitize();
            self.engine.dirty = true;
            self.engine.last_interaction = now;
            self.engine.stage = 0;
            self.content.configure(&self.engine.config);
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
            panel::heading(ui, "今天的桌面小事", "记下本次陪伴中最近的 30 次互动。");
            if self.engine.history.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    panel::badge(ui, "故事从一次摸摸头开始");
                    ui.add_space(14.0);
                    ui.label(RichText::new("还没有新的小记忆").size(18.0).strong());
                    ui.label(
                        RichText::new("去和它打个招呼吧，这里会留下你们的互动。")
                            .color(panel::MUTED),
                    );
                    ui.add_space(46.0);
                });
            }
            for text in self.engine.history.iter().rev() {
                egui::Frame::new()
                    .fill(panel::BACKGROUND)
                    .corner_radius(10.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.colored_label(art::BLUE, "·");
                            ui.label(text);
                        });
                    });
                ui.add_space(4.0);
            }
        });
        ui.add_space(12.0);
        ui.label(
            RichText::new("完全本地运行 · 不截图读取屏幕 · 不上传数据 · 不需要账户")
                .small()
                .color(panel::MUTED),
        );
        ui.label(
            RichText::new("Mochi 0.1.0 / Rust + egui / Windows & macOS")
                .small()
                .color(panel::MUTED),
        );
    }

    fn load_skin(&mut self, context: &egui::Context, path: &Path) {
        let result = path
            .canonicalize()
            .and_then(|path| skin::load(&path).map(|skin| (path, skin)));
        match result {
            Ok((path, skin)) => {
                self.status = Some(format!("皮肤已加载：{}", skin.manifest.name));
                self.skin = SkinRenderer::new(skin, context);
                self.skin_path = skin_path_text(&path);
                self.engine.config.skin = Some(path);
                self.engine.dirty = true;
                self.next_cursor_poll = 0.0;
                context.request_repaint_of(ViewportId::ROOT);
            }
            Err(error) => self.status = Some(format!("皮肤加载失败：{error}")),
        }
    }

    fn dropped_skin(&mut self, context: &egui::Context) {
        let paths = context.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        if !paths.is_empty() {
            self.content
                .receive(vec![desktop_pet::content::Payload::Files(paths)]);
        }
    }

    fn skin_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        let zone = ui.add_sized(
            [ui.available_width(), 42.0],
            egui::Button::new("将皮肤包拖到这里使用 · 其他区域用于中转收纳").fill(panel::SOFT_BLUE),
        );
        self.skin_drop_area = Some(zone.rect);
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() * 0.38).min(224.0);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(width, 224.0), egui::Sense::hover());
                self.paint_preview(ui, rect, now);
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width().max(180.0));
                    ui.add_space(18.0);
                    panel::badge(ui, "当前使用");
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(&self.skin.skin.manifest.name)
                            .size(23.0)
                            .strong(),
                    );
                    if !self.skin.skin.manifest.author.is_empty() {
                        ui.label(
                            RichText::new(format!("作者 · {}", self.skin.skin.manifest.author))
                                .color(panel::MUTED),
                        );
                    }
                    ui.label(
                        RichText::new(format!("版本 {}", self.skin.skin.manifest.version))
                            .size(12.0)
                            .color(panel::MUTED),
                    );
                    ui.add_space(16.0);
                    ui.label(RichText::new("造型、表情和动画").color(panel::MUTED));
                    ui.label(RichText::new("换个样子，继续陪着你。").color(panel::MUTED));
                });
            });
        });
        ui.add_space(16.0);
        card(ui, |ui| {
            panel::heading(
                ui,
                "换一套新皮肤",
                "支持 .mochi、ZIP 皮肤包和皮肤文件夹。也可以直接拖入窗口。",
            );
            ui.label(RichText::new("皮肤路径").size(12.0).color(panel::MUTED));
            ui.add(
                egui::TextEdit::singleline(&mut self.skin_path)
                    .font(egui::TextStyle::Body)
                    .margin(Vec2::new(12.0, 10.0))
                    .desired_width(f32::INFINITY)
                    .hint_text("粘贴皮肤包或文件夹的完整路径"),
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add(panel::primary("加载并使用")).clicked() {
                    let path = PathBuf::from(self.skin_path.trim().trim_matches('"'));
                    self.load_skin(ui.ctx(), &path);
                }
                if ui
                    .add_enabled(
                        self.engine.config.skin.is_some(),
                        egui::Button::new("重新加载"),
                    )
                    .clicked()
                    && let Some(path) = self.engine.config.skin.clone()
                {
                    self.load_skin(ui.ctx(), &path);
                }
                if ui.button("恢复默认皮肤").clicked() {
                    self.skin = SkinRenderer::new(Skin::builtin(), ui.ctx());
                    self.skin_path.clear();
                    self.engine.config.skin = None;
                    self.engine.dirty = true;
                    self.next_cursor_poll = 0.0;
                    self.status = Some("已使用默认皮肤。".into());
                    ui.ctx().request_repaint_of(ViewportId::ROOT);
                }
            });
            ui.label(
                RichText::new("修改皮肤文件后，点击重新加载即可预览。")
                    .size(12.0)
                    .color(panel::MUTED),
            );
        });
    }

    fn utility_rects(&self, now: f64) -> Vec<Rect> {
        let scale = self.engine.config.scale;
        let mut rects = Vec::new();
        if self.content.receiving
            || self
                .content
                .notification
                .as_ref()
                .is_some_and(|n| n.2 > now)
        {
            let r = self.skin.skin.ui.bubble.rect;
            rects.push(Rect::from_min_size(
                Pos2::new(r[0], r[1]) * scale,
                Vec2::new(r[2], r[3]) * scale,
            ));
        }
        if !self.content.snapshot.transfer.is_empty() {
            let canvas = self.skin.skin.ui.canvas;
            rects.push(Rect::from_min_size(
                Pos2::new(canvas[0] - 30.0, canvas[1] - 28.0) * scale,
                Vec2::splat(24.0) * scale,
            ));
        }
        rects
    }
    fn draw_pet(&mut self, context: &egui::Context, now: f64) {
        let scale = self.engine.config.scale;
        let canvas = Vec2::from(self.skin.skin.ui.canvas);
        let gaze_origin = Vec2::from(self.skin.skin.ui.gaze_origin);
        let gaze_distance = self.skin.skin.ui.gaze_distance;
        let utility_rects = self.utility_rects(now);
        let bubble = self.skin.skin.ui.bubble.clone();
        let gaze = self
            .cursor
            .map(|cursor| {
                (Pos2::new(cursor.position[0], cursor.position[1])
                    - self.position
                    - gaze_origin * scale)
                    / gaze_distance
            })
            .unwrap_or(Vec2::ZERO);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(context, |ui| {
                let rect = ui.max_rect();
                self.skin
                    .paint(ui.painter(), rect.min, scale, self.engine.mood, now, gaze);
                let pet_rect = Rect::from_min_size(rect.min, canvas * scale);
                let response =
                    ui.interact(pet_rect, ui.id().with("pet"), egui::Sense::click_and_drag());
                let over_pet = context
                    .input(|input| input.pointer.interact_pos())
                    .is_some_and(|position| {
                        let local = position - rect.min;
                        !utility_rects
                            .iter()
                            .any(|rect| rect.contains(Pos2::new(local.x, local.y)))
                            && !self.content.receiving
                            && self.skin.skin.hit_test(
                                [local.x, local.y],
                                scale,
                                self.engine.mood,
                                now,
                                [gaze.x, gaze.y],
                            )
                    });
                if over_pet && response.secondary_clicked() {
                    self.settings = true;
                }
                if over_pet && response.double_clicked() {
                    self.action(Interaction::Poke, now);
                } else if over_pet && response.clicked() {
                    self.action(Interaction::Pet, now);
                }
                if over_pet && response.drag_started() {
                    self.action(Interaction::Drag, now);
                    context.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if self.engine.mood == Mood::Dragged
                    && !context.input(|input| input.pointer.primary_down())
                {
                    self.home = self.position;
                    self.action(Interaction::Release, now);
                }
                let utility_text = if self.content.receiving {
                    Some("松手即可收纳到中转站".to_owned())
                } else {
                    self.content
                        .notification
                        .as_ref()
                        .filter(|n| n.2 > now)
                        .map(|n| n.1.clone())
                };
                if let Some(text) = utility_text.as_deref().or(self.engine.bubble) {
                    let bubble_rect = Rect::from_min_size(
                        rect.min + Vec2::new(bubble.rect[0], bubble.rect[1]) * scale,
                        Vec2::new(
                            bubble.rect[2],
                            if self.engine.hidden {
                                bubble.hidden_height
                            } else {
                                bubble.rect[3]
                            },
                        ) * scale,
                    );
                    ui.painter().rect_filled(
                        bubble_rect,
                        bubble.radius * scale,
                        self.skin.skin.color(&bubble.fill),
                    );
                    ui.painter().rect_stroke(
                        bubble_rect,
                        bubble.radius * scale,
                        Stroke::new(
                            bubble.stroke.width * scale,
                            self.skin.skin.color(&bubble.stroke.color),
                        ),
                        egui::StrokeKind::Inside,
                    );
                    let galley = ui.painter().layout(
                        text.to_owned(),
                        egui::FontId::proportional(bubble.font_size * scale),
                        self.skin.skin.color(&bubble.text_color),
                        (bubble.rect[2] - bubble.text_inset[0] * 2.0) * scale,
                    );
                    ui.painter().galley(
                        bubble_rect.min + Vec2::from(bubble.text_inset) * scale,
                        galley,
                        self.skin.skin.color(&bubble.text_color),
                    );
                    if utility_text.is_some()
                        && !self.content.receiving
                        && ui
                            .interact(
                                bubble_rect,
                                ui.id().with("content-notification"),
                                egui::Sense::click(),
                            )
                            .clicked()
                    {
                        self.settings = true;
                        self.tab = 5;
                        if let Some((id, _, _)) = &self.content.notification {
                            let id = id.clone();
                            self.content.select(&id);
                        }
                    }
                    if self.engine.hidden {
                        let button_rect = Rect::from_min_size(
                            bubble_rect.min
                                + Vec2::new(bubble.restore_button[0], bubble.restore_button[1])
                                    * scale,
                            Vec2::new(bubble.restore_button[2], bubble.restore_button[3]) * scale,
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
                if !self.content.snapshot.transfer.is_empty() {
                    let badge_rect = Rect::from_min_size(
                        rect.min + Vec2::new(canvas.x - 30.0, canvas.y - 28.0) * scale,
                        Vec2::splat(24.0) * scale,
                    );
                    let count = self.content.snapshot.transfer.len();
                    ui.painter()
                        .circle_filled(badge_rect.center(), 12.0 * scale, art::BLUE);
                    ui.painter().text(
                        badge_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if count > 99 {
                            "99+".into()
                        } else {
                            count.to_string()
                        },
                        egui::FontId::proportional(10.0 * scale),
                        Color32::WHITE,
                    );
                    let response = ui.interact(
                        badge_rect,
                        ui.id().with("transfer-count"),
                        egui::Sense::click(),
                    );
                    if response.clicked() {
                        self.settings = true;
                        self.tab = 5;
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

    fn shutdown(&mut self, now: f64) {
        if self.exiting {
            return;
        }
        self.commit_nickname(now);
        self.exiting = true;
        self.content.bridge.enabled(false);
        self.engine.stop_mischief(now);
        self.move_to = None;
        self.was_chasing = false;
        self.hover.stop();
        // Restore even if the in-memory hidden flag was already cleared by an error.
        // Wait for an in-flight hide to finish and be undone before saving or exiting.
        if let Err(error) = self.desktop.stop() {
            eprintln!("Mochi: 退出时桌面恢复失败：{error}");
            self.status = Some(format!("退出时桌面恢复失败：{error}"));
        }
        // Restore the desktop before joining clipboard or file I/O workers.
        self.content.bridge.stop();
        self.content.service.stop();
        self.save();
    }

    fn request_exit(&mut self, context: &egui::Context, now: f64) {
        self.shutdown(now);
        context.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
    }
}

impl Drop for PetApp {
    fn drop(&mut self) {
        self.shutdown(self.started.elapsed().as_secs_f64());
    }
}

impl eframe::App for PetApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let now = self.started.elapsed().as_secs_f64();
        if self.exiting || context.input(|input| input.viewport().close_requested()) {
            self.request_exit(context, now);
            return;
        }
        if self
            .options
            .exit_after
            .is_some_and(|seconds| now >= seconds)
        {
            if self.options.screenshot.is_some() && !self.screenshot_done {
                eprintln!("Requested screenshot was not captured");
            }
            self.request_exit(context, now);
            return;
        }
        for drop in self.content.poll(context, now) {
            let skin_drop = drop.panel
                && self.tab == 3
                && self.skin_drop_area.is_some_and(|rect| {
                    rect.contains(Pos2::new(drop.position[0], drop.position[1]))
                });
            if skin_drop
                && drop.payloads.len() == 1
                && let desktop_pet::content::Payload::Files(files) = &drop.payloads[0]
                && files.len() == 1
            {
                self.load_skin(context, &files[0]);
            } else {
                self.content.receive(drop.payloads);
            }
        }
        self.dropped_skin(context);
        if !self.shell_initialized && self.options.screenshot.is_none() {
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
            if self.exiting {
                return;
            }
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
        let canvas = self.skin.skin.ui.canvas;
        if self.hover.receiver.try_iter().count() > 0 {
            self.next_cursor_poll = 0.0;
        }
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
                bounds[0] + (bounds[2] - (canvas[0] + 20.0) * scale).max(0.0),
                bounds[1] + (bounds[3] - (canvas[1] + 20.0) * scale).max(0.0),
            );
            self.position = self.home;
            self.positioned = true;
        } else if let Some(rect) = context.input(|input| input.viewport().outer_rect) {
            self.position = rect.min;
        }
        if let Some(position) = self.move_to.take() {
            self.position = position;
        }
        let chasing = self.engine.mood == Mood::Chase
            && !self.engine.quiet(now)
            && !self.content.receiving
            && !self.content.dragging;
        let delta = (now - self.previous).clamp(0.0, 0.25) as f32;
        if self.content.receiving || self.content.dragging {
            // Native drag sessions must keep the receiving pet stationary.
        } else if chasing {
            if let Some(cursor) = self.cursor {
                let target = Pos2::new(cursor.position[0], cursor.position[1])
                    - Vec2::from(self.skin.skin.ui.chase_anchor) * scale;
                self.position += art::limit_length(target - self.position, 420.0 * delta);
            }
        } else if self.was_chasing {
            self.position = self.home;
        } else if self.engine.mood == Mood::Walk
            && self.engine.config.roaming
            && !self.engine.quiet(now)
            && !self.content.receiving
            && !self.content.dragging
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
                    (bounds[0] + bounds[2] - canvas[0] * scale).max(bounds[0]),
                );
                self.position.y = self.position.y.clamp(
                    bounds[1],
                    (bounds[1] + bounds[3] - canvas[1] * scale).max(bounds[1]),
                );
            }
        }
        if !self.content.receiving && !self.content.dragging {
            self.was_chasing = chasing;
        }
        let interaction = self.skin.skin.ui.interaction_bounds;
        let pet_bounds = Rect::from_min_size(
            self.position + Vec2::new(interaction[0], interaction[1]) * scale,
            Vec2::new(interaction[2], interaction[3]) * scale,
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
                area[0] + (area[2] - (canvas[0] + 20.0) * scale).max(0.0),
                area[1] + (area[3] - (canvas[1] + 20.0) * scale).max(0.0),
            );
            self.position = self.home;
        }
        if self.engine.mood != Mood::Dragged && self.options.screenshot.is_none() {
            let outer = context.input(|input| input.viewport().outer_rect);
            if outer.is_none_or(|rect| rect.min.distance(self.position) > 0.5) {
                context.send_viewport_cmd(ViewportCommand::OuterPosition(self.position));
            }
            let size = Vec2::from(canvas) * scale;
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
        let gaze = self
            .cursor
            .map(|cursor| {
                (Vec2::from(cursor.position)
                    - self.position.to_vec2()
                    - Vec2::from(self.skin.skin.ui.gaze_origin) * scale)
                    / self.skin.skin.ui.gaze_distance
            })
            .unwrap_or(Vec2::ZERO);
        let over_pet = local.is_none_or(|point| {
            self.skin.skin.hit_test(
                [point.x, point.y],
                scale,
                self.engine.mood,
                now,
                [gaze.x, gaze.y],
            )
        });
        let over_recovery = self.engine.hidden
            && local.is_some_and(|point| self.skin.skin.recovery_hit([point.x, point.y], scale));
        let extras = self.utility_rects(now);
        let over_utility = local.is_some_and(|point| {
            extras
                .iter()
                .any(|rect| rect.contains(Pos2::new(point.x, point.y)))
        });
        let over_drop = crate::native_content::mouse_pressed()
            && self.cursor.is_some_and(|cursor| {
                pet_bounds.contains(Pos2::new(cursor.position[0], cursor.position[1]))
            });
        let passthrough = self.options.screenshot.is_none()
            && !over_drop
            && !over_utility
            && !self.content.receiving
            && (chasing || !(over_pet || over_recovery) && self.engine.mood != Mood::Dragged);
        if self.passthrough != passthrough {
            self.passthrough = passthrough;
            context.send_viewport_cmd(ViewportCommand::MousePassthrough(passthrough));
        }
        self.hover.publish(Probe {
            skin: self.skin.skin.clone(),
            position: self.position,
            scale,
            native_scale: context
                .input(|input| input.viewport().native_pixels_per_point.unwrap_or(1.0)),
            reference_height: self
                .cursor
                .map(|cursor| cursor.reference_height)
                .unwrap_or(0.0),
            enabled: self.options.screenshot.is_none() && self.cursor.is_some(),
            recovery: self.engine.hidden,
            mood: self.engine.mood,
            elapsed: now,
            gaze: [gaze.x, gaze.y],
            extras,
        });
        if self.options.screenshot.is_none() {
            self.draw_pet(context, now);
        }
        self.control_panel(context, now);
        if self.exiting {
            return;
        }
        if self.engine.dirty {
            self.engine.dirty = false;
            self.save_at = Some(now + 0.75);
        }
        if self.save_at.is_some_and(|deadline| now >= deadline) {
            self.save();
            self.save_at = None;
        }
        self.previous = now;
        let interval = self
            .engine
            .frame_interval_for_skin(now, self.skin.skin.animation_interval(self.engine.mood));
        context.request_repaint_after(interval);
    }
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown(self.started.elapsed().as_secs_f64());
    }
}

fn skin_path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(path) = text.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{path}")
    } else {
        text.strip_prefix("\\\\?\\").unwrap_or(&text).to_owned()
    }
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
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("mochi-chinese".into());
    context.set_fonts(fonts);
    let mut visuals = egui::Visuals::light();
    visuals.override_text_color = Some(art::INK);
    visuals.selection.bg_fill = art::BLUE;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.panel_fill = panel::BACKGROUND;
    visuals.window_fill = Color32::WHITE;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.faint_bg_color = panel::BACKGROUND;
    visuals.hyperlink_color = art::BLUE;
    visuals.window_corner_radius = 16.0.into();
    visuals.window_stroke = Stroke::new(1.0_f32, panel::BORDER);
    visuals.widgets.inactive.bg_fill = Color32::WHITE;
    visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, panel::BORDER);
    visuals.widgets.hovered.bg_fill = panel::SOFT_BLUE;
    visuals.widgets.hovered.weak_bg_fill = panel::SOFT_BLUE;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, art::BLUE);
    visuals.widgets.active.bg_fill = panel::SOFT_BLUE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, art::BLUE);
    visuals.widgets.inactive.corner_radius = 10.0.into();
    visuals.widgets.hovered.corner_radius = 10.0.into();
    visuals.widgets.active.corner_radius = 10.0.into();
    // eframe receives the system theme after creation; style both variants so
    // that first input and additional viewports cannot fall back to old widgets.
    context.set_theme(egui::Theme::Light);
    context.all_styles_mut(|style| {
        style.visuals = visuals.clone();
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.spacing.interact_size.y = 36.0;
        style.spacing.slider_width = 240.0;
        style.animation_time = 0.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn chinese_names_render_in_both_font_families_and_ime_commits_text() {
        let context = egui::Context::default();
        configure_style(&context);
        let id = egui::Id::new("nickname-field");
        let mut nickname = String::new();
        let draw = |context: &egui::Context, nickname: &mut String| {
            egui::CentralPanel::default().show(context, |ui| {
                ui.add(
                    egui::TextEdit::singleline(nickname)
                        .id(id)
                        .font(egui::TextStyle::Body),
                );
            });
        };
        let _ = context.run(egui::RawInput::default(), |context| {
            context.memory_mut(|memory| memory.request_focus(id));
            draw(context, &mut nickname);
        });
        let input = egui::RawInput {
            events: vec![
                egui::Event::Ime(egui::ImeEvent::Enabled),
                egui::Event::Ime(egui::ImeEvent::Preedit("xiyue".into())),
                egui::Event::Ime(egui::ImeEvent::Commit("曦玥珩璟團圓𠮷".into())),
            ],
            ..Default::default()
        };
        let _ = context.run(input, |context| {
            draw(context, &mut nickname);
            context.fonts_mut(|fonts| {
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    let font = egui::FontId::new(16.0, family);
                    assert!(fonts.has_glyphs(&font, "曦玥珩璟麒麟團圓龍鳳𠮷"));
                    // A valid cmap alone does not guarantee drawable outlines.
                    let galley =
                        fonts.layout_no_wrap("M曦玥珩璟麒麟團圓龍鳳𠮷".into(), font, art::INK);
                    for row in &galley.rows {
                        assert_eq!(row.visuals.glyph_vertex_range.len(), row.glyphs.len() * 4);
                    }
                }
            });
        });
        assert_eq!(nickname, "曦玥珩璟團圓𠮷");
    }

    #[test]
    fn nickname_draft_can_be_cleared_and_commit_preserves_chinese_characters() {
        let directory = std::env::temp_dir().join(format!(
            "mochi-nickname-{}-{}",
            std::process::id(),
            safety::timestamp()
        ));
        let context = egui::Context::default();
        let mut app = PetApp::new(
            &eframe::CreationContext::_new_kittest(context),
            config::Config {
                name: "曦玥".into(),
                ..Default::default()
            },
            directory.clone(),
            safety::lock(&directory.join("instance.lock")).unwrap(),
            None,
            Options::default(),
            Instant::now(),
        );
        app.nickname.clear();
        assert_eq!(app.engine.config.name, "曦玥");
        assert!(!app.commit_nickname(1.0));
        assert_eq!(app.nickname, "曦玥");
        app.nickname = "珩璟麒麟團圓𠮷".repeat(3);
        assert!(app.commit_nickname(2.0));
        assert_eq!(app.engine.config.name.chars().count(), 16);
        assert!(app.engine.config.name.starts_with("珩璟麒麟團圓𠮷"));
        drop(app);
        assert_eq!(
            config::load(&directory.join("config.json"))
                .unwrap()
                .name
                .chars()
                .count(),
            16
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn skin_reload_updates_ui_preserves_behavior_and_keeps_last_skin_on_error() {
        let directory = std::env::temp_dir().join(format!(
            "mochi-skin-reload-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = directory.join("skin");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            source.join("manifest.json"),
            include_bytes!("../skins/blue/manifest.json"),
        )
        .unwrap();
        std::fs::write(
            source.join("pet.json"),
            include_bytes!("../skins/blue/pet.json"),
        )
        .unwrap();
        let context = egui::Context::default();
        let mut app = PetApp::new(
            &eframe::CreationContext::_new_kittest(context.clone()),
            config::Config::default(),
            directory.clone(),
            safety::lock(&directory.join("instance.lock")).unwrap(),
            None,
            Options::default(),
            Instant::now(),
        );
        app.engine
            .say("接招！这个球是我的了。", Mood::Play, 0.0, 12.0);
        app.load_skin(&context, &source);
        assert_eq!(app.skin.skin.manifest.id, "mochi-blue");
        assert_eq!(app.engine.mood, Mood::Play);
        assert!(!app.engine.config.allow_hide_icons);
        let path = source.join("pet.json");
        let mut ui: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        ui["palette"]["cream"] = "#aabbcc".into();
        ui["canvas"] = serde_json::json!([300, 280]);
        std::fs::write(&path, serde_json::to_vec(&ui).unwrap()).unwrap();
        app.load_skin(&context, &source);
        assert_eq!(
            app.skin.skin.color("cream"),
            Color32::from_rgb(170, 187, 204)
        );
        assert_eq!(app.skin.skin.ui.canvas, [300.0, 280.0]);
        let selected = app.skin.skin.clone();
        ui["canvas"] = serde_json::json!([0, 0]);
        std::fs::write(path, serde_json::to_vec(&ui).unwrap()).unwrap();
        app.load_skin(&context, &source);
        assert!(Arc::ptr_eq(&selected, &app.skin.skin));
        assert!(app.status.as_ref().unwrap().contains("皮肤加载失败"));
        app.shutdown(1.0);
        drop(app);
        assert_eq!(
            config::load(&directory.join("config.json")).unwrap().skin,
            Some(source.canonicalize().unwrap())
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stopping_restores_unconditionally_and_is_idempotent() {
        let restores = Arc::new(AtomicUsize::new(0));
        let restored = restores.clone();
        let mut worker = DesktopWorker::with_operations(
            PathBuf::new(),
            egui::Context::default(),
            |_, _| panic!("shutdown must not hide icons"),
            move |_| {
                restored.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            },
        );
        worker.stop().unwrap();
        worker.stop().unwrap();
        drop(worker);
        assert_eq!(restores.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn shutdown_waits_for_active_hide_skips_pending_hides_and_restores() {
        let visible = Arc::new(AtomicBool::new(true));
        let hidden = visible.clone();
        let restored = visible.clone();
        let hides = Arc::new(AtomicUsize::new(0));
        let hide_count = hides.clone();
        let (resume, paused) = mpsc::channel();
        let (started, waiting) = mpsc::channel();
        let mut worker = DesktopWorker::with_operations(
            PathBuf::new(),
            egui::Context::default(),
            move |_, _| {
                hide_count.fetch_add(1, Ordering::SeqCst);
                hidden.store(false, Ordering::SeqCst);
                started.send(()).unwrap();
                paused.recv_timeout(Duration::from_secs(5)).unwrap();
                Err(std::io::Error::other("simulated partial hide"))
            },
            move |_| {
                restored.store(true, Ordering::SeqCst);
                Ok(true)
            },
        );
        worker.sender.send(DesktopCommand::Hide(15)).unwrap();
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!visible.load(Ordering::SeqCst));
        worker.sender.send(DesktopCommand::Hide(15)).unwrap();
        let stopping = worker.stopping.clone();
        let release = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            resume.send(()).unwrap();
            assert!(stopping.load(Ordering::Acquire));
        });

        worker.stop().unwrap();
        release.join().unwrap();
        assert!(visible.load(Ordering::SeqCst));
        assert_eq!(hides.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_worker_restores_without_an_explicit_stop() {
        let visible = Arc::new(AtomicBool::new(false));
        let restored = visible.clone();
        let worker = DesktopWorker::with_operations(
            PathBuf::new(),
            egui::Context::default(),
            |_, _| panic!("drop must not hide icons"),
            move |_| {
                restored.store(true, Ordering::SeqCst);
                Ok(true)
            },
        );
        drop(worker);
        assert!(visible.load(Ordering::SeqCst));
    }

    #[test]
    fn shutdown_retries_and_reports_restore_failure() {
        let restores = Arc::new(AtomicUsize::new(0));
        let attempted = restores.clone();
        let mut worker = DesktopWorker::with_operations(
            PathBuf::new(),
            egui::Context::default(),
            |_, _| panic!("shutdown must not hide icons"),
            move |_| {
                attempted.fetch_add(1, Ordering::SeqCst);
                Err(std::io::Error::other("desktop unavailable"))
            },
        );
        assert_eq!(worker.stop().unwrap_err(), "desktop unavailable");
        assert!(restores.load(Ordering::SeqCst) > 1);
    }

    #[test]
    fn every_exit_path_restores_before_closing_and_preserves_preferences() {
        use eframe::App;
        for path in ["tray", "panel", "window", "timeout", "on_exit", "drop"] {
            let directory = std::env::temp_dir().join(format!(
                "mochi-exit-{path}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let context = egui::Context::default();
            let config = config::Config {
                level: MischiefLevel::Devil,
                allow_cursor_chase: true,
                allow_hide_icons: true,
                hide_icons_consent_version: 1,
                ..config::Config::default()
            };
            let saved = serde_json::to_value(&config).unwrap();
            let mut app = PetApp::new(
                &eframe::CreationContext::_new_kittest(context.clone()),
                config,
                directory.clone(),
                safety::lock(&directory.join("instance.lock")).unwrap(),
                None,
                Options::default(),
                Instant::now(),
            );
            let visible = Arc::new(AtomicBool::new(false));
            let restored = visible.clone();
            let restores = Arc::new(AtomicUsize::new(0));
            let count = restores.clone();
            app.desktop = DesktopWorker::with_operations(
                directory.clone(),
                context.clone(),
                |_, _| panic!("exit must not start another prank"),
                move |_| {
                    restored.store(true, Ordering::SeqCst);
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(true)
                },
            );
            app.engine.say("别看那里，看我！", Mood::Chase, 0.0, 10.0);
            // Recovery must not depend on this flag matching the actual desktop.
            app.engine.hidden = path == "tray";
            app.was_chasing = true;
            app.move_to = Some(Pos2::new(20.0, 20.0));
            if path == "drop" {
                drop(app);
            } else {
                let mut input = egui::RawInput::default();
                if path == "window" {
                    input
                        .viewports
                        .get_mut(&ViewportId::ROOT)
                        .unwrap()
                        .events
                        .push(egui::ViewportEvent::Close);
                }
                if path == "timeout" {
                    app.options.exit_after = Some(0.0);
                }
                let output = context.run(input, |context| match path {
                    "tray" => app.shell_action(context, ShellAction::Quit, 1.0),
                    "panel" => app.request_exit(context, 1.0),
                    "window" | "timeout" => {
                        app.update(context, &mut eframe::Frame::_new_kittest());
                    }
                    "on_exit" => app.on_exit(None),
                    _ => unreachable!(),
                });
                assert!(visible.load(Ordering::SeqCst), "{path}");
                assert!(app.exiting);
                assert!(!app.engine.hidden);
                assert!(!app.was_chasing);
                assert!(app.move_to.is_none());
                app.action(Interaction::Wake, 2.0);
                assert_eq!(app.engine.mood, Mood::Sleep);
                if path != "on_exit" {
                    assert!(
                        output.viewport_output[&ViewportId::ROOT]
                            .commands
                            .contains(&ViewportCommand::Close),
                        "{path}"
                    );
                }
                app.on_exit(None);
                drop(app);
            }
            assert!(visible.load(Ordering::SeqCst), "{path}");
            assert_eq!(restores.load(Ordering::SeqCst), 1, "{path}");
            assert_eq!(
                serde_json::to_value(config::load(&directory.join("config.json")).unwrap())
                    .unwrap(),
                saved,
                "{path}"
            );
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}

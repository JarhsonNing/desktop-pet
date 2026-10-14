use crate::{
    native_content::{self, Bridge},
    panel,
};
use desktop_pet::{
    config::Config,
    content::{self, Entry, Event, Item, Payload, Service, Snapshot, Source, State},
};
use eframe::egui::{self, RichText, Vec2};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
};

pub struct Drop {
    pub payloads: Vec<Payload>,
    pub panel: bool,
    pub position: [f32; 2],
}
enum Operation {
    Copy,
    Save(PathBuf),
}
struct Export {
    key: String,
    files: Vec<PathBuf>,
    payloads: Vec<Payload>,
}

pub struct ContentPanel {
    pub service: Service,
    pub bridge: Bridge,
    events: native_content::Events,
    pub snapshot: Snapshot,
    pub receiving: bool,
    pub dragging: bool,
    clipboard_lease: bool,
    pub notification: Option<(String, String, f64)>,
    queries: [String; 2],
    filters: [usize; 2],
    favorites: [bool; 2],
    results: [Option<Vec<Entry>>; 2],
    selection: BTreeSet<String>,
    source: Source,
    preview_id: String,
    preview_text: String,
    image: Option<egui::TextureHandle>,
    image_zoom: bool,
    export: Option<Export>,
    operation: Option<Operation>,
    preparing: bool,
    progress: HashMap<String, (String, u64)>,
    access: Option<String>,
    message: Option<String>,
    confirm_clear: bool,
    windows_at: f64,
}
impl ContentPanel {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        directory: PathBuf,
        config: &Config,
    ) -> Self {
        let context = creation.egui_ctx.clone();
        let service = Service::start(directory.clone(), policy(config), move || {
            context.request_repaint_of(egui::ViewportId::ROOT)
        });
        let (bridge, events) = Bridge::new(creation, directory, config.clipboard_enabled);
        bridge.quota(policy(config).quota);
        Self {
            service,
            bridge,
            events,
            snapshot: Snapshot::default(),
            receiving: false,
            dragging: false,
            clipboard_lease: false,
            notification: None,
            queries: Default::default(),
            filters: [0; 2],
            favorites: [false; 2],
            results: [None, None],
            selection: BTreeSet::new(),
            source: Source::Clipboard,
            preview_id: String::new(),
            preview_text: String::new(),
            image: None,
            image_zoom: false,
            export: None,
            operation: None,
            preparing: false,
            progress: HashMap::new(),
            access: None,
            message: None,
            confirm_clear: false,
            windows_at: 0.0,
        }
    }
    pub fn poll(&mut self, context: &egui::Context, now: f64) -> Vec<Drop> {
        if now >= self.windows_at {
            self.bridge.refresh_windows();
            self.windows_at = now + 0.5;
        }
        let mut drops = Vec::new();
        for event in self.events.receiver.try_iter() {
            match event {
                native_content::Event::Clipboard(payloads) => {
                    self.access = None;
                    self.service.import(Source::Clipboard, payloads);
                }
                native_content::Event::Dropped {
                    payloads,
                    panel,
                    position,
                } => drops.push(Drop {
                    payloads,
                    panel,
                    position,
                }),
                native_content::Event::Hover(value) => self.receiving = value,
                native_content::Event::Access(error) => self.access = Some(error),
                native_content::Event::Error(error) => self.message = Some(error),
                native_content::Event::DragFinished => {
                    self.dragging = false;
                    if !self.clipboard_lease {
                        self.service.release_exports();
                    }
                }
                native_content::Event::OwnershipLost => {
                    self.clipboard_lease = false;
                    if !self.dragging {
                        self.service.release_exports();
                    }
                }
                native_content::Event::Receiving { id, label, bytes } => {
                    self.progress.insert(id, (label, bytes));
                }
                native_content::Event::ReceiveFinished(id) => {
                    self.progress.remove(&id);
                }
            }
        }
        let events = self.service.receiver.try_iter().collect::<Vec<_>>();
        for event in events {
            match event {
                Event::Snapshot(snapshot) => {
                    self.snapshot = snapshot;
                    for source in [Source::Clipboard, Source::Transfer] {
                        let index = index(source);
                        if !self.queries[index].is_empty() {
                            self.service.search(source, &self.queries[index]);
                        }
                    }
                }
                Event::Progress { id, label, bytes } => {
                    self.progress.insert(id, (label, bytes));
                }
                Event::Completed { id, source, count } => {
                    self.service.completed(&id);
                    self.progress.remove(&id);
                    if source == Source::Transfer {
                        self.notification = Some((
                            id.clone(),
                            if count > 0 {
                                format!("已收纳 {count} 项内容 · 点击查看")
                            } else {
                                "收纳未完成 · 点击查看原因".into()
                            },
                            now + 8.0,
                        ));
                    }
                }
                Event::Preview { id, text, image } if id == self.preview_id => {
                    self.preview_text = text.unwrap_or_default();
                    self.image = image.map(|(rgba, w, h)| {
                        context.load_texture(
                            format!("content-{id}"),
                            egui::ColorImage::from_rgba_unmultiplied(
                                [w as usize, h as usize],
                                &rgba,
                            ),
                            egui::TextureOptions::LINEAR,
                        )
                    });
                }
                Event::Exported {
                    id,
                    files,
                    payloads,
                } => {
                    if id != self.key() {
                        continue;
                    }
                    self.preparing = false;
                    let export = Export {
                        key: id,
                        files,
                        payloads,
                    };
                    if let Some(operation) = self.operation.take() {
                        self.perform(&export, operation);
                    }
                    self.export = Some(export);
                }
                Event::Search {
                    source,
                    query,
                    entries,
                } => {
                    let i = index(source);
                    if self.queries[i] == query {
                        self.results[i] = Some(entries);
                    }
                }
                Event::Error(error) => {
                    self.message = Some(error);
                    self.preparing = false;
                    self.operation = None;
                }
                Event::Saved { count, errors } => {
                    self.message = Some(if errors.is_empty() {
                        format!("已保存 {count} 项内容。")
                    } else {
                        format!("已保存 {count} 项；{}", errors.join("；"))
                    });
                }
                Event::Finished(id) => {
                    self.service.completed(&id);
                    self.progress.remove(&id);
                }
                _ => {}
            }
        }
        drops
    }
    pub fn configure(&mut self, config: &Config) {
        self.bridge.enabled(config.clipboard_enabled);
        self.bridge.quota(policy(config).quota);
        self.service.policy(policy(config));
    }
    pub fn receive(&mut self, payloads: Vec<Payload>) {
        self.service.import(Source::Transfer, payloads);
    }
    pub fn stop(&mut self) {
        self.bridge.stop();
        self.service.stop();
    }
    pub fn select(&mut self, id: &str) {
        self.source = Source::Transfer;
        self.selection.clear();
        self.selection.insert(id.into());
        self.show_selection();
    }
    fn key(&self) -> String {
        self.selection.iter().cloned().collect::<Vec<_>>().join("|")
    }
    fn show_selection(&mut self) {
        self.operation = None;
        self.image_zoom = false;
        let id = self.selection.iter().next().cloned().unwrap_or_default();
        if id != self.preview_id {
            self.preview_id = id.clone();
            self.preview_text.clear();
            self.image = None;
            if !id.is_empty() {
                self.service.preview(&id);
            }
        }
        if self.export.as_ref().is_none_or(|e| e.key != self.key()) {
            self.export = None;
            let entries = self
                .snapshot
                .clipboard
                .iter()
                .chain(&self.snapshot.transfer)
                .filter(|e| self.selection.contains(&e.id));
            let small=entries.flat_map(|e|e.items.iter()).all(|item| !matches!(item,Item::Files(files) if files.iter().any(|file|file.directory || file.bytes>16*1024*1024)));
            self.preparing = false;
            if !self.selection.is_empty() && small {
                self.preparing = true;
                self.service
                    .export(self.selection.iter().cloned().collect());
            }
        }
    }
    fn request(&mut self, operation: Operation) {
        if let Some(export) = self.export.take() {
            if export.key == self.key() && export.files.iter().all(|path| path.exists()) {
                self.perform(&export, operation);
                self.export = Some(export);
                return;
            }
            self.export = Some(export);
        }
        self.operation = Some(operation);
        if !self.preparing {
            self.preparing = true;
            self.service
                .export(self.selection.iter().cloned().collect());
        }
    }
    fn perform(&mut self, export: &Export, operation: Operation) {
        match operation {
            Operation::Copy => match self.bridge.copy(&export.files, &export.payloads) {
                Ok(()) => {
                    self.clipboard_lease = native_content::clipboard_uses_files(&export.payloads);
                    if self.clipboard_lease {
                        self.service.protect_exports(export.files.clone());
                    }
                    self.message = Some("已复制，可以在其他应用中粘贴。".into());
                }
                Err(e) => self.message = Some(e),
            },
            Operation::Save(directory) => {
                self.service.save_exports(export.files.clone(), directory);
                self.message = Some("正在保存到所选文件夹…".into());
            }
        }
    }
    pub fn draw(&mut self, ui: &mut egui::Ui, source: Source, config: &mut Config) {
        if self.source != source {
            self.source = source;
            self.selection.clear();
            self.preview_id.clear();
            self.preview_text.clear();
            self.image = None;
            self.export = None;
            self.operation = None;
            self.preparing = false;
        }
        let i = index(source);
        panel::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if source == Source::Clipboard {
                    panel::badge(
                        ui,
                        if !config.clipboard_enabled {
                            "已暂停"
                        } else if self.access.is_some() {
                            "访问受限"
                        } else {
                            "正在记录"
                        },
                    );
                    if ui
                        .button(if config.clipboard_enabled {
                            "暂停记录"
                        } else {
                            "开始记录"
                        })
                        .clicked()
                    {
                        config.clipboard_enabled = !config.clipboard_enabled;
                        self.access = None;
                        self.configure(config);
                    }
                    if ui.button("清空历史").clicked() {
                        self.confirm_clear = true;
                    }
                } else {
                    panel::badge(ui, &format!("{} 条记录", self.snapshot.transfer.len()));
                    if ui.button("添加文件").clicked()
                        && let Some(files) = rfd::FileDialog::new().pick_files()
                    {
                        self.receive(vec![Payload::Files(files)]);
                    }
                    if ui.button("添加文件夹").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                    {
                        self.receive(vec![Payload::Files(vec![folder])]);
                    }
                    if ui.button("粘贴当前剪切板").clicked() {
                        match self.bridge.read() {
                            Ok(payloads) if !payloads.is_empty() => self.receive(payloads),
                            Ok(_) => self.message = Some("当前剪切板没有可收纳的内容。".into()),
                            Err(e) => self.message = Some(e),
                        }
                    }
                }
            });
            ui.add_space(8.0);
            if source == Source::Clipboard {
                ui.label(
                    RichText::new(format!(
                        "自动保留 {} 天 / 最近 {} 条 · 收藏记录保留 · 文件仅记录原路径",
                        config.clipboard_days, config.clipboard_limit
                    ))
                    .small()
                    .color(panel::MUTED),
                );
            } else {
                ui.label(
                    RichText::new(format!(
                        "内容库 {} / {} GB · 临时导出 {} · 拖到宠物上即可收纳",
                        content::bytes_label(self.snapshot.bytes),
                        config.content_quota_gib,
                        content::bytes_label(self.snapshot.export_bytes)
                    ))
                    .small()
                    .color(panel::MUTED),
                );
            }
            if let Some(error) = &self.access {
                ui.colored_label(egui::Color32::from_rgb(169, 100, 70), error);
            }
            if let Some(message) = &self.message {
                ui.label(RichText::new(message).small().color(panel::MUTED));
            }
            for (id, (label, bytes)) in &self.progress {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("{label} · {}", content::bytes_label(*bytes)));
                    if ui.small_button("取消").clicked() && !self.bridge.cancel_receive(id) {
                        self.service.cancel(id);
                    }
                });
            }
        });
        ui.add_space(14.0);
        ui.horizontal_wrapped(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.queries[i])
                    .hint_text("搜索文字、链接或文件名")
                    .desired_width(240.0),
            );
            if response.changed() {
                self.results[i] = None;
                if !self.queries[i].is_empty() {
                    self.service.search(source, &self.queries[i]);
                }
            }
            egui::ComboBox::from_id_salt(("content-kind", i))
                .selected_text(
                    ["全部类型", "文本", "链接", "图片", "文件", "混合"][self.filters[i]],
                )
                .show_ui(ui, |ui| {
                    for (k, label) in ["全部类型", "文本", "链接", "图片", "文件", "混合"]
                        .iter()
                        .enumerate()
                    {
                        ui.selectable_value(&mut self.filters[i], k, *label);
                    }
                });
            ui.checkbox(&mut self.favorites[i], "只看收藏");
        });
        ui.add_space(12.0);
        let entries = if self.queries[i].is_empty() {
            if source == Source::Clipboard {
                &self.snapshot.clipboard
            } else {
                &self.snapshot.transfer
            }
        } else {
            self.results[i].as_ref().unwrap_or_else(|| {
                if source == Source::Clipboard {
                    &self.snapshot.clipboard
                } else {
                    &self.snapshot.transfer
                }
            })
        };
        let entries = entries
            .iter()
            .filter(|e| !self.favorites[i] || e.favorite)
            .filter(|e| {
                self.filters[i] == 0
                    || e.kind == ["", "text", "link", "image", "files", "mixed"][self.filters[i]]
            })
            .cloned()
            .collect::<Vec<_>>();
        if self.selection.is_empty()
            && let Some(entry) = entries.first()
        {
            self.selection.insert(entry.id.clone());
            self.show_selection();
        }
        if entries.is_empty() {
            panel::card(ui, |ui| {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(if source == Source::Clipboard {
                            "复制的内容，会在这里留下记录"
                        } else {
                            "把想暂存的东西交给桌宠"
                        })
                        .size(18.0),
                    );
                    ui.label(
                        RichText::new("文本、图片、文件和附件，都可以在这里取回。")
                            .color(panel::MUTED),
                    );
                });
                ui.add_space(24.0);
            });
        } else {
            let list_width = (ui.available_width() * 0.40).clamp(190.0, 290.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(list_width, 400.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(list_width);
                        egui::ScrollArea::vertical()
                            .id_salt(("content-list", i))
                            .max_height(400.0)
                            .show_rows(ui, 72.0, entries.len(), |ui, range| {
                                for entry in &entries[range] {
                                    let selected = self.selection.contains(&entry.id);
                                    let label = format!(
                                        "{}{}\n{} · {}{}",
                                        if entry.favorite { "★ " } else { "" },
                                        entry.title,
                                        kind_label(&entry.kind),
                                        age(entry.timestamp),
                                        match entry.state {
                                            State::Saving => " · 保存中",
                                            State::Partial => " · 部分完成",
                                            State::Failed => " · 失败",
                                            State::Ready => "",
                                        }
                                    );
                                    let response = ui.add_sized(
                                        [list_width, 64.0],
                                        egui::Button::new(RichText::new(label).color(
                                            if selected {
                                                egui::Color32::WHITE
                                            } else {
                                                egui::Color32::from_rgb(39, 48, 72)
                                            },
                                        ))
                                        .selected(selected),
                                    );
                                    if response.clicked() {
                                        if ui.input(|input| {
                                            input.modifiers.command || input.modifiers.shift
                                        }) {
                                            if !self.selection.insert(entry.id.clone()) {
                                                self.selection.remove(&entry.id);
                                            }
                                        } else {
                                            self.selection.clear();
                                            self.selection.insert(entry.id.clone());
                                        }
                                        self.show_selection();
                                    }
                                    response.on_hover_text(&entry.preview);
                                }
                            });
                    },
                );
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    panel::card(ui, |ui| self.details(ui, &entries));
                });
            });
        }
        if self.confirm_clear {
            egui::Window::new("清空剪切板历史？")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label("删除非收藏的历史记录。收藏和中转站内容会保留。");
                    ui.horizontal(|ui| {
                        if ui.button("清空历史").clicked() {
                            self.service.clear_clipboard();
                            self.confirm_clear = false;
                        }
                        if ui.button("取消").clicked() {
                            self.confirm_clear = false;
                        }
                    });
                });
        }
        if self.image_zoom
            && let Some(texture) = &self.image
        {
            egui::Window::new("图片预览")
                .open(&mut self.image_zoom)
                .default_size([620.0, 620.0])
                .vscroll(true)
                .show(ui.ctx(), |ui| {
                    let size = texture.size_vec2();
                    let scale = (ui.available_width() / size.x).min(1.0);
                    ui.add(egui::Image::new(texture).fit_to_exact_size(size * scale));
                });
        }
    }
    fn details(&mut self, ui: &mut egui::Ui, entries: &[Entry]) {
        let Some(entry) = entries
            .iter()
            .find(|e| self.selection.contains(&e.id))
            .cloned()
        else {
            ui.label("选择左侧记录查看详情。按住 Ctrl / Command 可多选。");
            return;
        };
        panel::heading(
            ui,
            if self.selection.len() > 1 {
                "已选择多项内容"
            } else {
                &entry.title
            },
            if entry.kind == "image" {
                &entry.preview
            } else {
                kind_label(&entry.kind)
            },
        );
        let can_take = self
            .selection
            .iter()
            .all(|id| entries.iter().any(|e| &e.id == id && !e.items.is_empty()));
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(can_take, panel::primary("复制")).clicked() {
                self.request(Operation::Copy);
            }
            if ui
                .add_enabled(can_take, egui::Button::new("另存到"))
                .clicked()
                && let Some(folder) = rfd::FileDialog::new().pick_folder()
            {
                self.request(Operation::Save(folder));
            }
            if ui
                .button(if entry.favorite {
                    "取消收藏"
                } else {
                    "收藏"
                })
                .clicked()
            {
                for id in &self.selection {
                    self.service.favorite(id, !entry.favorite);
                }
            }
            if ui.button("删除").clicked() {
                self.service
                    .delete(self.selection.iter().cloned().collect());
                self.selection.clear();
                self.preview_id.clear();
            }
            if self.source == Source::Clipboard
                && ui
                    .add_enabled(can_take, egui::Button::new("收纳到中转站"))
                    .clicked()
            {
                for id in &self.selection {
                    self.service.collect(id);
                }
            }
        });
        ui.add_space(10.0);
        let ready = self
            .export
            .as_ref()
            .is_some_and(|e| e.key == self.key() && e.files.iter().all(|path| path.exists()));
        if can_take && !ready && !self.preparing && ui.button("准备拖出").clicked() {
            self.preparing = true;
            self.service
                .export(self.selection.iter().cloned().collect());
        }
        let response = ui.add_enabled(
            ready,
            egui::Button::new(if self.preparing {
                "正在准备拖出…"
            } else {
                "拖动这里，带到其他应用"
            })
            .sense(egui::Sense::click_and_drag())
            .fill(panel::SOFT_BLUE),
        );
        if response.drag_started()
            && let Some(export) = &self.export
        {
            match self
                .bridge
                .drag(export.files.clone(), export.payloads.clone())
            {
                Ok(()) => {
                    self.dragging = true;
                    self.service.protect_exports(export.files.clone());
                }
                Err(e) => self.message = Some(e),
            }
        }
        for error in &entry.errors {
            ui.colored_label(egui::Color32::from_rgb(169, 100, 70), error);
        }
        if entry.state != State::Ready
            && let Some(url) = &entry.origin
        {
            ui.label(RichText::new(url).small().color(panel::MUTED));
            if ui.button("重试下载").clicked() {
                self.receive(vec![Payload::Remote {
                    url: url.clone(),
                    name: None,
                }]);
            }
        }
        if entry.state == State::Failed {
            ui.label("可以重新拖入或重新收纳来源内容。");
        }
        ui.add_space(12.0);
        if let Some(texture) = &self.image {
            let size = texture.size_vec2();
            let scale = (ui.available_width() / size.x).min(140.0 / size.y).min(1.0);
            if ui
                .add(
                    egui::Image::new(texture)
                        .fit_to_exact_size(size * scale)
                        .sense(egui::Sense::click()),
                )
                .on_hover_text("点击查看大图")
                .clicked()
            {
                self.image_zoom = true;
            }
        }
        if !self.preview_text.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("content-preview")
                .max_height(250.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.preview_text)
                            .font(egui::TextStyle::Body)
                            .desired_width(f32::INFINITY)
                            .interactive(false)
                            .frame(false),
                    );
                });
        }
        if entry
            .items
            .iter()
            .any(|item| matches!(item,Item::Files(files) if files.iter().any(|f|!f.managed)))
        {
            ui.label(
                RichText::new("文件历史使用原路径；收纳后才保存独立副本。")
                    .small()
                    .color(panel::MUTED),
            );
        }
    }
}
impl std::ops::Drop for ContentPanel {
    fn drop(&mut self) {
        self.stop();
    }
}
pub fn policy(config: &Config) -> content::Policy {
    content::Policy {
        days: config.clipboard_days,
        limit: config.clipboard_limit,
        quota: config.content_quota_gib as u64 * 1024 * 1024 * 1024,
        download: config.download_dropped_resources,
    }
}
fn index(source: Source) -> usize {
    usize::from(source == Source::Transfer)
}
fn kind_label(kind: &str) -> &str {
    match kind {
        "text" => "文本",
        "link" => "链接",
        "image" => "图片",
        "files" => "文件",
        "mixed" => "混合内容",
        _ => "内容",
    }
}
fn age(stamp: u64) -> String {
    let seconds = desktop_pet::safety::timestamp().saturating_sub(stamp);
    if seconds < 60 {
        "刚刚".into()
    } else if seconds < 3600 {
        format!("{} 分钟前", seconds / 60)
    } else if seconds < 86400 {
        format!("{} 小时前", seconds / 3600)
    } else {
        format!("{} 天前", seconds / 86400)
    }
}

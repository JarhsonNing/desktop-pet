use crate::content::Payload;
use std::{path::PathBuf, sync::mpsc};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "macos")]
pub use macos::Bridge;
#[cfg(windows)]
pub use windows::Bridge;

#[derive(Debug)]
pub enum Event {
    Clipboard(Vec<Payload>),
    Dropped {
        payloads: Vec<Payload>,
        panel: bool,
        position: [f32; 2],
    },
    Hover(bool),
    Access(String),
    Error(String),
    DragFinished,
    OwnershipLost,
    Receiving {
        id: String,
        label: String,
        bytes: u64,
    },
    ReceiveFinished(String),
}

pub struct Events {
    pub receiver: mpsc::Receiver<Event>,
}

pub fn clipboard_uses_files(payloads: &[Payload]) -> bool {
    payloads.is_empty()
        || payloads.iter().any(|p| matches!(p, Payload::Files(_)))
        || payloads.len() > 1 && !payloads.iter().all(|p| matches!(p, Payload::Text { .. }))
}

pub fn mouse_pressed() -> bool {
    #[cfg(windows)]
    {
        unsafe { ::windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(1) < 0 }
    }
    #[cfg(target_os = "macos")]
    {
        objc2_app_kit::NSEvent::pressedMouseButtons() & 1 != 0
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}

pub fn remote_payload(url: &str, name: Option<String>, image_hint: bool) -> Payload {
    let url = url.trim();
    let extension = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.path().rsplit('.').next().map(str::to_ascii_lowercase));
    if image_hint
        || name.is_some()
        || extension.as_deref().is_some_and(|e| {
            matches!(
                e,
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "bmp"
                    | "pdf"
                    | "zip"
                    | "mp4"
                    | "mp3"
                    | "docx"
                    | "xlsx"
                    | "pptx"
                    | "csv"
            )
        })
    {
        Payload::Remote {
            url: url.into(),
            name,
        }
    } else {
        Payload::text(url)
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub struct Bridge;
#[cfg(not(any(windows, target_os = "macos")))]
impl Bridge {
    pub fn new(_: &eframe::CreationContext<'_>, _: PathBuf, _: bool) -> (Self, Events) {
        let (_, receiver) = mpsc::channel();
        (Self, Events { receiver })
    }
    pub fn new_window(
        _: Option<raw_window_handle::RawWindowHandle>,
        _: PathBuf,
        _: bool,
        _: eframe::egui::Context,
    ) -> (Self, Events) {
        let (_, receiver) = mpsc::channel();
        (Self, Events { receiver })
    }
    pub fn enabled(&self, _: bool) {}
    pub fn quota(&self, _: u64) {}
    pub fn cancel_receive(&self, _: &str) -> bool {
        false
    }
    pub fn refresh_windows(&mut self) {}
    pub fn bind_webview(&mut self) {}
    pub fn copy(&mut self, _: &[PathBuf], _: &[Payload]) -> Result<(), String> {
        Err("此平台尚不支持系统剪切板".into())
    }
    pub fn drag(&mut self, _: Vec<PathBuf>, _: Vec<Payload>) -> Result<(), String> {
        Err("此平台尚不支持系统拖放".into())
    }
    pub fn read(&self) -> Result<Vec<Payload>, String> {
        Err("此平台尚不支持系统剪切板".into())
    }
    pub fn stop(&mut self) {}
}

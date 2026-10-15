use desktop_pet::panel_ipc::{Message, Request};
use eframe::egui;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

pub struct Panel {
    child: Child,
    pub requests: Receiver<Request>,
    sender: Sender<Message>,
    pub closing: bool,
}

impl Panel {
    pub fn launch(
        directory: &Path,
        page: usize,
        binary: Option<&Path>,
        debug_port: Option<u16>,
        screenshot: Option<&Path>,
        context: egui::Context,
    ) -> Result<Self, String> {
        let binary = binary
            .map(Path::to_owned)
            .or_else(|| std::env::var_os("MOCHI_PANEL_BINARY").map(PathBuf::from))
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap_or_default()
                    .with_file_name(if cfg!(windows) {
                        "desktop-pet-panel.exe"
                    } else {
                        "desktop-pet-panel"
                    })
            });
        if !binary.is_file() {
            return Err(format!(
                "Tauri 面板未构建：{}。请运行 npm --prefix panel run build 和 cargo build --workspace。",
                binary.display()
            ));
        }
        let mut command = Command::new(binary);
        command
            .args(["--hosted", "--page", &page.to_string(), "--data-dir"])
            .arg(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(path) = screenshot {
            command.arg("--screenshot").arg(path);
        }
        if let Some(port) = debug_port {
            command.arg("--debug-port").arg(port.to_string());
            command.env(
                "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                format!("--remote-debugging-port={port}"),
            );
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let mut input = child.stdin.take().ok_or("Panel stdin unavailable")?;
        let output = child.stdout.take().ok_or("Panel stdout unavailable")?;
        let (sender, writes) = mpsc::channel::<Message>();
        thread::spawn(move || {
            for message in writes {
                if serde_json::to_writer(&mut input, &message).is_err()
                    || input.write_all(b"\n").is_err()
                    || input.flush().is_err()
                {
                    break;
                }
            }
        });
        let (requests_sender, requests) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Ok(request) = serde_json::from_str::<Request>(&line) {
                    if requests_sender.send(request).is_err() {
                        break;
                    }
                    context.request_repaint_of(egui::ViewportId::ROOT);
                }
            }
            context.request_repaint_of(egui::ViewportId::ROOT);
        });
        Ok(Self {
            child,
            requests,
            sender,
            closing: false,
        })
    }
    pub fn sender(&self) -> Sender<Message> {
        self.sender.clone()
    }
    pub fn send(&self, message: Message) {
        // The pet receives the user's click; authorize its separate panel process
        // to take the foreground before asking that process to show/focus itself.
        #[cfg(windows)]
        if matches!(message.event.as_deref(), Some("focus" | "navigate")) {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                    self.child.id(),
                );
            }
        }
        let _ = self.sender.send(message);
    }
    pub fn exited(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|exit| exit.is_some())
    }
    pub fn close(&mut self) {
        self.send(Message::event("shutdown", serde_json::Value::Null));
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.exited() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if !self.exited() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        self.close();
    }
}

/// Development-only control for lifecycle/performance tests; never enabled at normal startup.
pub fn test_requests(context: egui::Context) -> Receiver<Request> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(std::io::stdin()).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Ok(request) = serde_json::from_str::<Request>(&line) {
                if sender.send(request).is_err() {
                    break;
                }
                context.request_repaint_of(egui::ViewportId::ROOT);
            }
        }
    });
    receiver
}

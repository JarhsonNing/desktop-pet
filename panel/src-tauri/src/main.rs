#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use desktop_pet::{
    content::Payload,
    native_content::{Bridge, Event, Events},
    panel_ipc::{Message, Request},
};
use raw_window_handle::HasWindowHandle;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};
use tokio::sync::oneshot;

type Reply = Result<Value, String>;
struct Connection {
    outgoing: mpsc::Sender<Request>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Reply>>>,
    serial: AtomicU64,
}
impl Connection {
    async fn call(&self, command: &str, args: Value) -> Reply {
        let id = self.serial.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|e| e.to_string())?
            .insert(id, sender);
        if self
            .outgoing
            .send(Request {
                id,
                command: command.into(),
                args,
            })
            .is_err()
        {
            self.pending.lock().map_err(|e| e.to_string())?.remove(&id);
            return Err("桌宠连接已断开".into());
        }
        let result = tokio::time::timeout(Duration::from_secs(600), receiver).await;
        self.pending.lock().map_err(|e| e.to_string())?.remove(&id);
        result
            .map_err(|_| "桌宠响应超时，可取消任务后重试".to_owned())?
            .map_err(|_| "桌宠连接已断开".to_owned())?
    }
    fn notify(&self, command: &str, args: Value) {
        let _ = self.outgoing.send(Request {
            id: 0,
            command: command.into(),
            args,
        });
    }
}
struct Native {
    bridge: Bridge,
    events: Events,
    zone: Option<[f64; 4]>,
}
thread_local! { static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) }; }
#[derive(Clone)]
struct Launch {
    page: usize,
    screenshot: Option<PathBuf>,
}

#[tauri::command]
async fn host(
    command: String,
    args: Value,
    connection: tauri::State<'_, Arc<Connection>>,
    app: tauri::AppHandle,
) -> Reply {
    let value = connection.call(&command, args).await?;
    if command == "snapshot" {
        let quota = value["config"]["content_quota_gib"].as_u64().unwrap_or(5) * 1024 * 1024 * 1024;
        let _ = app.run_on_main_thread(move || {
            NATIVE.with(|native| {
                if let Some(native) = native.borrow().as_ref() {
                    native.bridge.quota(quota);
                }
            })
        });
    }
    Ok(value)
}
#[tauri::command]
fn context(launch: tauri::State<'_, Launch>) -> Value {
    json!({"page":launch.page,"capture":launch.screenshot.is_some()})
}
#[tauri::command]
async fn close_panel(
    app: tauri::AppHandle,
    connection: tauri::State<'_, Arc<Connection>>,
) -> Result<(), String> {
    // Acknowledge close before destroying the WebView. A subsequent open must
    // survive the old process being reaped on the parent's next frame.
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        connection.call("panel_closing", Value::Null),
    )
    .await;
    app.exit(0);
    Ok(())
}
#[tauri::command]
fn skin_zone(rect: Option<[f64; 4]>) {
    NATIVE.with(|native| {
        if let Some(native) = native.borrow_mut().as_mut() {
            native.zone = rect;
        }
    });
}

#[tauri::command]
async fn add_files(folder: bool, connection: tauri::State<'_, Arc<Connection>>) -> Reply {
    let paths = tauri::async_runtime::spawn_blocking(move || {
        if folder {
            rfd::FileDialog::new().pick_folder().map(|p| vec![p])
        } else {
            rfd::FileDialog::new().pick_files()
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(paths) = paths else {
        return Ok(json!(false));
    };
    connection
        .call("import", json!({"payloads":[Payload::Files(paths)]}))
        .await
}
#[tauri::command]
async fn choose_skin(folder: bool) -> Reply {
    let path = tauri::async_runtime::spawn_blocking(move || {
        if folder {
            rfd::FileDialog::new().pick_folder()
        } else {
            rfd::FileDialog::new()
                .add_filter("Mochi 皮肤", &["mochi", "zip"])
                .pick_file()
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!(path))
}
#[tauri::command]
async fn save_content(ids: Vec<String>, connection: tauri::State<'_, Arc<Connection>>) -> Reply {
    let folder = tauri::async_runtime::spawn_blocking(|| rfd::FileDialog::new().pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    let Some(folder) = folder else {
        return Ok(json!(false));
    };
    connection
        .call("save_content", json!({"ids":ids,"directory":folder}))
        .await
}
#[tauri::command]
fn paste_current(connection: tauri::State<'_, Arc<Connection>>) -> Result<(), String> {
    let payloads = NATIVE.with(|native| {
        native
            .borrow()
            .as_ref()
            .ok_or("原生窗口不可用")?
            .bridge
            .read()
    })?;
    if payloads.is_empty() {
        return Err("当前剪切板没有可收纳内容".into());
    }
    connection.notify("import", json!({"payloads":payloads}));
    Ok(())
}
#[tauri::command]
async fn cancel_task(
    id: String,
    connection: tauri::State<'_, Arc<Connection>>,
    app: tauri::AppHandle,
) -> Reply {
    let (sender, receiver) = oneshot::channel();
    let native_id = id.clone();
    app.run_on_main_thread(move || {
        let cancelled = NATIVE.with(|native| {
            native
                .borrow()
                .as_ref()
                .is_some_and(|n| n.bridge.cancel_receive(&native_id))
        });
        let _ = sender.send(cancelled);
    })
    .map_err(|e| e.to_string())?;
    if receiver.await.unwrap_or(false) {
        Ok(json!(true))
    } else {
        connection.call("cancel", json!({"id":id})).await
    }
}
#[tauri::command]
async fn drag_content(
    files: Vec<PathBuf>,
    payloads: Vec<Payload>,
    connection: tauri::State<'_, Arc<Connection>>,
    app: tauri::AppHandle,
) -> Reply {
    connection
        .call("drag_start", json!({"files":files}))
        .await?;
    let (sender, receiver) = oneshot::channel();
    app.run_on_main_thread(move || {
        let result = NATIVE.with(|native| {
            native
                .borrow_mut()
                .as_mut()
                .ok_or("面板拖放尚未就绪".to_owned())?
                .bridge
                .drag(files, payloads)
        });
        let _ = sender.send(result);
    })
    .map_err(|e| e.to_string())?;
    match receiver.await.map_err(|e| e.to_string())? {
        Ok(()) => Ok(json!(true)),
        Err(error) => {
            connection.notify("drag_finish", Value::Null);
            Err(error)
        }
    }
}

fn pump(app: &tauri::AppHandle, connection: &Connection) {
    NATIVE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(native) = state.as_mut() else {
            return;
        };
        for event in native.events.receiver.try_iter() {
            match event {
                Event::Dropped {
                    payloads, position, ..
                } => {
                    let skin = native.zone.is_some_and(|r| {
                        position[0] as f64 >= r[0]
                            && position[1] as f64 >= r[1]
                            && (position[0] as f64) < r[0] + r[2]
                            && (position[1] as f64) < r[1] + r[3]
                    });
                    if skin
                        && let [Payload::Files(paths)] = payloads.as_slice()
                        && paths.len() == 1
                    {
                        let _ = app.emit("skin-drop", json!({"path":paths[0]}));
                    } else {
                        connection.notify("import", json!({"payloads":payloads}));
                    }
                }
                Event::Hover(value) => {
                    connection.notify("drop_hover", json!({"value":value}));
                    let _ = app.emit("drop-hover", value);
                }
                Event::Receiving { id, label, bytes } => {
                    connection.notify("receiving", json!({"id":id,"label":label,"bytes":bytes}))
                }
                Event::ReceiveFinished(id) => {
                    connection.notify("receive_finished", json!({"id":id}))
                }
                Event::DragFinished => {
                    connection.notify("drag_finish", Value::Null);
                    let _ = app.emit("drag-finished", true);
                }
                Event::Error(error) | Event::Access(error) => {
                    let _ = app.emit("native-error", error);
                }
                _ => {}
            }
        }
    });
}

#[tauri::command]
async fn capture(
    app: tauri::AppHandle,
    launch: tauri::State<'_, Launch>,
    connection: tauri::State<'_, Arc<Connection>>,
) -> Reply {
    let path = launch
        .screenshot
        .clone()
        .ok_or("Capture is only available with --screenshot")?;
    capture_window(&app, path).await?;
    connection.call("captured", Value::Null).await
}
#[cfg(windows)]
async fn capture_window(app: &tauri::AppHandle, path: PathBuf) -> Result<(), String> {
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::System::{
        Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream},
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    };
    let window = app.get_webview_window("main").ok_or("No panel window")?;
    let (sender, receiver) = oneshot::channel();
    window
        .with_webview(move |view| {
            let result = (|| -> windows::core::Result<()> {
                let stream = unsafe { CreateStreamOnHGlobal(Default::default(), true)? };
                let saved = stream.clone();
                let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                    let output = (|| -> Result<(), String> {
                        result.map_err(|e| e.to_string())?;
                        let handle =
                            unsafe { GetHGlobalFromStream(&saved) }.map_err(|e| e.to_string())?;
                        let pointer = unsafe { GlobalLock(handle) };
                        if pointer.is_null() {
                            return Err("Capture buffer unavailable".into());
                        }
                        let bytes = unsafe {
                            std::slice::from_raw_parts(pointer.cast::<u8>(), GlobalSize(handle))
                        }
                        .to_vec();
                        let _ = unsafe { GlobalUnlock(handle) };
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                        }
                        std::fs::write(path, bytes).map_err(|e| e.to_string())?;
                        Ok(())
                    })();
                    let _ = sender.send(output);
                    Ok(())
                }));
                unsafe {
                    view.controller().CoreWebView2()?.CapturePreview(
                        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                        &stream,
                        &handler,
                    )?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("Mochi panel capture: {error}");
            }
        })
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(15), receiver)
        .await
        .map_err(|_| "Capture timed out".to_owned())?
        .map_err(|e| e.to_string())?
}
#[cfg(not(windows))]
async fn capture_window(_: &tauri::AppHandle, _: PathBuf) -> Result<(), String> {
    Err("Native Tauri capture is currently supported on Windows".into())
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !args.iter().any(|a| a == "--hosted") {
        eprintln!("请从 Mochi 桌宠打开此面板。 ");
        std::process::exit(2);
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let Some(directory) = value("--data-dir").map(PathBuf::from) else {
        std::process::exit(2);
    };
    let launch = Launch {
        page: value("--page").and_then(|v| v.parse().ok()).unwrap_or(0),
        screenshot: value("--screenshot").map(PathBuf::from),
    };
    let (outgoing, writes) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        let mut output = std::io::BufWriter::new(std::io::stdout());
        for request in writes {
            if serde_json::to_writer(&mut output, &request).is_err()
                || output.write_all(b"\n").is_err()
                || output.flush().is_err()
            {
                break;
            }
        }
    });
    let connection = Arc::new(Connection {
        outgoing,
        pending: Mutex::new(HashMap::new()),
        serial: AtomicU64::new(1),
    });
    let setup_connection = connection.clone();
    let mut tauri_context = tauri::generate_context!();
    if let Some(port) = value("--debug-port").and_then(|v| v.parse::<u16>().ok()) {
        tauri_context.config_mut().app.windows[0].additional_browser_args =
            Some(format!("--remote-debugging-port={port}"));
        tauri_context.config_mut().app.windows[0].devtools = Some(true);
    }
    let app = tauri::Builder::default()
        .manage(connection)
        .manage(launch)
        .invoke_handler(tauri::generate_handler![
            host,
            context,
            close_panel,
            skin_zone,
            add_files,
            choose_skin,
            save_content,
            paste_current,
            cancel_task,
            drag_content,
            capture
        ])
        .setup(move |app| {
            let window = app.get_webview_window("main").ok_or("No panel window")?;
            let (mut bridge, events) = Bridge::new_window(
                window.window_handle().ok().map(|h| h.as_raw()),
                directory,
                false,
                Default::default(),
            );
            bridge.bind_webview();
            NATIVE.with(|native| {
                *native.borrow_mut() = Some(Native {
                    bridge,
                    events,
                    zone: None,
                })
            });
            let reader_app = app.handle().clone();
            let reader = setup_connection.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(std::io::stdin()).lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    let Ok(message) = serde_json::from_str::<Message>(&line) else {
                        continue;
                    };
                    if let Some(id) = message.id {
                        if let Ok(mut pending) = reader.pending.lock()
                            && let Some(sender) = pending.remove(&id)
                        {
                            let _ = sender.send(message.error.map_or(Ok(message.value), Err));
                        }
                    } else if let Some(event) = message.event {
                        if event == "shutdown" {
                            break;
                        }
                        if event == "focus" || event == "navigate" {
                            let focus_app = reader_app.clone();
                            let _ = reader_app.run_on_main_thread(move || {
                                if let Some(window) = focus_app.get_webview_window("main") {
                                    let _ = window.show();
                                    let _ = window.unminimize();
                                    let _ = window.set_focus();
                                }
                            });
                        }
                        let _ = reader_app.emit(&event, message.value);
                    }
                }
                if let Ok(mut pending) = reader.pending.lock() {
                    for (_, sender) in pending.drain() {
                        let _ = sender.send(Err("桌宠已退出".into()));
                    }
                }
                reader_app.exit(0);
            });
            let pump_app = app.handle().clone();
            let pump_connection = setup_connection.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let app = pump_app.clone();
                    let connection = pump_connection.clone();
                    if pump_app
                        .run_on_main_thread(move || pump(&app, &connection))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("close-requested", ());
                // Let the frontend commit any nickname draft before exiting.
                // A broken/unfinished frontend must still allow the window to close.
                let app = window.app_handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    app.exit(0);
                });
            }
        })
        .build(tauri_context)
        .expect("Unable to start Mochi panel");
    app.run(|_, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            NATIVE.with(|native| {
                if let Some(mut native) = native.borrow_mut().take() {
                    native.bridge.stop();
                }
            });
        }
    });
}

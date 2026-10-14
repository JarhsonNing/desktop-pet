#![allow(non_snake_case)]
use super::*;
use ::windows::{
    Win32::{
        Foundation::*,
        System::{
            Com::Marshal::CoMarshalInterThreadInterfaceInStream,
            Com::StructuredStorage::CoGetInterfaceAndReleaseStream, Com::*, DataExchange::*,
            Memory::*, Ole::*, SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{HRESULT, Interface, PCWSTR, Ref, implement, w},
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::{Cursor, Write},
    mem::ManuallyDrop,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};

const DRAG_MESSAGE: u32 = WM_APP + 73;
const SUBCLASS: usize = 0x4d434843;
const MAX_DATA: usize = 256 * 1024 * 1024;

struct Shared {
    sender: mpsc::Sender<Event>,
    context: eframe::egui::Context,
    directory: PathBuf,
    enabled: AtomicBool,
    alive: AtomicBool,
    ignore: AtomicU32,
    baseline: AtomicU32,
    writing: AtomicBool,
    watch_window: AtomicUsize,
    quota: AtomicU64,
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}
impl Shared {
    fn emit(&self, event: Event) {
        let _ = self.sender.send(event);
        self.context
            .request_repaint_of(eframe::egui::ViewportId::ROOT);
    }
}

struct Binding {
    window: HWND,
    shared: Arc<Shared>,
    panel: bool,
    target: IDropTarget,
    alive: Cell<bool>,
    drag: RefCell<Option<(Vec<PathBuf>, Vec<Payload>)>>,
}

impl Binding {
    fn attached(&self) -> bool {
        self.alive.get() && unsafe { IsWindow(Some(self.window)).as_bool() }
    }
}
pub struct Bridge {
    root: HWND,
    bindings: HashMap<usize, Box<Binding>>,
    shared: Arc<Shared>,
    watcher: Option<thread::JoinHandle<()>>,
    ole: bool,
}

impl Bridge {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        directory: PathBuf,
        enabled: bool,
    ) -> (Self, Events) {
        let (sender, receiver) = mpsc::channel();
        let root = match creation.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => HWND(h.hwnd.get() as *mut _),
            _ => HWND::default(),
        };
        let shared = Arc::new(Shared {
            sender,
            context: creation.egui_ctx.clone(),
            directory: directory.join("content/incoming"),
            enabled: AtomicBool::new(enabled),
            alive: AtomicBool::new(true),
            ignore: AtomicU32::new(0),
            baseline: AtomicU32::new(unsafe { GetClipboardSequenceNumber() }),
            writing: AtomicBool::new(false),
            watch_window: AtomicUsize::new(0),
            quota: AtomicU64::new(5 * 1024 * 1024 * 1024),
            jobs: Mutex::new(HashMap::new()),
        });
        let ole = if root.is_invalid() {
            false
        } else {
            unsafe { OleInitialize(None).is_ok() }
        };
        let watcher = if root.is_invalid() {
            None
        } else {
            let state = shared.clone();
            Some(thread::spawn(move || watch(state)))
        };
        let mut bridge = Self {
            root,
            bindings: HashMap::new(),
            shared,
            watcher,
            ole,
        };
        if !root.is_invalid() {
            bridge.bind(root, false);
        }
        (bridge, Events { receiver })
    }
    fn bind(&mut self, window: HWND, panel: bool) {
        if self.bindings.contains_key(&(window.0 as usize)) {
            return;
        }
        let target: IDropTarget = DropTarget {
            shared: self.shared.clone(),
            window: window.0 as usize,
            panel,
            accepted: Cell::new(false),
        }
        .into();
        let binding = Box::new(Binding {
            window,
            shared: self.shared.clone(),
            panel,
            target,
            alive: Cell::new(true),
            drag: RefCell::new(None),
        });
        unsafe {
            // Windows builder disables winit's CF_HDROP-only target. Revoke also
            // handles previously-created secondary viewports without double delivery.
            let _ = RevokeDragDrop(window);
            if let Err(e) = RegisterDragDrop(window, &binding.target) {
                self.shared
                    .emit(Event::Error(format!("原生拖放注册失败：{e}")));
                return;
            }
            if !SetWindowSubclass(
                window,
                Some(window_proc),
                SUBCLASS,
                (&*binding as *const Binding) as usize,
            )
            .as_bool()
            {
                let _ = RevokeDragDrop(window);
                return;
            }
        }
        self.bindings.insert(window.0 as usize, binding);
    }
    pub fn enabled(&self, value: bool) {
        if value != self.shared.enabled.load(Ordering::Acquire) {
            self.shared
                .baseline
                .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
            self.shared.enabled.store(value, Ordering::Release);
        }
    }
    pub fn quota(&self, bytes: u64) {
        self.shared.quota.store(bytes, Ordering::Release);
    }
    pub fn cancel_receive(&self, id: &str) -> bool {
        if let Ok(jobs) = self.shared.jobs.lock()
            && let Some(cancel) = jobs.get(id)
        {
            cancel.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
    pub fn refresh_windows(&mut self) {
        self.bindings.retain(|_, binding| binding.attached());
        let mut candidates = Vec::<usize>::new();
        unsafe {
            let _ = EnumWindows(
                Some(enumerate),
                LPARAM((&mut candidates as *mut Vec<usize>) as isize),
            );
        }
        for candidate in candidates {
            if candidate != self.root.0 as usize {
                self.bind(HWND(candidate as *mut _), true);
            }
        }
    }
    pub fn read(&self) -> Result<Vec<Payload>, String> {
        if self.root.is_invalid() {
            return Err("没有可用的原生剪切板窗口".into());
        }
        read_clipboard().map_err(|e| e.to_string())
    }
    pub fn copy(&mut self, files: &[PathBuf], payloads: &[Payload]) -> Result<(), String> {
        if self.root.is_invalid() {
            return Err("没有可用的原生剪切板窗口".into());
        }
        self.shared.writing.store(true, Ordering::Release);
        let changed = Cell::new(false);
        let result = (|| -> ::windows::core::Result<()> {
            let formats = outgoing(
                if super::clipboard_uses_files(payloads) {
                    files
                } else {
                    &[]
                },
                payloads,
            )?;
            if formats.is_empty() {
                return Err(E_INVALIDARG.into());
            }
            unsafe {
                OpenClipboard(Some(self.root))?;
            }
            let guard = ClipboardGuard;
            unsafe {
                EmptyClipboard()?;
            }
            changed.set(true);
            for (format, data) in formats {
                let global = allocate(&data)?;
                if let Err(e) = unsafe { SetClipboardData(format as u32, Some(HANDLE(global.0))) } {
                    let _ = unsafe { GlobalFree(Some(global)) };
                    return Err(e);
                }
            }
            drop(guard);
            Ok(())
        })();
        if changed.get() {
            self.shared
                .ignore
                .store(unsafe { GetClipboardSequenceNumber() }, Ordering::Release);
        }
        self.shared.writing.store(false, Ordering::Release);
        result.map_err(|e| e.to_string())
    }
    pub fn drag(&mut self, files: Vec<PathBuf>, payloads: Vec<Payload>) -> Result<(), String> {
        let window = unsafe { GetForegroundWindow() };
        let binding = self
            .bindings
            .get(&(window.0 as usize))
            .or_else(|| self.bindings.values().find(|b| b.panel))
            .ok_or("面板原生窗口尚未就绪")?;
        *binding.drag.borrow_mut() = Some((files, payloads));
        unsafe {
            PostMessageW(Some(binding.window), DRAG_MESSAGE, WPARAM(0), LPARAM(0))
                .map_err(|e| e.to_string())
        }
    }
    pub fn stop(&mut self) {
        if !self.shared.alive.swap(false, Ordering::AcqRel) {
            return;
        }
        let window = self.shared.watch_window.load(Ordering::Acquire);
        if window != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(window as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        for binding in self.bindings.values() {
            unsafe {
                if binding.attached() {
                    let _ = RevokeDragDrop(binding.window);
                    let _ = RemoveWindowSubclass(binding.window, Some(window_proc), SUBCLASS);
                }
            }
        }
        self.bindings.clear();
        if self.ole {
            unsafe {
                OleUninitialize();
            }
            self.ole = false;
        }
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "system" fn enumerate(window: HWND, parameter: LPARAM) -> ::windows::core::BOOL {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    if pid == std::process::id() {
        let mut title = [0u16; 128];
        let count = unsafe { GetWindowTextW(window, &mut title) };
        if String::from_utf16_lossy(&title[..count.max(0) as usize]) == "麻薯 Mochi · 控制面板"
        {
            unsafe {
                (*(parameter.0 as *mut Vec<usize>)).push(window.0 as usize);
            }
        }
    }
    true.into()
}
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    if message == DRAG_MESSAGE {
        let binding = unsafe { &*(data as *const Binding) };
        let drag = binding.drag.borrow_mut().take();
        if let Some((files, payloads)) = drag {
            match outgoing(&files, &payloads) {
                Ok(formats) => {
                    let object: IDataObject = DataObject { formats }.into();
                    let source: IDropSource = DragSource.into();
                    let mut effect = DROPEFFECT_NONE;
                    unsafe {
                        let result = DoDragDrop(&object, &source, DROPEFFECT_COPY, &mut effect);
                        if result.is_err() {
                            binding
                                .shared
                                .emit(Event::Error(format!("拖出失败：{result:?}")));
                        }
                    }
                }
                Err(e) => binding.shared.emit(Event::Error(format!("拖出失败：{e}"))),
            }
            binding.shared.emit(Event::DragFinished);
        }
        return LRESULT(0);
    }
    if message == WM_NCDESTROY {
        unsafe { &*(data as *const Binding) }.alive.set(false);
        unsafe {
            let _ = RevokeDragDrop(window);
            let _ = RemoveWindowSubclass(window, Some(window_proc), SUBCLASS);
        }
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

struct ClipboardGuard;
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
fn registered(name: &str) -> u16 {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) as u16 }
}
fn global_bytes(handle: HGLOBAL) -> ::windows::core::Result<Vec<u8>> {
    let size = unsafe { GlobalSize(handle) };
    if size > MAX_DATA {
        return Err(::windows::core::Error::from_hresult(E_OUTOFMEMORY));
    }
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        return Err(::windows::core::Error::from_win32());
    }
    let data = unsafe { std::slice::from_raw_parts(pointer as *const u8, size) }.to_vec();
    unsafe {
        let _ = GlobalUnlock(handle);
    }
    Ok(data)
}
fn clipboard_data(format: u16) -> Option<Vec<u8>> {
    unsafe {
        GetClipboardData(format as u32)
            .ok()
            .and_then(|h| global_bytes(HGLOBAL(h.0)).ok())
    }
}
fn text_bytes(bytes: &[u8]) -> String {
    let chars = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .take_while(|c| *c != 0)
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&chars)
}
fn ansi_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default()).into_owned()
}
fn files_from_handle(global: HGLOBAL) -> Vec<PathBuf> {
    let drop = HDROP(global.0);
    let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
    (0..count.min(100000))
        .map(|index| {
            let size = unsafe { DragQueryFileW(drop, index, None) };
            let mut name = vec![0; size as usize + 1];
            unsafe {
                DragQueryFileW(drop, index, Some(&mut name));
            }
            PathBuf::from(String::from_utf16_lossy(&name[..size as usize]))
        })
        .collect()
}
fn read_clipboard() -> ::windows::core::Result<Vec<Payload>> {
    unsafe {
        OpenClipboard(None)?;
    }
    let _guard = ClipboardGuard;
    if clipboard_data(registered("ExcludeClipboardContentFromMonitorProcessing")).is_some()
        || clipboard_data(registered("CanIncludeInClipboardHistory"))
            .is_some_and(|b| b.get(..4) == Some(&[0, 0, 0, 0]))
    {
        return Ok(vec![]);
    }
    if let Ok(h) = unsafe { GetClipboardData(CF_HDROP.0 as u32) } {
        return Ok(vec![Payload::Files(files_from_handle(HGLOBAL(h.0)))]);
    }
    if let Some(png) = clipboard_data(registered("PNG")) {
        return Ok(vec![Payload::Image(png)]);
    }
    if let Some(dib) = clipboard_data(CF_DIBV5.0).or_else(|| clipboard_data(CF_DIB.0))
        && let Ok(png) = dib_png(&dib)
    {
        return Ok(vec![Payload::Image(png)]);
    }
    if let Some(text) = clipboard_data(CF_UNICODETEXT.0) {
        return Ok(vec![Payload::Text {
            plain: text_bytes(&text),
            html: clipboard_data(registered("HTML Format")),
            rtf: clipboard_data(registered("Rich Text Format")),
        }]);
    }
    Ok(vec![])
}

fn watch(shared: Arc<Shared>) {
    // A private message-only window receives clipboard notifications without
    // waking the pet's rendering loop for unchanged content.
    let class: Vec<u16> = format!("MochiClipboard-{}", std::process::id())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut state = Box::new(Watch {
        shared: shared.clone(),
        sequence: unsafe { GetClipboardSequenceNumber() },
        enabled: shared.enabled.load(Ordering::Acquire),
        pending: false,
        attempts: 0,
    });
    unsafe {
        let definition = WNDCLASSW {
            lpfnWndProc: Some(watch_proc),
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        RegisterClassW(&definition);
        let window = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class.as_ptr()),
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            Some((&mut *state as *mut Watch).cast()),
        ) {
            Ok(w) => w,
            Err(e) => {
                shared.emit(Event::Access(format!("剪切板监听无法启动：{e}")));
                return;
            }
        };
        shared
            .watch_window
            .store(window.0 as usize, Ordering::Release);
        if let Err(e) = AddClipboardFormatListener(window) {
            shared.emit(Event::Access(e.to_string()));
            let _ = DestroyWindow(window);
            return;
        }
        SetTimer(Some(window), 1, 100, None);
        let mut message = MSG::default();
        while shared.alive.load(Ordering::Acquire) && GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let _ = RemoveClipboardFormatListener(window);
        let _ = DestroyWindow(window);
        let _ = UnregisterClassW(PCWSTR(class.as_ptr()), None);
        shared.watch_window.store(0, Ordering::Release);
    }
}
struct Watch {
    shared: Arc<Shared>,
    sequence: u32,
    enabled: bool,
    pending: bool,
    attempts: u8,
}
unsafe extern "system" fn watch_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut Watch;
    if !pointer.is_null() && matches!(message, WM_TIMER | WM_CLIPBOARDUPDATE) {
        let state = unsafe { &mut *pointer };
        let enabled = state.shared.enabled.load(Ordering::Acquire);
        if enabled != state.enabled {
            state.enabled = enabled;
            state.sequence = state.shared.baseline.load(Ordering::Acquire);
            state.pending = false;
        }
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence != state.sequence && !state.shared.writing.load(Ordering::Acquire) {
            state.sequence = sequence;
            state.attempts = 0;
            state.pending = enabled && sequence != state.shared.ignore.load(Ordering::Acquire);
            if sequence != state.shared.ignore.load(Ordering::Acquire) {
                state.shared.emit(Event::OwnershipLost);
            }
        }
        if state.pending {
            match read_clipboard() {
                Ok(payloads) => {
                    state.pending = false;
                    if !payloads.is_empty() {
                        state.shared.emit(Event::Clipboard(payloads));
                    }
                }
                Err(e) => {
                    state.attempts += 1;
                    if state.attempts >= 3 {
                        state.pending = false;
                        state
                            .shared
                            .emit(Event::Access(format!("剪切板暂时无法读取：{e}")));
                    }
                }
            }
        }
    }
    if message == WM_CLOSE {
        unsafe {
            let _ = DestroyWindow(window);
            PostQuitMessage(0);
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn format(format: u16, tymed: TYMED, index: i32) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        tymed: tymed.0 as u32,
        lindex: index,
        dwAspect: DVASPECT_CONTENT.0,
        ..Default::default()
    }
}
struct Medium(STGMEDIUM);
impl Drop for Medium {
    fn drop(&mut self) {
        unsafe {
            ReleaseStgMedium(&mut self.0);
        }
    }
}
fn data(object: &IDataObject, format_id: u16) -> Option<Vec<u8>> {
    let medium = Medium(unsafe { object.GetData(&format(format_id, TYMED_HGLOBAL, -1)).ok()? });
    global_bytes(unsafe { medium.0.u.hGlobal }).ok()
}

#[implement(IDropTarget)]
struct DropTarget {
    shared: Arc<Shared>,
    window: usize,
    panel: bool,
    accepted: Cell<bool>,
}
impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        object: Ref<'_, IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> ::windows::core::Result<()> {
        let accepted = object.as_ref().is_some_and(supported);
        self.accepted.set(accepted);
        unsafe {
            *effect = if accepted {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        self.shared.emit(Event::Hover(accepted));
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> ::windows::core::Result<()> {
        unsafe {
            *effect = if self.accepted.get() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        Ok(())
    }
    fn DragLeave(&self) -> ::windows::core::Result<()> {
        self.shared.emit(Event::Hover(false));
        Ok(())
    }
    fn Drop(
        &self,
        object: Ref<'_, IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> ::windows::core::Result<()> {
        self.shared.emit(Event::Hover(false));
        let Some(object) = object.as_ref() else {
            return Err(E_INVALIDARG.into());
        };
        unsafe {
            *effect = DROPEFFECT_COPY;
        }
        let mut local = POINT {
            x: point.x,
            y: point.y,
        };
        unsafe {
            let _ = ::windows::Win32::Graphics::Gdi::ScreenToClient(
                HWND(self.window as *mut _),
                &mut local,
            );
        }
        let scale = self
            .shared
            .context
            .input(|input| input.viewport().native_pixels_per_point.unwrap_or(1.0));
        let position = [local.x as f32 / scale, local.y as f32 / scale];
        if data(object, registered("FileGroupDescriptorW")).is_some() {
            let capability = object
                .cast::<IDataObjectAsyncCapability>()
                .ok()
                .filter(|c| unsafe {
                    c.SetAsyncMode(true).is_ok() && c.StartOperation(None).is_ok()
                });
            if capability.is_none() {
                // A synchronous provider may revoke its temporary streams as soon
                // as Drop returns. Materialize them before returning to the source.
                let payload = virtual_files(object, &self.shared)
                    .unwrap_or_else(|error| Payload::Error(format!("附件接收失败：{error}")));
                self.shared.emit(Event::Dropped {
                    payloads: vec![payload],
                    panel: self.panel,
                    position,
                });
                return Ok(());
            }
            // Marshal the provider to a dedicated STA; no unsafe Send of COM objects.
            let marshaled = match unsafe {
                CoMarshalInterThreadInterfaceInStream(&IDataObject::IID, object)
            } {
                Ok(stream) => stream.into_raw() as usize,
                Err(error) => {
                    if let Some(capability) = capability {
                        unsafe {
                            let _ = capability.EndOperation(error.code(), None, DROPEFFECT_NONE.0);
                        }
                    }
                    self.shared.emit(Event::Dropped {
                        payloads: vec![Payload::Error(format!("附件接收失败：{error}"))],
                        panel: self.panel,
                        position,
                    });
                    return Ok(());
                }
            };
            let shared = self.shared.clone();
            let panel = self.panel;
            thread::spawn(move || {
                unsafe {
                    let initialized = OleInitialize(None).is_ok();
                    let stream = IStream::from_raw(marshaled as *mut _);
                    match CoGetInterfaceAndReleaseStream::<_, IDataObject>(&stream).and_then(
                        |object| {
                            let result = virtual_files(&object, &shared);
                            if let Ok(capability) = object.cast::<IDataObjectAsyncCapability>() {
                                let _ = capability.EndOperation(
                                    if result.is_ok() { S_OK } else { E_FAIL },
                                    None,
                                    DROPEFFECT_COPY.0,
                                );
                            }
                            result
                        },
                    ) {
                        Ok(payload) => shared.emit(Event::Dropped {
                            payloads: vec![payload],
                            panel,
                            position,
                        }),
                        Err(e) => shared.emit(Event::Dropped {
                            payloads: vec![Payload::Error(format!("附件接收失败：{e}"))],
                            panel,
                            position,
                        }),
                    }
                    // CoGetInterfaceAndReleaseStream consumed the marshaled stream.
                    std::mem::forget(stream);
                    if initialized {
                        OleUninitialize();
                    }
                }
            });
        } else {
            let payloads = decode_drop(object)?;
            if payloads.is_empty() {
                self.shared.emit(Event::Error("不支持的内容类型".into()));
            } else {
                self.shared.emit(Event::Dropped {
                    payloads,
                    panel: self.panel,
                    position,
                });
            }
        }
        Ok(())
    }
}
fn supported(object: &IDataObject) -> bool {
    [
        CF_HDROP.0,
        CF_UNICODETEXT.0,
        CF_DIBV5.0,
        CF_DIB.0,
        registered("PNG"),
        registered("FileGroupDescriptorW"),
        registered("UniformResourceLocatorW"),
        registered("UniformResourceLocator"),
        registered("DownloadURL"),
    ]
    .into_iter()
    .any(|id| unsafe { object.QueryGetData(&format(id, TYMED_HGLOBAL, -1)).is_ok() })
}
fn decode_drop(object: &IDataObject) -> ::windows::core::Result<Vec<Payload>> {
    if let Some(bytes) = data(object, CF_HDROP.0) {
        let handle = allocate(&bytes)?;
        let files = files_from_handle(handle);
        unsafe {
            let _ = GlobalFree(Some(handle));
        }
        return Ok(vec![Payload::Files(files)]);
    }
    if let Some(png) = data(object, registered("PNG")) {
        return Ok(vec![Payload::Image(png)]);
    }
    if let Some(dib) = data(object, CF_DIBV5.0).or_else(|| data(object, CF_DIB.0))
        && let Ok(png) = dib_png(&dib)
    {
        return Ok(vec![Payload::Image(png)]);
    }
    if let Some(bytes) = data(object, registered("DownloadURL")) {
        let text = if bytes.get(1) == Some(&0) {
            text_bytes(&bytes)
        } else {
            ansi_text(&bytes)
        };
        let fields = text.splitn(3, ':').collect::<Vec<_>>();
        if fields.len() == 3 {
            return Ok(vec![super::remote_payload(
                fields[2],
                Some(fields[1].into()),
                fields[0].starts_with("image/"),
            )]);
        }
    }
    let url = data(object, registered("UniformResourceLocatorW"))
        .map(|b| text_bytes(&b))
        .or_else(|| data(object, registered("UniformResourceLocator")).map(|b| ansi_text(&b)));
    if let Some(url) = url {
        let html = data(object, registered("HTML Format")).map(|b| ansi_text(&b));
        return Ok(vec![super::remote_payload(
            &url,
            None,
            html.is_some_and(|h| h.to_ascii_lowercase().contains("<img")),
        )]);
    }
    if let Some(text) = data(object, CF_UNICODETEXT.0) {
        return Ok(vec![Payload::Text {
            plain: text_bytes(&text),
            html: data(object, registered("HTML Format")),
            rtf: data(object, registered("Rich Text Format")),
        }]);
    }
    Ok(vec![])
}
struct Receiving<'a> {
    shared: &'a Shared,
    id: String,
}
impl Drop for Receiving<'_> {
    fn drop(&mut self) {
        if let Ok(mut jobs) = self.shared.jobs.lock() {
            jobs.remove(&self.id);
        }
        self.shared.emit(Event::ReceiveFinished(self.id.clone()));
    }
}
fn virtual_files(object: &IDataObject, shared: &Shared) -> ::windows::core::Result<Payload> {
    let descriptors = data(object, registered("FileGroupDescriptorW"))
        .ok_or_else(|| ::windows::core::Error::from_hresult(DV_E_FORMATETC))?;
    if descriptors.len() < 4 {
        return Err(E_INVALIDARG.into());
    }
    let count = u32::from_le_bytes(descriptors[..4].try_into().unwrap()) as usize;
    let size = std::mem::size_of::<FILEDESCRIPTORW>();
    if count == 0 || count > 10000 || descriptors.len() < 4 + count * size {
        return Err(E_INVALIDARG.into());
    }
    let mut names = Vec::new();
    for index in 0..count {
        let descriptor = unsafe {
            std::ptr::read_unaligned(
                descriptors
                    .as_ptr()
                    .add(4 + index * size)
                    .cast::<FILEDESCRIPTORW>(),
            )
        };
        let chars = descriptor.cFileName;
        let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
        let name = String::from_utf16_lossy(&chars[..end]);
        let path = PathBuf::from(&name);
        if name.is_empty()
            || name.contains(':')
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(E_INVALIDARG.into());
        }
        names.push(path);
    }
    std::fs::create_dir_all(&shared.directory).map_err(|_| ::windows::core::Error::from_win32())?;
    let id = format!(
        "attachment-{}-{}",
        desktop_pet::safety::timestamp(),
        NEXT_INCOMING.fetch_add(1, Ordering::Relaxed)
    );
    let directory = shared.directory.join(&id);
    std::fs::create_dir(&directory).map_err(|_| ::windows::core::Error::from_win32())?;
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut jobs) = shared.jobs.lock() {
        jobs.insert(id.clone(), cancel.clone());
    }
    let receive = Receiving { shared, id };
    shared.emit(Event::Receiving {
        id: receive.id.clone(),
        label: "正在接收附件".into(),
        bytes: 0,
    });
    let mut paths = Vec::new();
    let mut errors = Vec::new();
    let mut total = 0u64;
    let mut emitted = 0u64;
    for (index, name) in names.into_iter().enumerate() {
        if !shared.alive.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
            errors.push("附件收纳已取消".into());
            break;
        }
        let mut path = directory.join(name);
        if path.exists() {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            path.set_file_name(format!("{}-{name}", index + 1));
        }
        let result = (|| -> ::windows::core::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|_| ::windows::core::Error::from_win32())?;
            }
            let medium = Medium(unsafe {
                object.GetData(&format(
                    registered("FileContents"),
                    TYMED(TYMED_ISTREAM.0 | TYMED_HGLOBAL.0),
                    index as i32,
                ))?
            });
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|_| ::windows::core::Error::from_win32())?;
            if medium.0.tymed == TYMED_ISTREAM.0 as u32 {
                let stream = unsafe { &medium.0.u.pstm };
                let stream = stream
                    .as_ref()
                    .ok_or_else(|| ::windows::core::Error::from_hresult(E_INVALIDARG))?;
                let mut buffer = vec![0; 64 * 1024];
                loop {
                    if !shared.alive.load(Ordering::Acquire) || cancel.load(Ordering::Acquire) {
                        return Err(E_ABORT.into());
                    }
                    let mut count = 0;
                    unsafe {
                        stream
                            .Read(
                                buffer.as_mut_ptr().cast(),
                                buffer.len() as u32,
                                Some(&mut count),
                            )
                            .ok()?;
                    }
                    if count == 0 {
                        break;
                    }
                    total += count as u64;
                    if total > shared.quota.load(Ordering::Acquire) {
                        return Err(E_OUTOFMEMORY.into());
                    }
                    output
                        .write_all(&buffer[..count as usize])
                        .map_err(|_| ::windows::core::Error::from_win32())?;
                    if total - emitted >= 1024 * 1024 {
                        emitted = total;
                        shared.emit(Event::Receiving {
                            id: receive.id.clone(),
                            label: "正在接收附件".into(),
                            bytes: total,
                        });
                    }
                }
            } else if medium.0.tymed == TYMED_HGLOBAL.0 as u32 {
                let bytes = global_bytes(unsafe { medium.0.u.hGlobal })?;
                total += bytes.len() as u64;
                if total > shared.quota.load(Ordering::Acquire) {
                    return Err(E_OUTOFMEMORY.into());
                }
                output
                    .write_all(&bytes)
                    .map_err(|_| ::windows::core::Error::from_win32())?;
            } else {
                return Err(DV_E_TYMED.into());
            }
            output
                .sync_all()
                .map_err(|_| ::windows::core::Error::from_win32())?;
            Ok(())
        })();
        match result {
            Ok(()) => paths.push(path),
            Err(e) => {
                errors.push(format!(
                    "{}：{e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    Ok(Payload::TemporaryFiles {
        paths,
        directory,
        errors,
    })
}

static NEXT_INCOMING: AtomicUsize = AtomicUsize::new(0);

fn allocate(bytes: &[u8]) -> ::windows::core::Result<HGLOBAL> {
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1))? };
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        unsafe {
            let _ = GlobalFree(Some(handle));
        }
        return Err(::windows::core::Error::from_win32());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
        let _ = GlobalUnlock(handle);
    }
    Ok(handle)
}
fn unicode_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}
fn outgoing(
    files: &[PathBuf],
    payloads: &[Payload],
) -> ::windows::core::Result<Vec<(u16, Vec<u8>)>> {
    let mut formats = Vec::new();
    let all_text = payloads.len() > 1 && payloads.iter().all(|p| matches!(p, Payload::Text { .. }));
    if all_text {
        let text = payloads
            .iter()
            .filter_map(|p| {
                if let Payload::Text { plain, .. } = p {
                    Some(plain.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        formats.push((CF_UNICODETEXT.0, unicode_bytes(&text)));
    }
    for payload in payloads {
        match payload {
            Payload::Text { plain, html, rtf } => {
                if !formats.iter().any(|(f, _)| *f == CF_UNICODETEXT.0) {
                    formats.push((CF_UNICODETEXT.0, unicode_bytes(plain)));
                    if let Some(html) = html {
                        formats.push((registered("HTML Format"), html.clone()));
                    }
                    if let Some(rtf) = rtf {
                        formats.push((registered("Rich Text Format"), rtf.clone()));
                    }
                }
            }
            Payload::Image(png) if !formats.iter().any(|(id, _)| *id == registered("PNG")) => {
                formats.push((registered("PNG"), png.clone()));
                if let Ok(dib) = png_dib(png) {
                    formats.push((CF_DIBV5.0, dib));
                }
            }
            _ => {}
        }
    }
    // Text/image clipboard copies keep their native representation; files are
    // added for external drag targets that only understand filesystem paths.
    if !files.is_empty() {
        let header = DROPFILES {
            pFiles: std::mem::size_of::<DROPFILES>() as u32,
            fWide: true.into(),
            ..Default::default()
        };
        let mut bytes = unsafe {
            std::slice::from_raw_parts(
                (&header as *const DROPFILES).cast::<u8>(),
                std::mem::size_of::<DROPFILES>(),
            )
        }
        .to_vec();
        for path in files {
            let raw = path.to_string_lossy();
            let name = if let Some(unc) = raw.strip_prefix("\\\\?\\UNC\\") {
                format!("\\\\{unc}")
            } else {
                raw.strip_prefix("\\\\?\\").unwrap_or(&raw).to_owned()
            };
            bytes.extend(unicode_bytes(&name));
        }
        bytes.extend([0, 0]);
        formats.push((CF_HDROP.0, bytes));
    }
    Ok(formats)
}

#[implement(IDataObject)]
struct DataObject {
    formats: Vec<(u16, Vec<u8>)>,
}
impl IDataObject_Impl for DataObject_Impl {
    fn GetData(&self, requested: *const FORMATETC) -> ::windows::core::Result<STGMEDIUM> {
        let requested = unsafe { requested.as_ref() }
            .ok_or_else(|| ::windows::core::Error::from_hresult(E_POINTER))?;
        let (_, data) = self
            .formats
            .iter()
            .find(|(f, _)| {
                *f == requested.cfFormat && requested.tymed & TYMED_HGLOBAL.0 as u32 != 0
            })
            .ok_or_else(|| ::windows::core::Error::from_hresult(DV_E_FORMATETC))?;
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 {
                hGlobal: allocate(data)?,
            },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
    fn QueryGetData(&self, p: *const FORMATETC) -> HRESULT {
        if unsafe { p.as_ref() }.is_some_and(|r| {
            self.formats.iter().any(|(f, _)| *f == r.cfFormat)
                && r.tymed & TYMED_HGLOBAL.0 as u32 != 0
        }) {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }
    fn EnumFormatEtc(&self, direction: u32) -> ::windows::core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe {
            SHCreateStdEnumFmtEtc(
                &self
                    .formats
                    .iter()
                    .map(|(f, _)| format(*f, TYMED_HGLOBAL, -1))
                    .collect::<Vec<_>>(),
            )
        }
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> ::windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, _: *mut FORMATETC) -> HRESULT {
        E_NOTIMPL
    }
    fn SetData(
        &self,
        _: *const FORMATETC,
        _: *const STGMEDIUM,
        _: ::windows::core::BOOL,
    ) -> ::windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<'_, IAdviseSink>,
    ) -> ::windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn DUnadvise(&self, _: u32) -> ::windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
    fn EnumDAdvise(&self) -> ::windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}
#[implement(IDropSource)]
struct DragSource;
impl IDropSource_Impl for DragSource_Impl {
    fn QueryContinueDrag(
        &self,
        escape: ::windows::core::BOOL,
        keys: MODIFIERKEYS_FLAGS,
    ) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & 1 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

fn dib_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 40 {
        return Err("位图头部缺失".into());
    }
    let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let header = u32_at(0) as usize;
    let width = u32_at(4) as i32;
    let height = u32_at(8) as i32;
    let bits = u16::from_le_bytes(bytes[14..16].try_into().unwrap());
    let compression = u32_at(16);
    if width <= 0
        || height == 0
        || height == i32::MIN
        || !matches!(bits, 24 | 32)
        || !matches!(compression, 0 | 3)
        || header < 40
        || header > bytes.len()
    {
        return Err("不支持的剪切板位图格式".into());
    }
    let height_abs = height.unsigned_abs();
    if width as u64 * height_abs as u64 > 64 * 1024 * 1024 {
        return Err("剪切板图片过大".into());
    }
    let stride = (width as usize * bits as usize).div_ceil(32) * 4;
    let start = header
        + if header == 40 && compression == 3 {
            12
        } else {
            0
        };
    if start
        .checked_add(stride * height_abs as usize)
        .is_none_or(|end| end > bytes.len())
    {
        return Err("位图内容不完整".into());
    }
    let mut image = image::RgbaImage::new(width as u32, height_abs);
    let alpha = bits == 32
        && (header >= 56 && u32_at(52) != 0
            || bytes[start..].as_chunks::<4>().0.iter().any(|p| p[3] != 0));
    for y in 0..height_abs as usize {
        let row = if height < 0 {
            y
        } else {
            height_abs as usize - 1 - y
        };
        for x in 0..width as usize {
            let i = start + row * stride + x * (bits as usize / 8);
            image.put_pixel(
                x as u32,
                y as u32,
                image::Rgba([
                    bytes[i + 2],
                    bytes[i + 1],
                    bytes[i],
                    if alpha { bytes[i + 3] } else { 255 },
                ]),
            );
        }
    }
    let mut output = Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(output.into_inner())
}
fn png_dib(png: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory(png)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let mut bytes = vec![0u8; 124];
    bytes[0..4].copy_from_slice(&124u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&width.to_le_bytes());
    bytes[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes());
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
    bytes[16..20].copy_from_slice(&3u32.to_le_bytes());
    for (offset, mask) in [
        (40, 0x00ff0000u32),
        (44, 0x0000ff00),
        (48, 0x000000ff),
        (52, 0xff000000),
        (56, 0x73524742),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&mask.to_le_bytes());
    }
    for pixel in image.pixels() {
        bytes.extend([pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn data_object_roundtrips_unicode_paths_and_rejects_unknown_formats() {
        let files = vec![
            PathBuf::from("C:/资料/曦玥.txt"),
            PathBuf::from("C:/资料/second.png"),
        ];
        let object: IDataObject = DataObject {
            formats: outgoing(&files, &[]).unwrap(),
        }
        .into();
        let result = decode_drop(&object).unwrap();
        assert!(matches!(&result[0],Payload::Files(actual) if actual==&files));
        assert!(unsafe {
            object
                .QueryGetData(&format(65500, TYMED_HGLOBAL, -1))
                .is_err()
        });
        let object: IDataObject = DataObject {
            formats: outgoing(&[], &[Payload::text("曦玥\n剪切板")]).unwrap(),
        }
        .into();
        assert!(
            matches!(&decode_drop(&object).unwrap()[0],Payload::Text {plain,..} if plain=="曦玥\n剪切板")
        );
    }
    #[test]
    fn dib_v5_preserves_pixels_and_fully_transparent_images() {
        for pixel in [[1, 2, 3, 255], [8, 7, 6, 0]] {
            let mut png = Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(4, 3, image::Rgba(pixel))
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            let dib = png_dib(&png.into_inner()).unwrap();
            let decoded = image::load_from_memory(&dib_png(&dib).unwrap())
                .unwrap()
                .into_rgba8();
            assert_eq!(decoded.dimensions(), (4, 3));
            assert!(decoded.pixels().all(|p| p.0 == pixel));
        }
        assert!(dib_png(&[0; 40]).is_err());
    }
    #[test]
    fn browser_download_formats_distinguish_files_from_page_links_and_prefer_pixels() {
        let object: IDataObject = DataObject {
            formats: vec![(
                registered("DownloadURL"),
                b"image/png:cat.png:https://example.com/image".to_vec(),
            )],
        }
        .into();
        assert!(
            matches!(&decode_drop(&object).unwrap()[0], Payload::Remote { url, name: Some(name) } if url == "https://example.com/image" && name == "cat.png")
        );
        let object: IDataObject = DataObject {
            formats: vec![(
                registered("UniformResourceLocatorW"),
                unicode_bytes("https://example.com/page"),
            )],
        }
        .into();
        assert!(
            matches!(&decode_drop(&object).unwrap()[0], Payload::Text { plain, .. } if plain == "https://example.com/page")
        );
        let object: IDataObject = DataObject {
            formats: vec![
                (
                    registered("UniformResourceLocatorW"),
                    unicode_bytes("https://example.com/image"),
                ),
                (
                    registered("HTML Format"),
                    b"<img src=\"https://example.com/image\">".to_vec(),
                ),
            ],
        }
        .into();
        assert!(matches!(
            &decode_drop(&object).unwrap()[0],
            Payload::Remote { .. }
        ));
        let object: IDataObject = DataObject {
            formats: vec![
                (registered("PNG"), b"pixels".to_vec()),
                (
                    registered("DownloadURL"),
                    b"image/png:cat.png:https://example.com/image".to_vec(),
                ),
            ],
        }
        .into();
        assert!(
            matches!(&decode_drop(&object).unwrap()[0], Payload::Image(bytes) if bytes == b"pixels")
        );
        let files = vec![PathBuf::from("C:/export/图片.png")];
        let formats = outgoing(&files, &[Payload::Image(vec![])]).unwrap();
        assert!(formats.iter().any(|(id, _)| *id == CF_HDROP.0));
        assert!(formats.iter().any(|(id, _)| *id == registered("PNG")));
    }
    #[test]
    fn virtual_attachment_materializes_bytes_and_rejects_parent_paths() {
        let root = std::env::temp_dir().join(format!(
            "mochi-native-{}-{}",
            std::process::id(),
            NEXT_INCOMING.fetch_add(1, Ordering::Relaxed)
        ));
        let (sender, _) = mpsc::channel();
        let shared = Shared {
            sender,
            context: eframe::egui::Context::default(),
            directory: root.clone(),
            enabled: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            ignore: AtomicU32::new(0),
            baseline: AtomicU32::new(0),
            writing: AtomicBool::new(false),
            watch_window: AtomicUsize::new(0),
            quota: AtomicU64::new(5 * 1024 * 1024 * 1024),
            jobs: Mutex::new(HashMap::new()),
        };
        for name in ["附件.txt", "..\\outside.txt"] {
            let mut descriptor = FILEDESCRIPTORW::default();
            let mut filename = [0u16; 260];
            for (target, c) in filename.iter_mut().zip(name.encode_utf16()) {
                *target = c;
            }
            descriptor.cFileName = filename;
            let mut bytes = 1u32.to_le_bytes().to_vec();
            bytes.extend(unsafe {
                std::slice::from_raw_parts(
                    (&descriptor as *const FILEDESCRIPTORW).cast::<u8>(),
                    std::mem::size_of::<FILEDESCRIPTORW>(),
                )
            });
            let object: IDataObject = DataObject {
                formats: vec![
                    (registered("FileGroupDescriptorW"), bytes),
                    (registered("FileContents"), b"attachment body".to_vec()),
                ],
            }
            .into();
            let result = virtual_files(&object, &shared);
            if name.starts_with("..") {
                assert!(result.is_err());
                assert!(!root.parent().unwrap().join("outside.txt").exists());
            } else {
                let Payload::TemporaryFiles { paths, .. } = result.unwrap() else {
                    panic!("missing attachment")
                };
                assert_eq!(std::fs::read(&paths[0]).unwrap(), b"attachment body");
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn private_clipboard_listener_starts_and_stops_without_reading_content() {
        let (sender, _) = mpsc::channel();
        let shared = Arc::new(Shared {
            sender,
            context: eframe::egui::Context::default(),
            directory: PathBuf::new(),
            enabled: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            ignore: AtomicU32::new(0),
            baseline: AtomicU32::new(0),
            writing: AtomicBool::new(false),
            watch_window: AtomicUsize::new(0),
            quota: AtomicU64::new(5 * 1024 * 1024 * 1024),
            jobs: Mutex::new(HashMap::new()),
        });
        let state = shared.clone();
        let worker = thread::spawn(move || watch(state));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while shared.watch_window.load(Ordering::Acquire) == 0
            && std::time::Instant::now() < deadline
        {
            thread::sleep(std::time::Duration::from_millis(10));
        }
        let handle = shared.watch_window.load(Ordering::Acquire);
        assert_ne!(handle, 0);
        shared.alive.store(false, Ordering::Release);
        unsafe {
            let _ = PostMessageW(Some(HWND(handle as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        worker.join().unwrap();
        assert_eq!(shared.watch_window.load(Ordering::Acquire), 0);
    }
}

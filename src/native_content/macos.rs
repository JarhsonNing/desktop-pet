use super::*;
use block2::RcBlock;
use objc2::{
    AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::Cell,
    collections::HashMap,
    ptr::NonNull,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

struct Shared {
    sender: mpsc::Sender<Event>,
    context: eframe::egui::Context,
    directory: PathBuf,
    alive: AtomicBool,
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
struct Watch {
    shared: Arc<Shared>,
    enabled: Cell<bool>,
    sequence: Cell<isize>,
    ignore: Cell<isize>,
    denied: Cell<bool>,
}
struct DelegateIvars {
    shared: Arc<Shared>,
    original: Option<Retained<ProtocolObject<dyn NSWindowDelegate>>>,
    panel: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MochiContentWindowDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSWindowDelegate for Delegate {}
    impl Delegate {
        #[unsafe(method(forwardingTargetForSelector:))]
        fn forward(&self,_selector:Sel)->*mut AnyObject {self.ivars().original.as_ref().map_or(std::ptr::null_mut(), |o| Retained::as_ptr(o) as *mut AnyObject)}
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self,selector:Sel)->bool {
            let own:bool=unsafe {msg_send![super(self),respondsToSelector:selector]};
            own || self.ivars().original.as_ref().is_some_and(|o|o.respondsToSelector(selector))
        }
    }
    unsafe impl NSDraggingDestination for Delegate {
        #[unsafe(method(draggingEntered:))]
        fn entered(&self,sender:&ProtocolObject<dyn NSDraggingInfo>)->NSDragOperation {
            let board=sender.draggingPasteboard();let accepted=supported(&board);
            self.ivars().shared.emit(Event::Hover(accepted));
            if accepted {NSDragOperation::Copy}else{NSDragOperation::None}
        }
        #[unsafe(method(draggingUpdated:))]
        fn updated(&self,sender:&ProtocolObject<dyn NSDraggingInfo>)->NSDragOperation {if supported(&sender.draggingPasteboard()){NSDragOperation::Copy}else{NSDragOperation::None}}
        #[unsafe(method(draggingExited:))]
        fn exited(&self,_:Option<&ProtocolObject<dyn NSDraggingInfo>>){self.ivars().shared.emit(Event::Hover(false));}
        #[unsafe(method(prepareForDragOperation:))]
        fn prepare(&self,_:&ProtocolObject<dyn NSDraggingInfo>)->bool {true}
        #[unsafe(method(performDragOperation:))]
        fn dropped(&self,sender:&ProtocolObject<dyn NSDraggingInfo>)->bool {
            let board=sender.draggingPasteboard();let mut point=sender.draggingLocation();
            if let Some(window)=sender.draggingDestinationWindow() && let Some(view)=window.contentView(){point=view.convertPoint_fromView(point,None);}
            self.ivars().shared.emit(Event::Hover(false));
            if receive_promises(&board,self.ivars().shared.clone(),self.ivars().panel,[point.x as f32,point.y as f32]) {return true.into();}
            match read_board(&board,true) {
                Ok(payloads) if !payloads.is_empty()=>{self.ivars().shared.emit(Event::Dropped {payloads,panel:self.ivars().panel,position:[point.x as f32,point.y as f32]});true},
                Ok(_)=>{self.ivars().shared.emit(Event::Error("不支持的内容类型".into()));false},
                Err(e)=>{self.ivars().shared.emit(Event::Error(e));false},
            }
        }
    }
    unsafe impl NSDraggingSource for Delegate {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operations(&self,_:&NSDraggingSession,_:NSDraggingContext)->NSDragOperation {NSDragOperation::Copy}
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn drag_ended(&self,_:&NSDraggingSession,_:NSPoint,_:NSDragOperation){self.ivars().shared.emit(Event::DragFinished);}
        #[unsafe(method(ignoreModifierKeysForDraggingSession:))]
        fn ignore_modifiers(&self,_:&NSDraggingSession)->bool {true}
    }
);

pub struct Bridge {
    root: Option<Retained<NSWindow>>,
    bindings: HashMap<isize, (Retained<NSWindow>, Retained<Delegate>)>,
    watch: Rc<Watch>,
    timer: Option<Retained<NSTimer>>,
}
impl Bridge {
    pub fn new(
        creation: &eframe::CreationContext<'_>,
        directory: PathBuf,
        enabled: bool,
    ) -> (Self, Events) {
        Self::new_window(
            creation.window_handle().ok().map(|h| h.as_raw()),
            directory,
            enabled,
            creation.egui_ctx.clone(),
        )
    }
    pub fn new_window(
        window: Option<RawWindowHandle>,
        directory: PathBuf,
        enabled: bool,
        context: eframe::egui::Context,
    ) -> (Self, Events) {
        let (sender, receiver) = mpsc::channel();
        let root = match window {
            Some(RawWindowHandle::AppKit(h)) => unsafe {
                let view = &*(h.ns_view.as_ptr() as *const NSView);
                view.window()
            },
            _ => None,
        };
        let shared = Arc::new(Shared {
            sender,
            context,
            directory: directory.join("content/incoming"),
            alive: AtomicBool::new(true),
            quota: AtomicU64::new(5 * 1024 * 1024 * 1024),
            jobs: Mutex::new(HashMap::new()),
        });
        let board = NSPasteboard::generalPasteboard();
        let watch = Rc::new(Watch {
            shared,
            enabled: Cell::new(enabled),
            sequence: Cell::new(board.changeCount()),
            ignore: Cell::new(-1),
            denied: Cell::new(false),
        });
        let timer = if root.is_some() {
            let state = watch.clone();
            let block = RcBlock::new(move |_: NonNull<NSTimer>| poll(&state));
            Some(unsafe {
                NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, &block)
            })
        } else {
            None
        };
        if let Some(timer) = &timer {
            unsafe {
                NSRunLoop::mainRunLoop().addTimer_forMode(timer, NSRunLoopCommonModes);
            }
        }
        let mut bridge = Self {
            root,
            bindings: HashMap::new(),
            watch,
            timer,
        };
        bridge.refresh_windows();
        (bridge, Events { receiver })
    }
    fn bind(&mut self, window: Retained<NSWindow>, panel: bool) {
        let number = window.windowNumber();
        if self.bindings.contains_key(&number) {
            return;
        }
        let marker =
            MainThreadMarker::new().expect("AppKit window binding runs on the main thread");
        let delegate = Delegate::alloc(marker).set_ivars(DelegateIvars {
            shared: self.watch.shared.clone(),
            original: window.delegate(),
            panel,
        });
        let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
        let mut types = vec![
            NSString::from_str("public.utf8-plain-text"),
            NSString::from_str("public.url"),
            NSString::from_str("public.file-url"),
            NSString::from_str("public.png"),
            NSString::from_str("public.tiff"),
            NSString::from_str("public.html"),
        ];
        types.extend(NSFilePromiseReceiver::readableDraggedTypes().iter());
        window.registerForDraggedTypes(&NSArray::from_retained_slice(&types));
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        self.bindings.insert(number, (window, delegate));
    }
    pub fn refresh_windows(&mut self) {
        let Some(marker) = MainThreadMarker::new() else {
            return;
        };
        let application = NSApplication::sharedApplication(marker);
        let windows = application.windows();
        let open = windows.iter().map(|w| w.windowNumber()).collect::<Vec<_>>();
        self.bindings.retain(|number, _| open.contains(number));
        if let Some(root) = &self.root {
            self.bind(root.clone(), false);
        }
        for window in windows.iter() {
            if window.title().to_string() == "麻薯 Mochi · 控制面板"
                && self
                    .root
                    .as_ref()
                    .is_none_or(|r| r.windowNumber() != window.windowNumber())
            {
                self.bind(window, true);
            }
        }
    }
    pub fn bind_webview(&mut self) {
        fn unregister(view: &NSView) {
            view.unregisterDraggedTypes();
            for child in view.subviews() {
                unregister(&child);
            }
        }
        if let Some(view) = self.root.as_ref().and_then(|window| window.contentView()) {
            unregister(&view);
        }
    }
    pub fn enabled(&self, value: bool) {
        if self.watch.enabled.replace(value) != value {
            self.watch
                .sequence
                .set(NSPasteboard::generalPasteboard().changeCount());
            self.watch.denied.set(false);
        }
    }
    pub fn quota(&self, bytes: u64) {
        self.watch.shared.quota.store(bytes, Ordering::Release);
    }
    pub fn cancel_receive(&self, id: &str) -> bool {
        if let Ok(jobs) = self.watch.shared.jobs.lock()
            && let Some(cancel) = jobs.get(id)
        {
            cancel.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
    pub fn read(&self) -> Result<Vec<Payload>, String> {
        if self.root.is_none() {
            return Err("没有可用的原生剪切板窗口".into());
        }
        read_board(&NSPasteboard::generalPasteboard(), false)
    }
    pub fn copy(&mut self, files: &[PathBuf], payloads: &[Payload]) -> Result<(), String> {
        if self.root.is_none() {
            return Err("没有可用的原生剪切板窗口".into());
        }
        let board = NSPasteboard::generalPasteboard();
        let combined =
            if payloads.len() > 1 && payloads.iter().all(|p| matches!(p, Payload::Text { .. })) {
                Some(vec![Payload::text(
                    payloads
                        .iter()
                        .filter_map(|p| {
                            if let Payload::Text { plain, .. } = p {
                                Some(plain.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                )])
            } else {
                None
            };
        let payloads = combined.as_deref().unwrap_or(payloads);
        let items = outgoing(
            if super::clipboard_uses_files(payloads) {
                files
            } else {
                &[]
            },
            payloads,
        );
        board.clearContents();
        let writers = items
            .iter()
            .map(|i| ProtocolObject::<dyn NSPasteboardWriting>::from_retained(i.clone()))
            .collect::<Vec<_>>();
        if !board.writeObjects(&NSArray::from_retained_slice(&writers)) {
            return Err("系统剪切板写入失败".into());
        }
        self.watch.ignore.set(board.changeCount());
        self.watch.sequence.set(board.changeCount());
        Ok(())
    }
    pub fn drag(&mut self, files: Vec<PathBuf>, payloads: Vec<Payload>) -> Result<(), String> {
        let marker = MainThreadMarker::new().ok_or("拖放必须在主线程执行")?;
        let app = NSApplication::sharedApplication(marker);
        let event = app.currentEvent().ok_or("当前鼠标拖动事件不可用，请重试")?;
        let window = event
            .window(marker)
            .or_else(|| app.keyWindow())
            .ok_or("面板窗口不可用")?;
        let (_, delegate) = self
            .bindings
            .get(&window.windowNumber())
            .ok_or("面板原生拖放尚未就绪")?;
        let view = window.contentView().ok_or("面板视图不可用")?;
        let point = view.convertPoint_fromView(event.locationInWindow(), None);
        let icon = app.applicationIconImage();
        let mut items = Vec::new();
        for writer in outgoing(&files, &payloads) {
            let item = NSDraggingItem::initWithPasteboardWriter(
                NSDraggingItem::alloc(),
                ProtocolObject::from_ref(&*writer),
            );
            unsafe {
                item.setDraggingFrame_contents(
                    NSRect::new(point, NSSize::new(48.0, 48.0)),
                    icon.as_ref().map(|image| &**image as &AnyObject),
                );
            }
            items.push(item);
        }
        view.beginDraggingSessionWithItems_event_source(
            &NSArray::from_retained_slice(&items),
            &event,
            ProtocolObject::from_ref(&**delegate),
        );
        Ok(())
    }
    pub fn stop(&mut self) {
        self.watch.shared.alive.store(false, Ordering::Release);
        if let Some(timer) = self.timer.take() {
            timer.invalidate();
        }
        for (window, delegate) in self.bindings.values() {
            window.setDelegate(delegate.ivars().original.as_deref());
        }
        self.bindings.clear();
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

fn poll(watch: &Watch) {
    let board = NSPasteboard::generalPasteboard();
    let count = board.changeCount();
    if watch.sequence.replace(count) == count {
        return;
    }
    watch.shared.emit(Event::OwnershipLost);
    if !watch.enabled.get() || count == watch.ignore.get() || watch.denied.get() {
        return;
    }
    // New accessBehavior is queried only when available on the running OS.
    if board.respondsToSelector(sel!(accessBehavior))
        && board.accessBehavior() == NSPasteboardAccessBehavior::AlwaysDeny
    {
        watch.denied.set(true);
        watch.shared.emit(Event::Access(
            "macOS 已禁止读取剪切板，请在系统设置中允许后重新开启记录。".into(),
        ));
        return;
    }
    match read_board(&board, false) {
        Ok(payloads) => {
            if !payloads.is_empty() {
                watch.shared.emit(Event::Clipboard(payloads));
            }
        }
        Err(e) => {
            watch.denied.set(true);
            watch.shared.emit(Event::Access(e));
        }
    }
}
fn supported(board: &NSPasteboard) -> bool {
    board.types().is_some_and(|types| {
        types.iter().any(|t| {
            matches!(
                t.to_string().as_str(),
                "public.utf8-plain-text"
                    | "public.url"
                    | "public.file-url"
                    | "public.png"
                    | "public.tiff"
                    | "public.html"
                    | "com.apple.pasteboard.promised-file-url"
                    | "com.apple.pasteboard.promised-file-content-type"
            ) || NSFilePromiseReceiver::readableDraggedTypes()
                .iter()
                .any(|p| p == t)
        })
    })
}
fn read_board(board: &NSPasteboard, drop: bool) -> Result<Vec<Payload>, String> {
    let types = board.types();
    if types.as_ref().is_some_and(|types| {
        types.iter().any(|t| {
            matches!(
                t.to_string().as_str(),
                "org.nspasteboard.ConcealedType"
                    | "org.nspasteboard.TransientType"
                    | "org.nspasteboard.AutoGeneratedType"
            )
        })
    }) {
        return Ok(vec![]);
    }
    let mut files = Vec::new();
    let mut payloads = Vec::new();
    if let Some(items) = board.pasteboardItems() {
        for item in items.iter() {
            if let Some(value) = item.stringForType(&NSString::from_str("public.file-url"))
                && let Ok(url) = reqwest::Url::parse(&value.to_string())
                && let Ok(path) = url.to_file_path()
            {
                files.push(path);
                continue;
            }
            if let Some(data) = item.dataForType(&NSString::from_str("public.png")) {
                payloads.push(Payload::Image(data.to_vec()));
                continue;
            }
            if let Some(data) = item.dataForType(&NSString::from_str("public.tiff")) {
                let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data);
                if let Some(bitmap) = bitmap
                    && let Some(png) = unsafe {
                        bitmap.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                    }
                {
                    payloads.push(Payload::Image(png.to_vec()));
                    continue;
                }
            }
            if drop && let Some(url) = item.stringForType(&NSString::from_str("public.url")) {
                let hint = item
                    .stringForType(&NSString::from_str("public.html"))
                    .is_some_and(|html| html.to_string().to_ascii_lowercase().contains("<img"));
                payloads.push(super::remote_payload(&url.to_string(), None, hint));
                continue;
            }
            if let Some(text) = item.stringForType(&NSString::from_str("public.utf8-plain-text")) {
                payloads.push(Payload::Text {
                    plain: text.to_string(),
                    html: item
                        .dataForType(&NSString::from_str("public.html"))
                        .map(|d| d.to_vec()),
                    rtf: item
                        .dataForType(&NSString::from_str("public.rtf"))
                        .map(|d| d.to_vec()),
                });
            }
        }
    }
    if !files.is_empty() {
        payloads.insert(0, Payload::Files(files));
    }
    Ok(payloads)
}
fn outgoing(files: &[PathBuf], payloads: &[Payload]) -> Vec<Retained<NSPasteboardItem>> {
    let mut items = Vec::new();
    for (index, path) in files.iter().enumerate() {
        let item = NSPasteboardItem::new();
        if let Ok(url) = reqwest::Url::from_file_path(path) {
            item.setString_forType(
                &NSString::from_str(url.as_str()),
                &NSString::from_str("public.file-url"),
            );
        }
        if let Some(payload) = payloads.get(index) {
            add_representation(&item, payload);
        }
        items.push(item);
    }
    if items.is_empty() {
        for payload in payloads {
            let item = NSPasteboardItem::new();
            add_representation(&item, payload);
            items.push(item);
        }
    }
    items
}
fn add_representation(item: &NSPasteboardItem, payload: &Payload) {
    match payload {
        Payload::Text { plain, html, rtf } => {
            item.setString_forType(
                &NSString::from_str(plain),
                &NSString::from_str("public.utf8-plain-text"),
            );
            if let Some(html) = html {
                item.setData_forType(
                    &NSData::with_bytes(html),
                    &NSString::from_str("public.html"),
                );
            }
            if let Some(rtf) = rtf {
                item.setData_forType(&NSData::with_bytes(rtf), &NSString::from_str("public.rtf"));
            }
        }
        Payload::Image(png) => {
            item.setData_forType(&NSData::with_bytes(png), &NSString::from_str("public.png"));
        }
        _ => {}
    }
}
static NEXT_INCOMING: AtomicUsize = AtomicUsize::new(0);
fn receive_promises(
    board: &NSPasteboard,
    shared: Arc<Shared>,
    panel: bool,
    position: [f32; 2],
) -> bool {
    let classes = NSArray::from_slice(&[NSFilePromiseReceiver::class()]);
    let Some(objects) = (unsafe { board.readObjectsForClasses_options(&classes, None) }) else {
        return false;
    };
    if objects.is_empty() {
        return false;
    }
    let directory = shared.directory.join(format!(
        "promise-{}-{}",
        crate::safety::timestamp(),
        NEXT_INCOMING.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::create_dir_all(&directory) {
        shared.emit(Event::Error(e.to_string()));
        return false;
    }
    let destination = NSURL::fileURLWithPath(&NSString::from_str(&directory.to_string_lossy()));
    let receivers = objects
        .iter()
        .map(|o| unsafe { Retained::cast_unchecked::<NSFilePromiseReceiver>(o) })
        .collect::<Vec<_>>();
    let total = receivers.iter().map(|r| r.fileNames().len()).sum::<usize>();
    if total == 0 {
        return false;
    }
    let id = directory
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut jobs) = shared.jobs.lock() {
        jobs.insert(id.clone(), cancel.clone());
    }
    shared.emit(Event::Receiving {
        id: id.clone(),
        label: "正在接收系统文件承诺".into(),
        bytes: 0,
    });
    let pending = Arc::new(AtomicUsize::new(total));
    let paths = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let queue = NSOperationQueue::new();
    queue.setMaxConcurrentOperationCount(2);
    for receiver in receivers {
        let shared = shared.clone();
        let directory = directory.clone();
        let pending = pending.clone();
        let paths = paths.clone();
        let errors = errors.clone();
        let id = id.clone();
        let cancel = cancel.clone();
        let callback = RcBlock::new(move |url: NonNull<NSURL>, error: *mut NSError| {
            if !error.is_null() {
                if let Ok(mut errors) = errors.lock() {
                    errors.push(unsafe { (*error).localizedDescription().to_string() });
                }
            } else if let Some(path) = unsafe { url.as_ref() }.path() {
                let path = PathBuf::from(path.to_string());
                if std::fs::metadata(&path)
                    .is_ok_and(|m| m.len() > shared.quota.load(Ordering::Acquire))
                {
                    if let Ok(mut errors) = errors.lock() {
                        errors.push("承诺文件超过内容库空间限额".into());
                    }
                } else if let Ok(mut files) = paths.lock() {
                    files.push(path);
                }
            }
            if pending.fetch_sub(1, Ordering::AcqRel) == 1 && shared.alive.load(Ordering::Acquire) {
                let files = paths.lock().map(|p| p.clone()).unwrap_or_default();
                let mut errors = errors.lock().map(|e| e.clone()).unwrap_or_default();
                let files = if cancel.load(Ordering::Acquire) {
                    errors.push("系统文件承诺收纳已取消".into());
                    vec![]
                } else {
                    files
                };
                shared.emit(Event::Dropped {
                    payloads: vec![Payload::TemporaryFiles {
                        paths: files,
                        directory: directory.clone(),
                        errors,
                    }],
                    panel,
                    position,
                });
                if let Ok(mut jobs) = shared.jobs.lock() {
                    jobs.remove(&id);
                }
                shared.emit(Event::ReceiveFinished(id.clone()));
            }
        });
        unsafe {
            receiver.receivePromisedFilesAtDestination_options_operationQueue_reader(
                &destination,
                &NSDictionary::new(),
                &queue,
                &callback,
            );
        }
    }
    true
}

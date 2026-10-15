//! Pi is a lazily started, package-owned RPC child. The pet owns its lifetime.
mod configuration;
mod process;
mod skills;
mod stream;
#[cfg(test)]
mod tests;

use crate::panel_ipc::{Message, Request};
use configuration::{Paths, validate_mcp};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

enum Work {
    Request(Request),
    Panel(bool),
    Stop,
}
pub struct Service {
    sender: Sender<Work>,
    pub messages: Receiver<Message>,
    worker: Option<JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
}
impl Service {
    pub fn new(data: &Path) -> Self {
        let paths = Paths::new(data);
        let (sender, receiver) = mpsc::channel();
        let (outgoing, messages) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = stopped.clone();
        let worker = thread::spawn(move || {
            let mut worker = Worker::new(paths, outgoing);
            worker.stopped = worker_stopped;
            worker.run(receiver);
        });
        Self {
            sender,
            messages,
            worker: Some(worker),
            stopped,
        }
    }
    pub fn request(&self, request: Request) -> Result<(), String> {
        self.sender
            .send(Work::Request(request))
            .map_err(|_| "Agent 服务已停止".into())
    }
    pub fn panel(&self, open: bool) {
        let _ = self.sender.send(Work::Panel(open));
    }
    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.sender.send(Work::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Runtime {
    child: Child,
    input: Option<ChildStdin>,
    records: Receiver<Value>,
    tree: Option<process::Tree>,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(input) = &mut self.input {
            let _ = input.write_all(b"{\"type\":\"clear_queue\"}\n{\"type\":\"abort\"}\n");
            let _ = input.flush();
        }
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        self.tree.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Worker {
    paths: Paths,
    output: Sender<Message>,
    runtime: Option<Runtime>,
    generation: u64,
    sequence: u64,
    serial: u64,
    responses: HashMap<String, Value>,
    state: Value,
    partial: Value,
    interactions: BTreeMap<String, Value>,
    busy: bool,
    starting: bool,
    panel: bool,
    idle: Instant,
    dirty: bool,
    pending_model: Option<Value>,
    remembered: Option<String>,
    tool_progress: BTreeMap<String, Value>,
    interaction_deadlines: BTreeMap<String, Instant>,
    stopped: Arc<AtomicBool>,
    error: Option<String>,
}
impl Worker {
    fn new(paths: Paths, output: Sender<Message>) -> Self {
        Self {
            paths,
            output,
            runtime: None,
            generation: 0,
            sequence: 0,
            serial: 0,
            responses: HashMap::new(),
            state: json!({}),
            partial: Value::Null,
            interactions: BTreeMap::new(),
            busy: false,
            starting: false,
            panel: false,
            idle: Instant::now(),
            dirty: false,
            pending_model: None,
            remembered: None,
            tool_progress: BTreeMap::new(),
            interaction_deadlines: BTreeMap::new(),
            stopped: Arc::new(AtomicBool::new(false)),
            error: None,
        }
    }
    fn run(mut self, requests: Receiver<Work>) {
        loop {
            if self.stopped.load(Ordering::Acquire) {
                break;
            }
            self.drain();
            let expired: Vec<_> = self
                .interaction_deadlines
                .iter()
                .filter(|(_, deadline)| Instant::now() >= **deadline)
                .map(|(id, _)| id.clone())
                .collect();
            for id in expired {
                self.interaction_deadlines.remove(&id);
                self.interactions.remove(&id);
                self.emit(json!({"type":"extension_ui_expired","id":id}));
            }
            if let Some(file) = self.state["sessionFile"].as_str()
                && Path::new(file).is_file()
                && self.remembered.as_deref() != Some(file)
                && self.paths.remember(file).is_ok()
            {
                self.remembered = Some(file.into());
            }
            if let Some(runtime) = &mut self.runtime
                && runtime.child.try_wait().ok().flatten().is_some()
            {
                self.runtime.take();
                self.busy = false;
                self.starting = false;
                self.partial = Value::Null;
                self.interactions.clear();
                self.error = Some(
                    "Pi 进程已退出，任务可能未完成；不会自动重发。下次操作将恢复会话。".into(),
                );
                self.emit(json!({"type":"runtime_exit"}));
            }
            if self.runtime.is_some()
                && !self.busy
                && !self.starting
                && self.interactions.is_empty()
            {
                if self.dirty {
                    self.runtime.take();
                    self.dirty = false;
                    let pending = self.pending_model.take();
                    if let Err(error) = (|| {
                        self.start()?;
                        if let Some(model) = pending {
                            self.rpc(model)?;
                        }
                        self.state = self.rpc(json!({"type":"get_state"}))?;
                        Ok::<_, String>(())
                    })() {
                        self.error = Some(self.redact(&error));
                    }
                    self.emit(json!({"type":"configuration_applied"}));
                } else if !self.panel && self.idle.elapsed() >= Duration::from_secs(60) {
                    self.runtime.take();
                    self.emit(json!({"type":"runtime_idle_exit"}));
                }
            }
            let wait = if self.runtime.is_some() {
                Duration::from_millis(20)
            } else {
                Duration::from_secs(60)
            };
            match requests.recv_timeout(wait) {
                Ok(Work::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Work::Panel(open)) => {
                    self.panel = open;
                    self.idle = Instant::now();
                }
                Ok(Work::Request(request)) => {
                    if matches!(
                        request.command.as_str(),
                        "agent_connect" | "agent_rpc" | "agent_mcp_save"
                    ) {
                        self.idle = Instant::now();
                    }
                    let result = self
                        .handle(&request.command, &request.args)
                        .map(|mut value| {
                            if !matches!(
                                request.command.as_str(),
                                "agent_mcp_get" | "agent_storage"
                            ) {
                                Self::hide_credentials(&mut value);
                            }
                            value
                        });
                    let message = match result {
                        Ok(value) => Message::result(request.id, value),
                        Err(error) => {
                            let error = self.redact(&error);
                            self.error = Some(error.clone());
                            Message::error(request.id, error)
                        }
                    };
                    let _ = self.output.send(message);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    fn status(&self) -> Value {
        json!({"source":"bundled","version":process::VERSION,"path":process::runtime_directory().ok(),"running":self.runtime.is_some(),"busy":self.busy || self.starting,"pendingConfiguration":self.dirty,"error":self.error,"generation":self.generation,"sequence":self.sequence})
    }
    fn hide_credentials(value: &mut Value) {
        match value {
            Value::Object(object) => {
                for name in [
                    "apiKey",
                    "headers",
                    "authorization",
                    "token",
                    "password",
                    "accessToken",
                    "refreshToken",
                ] {
                    object.remove(name);
                }
                for child in object.values_mut() {
                    Self::hide_credentials(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    Self::hide_credentials(child);
                }
            }
            _ => {}
        }
    }
    fn redact(&self, text: &str) -> String {
        fn secrets(value: &Value, values: &mut Vec<String>, sensitive: bool) {
            match value {
                Value::Object(object) => {
                    for (key, v) in object {
                        secrets(
                            v,
                            values,
                            sensitive
                                || [
                                    "key",
                                    "token",
                                    "secret",
                                    "password",
                                    "authorization",
                                    "env",
                                    "headers",
                                ]
                                .iter()
                                .any(|needle| key.to_lowercase().contains(needle)),
                        );
                    }
                }
                Value::Array(array) => {
                    for v in array {
                        secrets(v, values, sensitive);
                    }
                }
                Value::String(s) if sensitive && s.len() >= 4 => values.push(s.clone()),
                _ => {}
            }
        }
        let mut values = vec![];
        for name in ["auth.json", "models.json", "mcp.json"] {
            if let Ok(value) = self.paths.read(name) {
                secrets(&value, &mut values, false);
            }
        }
        values.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let mut result = text.to_owned();
        for secret in values {
            result = result.replace(&secret, "[已隐藏]");
        }
        result
    }
    fn emit(&mut self, event: Value) {
        self.sequence += 1;
        let _ = self.output.send(Message::event("agent-event", json!({"generation":self.generation,"sequence":self.sequence,"sessionId":self.state["sessionId"],"event":event,"runtime":self.status()})));
    }
    fn record(&mut self, record: Value) {
        if record["type"] == "response" {
            if let Some(id) = record["id"].as_str() {
                self.responses.insert(id.into(), record);
            }
            return;
        }
        match record["type"].as_str().unwrap_or("") {
            "agent_start" => {
                self.busy = true;
                self.starting = false;
                self.state["isStreaming"] = json!(true);
            }
            "agent_settled" => {
                self.busy = false;
                self.starting = false;
                self.state["isStreaming"] = json!(false);
                self.partial = Value::Null;
                self.idle = Instant::now();
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                self.partial = record["message"].clone()
            }
            "message_update" => stream::update(&mut self.partial, &record),
            "tool_execution_start" | "tool_execution_update" => {
                if let Some(id) = record["toolCallId"].as_str() {
                    self.tool_progress.insert(id.into(), record.clone());
                }
            }
            "tool_execution_end" => {
                if let Some(id) = record["toolCallId"].as_str() {
                    self.tool_progress.remove(id);
                }
            }
            "message_end" => self.partial = Value::Null,
            "extension_ui_request" => {
                if let Some(id) = record["id"].as_str()
                    && ["select", "confirm", "input", "editor"]
                        .contains(&record["method"].as_str().unwrap_or(""))
                {
                    self.interactions.insert(id.into(), record.clone());
                    if let Some(timeout) = record["timeout"].as_u64() {
                        self.interaction_deadlines.insert(
                            id.into(),
                            Instant::now() + Duration::from_millis(timeout.min(86_400_000)),
                        );
                    }
                }
            }
            _ => {}
        }
        // Pi diagnostics on stderr are discarded; transport events never include credentials.
        self.emit(record);
    }
    fn drain(&mut self) {
        let records = self
            .runtime
            .as_ref()
            .map(|r| r.records.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for record in records {
            self.record(record);
        }
    }
    fn rpc(&mut self, mut command: Value) -> Result<Value, String> {
        self.serial += 1;
        let id = format!("mochi-{}-{}", self.generation, self.serial);
        command["id"] = json!(id);
        let input = self
            .runtime
            .as_mut()
            .and_then(|r| r.input.as_mut())
            .ok_or("Pi 未运行")?;
        serde_json::to_writer(&mut *input, &command)
            .map_err(|_| "Pi 连接已断开；任务未自动重发")?;
        input
            .write_all(b"\n")
            .and_then(|_| input.flush())
            .map_err(|_| "Pi 连接已断开；任务未自动重发")?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.stopped.load(Ordering::Acquire) {
                return Err("Agent 正在退出".into());
            }
            if let Some(reply) = self.responses.remove(&id) {
                if reply["success"] == true {
                    return Ok(reply["data"].clone());
                }
                return Err(reply["error"].as_str().unwrap_or("Pi 请求失败").into());
            }
            if Instant::now() >= deadline {
                self.runtime.take();
                return Err("Pi 请求超时，连接已关闭；任务未自动重发".into());
            }
            let record = self
                .runtime
                .as_ref()
                .ok_or("Pi 未运行")?
                .records
                .recv_timeout(Duration::from_millis(20));
            match record {
                Ok(record) => self.record(record),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Pi 连接已断开；任务未自动重发".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    fn start(&mut self) -> Result<(), String> {
        if self.runtime.is_some() {
            return Ok(());
        }
        self.paths.ensure()?;
        let executable = process::verify(&process::runtime_directory()?)?;
        let mut version = Command::new(&executable);
        version
            .env_remove("PI_PACKAGE_DIR")
            .env("PI_OFFLINE", "1")
            .env("PI_SKIP_VERSION_CHECK", "1");
        process::hide(&mut version);
        let version = version
            .arg("--version")
            .output()
            .map_err(|_| "无法执行包内 Pi")?;
        if !version.status.success()
            || String::from_utf8_lossy(&version.stdout).trim() != process::VERSION
        {
            return Err("包内 Pi 版本不匹配，需要 1.2.0".into());
        }
        let mut command = Command::new(executable);
        command.args(["--mode", "rpc", "--no-skills"]);
        for skill in skills::enabled(&self.paths)? {
            command.arg("--skill").arg(skill);
        }
        command
            .args([
                "--no-extensions",
                "--extension",
                "builtin:mcp",
                "--no-prompt-templates",
                "--no-context-files",
                "--no-approve",
                "--session-dir",
            ])
            .arg(self.paths.sessions())
            .env("PI_CODING_AGENT_DIR", self.paths.config())
            .env_remove("PI_PACKAGE_DIR")
            .env("PI_OFFLINE", "1")
            .env("PI_SKIP_VERSION_CHECK", "1")
            .env("PI_TELEMETRY", "0")
            .env("PI_CODING_AGENT_SESSION_DIR", self.paths.sessions())
            .current_dir(self.paths.workspace())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Parent Pi directory/session overrides must never select user resources.
        command.env_remove("PI_CODING_AGENT_SESSION_ID");
        if let Some(current) = self.paths.current() {
            command.arg("--session").arg(current);
        }
        process::hide(&mut command);
        let mut child = command.spawn().map_err(|_| "包内 Pi 启动失败")?;
        let tree = match process::Tree::attach(&child) {
            Ok(tree) => tree,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let input = child.stdin.take();
        let output = child.stdout.take().ok_or("Pi stdout 不可用")?;
        let (sender, records) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if let Ok(record) = serde_json::from_str::<Value>(&line)
                    && sender.send(record).is_err()
                {
                    break;
                }
            }
        });
        self.runtime = Some(Runtime {
            child,
            input,
            records,
            tree: Some(tree),
        });
        self.generation += 1;
        self.sequence = 0;
        self.responses.clear();
        self.partial = Value::Null;
        self.interactions.clear();
        self.interaction_deadlines.clear();
        self.tool_progress.clear();
        self.error = None;
        self.dirty = false;
        let result = (|| {
            self.state = self.rpc(json!({"type":"get_state"}))?;
            let messages = self.rpc(json!({"type":"get_messages"}))?;
            let models = self.rpc(json!({"type":"get_available_models"}))?;
            self.rpc(json!({"type":"get_tree"}))?;
            self.rpc(json!({"type":"get_commands"}))?;
            if !self.state["sessionId"].is_string()
                || !self.state["isStreaming"].is_boolean()
                || !messages["messages"].is_array()
                || !models["models"].is_array()
            {
                return Err("包内 Pi RPC 能力检查失败".into());
            }
            Ok(())
        })();
        if result.is_err() {
            self.runtime.take();
        }
        result
    }
    fn snapshot(&mut self) -> Result<Value, String> {
        if self.runtime.is_none() {
            return Ok(
                json!({"runtime":self.status(),"state":{},"messages":[],"partial":null,"interactions":[],"generation":self.generation,"sequence":self.sequence}),
            );
        }
        self.state = self.rpc(json!({"type":"get_state"}))?;
        let history = self.rpc(json!({"type":"get_messages"}))?;
        self.busy = self.state["isStreaming"].as_bool().unwrap_or(false)
            || self.state["isCompacting"].as_bool().unwrap_or(false)
            || self.busy;
        if let Some(file) = self.state["sessionFile"].as_str()
            && Path::new(file).is_file()
        {
            self.paths.remember(file)?;
        }
        Ok(
            json!({"runtime":self.status(),"state":self.state,"messages":history["messages"],"partial":self.partial,"toolProgress":self.tool_progress.values().collect::<Vec<_>>(),"interactions":self.interactions.values().collect::<Vec<_>>(),"generation":self.generation,"sequence":self.sequence}),
        )
    }
    fn changed(&mut self) {
        self.dirty = self.runtime.is_some();
    }
    fn handle(&mut self, command: &str, args: &Value) -> Result<Value, String> {
        match command {
            "agent_status" => Ok(self.status()),
            "agent_snapshot" => self.snapshot(),
            "agent_restore" => {
                if self.paths.current().is_some() {
                    self.start()?;
                }
                self.snapshot()
            }
            "agent_connect" => {
                self.start()?;
                self.snapshot()
            }
            "agent_sessions" => self.paths.sessions_list(),
            "agent_session_delete" => {
                if self.busy || self.starting || !self.interactions.is_empty() {
                    return Err("请先停止任务并结束交互，再删除对话".into());
                }
                let path = self
                    .paths
                    .session_path(args["sessionPath"].as_str().ok_or("会话路径缺失")?)?;
                let current = self.paths.current().as_ref() == Some(&path)
                    || self.state["sessionFile"]
                        .as_str()
                        .and_then(|file| self.paths.session_path(file).ok())
                        .as_ref()
                        == Some(&path);
                if current {
                    // Close Pi before removing its active JSONL so it cannot recreate it.
                    self.runtime.take();
                }
                std::fs::remove_file(&path).map_err(|_| "无法删除对话，请检查文件是否被占用")?;
                if current {
                    let index = self.paths.root.join("current.json");
                    match std::fs::remove_file(index) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => return Err("对话已删除，但无法清理当前会话标识".into()),
                    }
                    self.state = json!({});
                    self.remembered = None;
                    self.partial = Value::Null;
                    self.tool_progress.clear();
                    self.responses.clear();
                    self.pending_model = None;
                    self.dirty = false;
                    self.error = None;
                    self.generation += 1;
                    self.sequence = 0;
                }
                self.emit(json!({"type":"session_deleted"}));
                self.snapshot()
            }
            "agent_model_default" => {
                let provider = args["provider"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("请选择服务商")?;
                let model = args["modelId"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("请填写模型 ID")?;
                let mut settings = self.paths.read("settings.json")?;
                settings["defaultProvider"] = json!(provider);
                settings["defaultModel"] = json!(model);
                self.paths.save("settings.json", &settings)?;
                self.changed();
                Ok(self.status())
            }
            "agent_skills_list" => serde_json::to_value(skills::list(&self.paths)?)
                .map_err(|_| "无法读取 Skills".into()),
            "agent_skills_install" => {
                let source = args["path"].as_str().ok_or("请选择 Skill 文件夹或 ZIP")?;
                let name = skills::install(&self.paths, Path::new(source))?;
                self.changed();
                Ok(json!({"name":name,"runtime":self.status()}))
            }
            "agent_skills_toggle" => {
                skills::toggle(
                    &self.paths,
                    args["id"].as_str().ok_or("Skill 标识缺失")?,
                    args["enabled"].as_bool().ok_or("启用状态无效")?,
                )?;
                self.changed();
                Ok(self.status())
            }
            "agent_skills_uninstall" => {
                if self.busy || self.starting || !self.interactions.is_empty() {
                    return Err("请先停止任务并结束交互，再卸载 Skill".into());
                }
                skills::uninstall(&self.paths, args["id"].as_str().ok_or("Skill 标识缺失")?)?;
                self.changed();
                Ok(self.status())
            }
            "agent_open_directory" => {
                self.paths.ensure()?;
                let path = match args["directory"].as_str() {
                    Some("skills") => self.paths.skills(),
                    Some("workspace") => self.paths.workspace(),
                    Some("config") => self.paths.config(),
                    _ => return Err("目录不受支持".into()),
                };
                let mut opener = Command::new(if cfg!(windows) {
                    "explorer.exe"
                } else {
                    "open"
                });
                process::hide(&mut opener);
                opener.arg(path).spawn().map_err(|_| "无法打开目录")?;
                Ok(json!(true))
            }
            "agent_mcp_get" => {
                let mut value = self.paths.read("mcp.json")?;
                if value.as_object().is_some_and(|v| v.is_empty()) {
                    value = json!({"mcpServers":{}});
                }
                Ok(value)
            }
            "agent_mcp_save" => {
                validate_mcp(&args["value"])?;
                self.paths.save("mcp.json", &args["value"])?;
                self.changed();
                Ok(self.status())
            }
            "agent_storage" => self.storage(args),
            "agent_rpc" => {
                let kind = args["type"].as_str().ok_or("RPC type 缺失")?;
                if ![
                    "prompt",
                    "stop",
                    "new_session",
                    "switch_session",
                    "get_available_models",
                    "get_available_thinking_levels",
                    "set_model",
                    "set_thinking_level",
                    "extension_ui_response",
                ]
                .contains(&kind)
                {
                    return Err("RPC 命令不受支持".into());
                }
                if ["new_session", "switch_session"].contains(&kind)
                    && (self.busy || self.starting || !self.interactions.is_empty())
                {
                    return Err("任务结束后才能切换会话".into());
                }
                if kind == "set_model"
                    && (self.busy || self.starting || !self.interactions.is_empty())
                {
                    let mut settings = self.paths.read("settings.json")?;
                    settings["defaultProvider"] = args["provider"].clone();
                    settings["defaultModel"] = args["modelId"].clone();
                    self.paths.save("settings.json", &settings)?;
                    self.pending_model = Some(args.clone());
                    self.changed();
                    return Ok(json!({"deferred":true}));
                }
                self.start()?;
                if kind == "stop" {
                    // A dialog can be awaiting input inside the active extension. Resolve
                    // it before abort so cancellation cannot wait on a hidden panel.
                    let input = self
                        .runtime
                        .as_mut()
                        .and_then(|r| r.input.as_mut())
                        .ok_or("Pi 未运行")?;
                    for id in self.interactions.keys() {
                        serde_json::to_writer(
                            &mut *input,
                            &json!({"type":"extension_ui_response","id":id,"cancelled":true}),
                        )
                        .map_err(|_| "Pi 连接已断开")?;
                        input.write_all(b"\n").map_err(|_| "Pi 连接已断开")?;
                    }
                    input.flush().map_err(|_| "Pi 连接已断开")?;
                    self.interactions.clear();
                    self.interaction_deadlines.clear();
                    self.rpc(json!({"type":"clear_queue"}))?;
                    self.rpc(json!({"type":"abort"}))?;
                    return self.snapshot();
                }
                if kind == "extension_ui_response" {
                    let id = args["id"].as_str().ok_or("交互编号缺失")?;
                    if !self.interactions.contains_key(id) {
                        return Err("交互已结束或不存在".into());
                    }
                    let input = self
                        .runtime
                        .as_mut()
                        .and_then(|r| r.input.as_mut())
                        .ok_or("Pi 未运行")?;
                    serde_json::to_writer(&mut *input, args).map_err(|_| "Pi 连接已断开")?;
                    input
                        .write_all(b"\n")
                        .and_then(|_| input.flush())
                        .map_err(|_| "Pi 连接已断开")?;
                    self.interactions.remove(id);
                    self.interaction_deadlines.remove(id);
                    return Ok(json!(true));
                }
                let mut request = args.clone();
                if kind == "switch_session" {
                    let path = self
                        .paths
                        .session_path(args["sessionPath"].as_str().ok_or("会话路径缺失")?)?;
                    request["sessionPath"] = json!(path);
                }
                if kind == "prompt" {
                    if self.busy || self.starting {
                        return Err("正在执行，请先停止或等待任务结束".into());
                    }
                    if args["message"].as_str().is_none() {
                        return Err("消息必须为文本".into());
                    }
                    self.starting = true;
                }
                let result = self.rpc(request);
                if kind == "prompt" {
                    if let Ok(value) = &result {
                        if value["disposition"] == "handled" {
                            self.starting = false;
                        }
                    } else {
                        self.starting = false;
                    }
                }
                if ["new_session", "switch_session"].contains(&kind) {
                    result?;
                    self.partial = Value::Null;
                    self.interactions.clear();
                    return self.snapshot();
                }
                if kind == "set_model" {
                    let value = result?;
                    let mut settings = self.paths.read("settings.json")?;
                    settings["defaultProvider"] = args["provider"].clone();
                    settings["defaultModel"] = args["modelId"].clone();
                    self.paths.save("settings.json", &settings)?;
                    return Ok(value);
                }
                result
            }
            _ => Err("Agent 命令不受支持".into()),
        }
    }
    fn storage(&mut self, args: &Value) -> Result<Value, String> {
        let store = args["store"].as_str().ok_or("存储类型缺失")?;
        let operation = args["operation"].as_str().ok_or("存储操作缺失")?;
        let key = args["key"].as_str().unwrap_or("");
        if !["provider-keys", "custom-providers", "settings"].contains(&store)
            || !["get", "set", "delete", "keys", "has"].contains(&operation)
        {
            return Err("存储操作不受支持".into());
        }
        if store == "provider-keys" {
            let mut auth = self.paths.read("auth.json")?;
            match operation {
                "keys" => {
                    return Ok(json!(
                        auth.as_object()
                            .ok_or("auth.json 无效")?
                            .keys()
                            .collect::<Vec<_>>()
                    ));
                }
                "get" => {
                    return Ok(if auth.get(key).is_some() {
                        json!("configured")
                    } else {
                        Value::Null
                    });
                }
                "has" => return Ok(json!(auth.get(key).is_some())),
                "set" => {
                    let api_key = args["value"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or("API Key 不能为空")?;
                    auth[key] = json!({"type":"api_key","key":api_key});
                }
                "delete" => {
                    auth.as_object_mut().ok_or("auth.json 无效")?.remove(key);
                }
                _ => unreachable!(),
            }
            self.paths.save("auth.json", &auth)?;
            self.changed();
            return Ok(json!(true));
        }
        let mut ui = self.paths.read("web-ui.json")?;
        if !ui[store].is_object() {
            ui[store] = json!({});
        }
        match operation {
            "keys" => {
                return Ok(json!(
                    ui[store]
                        .as_object()
                        .ok_or("配置无效")?
                        .keys()
                        .collect::<Vec<_>>()
                ));
            }
            "get" => return Ok(ui[store][key].clone()),
            "has" => return Ok(json!(ui[store].get(key).is_some())),
            "set" => ui[store][key] = args["value"].clone(),
            "delete" => {
                ui[store].as_object_mut().ok_or("配置无效")?.remove(key);
            }
            _ => unreachable!(),
        }
        if store == "custom-providers" {
            let mut providers = json!({});
            for (id, provider) in ui[store].as_object_mut().ok_or("配置无效")? {
                if ![
                    "openai-completions",
                    "openai-responses",
                    "anthropic-messages",
                ]
                .contains(&provider["type"].as_str().unwrap_or(""))
                {
                    return Err("首版仅支持手动配置兼容端点".into());
                }
                if let Some(secret) = provider["apiKey"].as_str().filter(|s| !s.is_empty()) {
                    let mut auth = self.paths.read("auth.json")?;
                    auth[id] = json!({"type":"api_key","key":secret});
                    self.paths.save("auth.json", &auth)?;
                }
                provider
                    .as_object_mut()
                    .ok_or("端点配置无效")?
                    .remove("apiKey");
                providers[id] = json!({"baseUrl":provider["baseUrl"],"api":provider["type"],"models":provider["models"]});
                // OpenAI-compatible local servers often accept no credentials. Pi still
                // needs a nonempty credential marker to advertise their models.
                if self.paths.read("auth.json")?.get(id).is_none() {
                    providers[id]["apiKey"] = json!("mochi-local");
                }
            }
            let mut models = self.paths.read("models.json")?;
            models["providers"] = providers;
            self.paths.save("models.json", &models)?;
        }
        self.paths.save("web-ui.json", &ui)?;
        self.changed();
        Ok(json!(true))
    }
}

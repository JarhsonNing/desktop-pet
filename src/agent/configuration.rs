use crate::config::atomic_json;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
}
impl Paths {
    pub fn new(data: &Path) -> Self {
        Self {
            root: std::path::absolute(data)
                .unwrap_or_else(|_| data.to_path_buf())
                .join("agent"),
        }
    }
    pub fn config(&self) -> PathBuf {
        self.root.join("config")
    }
    pub fn file(&self, name: &str) -> PathBuf {
        self.config().join(name)
    }
    pub fn sessions(&self) -> PathBuf {
        self.root.join("sessions")
    }
    pub fn skills(&self) -> PathBuf {
        self.root.join("skills")
    }
    pub fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }
    pub fn ensure(&self) -> Result<(), String> {
        for p in [
            self.config(),
            self.sessions(),
            self.skills(),
            self.workspace(),
        ] {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn read(&self, name: &str) -> Result<Value, String> {
        match fs::read(self.file(name)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| format!("{name} JSON 无效")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
            Err(_) => Err(format!("无法读取 {name}")),
        }
    }
    pub fn save(&self, name: &str, value: &Value) -> Result<(), String> {
        if !value.is_object() {
            return Err("配置必须为 JSON 对象".into());
        }
        self.ensure()?;
        atomic_json(&self.file(name), value).map_err(|_| format!("无法保存 {name}"))
    }
    pub fn current(&self) -> Option<PathBuf> {
        let value: Value =
            serde_json::from_slice(&fs::read(self.root.join("current.json")).ok()?).ok()?;
        self.session_path(value["file"].as_str()?).ok()
    }
    pub fn remember(&self, path: &str) -> Result<(), String> {
        let path = self.session_path(path)?;
        atomic_json(&self.root.join("current.json"), &json!({"file":path}))
            .map_err(|_| "无法保存当前会话标识".into())
    }
    pub fn session_path(&self, path: &str) -> Result<PathBuf, String> {
        let path = Path::new(path).canonicalize().map_err(|_| "会话不存在")?;
        let root = self
            .sessions()
            .canonicalize()
            .map_err(|_| "会话目录不存在")?;
        if !path.starts_with(root) || path.extension().is_none_or(|e| e != "jsonl") {
            return Err("只能打开麻薯专用目录中的 Pi 会话".into());
        }
        Ok(path)
    }
    pub fn sessions_list(&self) -> Result<Value, String> {
        let mut items = vec![];
        if !self.sessions().exists() {
            return Ok(json!(items));
        }
        fn walk(directory: &Path, items: &mut Vec<Value>) -> Result<(), String> {
            for entry in fs::read_dir(directory).map_err(|_| "无法读取会话目录")? {
                let entry = entry.map_err(|_| "无法读取会话目录")?;
                let kind = entry.file_type().map_err(|_| "无法读取会话文件")?;
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    walk(&entry.path(), items)?;
                } else if entry.path().extension().is_some_and(|e| e == "jsonl") {
                    let mut title = String::new();
                    let mut id = String::new();
                    let mut count = 0;
                    for line in
                        BufReader::new(fs::File::open(entry.path()).map_err(|_| "无法读取会话")?)
                            .lines()
                    {
                        let Ok(line) = line else { break };
                        let Ok(v) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        if v["type"] == "session" {
                            id = v["id"].as_str().unwrap_or("").into();
                        }
                        if v["type"] == "session_info" {
                            title = v["name"].as_str().unwrap_or("").into();
                        }
                        if v["type"] == "message" {
                            count += 1;
                            if title.is_empty() && v["message"]["role"] == "user" {
                                let content = &v["message"]["content"];
                                let text = content
                                    .as_str()
                                    .or_else(|| {
                                        content.as_array()?.iter().find_map(|b| b["text"].as_str())
                                    })
                                    .unwrap_or("图片会话");
                                title = text.chars().take(60).collect();
                            }
                        }
                    }
                    let modified = entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |t| t.as_secs());
                    items.push(json!({"id":id,"file":entry.path(),"title":if title.is_empty(){"新会话"}else{&title},"messageCount":count,"modified":modified}));
                }
            }
            Ok(())
        }
        walk(&self.sessions(), &mut items)?;
        items.sort_by_key(|v| std::cmp::Reverse(v["modified"].as_u64().unwrap_or(0)));
        Ok(json!(items))
    }
}

pub fn validate_mcp(value: &Value) -> Result<(), String> {
    let servers = value["mcpServers"]
        .as_object()
        .ok_or("MCP 配置需要 mcpServers 对象")?;
    let mut names = std::collections::HashSet::new();
    for (name, server) in servers {
        if !names.insert(name.replace('-', "_")) {
            return Err("MCP 服务名在规范化后重复".into());
        }
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err("MCP 服务名只能包含字母、数字、下划线和连字符".into());
        }
        if !server.is_object()
            || (server["command"].as_str().is_none() == server["url"].as_str().is_none())
        {
            return Err("MCP 服务需要 command 或 url，二选一".into());
        }
        if server["command"]
            .as_str()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err("MCP 启动程序不能为空".into());
        }
        if let Some(t) = server["type"].as_str()
            && !["stdio", "http", "streamable-http"].contains(&t)
        {
            return Err("MCP type 不受支持".into());
        }
        if let Some(value) = server.get("enabled")
            && !value.is_boolean()
        {
            return Err("MCP enabled 必须为布尔值".into());
        }
        let exposures = [
            "direct",
            "deferred",
            "codemode",
            "codemode-deferred",
            "hidden",
        ];
        if let Some(exposure) = server.get("exposure")
            && !exposures.contains(&exposure.as_str().unwrap_or(""))
        {
            return Err("MCP exposure 不受支持".into());
        }
        if let Some(overrides) = server.get("toolExposure")
            && overrides.as_object().is_none_or(|v| {
                v.values()
                    .any(|exposure| !exposures.contains(&exposure.as_str().unwrap_or("")))
            })
        {
            return Err("MCP toolExposure 配置无效".into());
        }
        if let Some(args) = server.get("args")
            && args
                .as_array()
                .is_none_or(|a| a.iter().any(|v| !v.is_string()))
        {
            return Err("MCP args 必须为字符串数组".into());
        }
        for field in ["env", "headers"] {
            if let Some(values) = server.get(field)
                && values
                    .as_object()
                    .is_none_or(|v| v.values().any(|v| !v.is_string()))
            {
                return Err(format!("MCP {field} 必须为字符串映射"));
            }
        }
        if let Some(url) = server["url"].as_str() {
            let url = reqwest::Url::parse(url).map_err(|_| "MCP URL 无效")?;
            if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none() {
                return Err("MCP URL 必须使用 HTTP 或 HTTPS".into());
            }
        }
    }
    Ok(())
}

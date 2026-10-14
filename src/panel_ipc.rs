//! Private parent/child pipe protocol. No TCP listener or background WebView.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub command: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Option<u64>,
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub error: Option<String>,
}

impl Message {
    pub fn result(id: u64, value: Value) -> Self {
        Self {
            id: Some(id),
            event: None,
            value,
            error: None,
        }
    }
    pub fn error(id: u64, error: impl Into<String>) -> Self {
        Self {
            id: Some(id),
            event: None,
            value: Value::Null,
            error: Some(error.into()),
        }
    }
    pub fn event(event: &str, value: Value) -> Self {
        Self {
            id: None,
            event: Some(event.into()),
            value,
            error: None,
        }
    }
}

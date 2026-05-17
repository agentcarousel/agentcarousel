use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct JsonOutput {
    pub ok: bool,
    pub command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonError>,
}

#[derive(Debug, Serialize)]
pub struct JsonError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

impl JsonOutput {
    pub fn ok(command: &'static str, data: impl Serialize) -> Self {
        Self {
            ok: true,
            command,
            data: Some(serde_json::to_value(data).unwrap_or(Value::Null)),
            error: None,
        }
    }

    pub fn err(command: &'static str, error: JsonError) -> Self {
        Self {
            ok: false,
            command,
            data: None,
            error: Some(error),
        }
    }

    pub fn print(&self) {
        println!(
            "{}",
            serde_json::to_string_pretty(self).unwrap_or_else(|_| r#"{"ok":false}"#.to_string())
        );
    }
}

impl JsonError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            suggestions: Vec::new(),
        }
    }

    pub fn with_suggestions(mut self, suggestions: impl IntoIterator<Item = String>) -> Self {
        self.suggestions = suggestions.into_iter().collect();
        self
    }
}

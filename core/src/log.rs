use humansize::{DECIMAL, format_size};
use std::fmt;
use tracing::Event;
use tracing::field::{Field, Visit};
pub const FILE_RESULT: &str = "squeezeit::file";

pub fn is_file_result(event: &Event<'_>) -> bool {
    event.metadata().target() == FILE_RESULT
}

#[derive(Default)]
pub struct Fields {
    message: String,
    path: Option<String>,
    reason: Option<String>,
    error: Option<String>,
    bytes_before: Option<u64>,
    bytes_after: Option<u64>,
}

impl Fields {
    pub fn of(event: &Event<'_>) -> Self {
        let mut fields = Self::default();
        event.record(&mut fields);
        fields
    }

    pub fn render(&self) -> String {
        let path = self.path.as_deref().unwrap_or_default();
        match (self.bytes_before, self.bytes_after) {
            (Some(before), Some(after)) => format!(
                "{:9} {path}  {} -> {}",
                self.message,
                format_size(before, DECIMAL),
                format_size(after, DECIMAL),
            ),
            _ => match self.reason.as_deref().or(self.error.as_deref()) {
                Some(extra) => format!("{:9} {path}  ({extra})", self.message),
                None => format!("{:9} {path}", self.message),
            },
        }
    }
}

impl Visit for Fields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "bytes_before" => self.bytes_before = Some(value),
            "bytes_after" => self.bytes_after = Some(value),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "path" => self.path = Some(value.to_owned()),
            "reason" => self.reason = Some(value.to_owned()),
            "error" => self.error = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let rendered = format!("{value:?}");
        match field.name() {
            "message" => self.message = rendered,
            "path" => drop(self.path.get_or_insert(rendered)),
            "reason" => drop(self.reason.get_or_insert(rendered)),
            "error" => drop(self.error.get_or_insert(rendered)),
            _ => {}
        }
    }
}

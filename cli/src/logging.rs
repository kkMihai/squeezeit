use std::fmt;

use humansize::{DECIMAL, format_size};
use indicatif::ProgressBar;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context as LayerContext;

pub struct BarLayer {
    pub bar: ProgressBar,
    pub verbose: bool,
}

impl<S: Subscriber> Layer<S> for BarLayer {
    fn on_event(&self, event: &Event<'_>, _: LayerContext<'_, S>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);

        if fields.path.is_some() {
            self.bar.inc(1);
        }
        let is_failure = *event.metadata().level() == Level::ERROR;
        if is_failure || (self.verbose && fields.path.is_some()) {
            self.bar.println(fields.render());
        }
    }
}

#[derive(Default)]
struct EventFields {
    message: String,
    path: Option<String>,
    reason: Option<String>,
    error: Option<String>,
    bytes_before: Option<u64>,
    bytes_after: Option<u64>,
}

impl EventFields {
    fn render(&self) -> String {
        let path = self.path.as_deref().unwrap_or_default();
        if let (Some(before), Some(after)) = (self.bytes_before, self.bytes_after) {
            format!(
                "  {:9} {path}  {} -> {}",
                self.message,
                format_size(before, DECIMAL),
                format_size(after, DECIMAL),
            )
        } else if let Some(extra) = self.reason.as_deref().or(self.error.as_deref()) {
            format!("  {:9} {path}  ({extra})", self.message)
        } else {
            format!("  {:9} {path}", self.message)
        }
    }
}

impl Visit for EventFields {
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

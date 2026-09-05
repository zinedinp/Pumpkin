//! A `tracing` layer that mirrors log lines into the window's scrollback.

use std::sync::Arc;

use pumpkin_gui::{LogLevel, LogRing};
use tracing::Subscriber;
use tracing_subscriber::Layer;

pub struct GuiLogLayer {
    ring: Arc<LogRing>,
}

/// Builds the layer for [`crate::init_logger_with`].
#[must_use]
pub fn layer(ring: Arc<LogRing>) -> crate::ExtraLayer {
    Box::new(GuiLogLayer { ring })
}

impl<S: Subscriber> Layer<S> for GuiLogLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();

        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        if visitor.0.is_empty() {
            return;
        }

        self.ring.push(
            level_of(*metadata.level()),
            metadata.target().to_owned(),
            super::strip_ansi(&visitor.0),
        );
    }
}

const fn level_of(level: tracing::Level) -> LogLevel {
    match level {
        tracing::Level::TRACE => LogLevel::Trace,
        tracing::Level::DEBUG => LogLevel::Debug,
        tracing::Level::WARN => LogLevel::Warn,
        tracing::Level::ERROR => LogLevel::Error,
        tracing::Level::INFO => LogLevel::Info,
    }
}

/// Pulls out the `message` field, matching the file logger
struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
            // `record_debug` quotes strings; the file logger strips them the same way.
            if self.0.starts_with('"') && self.0.ends_with('"') && self.0.len() >= 2 {
                self.0 = self.0[1..self.0.len() - 1].to_owned();
            }
        }
    }
}

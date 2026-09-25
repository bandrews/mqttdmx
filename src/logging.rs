// ABOUTME: Sets up logging to standard output, as text or JSON, at the configured level.
// ABOUTME: RUST_LOG, when set, replaces the configured level with a full tracing filter.

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

use crate::config::{LogFormat, LoggingConfig};

pub fn init(config: &LoggingConfig) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(config.level.name()));
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(std::io::stdout().is_terminal())
        .with_writer(std::io::stdout);
    match config.format {
        LogFormat::Text => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}

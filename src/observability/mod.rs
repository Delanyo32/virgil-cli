pub mod sampler;

use std::sync::atomic::{AtomicBool, Ordering};
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Compact,
    Json,
}

pub fn init(verbosity: u8, quiet: bool, format: LogFormat) {
    if INITIALIZED.swap(true, Ordering::SeqCst) {
        return;
    }

    let level = if quiet {
        "error"
    } else {
        match verbosity {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };

    let filter = EnvFilter::try_from_env("VIRGIL_LOG")
        .unwrap_or_else(|_| EnvFilter::new(format!("virgil_cli={level},warn")));

    // JSON format is meant for machine consumption (serve mode, log shippers) —
    // progress bars would corrupt that. Only enable indicatif for compact output
    // attached to a TTY.
    let want_bars = matches!(format, LogFormat::Compact)
        && !quiet
        && std::io::IsTerminal::is_terminal(&std::io::stderr());

    let registry = tracing_subscriber::registry().with(filter);

    if want_bars {
        let indicatif_layer = IndicatifLayer::new();
        let writer = indicatif_layer.get_stderr_writer();
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_target(false)
            .compact();
        registry.with(fmt_layer).with(indicatif_layer).init();
    } else {
        match format {
            LogFormat::Compact => {
                let layer = tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_target(false)
                    .compact();
                registry.with(layer).init();
            }
            LogFormat::Json => {
                let layer = tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_target(true)
                    .json();
                registry.with(layer).init();
            }
        }
    }
}

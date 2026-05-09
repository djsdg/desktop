use tracing_appender::non_blocking::WorkerGuard;

/// Keeps non-blocking file writers alive for as long as the owning process needs them.
#[derive(Debug, Default)]
pub struct LoggingGuard {
    guards: Vec<WorkerGuard>,
}

impl LoggingGuard {
    /// Creates a guard that owns the writer lifetimes for every active file-backed sink.
    pub fn new(guards: Vec<WorkerGuard>) -> Self {
        Self { guards }
    }

    /// Reports whether the active logging setup owns any file-backed writers.
    pub fn has_file_writer(&self) -> bool {
        !self.guards.is_empty()
    }
}

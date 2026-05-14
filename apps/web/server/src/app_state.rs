use crate::bootstrap::BootstrapProjectApi;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Holds the shared state that HTTP handlers need to serve requests.
#[derive(Clone)]
pub struct AppState {
    project_api: Arc<BootstrapProjectApi>,
    ready: Arc<AtomicBool>,
}

impl AppState {
    /// Creates one shared application state value with readiness disabled until bootstrap completes.
    pub fn new(project_api: Arc<BootstrapProjectApi>) -> Self {
        Self {
            project_api,
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the shared project API that routes delegate into.
    pub fn project_api(&self) -> &Arc<BootstrapProjectApi> {
        &self.project_api
    }

    /// Marks the runtime as ready after bootstrap finishes successfully.
    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Reports whether bootstrap has completed successfully for readiness checks.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct AppLifecycle {
    quitting: AtomicBool,
}

impl AppLifecycle {
    pub fn request_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }
}

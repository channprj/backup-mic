use std::{thread, time::Duration};

pub trait Clock: Send + Sync {
    fn sleep(&self, duration: Duration);
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

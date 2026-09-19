use std::collections::HashSet;
use std::time::{Duration, Instant};

pub struct PanicHotkey {
    combo: Vec<u16>,
    held: HashSet<u16>,
}

impl PanicHotkey {
    pub fn new(combo: Vec<u16>) -> Self {
        Self { combo, held: HashSet::new() }
    }

    pub fn observe(&mut self, code: u16, pressed: bool) -> bool {
        if pressed {
            self.held.insert(code);
        } else {
            self.held.remove(&code);
            return false;
        }
        !self.combo.is_empty() && self.combo.iter().all(|c| self.held.contains(c))
    }

    pub fn reset(&mut self) {
        self.held.clear();
    }
}

pub struct Watchdog {
    last_activity: Instant,
    timeout: Duration,
}

impl Watchdog {
    pub fn new(timeout: Duration) -> Self {
        Self { last_activity: Instant::now(), timeout }
    }

    pub fn poke(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.last_activity) > self.timeout
    }
}

use crate::frame::Frame;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

pub const RETRANSMIT_INTERVAL: Duration = Duration::from_millis(5);
pub const RETRANSMIT_GIVEUP: Duration = Duration::from_millis(200);

fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

#[derive(Debug, Default)]
pub struct SeqCounter(u32);

impl SeqCounter {
    pub fn next_seq(&mut self) -> u32 {
        let seq = self.0;
        self.0 = self.0.wrapping_add(1);
        seq
    }
}

struct Pending {
    packet: Vec<u8>,
    first_sent: Instant,
    last_sent: Instant,
}

#[derive(Default)]
pub struct ReliableSender {
    pending: BTreeMap<u32, Pending>,
}

impl ReliableSender {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn note_sent(&mut self, seq: u32, packet: Vec<u8>, now: Instant) {
        self.pending.insert(seq, Pending { packet, first_sent: now, last_sent: now });
    }

    pub fn on_ack(&mut self, seq: u32) {
        self.pending.remove(&seq);
    }

    pub fn due_retransmits(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let expired: Vec<u32> = self
            .pending
            .iter()
            .filter(|(_, p)| now.duration_since(p.first_sent) > RETRANSMIT_GIVEUP)
            .map(|(&seq, _)| seq)
            .collect();
        for seq in expired {
            self.pending.remove(&seq);
        }

        let mut due = Vec::new();
        for pkt in self.pending.values_mut() {
            if now.duration_since(pkt.last_sent) >= RETRANSMIT_INTERVAL {
                pkt.last_sent = now;
                due.push(pkt.packet.clone());
            }
        }
        due
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[derive(Default)]
pub struct ReliableReceiver {
    next_apply: u32,
    buffered: HashMap<u32, Frame>,
}

impl ReliableReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_receive(&mut self, seq: u32, frame: Frame) -> Vec<Frame> {
        if seq_lt(seq, self.next_apply) {
            return Vec::new();
        }
        if seq != self.next_apply {
            self.buffered.entry(seq).or_insert(frame);
            return Vec::new();
        }
        let mut ready = vec![frame];
        self.next_apply = self.next_apply.wrapping_add(1);
        while let Some(f) = self.buffered.remove(&self.next_apply) {
            ready.push(f);
            self.next_apply = self.next_apply.wrapping_add(1);
        }
        ready
    }
}

#[derive(Default)]
pub struct UnreliableReceiver {
    last_applied: Option<u32>,
}

impl UnreliableReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn accept(&mut self, seq: u32) -> bool {
        match self.last_applied {
            Some(last) if !seq_lt(last, seq) => false,
            _ => {
                self.last_applied = Some(seq);
                true
            }
        }
    }
}


use crate::crypto::SessionKeys;
use crate::frame::Frame;
use crate::reliable::{ReliableReceiver, ReliableSender, SeqCounter, UnreliableReceiver};
use crate::wire::{self, Channel, WireError};
use std::time::Instant;

#[derive(Debug, Default)]
pub struct Received {
    pub ready: Vec<Frame>,
    pub reply: Option<Vec<u8>>,
}

pub struct Session {
    keys: SessionKeys,

    reliable_tx_seq: SeqCounter,
    reliable_tx: ReliableSender,
    reliable_rx: ReliableReceiver,

    unreliable_tx_seq: SeqCounter,
    unreliable_rx: UnreliableReceiver,

    control_tx_seq: SeqCounter,
    control_rx: UnreliableReceiver,
}

impl Session {
    pub fn new(keys: SessionKeys) -> Self {
        Self {
            keys,
            reliable_tx_seq: SeqCounter::default(),
            reliable_tx: ReliableSender::new(),
            reliable_rx: ReliableReceiver::new(),
            unreliable_tx_seq: SeqCounter::default(),
            unreliable_rx: UnreliableReceiver::new(),
            control_tx_seq: SeqCounter::default(),
            control_rx: UnreliableReceiver::new(),
        }
    }

    pub fn send_reliable(&mut self, frame: &Frame, now: Instant) -> Vec<u8> {
        let seq = self.reliable_tx_seq.next_seq();
        let packet = encode(&self.keys, Channel::Reliable, seq, frame);
        self.reliable_tx.note_sent(seq, packet.clone(), now);
        packet
    }

    pub fn send_unreliable(&mut self, frame: &Frame) -> Vec<u8> {
        let seq = self.unreliable_tx_seq.next_seq();
        encode(&self.keys, Channel::Unreliable, seq, frame)
    }

    pub fn send_control(&mut self, frame: &Frame) -> Vec<u8> {
        let seq = self.control_tx_seq.next_seq();
        encode(&self.keys, Channel::Control, seq, frame)
    }

    pub fn due_retransmits(&mut self, now: Instant) -> Vec<Vec<u8>> {
        self.reliable_tx.due_retransmits(now)
    }

    pub fn pending_reliable_count(&self) -> usize {
        self.reliable_tx.pending_count()
    }

    pub fn on_datagram(&mut self, packet: &[u8]) -> Result<Received, WireError> {
        let (chan, seq, frame) = wire::decode(&self.keys.recv, packet)?;
        match chan {
            Channel::Reliable => {
                let reply = Some(self.send_control(&Frame::Ack { acked_seq: seq }));
                let ready = self.reliable_rx.on_receive(seq, frame);
                Ok(Received { ready, reply })
            }
            Channel::Unreliable => {
                let ready = if self.unreliable_rx.accept(seq) { vec![frame] } else { Vec::new() };
                Ok(Received { ready, reply: None })
            }
            Channel::Control => {
                if !self.control_rx.accept(seq) {
                    return Ok(Received::default());
                }
                if let Frame::Ack { acked_seq } = frame {
                    self.reliable_tx.on_ack(acked_seq);
                    Ok(Received::default())
                } else {
                    Ok(Received { ready: vec![frame], reply: None })
                }
            }
        }
    }
}

fn encode(keys: &SessionKeys, chan: Channel, seq: u32, frame: &Frame) -> Vec<u8> {
    wire::encode(&keys.send, chan, seq, frame).expect("encoding a well-formed Frame cannot fail")
}


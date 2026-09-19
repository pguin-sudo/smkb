use crate::event::{HelloInfo, InputEvent, WelcomeInfo};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Frame {
    Hello(HelloInfo),
    Welcome(WelcomeInfo),
    Event(InputEvent),
    Ack { acked_seq: u32 },
    Ping { nonce: u64 },
    Pong { nonce: u64 },
    Bye,
}

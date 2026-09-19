use crate::frame::Frame;
use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, Payload},
};

pub const WIRE_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 6;
pub const TAG_LEN: usize = 16;
pub const MAX_PACKET_LEN: usize = 1200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Reliable = 0,
    Unreliable = 1,
    Control = 2,
}

impl Channel {
    fn to_u8(self) -> u8 {
        self as u8
    }

    fn from_u8(b: u8) -> Result<Self, WireError> {
        match b {
            0 => Ok(Channel::Reliable),
            1 => Ok(Channel::Unreliable),
            2 => Ok(Channel::Control),
            other => Err(WireError::BadChannel(other)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("packet too short: {0} bytes")]
    TooShort(usize),
    #[error("unsupported wire version: {0}")]
    BadVersion(u8),
    #[error("unknown channel id: {0}")]
    BadChannel(u8),
    #[error("AEAD authentication failed (wrong key, corrupted, or replayed packet)")]
    Crypto,
    #[error("frame (de)serialization failed: {0}")]
    Codec(#[from] postcard::Error),
}

fn build_nonce(chan: Channel, seq: u32) -> Nonce {
    let mut bytes = [0u8; 12];
    bytes[7] = chan.to_u8();
    bytes[8..12].copy_from_slice(&seq.to_be_bytes());
    Nonce::from(bytes)
}

fn header(chan: Channel, seq: u32) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0] = WIRE_VERSION;
    h[1] = chan.to_u8();
    h[2..6].copy_from_slice(&seq.to_be_bytes());
    h
}

pub fn encode(cipher: &ChaCha20Poly1305, chan: Channel, seq: u32, frame: &Frame) -> Result<Vec<u8>, WireError> {
    let plaintext = postcard::to_allocvec(frame)?;
    let hdr = header(chan, seq);
    let nonce = build_nonce(chan, seq);
    let ciphertext = cipher
        .encrypt(&nonce, Payload { msg: &plaintext, aad: &hdr })
        .map_err(|_| WireError::Crypto)?;
    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(&hdr);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decode(cipher: &ChaCha20Poly1305, packet: &[u8]) -> Result<(Channel, u32, Frame), WireError> {
    if packet.len() < HEADER_LEN + TAG_LEN {
        return Err(WireError::TooShort(packet.len()));
    }
    let hdr = &packet[..HEADER_LEN];
    if hdr[0] != WIRE_VERSION {
        return Err(WireError::BadVersion(hdr[0]));
    }
    let chan = Channel::from_u8(hdr[1])?;
    let seq = u32::from_be_bytes(hdr[2..6].try_into().unwrap());
    let nonce = build_nonce(chan, seq);
    let ciphertext = &packet[HEADER_LEN..];
    let plaintext = cipher
        .decrypt(&nonce, Payload { msg: ciphertext, aad: hdr })
        .map_err(|_| WireError::Crypto)?;
    let frame: Frame = postcard::from_bytes(&plaintext)?;
    Ok((chan, seq, frame))
}

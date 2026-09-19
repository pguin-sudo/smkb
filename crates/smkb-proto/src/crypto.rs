use chacha20poly1305::{ChaCha20Poly1305, KeyInit};

const NOISE_PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("noise handshake error: {0}")]
    Noise(#[from] snow::Error),
    #[error("handshake not finished yet")]
    NotFinished,
    #[error("psk file must contain exactly 32 raw bytes or 64 hex characters, got {0} bytes")]
    BadPsk(usize),
}

pub type Psk = [u8; 32];

pub fn parse_psk(raw: &[u8]) -> Result<Psk, CryptoError> {
    if raw.len() == 32 {
        let mut psk = [0u8; 32];
        psk.copy_from_slice(raw);
        return Ok(psk);
    }
    let trimmed: Vec<u8> = raw
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if trimmed.len() == 64 {
        let mut psk = [0u8; 32];
        for (i, chunk) in trimmed.chunks_exact(2).enumerate() {
            let hi = hex_nibble(chunk[0]).ok_or(CryptoError::BadPsk(raw.len()))?;
            let lo = hex_nibble(chunk[1]).ok_or(CryptoError::BadPsk(raw.len()))?;
            psk[i] = (hi << 4) | lo;
        }
        return Ok(psk);
    }
    Err(CryptoError::BadPsk(raw.len()))
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub struct SessionKeys {
    pub send: ChaCha20Poly1305,
    pub recv: ChaCha20Poly1305,
}

pub struct Handshake {
    state: snow::HandshakeState,
    initiator: bool,
}

impl Handshake {
    pub fn new_initiator(psk: &Psk) -> Result<Self, CryptoError> {
        let params = NOISE_PATTERN.parse().expect("static pattern string is valid");
        let state = snow::Builder::new(params).psk(0, psk)?.build_initiator()?;
        Ok(Self { state, initiator: true })
    }

    pub fn new_responder(psk: &Psk) -> Result<Self, CryptoError> {
        let params = NOISE_PATTERN.parse().expect("static pattern string is valid");
        let state = snow::Builder::new(params).psk(0, psk)?.build_responder()?;
        Ok(Self { state, initiator: false })
    }

    pub fn is_finished(&self) -> bool {
        self.state.is_handshake_finished()
    }

    pub fn write_message(&mut self, buf: &mut [u8]) -> Result<usize, CryptoError> {
        Ok(self.state.write_message(&[], buf)?)
    }

    pub fn read_message(&mut self, msg: &[u8]) -> Result<(), CryptoError> {
        let mut discard = [0u8; 256];
        self.state.read_message(msg, &mut discard)?;
        Ok(())
    }

    pub fn finish(mut self) -> Result<SessionKeys, CryptoError> {
        if !self.state.is_handshake_finished() {
            return Err(CryptoError::NotFinished);
        }
        let (k_initiator, k_responder) = self.state.dangerously_get_raw_split();
        let (send_bytes, recv_bytes) =
            if self.initiator { (k_initiator, k_responder) } else { (k_responder, k_initiator) };
        Ok(SessionKeys {
            send: ChaCha20Poly1305::new_from_slice(&send_bytes).expect("32-byte key"),
            recv: ChaCha20Poly1305::new_from_slice(&recv_bytes).expect("32-byte key"),
        })
    }
}

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use std::fmt;

pub const HEADER_LEN: usize = 34;
pub const TAG_LEN: usize = 16;
pub const MAX_DATAGRAM_LEN: usize = 1200;
pub const MAX_PAYLOAD_LEN: usize = MAX_DATAGRAM_LEN - HEADER_LEN - TAG_LEN;
pub const HOST_TO_RECEIVER: u32 = 0x414e_5048; // ANPH
pub const RECEIVER_TO_HOST: u32 = 0x414e_5052; // ANPR

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PacketType {
    Format = 1,
    Audio = 2,
}

impl TryFrom<u8> for PacketType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Format),
            2 => Ok(Self::Audio),
            _ => Err(ProtocolError::UnsupportedType),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub packet_type: PacketType,
    pub session_id: u64,
    pub sequence: u64,
    pub sample_frame_index: u64,
    pub payload_len: u16,
}

impl Header {
    fn encode(self) -> [u8; HEADER_LEN] {
        let mut bytes = [0; HEADER_LEN];
        bytes[0..4].copy_from_slice(b"ANPD");
        bytes[4] = 1;
        bytes[5] = self.packet_type as u8;
        bytes[6..8].copy_from_slice(&0u16.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.session_id.to_be_bytes());
        bytes[16..24].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[24..32].copy_from_slice(&self.sample_frame_index.to_be_bytes());
        bytes[32..34].copy_from_slice(&self.payload_len.to_be_bytes());
        bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

impl PcmFormat {
    pub fn encode(self) -> Result<[u8; 7], ProtocolError> {
        if self.sample_rate == 0 || self.channels == 0 {
            return Err(ProtocolError::InvalidFormat);
        }
        let mut payload = [0; 7];
        payload[0] = 1; // interleaved PCM16 little-endian
        payload[1..3].copy_from_slice(&self.channels.to_be_bytes());
        payload[3..7].copy_from_slice(&self.sample_rate.to_be_bytes());
        Ok(payload)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != 7 || payload[0] != 1 {
            return Err(ProtocolError::InvalidFormat);
        }
        let channels = u16::from_be_bytes([payload[1], payload[2]]);
        let sample_rate =
            u32::from_be_bytes(payload[3..7].try_into().expect("fixed format payload"));
        if sample_rate == 0 || channels == 0 {
            return Err(ProtocolError::InvalidFormat);
        }
        Ok(Self {
            sample_rate,
            channels,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ProtocolError {
    BadHeader,
    UnsupportedVersion,
    UnsupportedType,
    UnsupportedFlags,
    WrongSession,
    Oversize,
    BadLength,
    Authentication,
    Replay,
    InvalidFormat,
    SequenceExhausted,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::BadHeader => "invalid ANPD header",
                Self::UnsupportedVersion => "unsupported ANPD version",
                Self::UnsupportedType => "unsupported ANPD packet type",
                Self::UnsupportedFlags => "unsupported ANPD flags",
                Self::WrongSession => "unexpected ANPD session identity",
                Self::Oversize => "ANPD datagram exceeds the limit",
                Self::BadLength => "ANPD payload length does not match the datagram",
                Self::Authentication => "ANPD authentication failed",
                Self::Replay => "ANPD sequence is outside the replay window or duplicated",
                Self::InvalidFormat => "invalid PCM format or payload",
                Self::SequenceExhausted => "ANPD sequence space exhausted",
            }
        )
    }
}

impl std::error::Error for ProtocolError {}

pub struct PacketEncoder {
    cipher: Aes256Gcm,
    session_id: u64,
    direction: u32,
    next_sequence: Option<u64>,
}

impl PacketEncoder {
    pub fn new(key: &[u8; 32], session_id: u64, direction: u32) -> Self {
        Self::with_sequence(key, session_id, direction, 0)
    }

    pub fn with_sequence(key: &[u8; 32], session_id: u64, direction: u32, sequence: u64) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(key).expect("AES-256 key has fixed length"),
            session_id,
            direction,
            next_sequence: Some(sequence),
        }
    }

    pub fn encode_next(
        &mut self,
        packet_type: PacketType,
        sample_frame_index: u64,
        payload: &[u8],
    ) -> Result<Vec<u8>, ProtocolError> {
        let sequence = self.next_sequence.ok_or(ProtocolError::SequenceExhausted)?;
        self.next_sequence = sequence.checked_add(1);
        self.encode_at(sequence, packet_type, sample_frame_index, payload)
    }

    pub(crate) fn encode_at(
        &self,
        sequence: u64,
        packet_type: PacketType,
        sample_frame_index: u64,
        payload: &[u8],
    ) -> Result<Vec<u8>, ProtocolError> {
        if payload.len() > MAX_PAYLOAD_LEN || payload.len() > u16::MAX as usize {
            return Err(ProtocolError::Oversize);
        }
        let header = Header {
            packet_type,
            session_id: self.session_id,
            sequence,
            sample_frame_index,
            payload_len: payload.len() as u16,
        }
        .encode();
        let nonce = nonce(self.direction, sequence);
        let nonce = Nonce::try_from(nonce.as_slice()).expect("nonce has fixed length");
        let encrypted = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: payload,
                    aad: &header,
                },
            )
            .map_err(|_| ProtocolError::Authentication)?;
        let mut datagram = Vec::with_capacity(HEADER_LEN + encrypted.len());
        datagram.extend_from_slice(&header);
        datagram.extend_from_slice(&encrypted);
        debug_assert!(datagram.len() <= MAX_DATAGRAM_LEN);
        Ok(datagram)
    }
}

#[derive(Debug)]
pub struct DecodedPacket {
    pub header: Header,
    pub payload: Vec<u8>,
}

pub struct PacketDecoder {
    cipher: Aes256Gcm,
    session_id: u64,
    direction: u32,
    replay: ReplayWindow,
}

impl PacketDecoder {
    pub fn new(key: &[u8; 32], session_id: u64, direction: u32) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(key).expect("AES-256 key has fixed length"),
            session_id,
            direction,
            replay: ReplayWindow::default(),
        }
    }

    pub fn open(&self, datagram: &[u8]) -> Result<DecodedPacket, ProtocolError> {
        let (header, header_bytes, encrypted) = parse_header(datagram)?;
        if header.session_id != self.session_id {
            return Err(ProtocolError::WrongSession);
        }
        if !self.replay.would_accept(header.sequence) {
            return Err(ProtocolError::Replay);
        }
        let nonce = nonce(self.direction, header.sequence);
        let nonce = Nonce::try_from(nonce.as_slice()).expect("nonce has fixed length");
        let payload = self
            .cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: encrypted,
                    aad: header_bytes,
                },
            )
            .map_err(|_| ProtocolError::Authentication)?;
        Ok(DecodedPacket { header, payload })
    }

    pub fn commit(&mut self, sequence: u64) {
        self.replay.commit(sequence);
    }
}

#[derive(Default)]
struct ReplayWindow {
    highest: Option<u64>,
    bitmap: u64,
}

impl ReplayWindow {
    fn would_accept(&self, sequence: u64) -> bool {
        let Some(highest) = self.highest else {
            return true;
        };
        if sequence > highest {
            return true;
        }
        let behind = highest - sequence;
        behind < 64 && self.bitmap & (1u64 << behind) == 0
    }

    fn commit(&mut self, sequence: u64) {
        match self.highest {
            None => {
                self.highest = Some(sequence);
                self.bitmap = 1;
            }
            Some(highest) if sequence > highest => {
                let advance = sequence - highest;
                self.bitmap = if advance >= 64 {
                    1
                } else {
                    (self.bitmap << advance) | 1
                };
                self.highest = Some(sequence);
            }
            Some(highest) => self.bitmap |= 1u64 << (highest - sequence),
        }
    }
}

fn parse_header(datagram: &[u8]) -> Result<(Header, &[u8], &[u8]), ProtocolError> {
    if datagram.len() > MAX_DATAGRAM_LEN {
        return Err(ProtocolError::Oversize);
    }
    if datagram.len() < HEADER_LEN + TAG_LEN {
        return Err(ProtocolError::BadLength);
    }
    if &datagram[0..4] != b"ANPD" {
        return Err(ProtocolError::BadHeader);
    }
    if datagram[4] != 1 {
        return Err(ProtocolError::UnsupportedVersion);
    }
    let packet_type = PacketType::try_from(datagram[5])?;
    if datagram[6..8] != [0, 0] {
        return Err(ProtocolError::UnsupportedFlags);
    }
    let header = Header {
        packet_type,
        session_id: u64::from_be_bytes(datagram[8..16].try_into().expect("fixed header")),
        sequence: u64::from_be_bytes(datagram[16..24].try_into().expect("fixed header")),
        sample_frame_index: u64::from_be_bytes(datagram[24..32].try_into().expect("fixed header")),
        payload_len: u16::from_be_bytes([datagram[32], datagram[33]]),
    };
    if header.payload_len as usize > MAX_PAYLOAD_LEN {
        return Err(ProtocolError::Oversize);
    }
    let expected = HEADER_LEN + header.payload_len as usize + TAG_LEN;
    if datagram.len() != expected {
        return Err(ProtocolError::BadLength);
    }
    Ok((header, &datagram[..HEADER_LEN], &datagram[HEADER_LEN..]))
}

fn nonce(direction: u32, sequence: u64) -> [u8; 12] {
    let mut nonce = [0; 12];
    nonce[..4].copy_from_slice(&direction.to_be_bytes());
    nonce[4..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;

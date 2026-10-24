use std::io::{self, Read, Write};

const HEADER_LEN: usize = 8;
const MAX_BODY_LEN: usize = 1200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum RecordType {
    Handshake = 1,
    Control = 2,
    Audio = 3,
}

impl TryFrom<u8> for RecordType {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Handshake),
            2 => Ok(Self::Control),
            3 => Ok(Self::Audio),
            _ => Err(invalid_data("unknown ANPU record type")),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Record {
    pub(crate) kind: RecordType,
    pub(crate) body: Vec<u8>,
}

impl Record {
    pub(crate) fn new(kind: RecordType, body: Vec<u8>) -> io::Result<Self> {
        validate_body(kind, &body)?;
        Ok(Self { kind, body })
    }
}

pub(crate) fn encode_record(record: &Record) -> io::Result<Vec<u8>> {
    validate_body(record.kind, &record.body)?;
    let body_len =
        u16::try_from(record.body.len()).map_err(|_| invalid_input("ANPU body too large"))?;
    let mut bytes = Vec::with_capacity(HEADER_LEN + record.body.len());
    bytes.extend_from_slice(b"ANPU");
    bytes.extend_from_slice(&[1, record.kind as u8]);
    bytes.extend_from_slice(&body_len.to_be_bytes());
    bytes.extend_from_slice(&record.body);
    Ok(bytes)
}

pub(crate) fn decode_record(bytes: &[u8]) -> io::Result<Record> {
    let (kind, body_len) = parse_header(
        bytes
            .get(..HEADER_LEN)
            .ok_or_else(|| invalid_data("truncated ANPU header"))?,
    )?;
    if bytes.len() != HEADER_LEN + body_len {
        return Err(invalid_data("ANPU record length mismatch"));
    }
    Record::new(kind, bytes[HEADER_LEN..].to_vec())
}

/// Reads exactly one record. `Read::read == 0` is EOF. The caller must treat
/// every returned error as session-fatal because `read_exact` may have already
/// consumed part of this record; a live USB adapter absorbs bounded no-progress
/// and cancellation handling before exposing a `Read` implementation here.
pub(crate) fn read_record(reader: &mut impl Read) -> io::Result<Record> {
    let mut header = [0; HEADER_LEN];
    reader.read_exact(&mut header)?;
    let (kind, body_len) = parse_header(&header)?;
    let mut body = vec![0; body_len];
    reader.read_exact(&mut body)?;
    Record::new(kind, body)
}

pub(crate) fn write_record(writer: &mut impl Write, record: &Record) -> io::Result<()> {
    let bytes = encode_record(record)?;
    // An error after a partial write invalidates the stream; never restart it.
    writer.write_all(&bytes)
}

fn parse_header(header: &[u8]) -> io::Result<(RecordType, usize)> {
    if header.len() != HEADER_LEN || &header[..4] != b"ANPU" || header[4] != 1 {
        return Err(invalid_data("invalid ANPU header"));
    }
    let kind = RecordType::try_from(header[5])?;
    let body_len = u16::from_be_bytes([header[6], header[7]]) as usize;
    if !(1..=MAX_BODY_LEN).contains(&body_len) {
        return Err(invalid_data("invalid ANPU body length"));
    }
    Ok((kind, body_len))
}

fn validate_body(kind: RecordType, body: &[u8]) -> io::Result<()> {
    if body.is_empty() || body.len() > MAX_BODY_LEN {
        return Err(invalid_input("invalid ANPU body length"));
    }
    match kind {
        RecordType::Handshake => {
            if body.len() < 8 || &body[..4] != b"ANP5" || body[4] != 2 {
                return Err(invalid_input("invalid ANPU handshake body"));
            }
            let expected_len = match body[5] {
                1 => 96,
                2 => 104,
                3 | 4 => 40,
                _ => return Err(invalid_input("unknown P5-A v2 message type")),
            };
            if body.len() != expected_len
                || u16::from_be_bytes([body[6], body[7]]) as usize != body.len()
            {
                return Err(invalid_input("invalid P5-A v2 message length"));
            }
        }
        RecordType::Control => {
            if body.len() < 62 || &body[..4] != b"ANPC" || body[4] != 1 || body[6..8] != [0, 0] {
                return Err(invalid_input("invalid P5-C control body"));
            }
            let payload_len = u16::from_be_bytes([body[28], body[29]]) as usize;
            if body.len() != 30 + payload_len + 32 {
                return Err(invalid_input("invalid P5-C control length"));
            }
        }
        RecordType::Audio => {
            if body.len() < 50
                || &body[..4] != b"ANPD"
                || body[4] != 2
                || !matches!(body[5], 1 | 2)
                || body[6..8] != [0, 0]
            {
                return Err(invalid_input("invalid ANPD v2 audio body"));
            }
            let payload_len = u16::from_be_bytes([body[32], body[33]]) as usize;
            if body.len() != 34 + payload_len + 16 || (body[5] == 1 && payload_len != 16) {
                return Err(invalid_input("invalid ANPD v2 packet length"));
            }
        }
    }
    Ok(())
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, ErrorKind};

    const VECTORS: &str = include_str!("../../tests/aoa-v1-vectors.txt");

    fn vector(name: &str) -> Vec<u8> {
        let value = VECTORS
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("missing vector {name}"));
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    struct ShortReader<R> {
        inner: R,
        max: usize,
    }

    impl<R: Read> Read for ShortReader<R> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let len = output.len().min(self.max);
            self.inner.read(&mut output[..len])
        }
    }

    struct ShortWriter {
        bytes: Vec<u8>,
        max: usize,
    }

    impl Write for ShortWriter {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            let len = input.len().min(self.max);
            self.bytes.extend_from_slice(&input[..len]);
            Ok(len)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn shared_vectors_roundtrip_handshake_control_and_audio_records() {
        for key in [
            "profile.1.anpu_hello",
            "profile.2.anpu_audio",
            "profile.3.anpu_format",
            "profile.3.anpu_capabilities_query",
            "profile.3.anpu_aoa_bind",
        ] {
            let bytes = vector(key);
            let record = decode_record(&bytes).unwrap();
            assert_eq!(encode_record(&record).unwrap(), bytes, "{key}");
        }
    }

    #[test]
    fn stream_read_write_handles_short_transfers_and_truncation() {
        let expected = vector("profile.2.anpu_audio");
        let mut reader = ShortReader {
            inner: Cursor::new(expected.clone()),
            max: 1,
        };
        let record = read_record(&mut reader).unwrap();
        assert_eq!(encode_record(&record).unwrap(), expected);

        let mut writer = ShortWriter {
            bytes: Vec::new(),
            max: 2,
        };
        write_record(&mut writer, &record).unwrap();
        assert_eq!(writer.bytes, expected);

        assert_eq!(
            read_record(&mut Cursor::new(&expected[..4]))
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof
        );
        assert_eq!(
            read_record(&mut Cursor::new(&expected[..expected.len() - 1]))
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn cancellation_after_partial_transfer_is_fatal_and_not_retried() {
        struct CancelRead(u8);
        impl Read for CancelRead {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    self.0 = 1;
                    output[0] = b'A';
                    Ok(1)
                } else {
                    Err(io::Error::new(ErrorKind::ConnectionAborted, "cancelled"))
                }
            }
        }
        let mut reader = CancelRead(0);
        assert_eq!(
            read_record(&mut reader).unwrap_err().kind(),
            ErrorKind::ConnectionAborted
        );
        assert_eq!(reader.0, 1);

        struct CancelWrite(u8);
        impl Write for CancelWrite {
            fn write(&mut self, input: &[u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    self.0 = 1;
                    assert!(!input.is_empty());
                    Ok(1)
                } else {
                    Err(io::Error::new(ErrorKind::ConnectionAborted, "cancelled"))
                }
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let record = decode_record(&vector("profile.1.anpu_hello")).unwrap();
        let mut writer = CancelWrite(0);
        assert_eq!(
            write_record(&mut writer, &record).unwrap_err().kind(),
            ErrorKind::ConnectionAborted
        );
        assert_eq!(writer.0, 1);
    }

    #[test]
    fn rejects_bad_type_length_magic_and_zero_progress_write() {
        let mut bytes = vector("profile.1.anpu_hello");
        bytes[5] = 4;
        assert_eq!(
            decode_record(&bytes).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        bytes = vector("profile.1.anpu_hello");
        bytes[4] = 3;
        assert_eq!(
            decode_record(&bytes).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        bytes = vector("profile.1.anpu_hello");
        bytes[7] = 0;
        assert_eq!(
            decode_record(&bytes).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        bytes = vector("profile.1.anpu_hello");
        bytes[8] ^= 1;
        assert_eq!(
            decode_record(&bytes).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
        bytes = vector("profile.1.anpu_format");
        bytes[8 + 32..8 + 34].copy_from_slice(&15u16.to_be_bytes());
        assert_eq!(
            decode_record(&bytes).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );

        struct ZeroWriter;
        impl Write for ZeroWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Ok(0)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let record = decode_record(&vector("profile.1.anpu_hello")).unwrap();
        assert_eq!(
            write_record(&mut ZeroWriter, &record).unwrap_err().kind(),
            ErrorKind::WriteZero
        );
    }
}

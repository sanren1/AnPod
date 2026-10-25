use crate::protocol::AudioFormatV2;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::fmt;

type HmacSha256 = Hmac<Sha256>;

const MAGIC: &[u8; 4] = b"ANP5";
const VERSION: u8 = 1;
const HOST_ROLE: u8 = 1;
const RECEIVER_ROLE: u8 = 2;
const HELLO: u8 = 1;
const CHALLENGE: u8 = 2;
const FINISH: u8 = 3;
const READY: u8 = 4;
const BINDING_MAGIC: &[u8; 4] = b"ANPB";
const BINDING_VERSION: u8 = 1;
const BINDING_BYTES: usize = 69;
const AUDIO_KEY_HOST_TO_RECEIVER: &str = "AnPod P5-A key host-to-receiver audio AES-256-GCM v1";
const AUDIO_KEY_RECEIVER_TO_HOST: &str = "AnPod P5-A key receiver-to-host audio AES-256-GCM v1";
const CONTROL_KEY_HOST_TO_RECEIVER: &str = "AnPod P5-A key host-to-receiver control HMAC-SHA256 v1";
const CONTROL_KEY_RECEIVER_TO_HOST: &str = "AnPod P5-A key receiver-to-host control HMAC-SHA256 v1";

#[derive(Debug)]
pub(crate) enum SecurityError {
    Random,
    Malformed,
    WrongIdentity,
    InvalidProof,
    InvalidParameters,
    Io(std::io::Error),
    Protection,
    BindingConflict,
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Random => "secure random generation failed",
            Self::Malformed => "malformed P5-A handshake message or binding record",
            Self::WrongIdentity => "P5-A peer identity or role mismatch",
            Self::InvalidProof => "P5-A authentication failed",
            Self::InvalidParameters => "unsupported P5-A audio parameters",
            Self::Io(_) => "P5-A binding storage I/O failed",
            Self::Protection => "Windows current-user DPAPI operation failed",
            Self::BindingConflict => "P5-A binding conflict",
        })
    }
}

impl std::error::Error for SecurityError {}

impl From<std::io::Error> for SecurityError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Shared USB-provisioned record. It is not Clone and never formats its secret.
pub(crate) struct BindingMaterial {
    host_id: [u8; 16],
    receiver_id: [u8; 16],
    secret: [u8; 32],
}

impl BindingMaterial {
    pub(crate) fn host_id(&self) -> [u8; 16] {
        self.host_id
    }

    pub(crate) fn derive_adb_stream(&self) -> Self {
        let mut salt_input = b"AnPod ADB Stream binding salt v1\0".to_vec();
        salt_input.extend_from_slice(&self.host_id);
        salt_input.extend_from_slice(&self.receiver_id);
        let salt = Sha256::digest(&salt_input);
        salt_input.fill(0);

        let hkdf = Hkdf::<Sha256>::new(Some(&salt), &self.secret);
        let mut secret = [0; 32];
        hkdf.expand(b"AnPod ADB Stream P5-A binding secret v1", &mut secret)
            .expect("32-byte HKDF output");
        Self {
            host_id: self.host_id,
            receiver_id: self.receiver_id,
            secret,
        }
    }

    pub(crate) fn derive_aoa_transport(&self) -> Self {
        let mut salt_input = b"AnPod AOA Direct binding salt v1\0".to_vec();
        salt_input.extend_from_slice(&self.host_id);
        salt_input.extend_from_slice(&self.receiver_id);
        let salt = Sha256::digest(&salt_input);
        salt_input.fill(0);

        let hkdf = Hkdf::<Sha256>::new(Some(&salt), &self.secret);
        let mut secret = [0; 32];
        hkdf.expand(b"AnPod AOA Direct P5-A binding secret v1", &mut secret)
            .expect("32-byte HKDF output");
        Self {
            host_id: self.host_id,
            receiver_id: self.receiver_id,
            secret,
        }
    }

    pub(crate) fn duplicate_for_session(&self) -> Self {
        Self {
            host_id: self.host_id,
            receiver_id: self.receiver_id,
            secret: self.secret,
        }
    }

    pub(crate) fn generate() -> Result<Self, SecurityError> {
        let mut bytes = [0u8; 64];
        getrandom::fill(&mut bytes).map_err(|_| SecurityError::Random)?;
        let mut host_id = [0u8; 16];
        let mut receiver_id = [0u8; 16];
        let mut secret = [0u8; 32];
        host_id.copy_from_slice(&bytes[..16]);
        receiver_id.copy_from_slice(&bytes[16..32]);
        secret.copy_from_slice(&bytes[32..]);
        bytes.fill(0);
        Ok(Self {
            host_id,
            receiver_id,
            secret,
        })
    }

    pub(crate) fn encode(&self) -> SensitiveBytes {
        let mut bytes = [0u8; BINDING_BYTES];
        bytes[..4].copy_from_slice(BINDING_MAGIC);
        bytes[4] = BINDING_VERSION;
        bytes[5..21].copy_from_slice(&self.host_id);
        bytes[21..37].copy_from_slice(&self.receiver_id);
        bytes[37..69].copy_from_slice(&self.secret);
        SensitiveBytes(bytes.to_vec())
    }

    fn decode(bytes: &[u8]) -> Result<Self, SecurityError> {
        if bytes.len() != BINDING_BYTES
            || &bytes[..4] != BINDING_MAGIC
            || bytes[4] != BINDING_VERSION
        {
            return Err(SecurityError::Malformed);
        }
        let mut host_id = [0u8; 16];
        let mut receiver_id = [0u8; 16];
        let mut secret = [0u8; 32];
        host_id.copy_from_slice(&bytes[5..21]);
        receiver_id.copy_from_slice(&bytes[21..37]);
        secret.copy_from_slice(&bytes[37..69]);
        Ok(Self {
            host_id,
            receiver_id,
            secret,
        })
    }

    pub(crate) fn commitment(&self) -> [u8; 32] {
        let record = self.encode();
        Sha256::digest(&record.0).into()
    }

    /// Copy of the binding secret, for authenticated discovery MACs only.
    pub(crate) fn discovery_secret(&self) -> [u8; 32] {
        self.secret
    }

    fn matches(&self, other: &Self) -> bool {
        self.host_id
            .iter()
            .chain(&self.receiver_id)
            .chain(&self.secret)
            .zip(
                other
                    .host_id
                    .iter()
                    .chain(&other.receiver_id)
                    .chain(&other.secret),
            )
            .fold(0u8, |difference, (left, right)| difference | (left ^ right))
            == 0
    }
}

#[cfg(windows)]
pub(crate) enum BindingRemoval {
    Removed,
    AlreadyMissing,
}

impl Drop for BindingMaterial {
    fn drop(&mut self) {
        self.secret.fill(0);
        self.host_id.fill(0);
        self.receiver_id.fill(0);
    }
}

pub(crate) struct SensitiveBytes(pub(crate) Vec<u8>);

impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// Explicit-path, create-only storage. Existing files are never overwritten or removed.
#[cfg(windows)]
pub(crate) struct BindingStore;

#[cfg(windows)]
impl BindingStore {
    pub(crate) fn create_or_confirm(
        path: &std::path::Path,
        material: &BindingMaterial,
    ) -> Result<(), SecurityError> {
        match Self::create_if_absent(path, material) {
            Ok(()) => Ok(()),
            Err(SecurityError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = Self::load(path)?;
                if material.matches(&existing) {
                    Ok(())
                } else {
                    Err(SecurityError::BindingConflict)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn create_if_absent(
        path: &std::path::Path,
        material: &BindingMaterial,
    ) -> Result<(), SecurityError> {
        use std::fs::{self, OpenOptions};
        use std::io::Write;

        let parent = path.parent().ok_or(SecurityError::Malformed)?;
        let mut plaintext = material.encode();
        let protected = dpapi::protect(&plaintext.0)?;
        plaintext.0.fill(0);
        let mut random_name = [0u8; 16];
        getrandom::fill(&mut random_name).map_err(|_| SecurityError::Random)?;
        let temp = parent.join(format!(".anpod-binding-{}.tmp", hex(&random_name)));
        let result = (|| -> Result<(), SecurityError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(&protected)?;
            file.sync_all()?;
            drop(file);
            // A hard link publishes a complete file atomically and fails if a binding already exists.
            fs::hard_link(&temp, path)?;
            Ok(())
        })();
        let _ = fs::remove_file(&temp);
        result
    }

    pub(crate) fn load(path: &std::path::Path) -> Result<BindingMaterial, SecurityError> {
        use std::fs::File;
        use std::io::Read;

        let file = File::open(path)?;
        let mut protected = Vec::new();
        file.take(4097).read_to_end(&mut protected)?;
        if protected.is_empty() || protected.len() > 4096 {
            return Err(SecurityError::Malformed);
        }
        let mut plaintext = dpapi::unprotect(&protected)?;
        protected.fill(0);
        let result = BindingMaterial::decode(&plaintext.0);
        plaintext.0.fill(0);
        result
    }

    pub(crate) fn remove_if_matches(
        path: &std::path::Path,
        expected: Option<&[u8; 32]>,
    ) -> Result<BindingRemoval, SecurityError> {
        use std::fs;

        match Self::load(path) {
            Ok(material) => {
                let Some(expected) = expected else {
                    return Err(SecurityError::BindingConflict);
                };
                if material.commitment() != *expected {
                    return Err(SecurityError::BindingConflict);
                }
                fs::remove_file(path)?;
                if path.exists() {
                    return Err(SecurityError::Io(std::io::Error::other(
                        "binding file remains after removal",
                    )));
                }
                Ok(BindingRemoval::Removed)
            }
            Err(SecurityError::Io(error))
                if error.kind() == std::io::ErrorKind::NotFound && expected.is_none() =>
            {
                Ok(BindingRemoval::AlreadyMissing)
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

#[cfg(windows)]
mod dpapi {
    use super::SecurityError;
    use std::ptr;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    struct OutputBlob(CRYPT_INTEGER_BLOB);

    impl Default for OutputBlob {
        fn default() -> Self {
            Self(CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: ptr::null_mut(),
            })
        }
    }

    impl Drop for OutputBlob {
        fn drop(&mut self) {
            if self.0.pbData.is_null() {
                return;
            }
            for index in 0..self.0.cbData as usize {
                // DPAPI's unprotect output is plaintext allocated by LocalAlloc.
                unsafe {
                    ptr::write_volatile(self.0.pbData.add(index), 0);
                }
            }
            unsafe {
                LocalFree(Some(HLOCAL(self.0.pbData.cast())));
            }
        }
    }

    fn copy_output(output: &CRYPT_INTEGER_BLOB) -> Result<Vec<u8>, SecurityError> {
        if output.pbData.is_null() || output.cbData == 0 {
            return Err(SecurityError::Protection);
        }
        Ok(unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec())
    }

    pub(super) fn protect(plaintext: &[u8]) -> Result<Vec<u8>, SecurityError> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(plaintext.len()).map_err(|_| SecurityError::Malformed)?,
            pbData: plaintext.as_ptr().cast_mut(),
        };
        let mut output = OutputBlob::default();
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
            .map_err(|_| SecurityError::Protection)?;
        }
        copy_output(&output.0)
    }

    pub(super) fn unprotect(ciphertext: &[u8]) -> Result<super::SensitiveBytes, SecurityError> {
        let input = CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(ciphertext.len()).map_err(|_| SecurityError::Malformed)?,
            pbData: ciphertext.as_ptr().cast_mut(),
        };
        let mut output = OutputBlob::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
            .map_err(|_| SecurityError::Protection)?;
        }
        Ok(super::SensitiveBytes(copy_output(&output.0)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AudioParameters {
    pub(crate) sample_rate: u32,
    pub(crate) channels: u8,
    /// 0 = balanced, 1 = low latency.
    pub(crate) mode: u8,
}

impl AudioParameters {
    fn encode(self) -> [u8; 7] {
        let mut bytes = [0u8; 7];
        bytes[0] = 1; // PCM16LE
        bytes[1] = self.channels;
        bytes[2..6].copy_from_slice(&self.sample_rate.to_be_bytes());
        bytes[6] = self.mode;
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, SecurityError> {
        if bytes.len() != 7 || bytes[0] != 1 {
            return Err(SecurityError::InvalidParameters);
        }
        let value = Self {
            channels: bytes[1],
            sample_rate: u32::from_be_bytes(bytes[2..6].try_into().unwrap()),
            mode: bytes[6],
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(self) -> Result<(), SecurityError> {
        if !(8_000..=192_000).contains(&self.sample_rate)
            || !matches!(self.channels, 1 | 2)
            || self.mode > 1
        {
            return Err(SecurityError::InvalidParameters);
        }
        Ok(())
    }
}

pub(crate) struct HostHandshake {
    material: BindingMaterial,
    parameters: AudioParameters,
    host_nonce: [u8; 32],
    session_id: u64,
}

impl HostHandshake {
    pub(crate) fn start(
        material: BindingMaterial,
        parameters: AudioParameters,
    ) -> Result<(Self, Vec<u8>), SecurityError> {
        parameters.validate()?;
        let mut host_nonce = [0u8; 32];
        let mut session_bytes = [0u8; 8];
        getrandom::fill(&mut host_nonce).map_err(|_| SecurityError::Random)?;
        getrandom::fill(&mut session_bytes).map_err(|_| SecurityError::Random)?;
        let session_id = u64::from_be_bytes(session_bytes);
        let hello = encode_hello(&material, &host_nonce, session_id, parameters);
        Ok((
            Self {
                material,
                parameters,
                host_nonce,
                session_id,
            },
            hello,
        ))
    }

    pub(crate) fn accept_challenge(
        self,
        challenge: &[u8],
    ) -> Result<(HostAwaitReady, Vec<u8>), SecurityError> {
        let body = parse_message(challenge, CHALLENGE, 86)?;
        if body[0..16] != self.material.receiver_id {
            return Err(SecurityError::WrongIdentity);
        }
        let receiver_nonce: [u8; 32] = body[16..48].try_into().unwrap();
        let receiver_proof: [u8; 32] = body[48..80].try_into().unwrap();
        let transcript = transcript(
            &self.material,
            &self.host_nonce,
            &receiver_nonce,
            self.session_id,
            self.parameters,
        );
        verify_hmac(
            &self.material.secret,
            b"AnPod P5-A proof receiver v1",
            &transcript,
            &[],
            &receiver_proof,
        )?;
        let host_proof = proof_host(&transcript, &self.material.secret);
        let finish = encode_proof_message(FINISH, &host_proof);
        Ok((
            HostAwaitReady {
                material: self.material,
                parameters: self.parameters,
                session_id: self.session_id,
                transcript,
                receiver_proof,
                host_proof,
            },
            finish,
        ))
    }
}

pub(crate) struct HostAwaitReady {
    material: BindingMaterial,
    parameters: AudioParameters,
    session_id: u64,
    transcript: Vec<u8>,
    receiver_proof: [u8; 32],
    host_proof: [u8; 32],
}

impl HostAwaitReady {
    pub(crate) fn accept_ready(self, ready: &[u8]) -> Result<AuthenticatedSession, SecurityError> {
        let body = parse_message(ready, READY, 38)?;
        verify_hmac(
            &self.material.secret,
            b"AnPod P5-A ready v1",
            &self.transcript,
            &[self.host_proof.as_slice(), self.receiver_proof.as_slice()].concat(),
            body,
        )?;
        Ok(AuthenticatedSession::new(
            self.session_id,
            self.parameters,
            &self.transcript,
            &self.material.secret,
        ))
    }
}

pub(crate) struct ReceiverHandshake;

impl ReceiverHandshake {
    pub(crate) fn accept_hello(
        material: BindingMaterial,
        hello: &[u8],
    ) -> Result<(ReceiverAwaitFinish, Vec<u8>), SecurityError> {
        let body = parse_message(hello, HELLO, 85)?;
        if body[0..16] != material.host_id || body[16..32] != material.receiver_id {
            return Err(SecurityError::WrongIdentity);
        }
        let host_nonce: [u8; 32] = body[32..64].try_into().unwrap();
        let session_id = u64::from_be_bytes(body[64..72].try_into().unwrap());
        let parameters = AudioParameters::decode(&body[72..79])?;
        let mut receiver_nonce = [0u8; 32];
        getrandom::fill(&mut receiver_nonce).map_err(|_| SecurityError::Random)?;
        let transcript = transcript(
            &material,
            &host_nonce,
            &receiver_nonce,
            session_id,
            parameters,
        );
        let receiver_proof = proof_receiver(&transcript, &material.secret);
        let mut challenge = Vec::with_capacity(86);
        start_message(&mut challenge, CHALLENGE);
        challenge.extend_from_slice(&material.receiver_id);
        challenge.extend_from_slice(&receiver_nonce);
        challenge.extend_from_slice(&receiver_proof);
        Ok((
            ReceiverAwaitFinish {
                material,
                parameters,
                session_id,
                transcript,
                receiver_proof,
            },
            challenge,
        ))
    }
}

pub(crate) struct ReceiverAwaitFinish {
    material: BindingMaterial,
    parameters: AudioParameters,
    session_id: u64,
    transcript: Vec<u8>,
    receiver_proof: [u8; 32],
}

impl ReceiverAwaitFinish {
    pub(crate) fn accept_finish(
        self,
        finish: &[u8],
    ) -> Result<(Vec<u8>, AuthenticatedSession), SecurityError> {
        let body = parse_message(finish, FINISH, 38)?;
        let host_proof = proof_host(&self.transcript, &self.material.secret);
        verify_hmac(
            &self.material.secret,
            b"AnPod P5-A proof host v1",
            &self.transcript,
            &[],
            body,
        )?;
        let ready = ready_proof(
            &self.transcript,
            &host_proof,
            &self.receiver_proof,
            &self.material.secret,
        );
        let ready_message = encode_proof_message(READY, &ready);
        let session = AuthenticatedSession::new(
            self.session_id,
            self.parameters,
            &self.transcript,
            &self.material.secret,
        );
        Ok((ready_message, session))
    }
}

pub(crate) struct AuthenticatedSession {
    session_id: u64,
    parameters: AudioParameters,
    keys: Option<SessionKeys>,
}

impl AuthenticatedSession {
    fn new(
        session_id: u64,
        parameters: AudioParameters,
        transcript: &[u8],
        secret: &[u8; 32],
    ) -> Self {
        let mut salt_input = b"AnPod P5-A HKDF-SHA256 salt v1\0".to_vec();
        salt_input.extend_from_slice(transcript);
        let salt = Sha256::digest(&salt_input);
        salt_input.fill(0);
        let hkdf = Hkdf::<Sha256>::new(Some(&salt), secret);
        let mut keys = SessionKeys {
            host_to_receiver_audio: [0; 32],
            receiver_to_host_audio: [0; 32],
            host_to_receiver_control: [0; 32],
            receiver_to_host_control: [0; 32],
        };
        hkdf.expand(
            AUDIO_KEY_HOST_TO_RECEIVER.as_bytes(),
            &mut keys.host_to_receiver_audio,
        )
        .expect("32-byte HKDF output");
        hkdf.expand(
            AUDIO_KEY_RECEIVER_TO_HOST.as_bytes(),
            &mut keys.receiver_to_host_audio,
        )
        .expect("32-byte HKDF output");
        hkdf.expand(
            CONTROL_KEY_HOST_TO_RECEIVER.as_bytes(),
            &mut keys.host_to_receiver_control,
        )
        .expect("32-byte HKDF output");
        hkdf.expand(
            CONTROL_KEY_RECEIVER_TO_HOST.as_bytes(),
            &mut keys.receiver_to_host_control,
        )
        .expect("32-byte HKDF output");
        Self {
            session_id,
            parameters,
            keys: Some(keys),
        }
    }

    pub(crate) fn session_id(&self) -> u64 {
        self.session_id
    }
    pub(crate) fn parameters(&self) -> AudioParameters {
        self.parameters
    }
    pub(crate) fn into_keys(mut self) -> SessionKeys {
        self.keys.take().expect("session keys are consumed once")
    }
}

pub(crate) struct SessionKeys {
    pub(crate) host_to_receiver_audio: [u8; 32],
    pub(crate) receiver_to_host_audio: [u8; 32],
    pub(crate) host_to_receiver_control: [u8; 32],
    pub(crate) receiver_to_host_control: [u8; 32],
}

impl Drop for SessionKeys {
    fn drop(&mut self) {
        self.host_to_receiver_audio.fill(0);
        self.receiver_to_host_audio.fill(0);
        self.host_to_receiver_control.fill(0);
        self.receiver_to_host_control.fill(0);
    }
}

#[cfg(test)]
pub(crate) fn test_binding_material() -> BindingMaterial {
    BindingMaterial {
        host_id: [0x11; 16],
        receiver_id: [0x22; 16],
        secret: [0x33; 32],
    }
}

#[cfg(test)]
pub(crate) fn test_adb_binding_material() -> BindingMaterial {
    BindingMaterial {
        host_id: [0x01; 16],
        receiver_id: [0x02; 16],
        secret: [0x03; 32],
    }
}

#[cfg(all(test, windows))]
pub(crate) fn write_test_binding_file(
    path: &std::path::Path,
    material: &BindingMaterial,
) -> Result<(), SecurityError> {
    let mut plaintext = material.encode();
    let protected = dpapi::protect(&plaintext.0)?;
    plaintext.0.fill(0);
    std::fs::write(path, protected)?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_binding_material_with_wrong_identity() -> BindingMaterial {
    let mut material = test_binding_material();
    material.receiver_id[0] ^= 1;
    material
}

#[cfg(test)]
pub(crate) fn test_binding_material_with_wrong_secret() -> BindingMaterial {
    let mut material = test_binding_material();
    material.secret[0] ^= 1;
    material
}

fn encode_hello(
    material: &BindingMaterial,
    host_nonce: &[u8; 32],
    session_id: u64,
    parameters: AudioParameters,
) -> Vec<u8> {
    let mut message = Vec::with_capacity(85);
    start_message(&mut message, HELLO);
    message.extend_from_slice(&material.host_id);
    message.extend_from_slice(&material.receiver_id);
    message.extend_from_slice(host_nonce);
    message.extend_from_slice(&session_id.to_be_bytes());
    message.extend_from_slice(&parameters.encode());
    message
}

fn encode_proof_message(message_type: u8, proof: &[u8; 32]) -> Vec<u8> {
    let mut message = Vec::with_capacity(38);
    start_message(&mut message, message_type);
    message.extend_from_slice(proof);
    message
}

fn start_message(message: &mut Vec<u8>, message_type: u8) {
    message.extend_from_slice(MAGIC);
    message.push(VERSION);
    message.push(message_type);
}

fn parse_message(
    message: &[u8],
    expected_type: u8,
    expected_len: usize,
) -> Result<&[u8], SecurityError> {
    if message.len() != expected_len || &message[..4] != MAGIC || message[4] != VERSION {
        return Err(SecurityError::Malformed);
    }
    if message[5] != expected_type {
        return Err(SecurityError::WrongIdentity);
    }
    Ok(&message[6..])
}

fn transcript(
    material: &BindingMaterial,
    host_nonce: &[u8; 32],
    receiver_nonce: &[u8; 32],
    session_id: u64,
    parameters: AudioParameters,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(118);
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.push(HOST_ROLE);
    bytes.push(RECEIVER_ROLE);
    bytes.extend_from_slice(&material.host_id);
    bytes.extend_from_slice(&material.receiver_id);
    bytes.extend_from_slice(host_nonce);
    bytes.extend_from_slice(receiver_nonce);
    bytes.extend_from_slice(&session_id.to_be_bytes());
    bytes.extend_from_slice(&parameters.encode());
    bytes
}

fn hmac(secret: &[u8; 32], label: &[u8], transcript: &[u8], extra: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts a 32-byte key");
    mac.update(label);
    mac.update(&[0]);
    mac.update(transcript);
    mac.update(extra);
    mac.finalize().into_bytes().into()
}

fn proof_host(transcript: &[u8], secret: &[u8; 32]) -> [u8; 32] {
    hmac(secret, b"AnPod P5-A proof host v1", transcript, &[])
}

fn proof_receiver(transcript: &[u8], secret: &[u8; 32]) -> [u8; 32] {
    hmac(secret, b"AnPod P5-A proof receiver v1", transcript, &[])
}

fn ready_proof(
    transcript: &[u8],
    host_proof: &[u8; 32],
    receiver_proof: &[u8; 32],
    secret: &[u8; 32],
) -> [u8; 32] {
    let mut proofs = Vec::with_capacity(64);
    proofs.extend_from_slice(host_proof);
    proofs.extend_from_slice(receiver_proof);
    let value = hmac(secret, b"AnPod P5-A ready v1", transcript, &proofs);
    proofs.fill(0);
    value
}

fn verify_hmac(
    secret: &[u8; 32],
    label: &[u8],
    transcript: &[u8],
    extra: &[u8],
    received: &[u8],
) -> Result<(), SecurityError> {
    if received.len() != 32 {
        return Err(SecurityError::InvalidProof);
    }
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts a 32-byte key");
    mac.update(label);
    mac.update(&[0]);
    mac.update(transcript);
    mac.update(extra);
    // Use the crate's constant-time tag verifier rather than ordinary equality.
    mac.verify_slice(received)
        .map_err(|_| SecurityError::InvalidProof)
}

// Offline Phase 1 v2 simulator. Product pairing deliberately continues to call the v1 API.
const V2_VERSION: u8 = 2;
const V2_HEADER_BYTES: usize = 8;

pub(crate) struct HostHandshakeV2 {
    material: BindingMaterial,
    format: AudioFormatV2,
    host_nonce: [u8; 32],
    session_id: u64,
}

impl HostHandshakeV2 {
    pub(crate) fn start(
        material: BindingMaterial,
        format: AudioFormatV2,
    ) -> Result<(Self, Vec<u8>), SecurityError> {
        format
            .encode()
            .map_err(|_| SecurityError::InvalidParameters)?;
        let mut host_nonce = [0; 32];
        let mut id = [0; 8];
        getrandom::fill(&mut host_nonce).map_err(|_| SecurityError::Random)?;
        getrandom::fill(&mut id).map_err(|_| SecurityError::Random)?;
        let session_id = u64::from_be_bytes(id);
        let hello = v2_message(
            HELLO,
            96,
            &[
                &material.host_id,
                &material.receiver_id,
                &host_nonce,
                &id,
                &format
                    .encode()
                    .map_err(|_| SecurityError::InvalidParameters)?,
            ],
        );
        Ok((
            Self {
                material,
                format,
                host_nonce,
                session_id,
            },
            hello,
        ))
    }

    pub(crate) fn accept_challenge(
        self,
        message: &[u8],
    ) -> Result<(HostAwaitReadyV2, Vec<u8>), SecurityError> {
        let body = parse_v2(message, CHALLENGE, 104)?;
        if body[..16] != self.material.receiver_id {
            return Err(SecurityError::WrongIdentity);
        }
        let receiver_nonce: [u8; 32] = body[16..48].try_into().unwrap();
        let selected =
            AudioFormatV2::decode(&body[48..64]).map_err(|_| SecurityError::InvalidParameters)?;
        if selected != self.format {
            return Err(SecurityError::InvalidParameters);
        }
        let transcript = transcript_v2(
            &self.material,
            &self.host_nonce,
            &receiver_nonce,
            self.session_id,
            self.format,
        )?;
        let receiver_proof: [u8; 32] = body[64..96].try_into().unwrap();
        verify_hmac(
            &self.material.secret,
            b"AnPod P5-A proof receiver v2",
            &transcript,
            &[],
            &receiver_proof,
        )?;
        let host_proof = hmac(
            &self.material.secret,
            b"AnPod P5-A proof host v2",
            &transcript,
            &[],
        );
        let finish = v2_message(FINISH, 40, &[&host_proof]);
        Ok((
            HostAwaitReadyV2 {
                material: self.material,
                format: self.format,
                session_id: self.session_id,
                transcript,
                receiver_proof,
                host_proof,
            },
            finish,
        ))
    }
}

pub(crate) struct HostAwaitReadyV2 {
    material: BindingMaterial,
    format: AudioFormatV2,
    session_id: u64,
    transcript: Vec<u8>,
    receiver_proof: [u8; 32],
    host_proof: [u8; 32],
}

impl HostAwaitReadyV2 {
    pub(crate) fn accept_ready(
        self,
        message: &[u8],
    ) -> Result<AuthenticatedSessionV2, SecurityError> {
        let body = parse_v2(message, READY, 40)?;
        let mut proofs = [0; 64];
        proofs[..32].copy_from_slice(&self.host_proof);
        proofs[32..].copy_from_slice(&self.receiver_proof);
        let verify = verify_hmac(
            &self.material.secret,
            b"AnPod P5-A ready v2",
            &self.transcript,
            &proofs,
            body,
        );
        proofs.fill(0);
        verify?;
        Ok(AuthenticatedSessionV2 {
            session_id: self.session_id,
            format: self.format,
            keys: Some(derive_v2_keys(&self.material.secret, &self.transcript)),
        })
    }
}

pub(crate) struct ReceiverHandshakeV2;

impl ReceiverHandshakeV2 {
    pub(crate) fn accept_hello(
        material: BindingMaterial,
        message: &[u8],
    ) -> Result<(ReceiverAwaitFinishV2, Vec<u8>), SecurityError> {
        let body = parse_v2(message, HELLO, 96)?;
        if body[..16] != material.host_id || body[16..32] != material.receiver_id {
            return Err(SecurityError::WrongIdentity);
        }
        let host_nonce: [u8; 32] = body[32..64].try_into().unwrap();
        let session_id = u64::from_be_bytes(body[64..72].try_into().unwrap());
        let requested =
            AudioFormatV2::decode(&body[72..88]).map_err(|_| SecurityError::InvalidParameters)?;
        let mut receiver_nonce = [0; 32];
        getrandom::fill(&mut receiver_nonce).map_err(|_| SecurityError::Random)?;
        let transcript = transcript_v2(
            &material,
            &host_nonce,
            &receiver_nonce,
            session_id,
            requested,
        )?;
        let receiver_proof = hmac(
            &material.secret,
            b"AnPod P5-A proof receiver v2",
            &transcript,
            &[],
        );
        let challenge = v2_message(
            CHALLENGE,
            104,
            &[
                &material.receiver_id,
                &receiver_nonce,
                &requested
                    .encode()
                    .map_err(|_| SecurityError::InvalidParameters)?,
                &receiver_proof,
            ],
        );
        Ok((
            ReceiverAwaitFinishV2 {
                material,
                format: requested,
                session_id,
                transcript,
                receiver_proof,
            },
            challenge,
        ))
    }
}

pub(crate) struct ReceiverAwaitFinishV2 {
    material: BindingMaterial,
    format: AudioFormatV2,
    session_id: u64,
    transcript: Vec<u8>,
    receiver_proof: [u8; 32],
}

impl ReceiverAwaitFinishV2 {
    pub(crate) fn accept_finish(
        self,
        message: &[u8],
    ) -> Result<(Vec<u8>, AuthenticatedSessionV2), SecurityError> {
        let host_proof = parse_v2(message, FINISH, 40)?;
        verify_hmac(
            &self.material.secret,
            b"AnPod P5-A proof host v2",
            &self.transcript,
            &[],
            host_proof,
        )?;
        let ready = hmac(
            &self.material.secret,
            b"AnPod P5-A ready v2",
            &self.transcript,
            &[host_proof, self.receiver_proof.as_slice()].concat(),
        );
        let response = v2_message(READY, 40, &[&ready]);
        let session = AuthenticatedSessionV2 {
            session_id: self.session_id,
            format: self.format,
            keys: Some(derive_v2_keys(&self.material.secret, &self.transcript)),
        };
        Ok((response, session))
    }
}

pub(crate) struct AuthenticatedSessionV2 {
    pub(crate) session_id: u64,
    pub(crate) format: AudioFormatV2,
    keys: Option<SessionKeys>,
}

impl AuthenticatedSessionV2 {
    pub(crate) fn into_keys(mut self) -> SessionKeys {
        self.keys.take().expect("session keys consumed once")
    }
}

fn v2_message(kind: u8, total_len: u16, parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(total_len as usize);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[V2_VERSION, kind]);
    out.extend_from_slice(&total_len.to_be_bytes());
    for part in parts {
        out.extend_from_slice(part);
    }
    debug_assert_eq!(out.len(), total_len as usize);
    out
}

fn parse_v2(message: &[u8], kind: u8, len: usize) -> Result<&[u8], SecurityError> {
    if message.len() != len
        || &message[..4] != MAGIC
        || message[4] != V2_VERSION
        || message[5] != kind
        || u16::from_be_bytes([message[6], message[7]]) as usize != len
    {
        return Err(SecurityError::Malformed);
    }
    Ok(&message[V2_HEADER_BYTES..])
}

fn transcript_v2(
    material: &BindingMaterial,
    host_nonce: &[u8; 32],
    receiver_nonce: &[u8; 32],
    session_id: u64,
    format: AudioFormatV2,
) -> Result<Vec<u8>, SecurityError> {
    let descriptor = format
        .encode()
        .map_err(|_| SecurityError::InvalidParameters)?;
    let mut out = Vec::with_capacity(143);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[V2_VERSION, HOST_ROLE, RECEIVER_ROLE]);
    out.extend_from_slice(&material.host_id);
    out.extend_from_slice(&material.receiver_id);
    out.extend_from_slice(host_nonce);
    out.extend_from_slice(receiver_nonce);
    out.extend_from_slice(&session_id.to_be_bytes());
    out.extend_from_slice(&descriptor);
    out.extend_from_slice(&descriptor);
    Ok(out)
}

fn derive_v2_keys(secret: &[u8; 32], transcript: &[u8]) -> SessionKeys {
    let mut salt_input = b"AnPod P5-A HKDF-SHA256 salt v2\0".to_vec();
    salt_input.extend_from_slice(transcript);
    let salt = Sha256::digest(&salt_input);
    salt_input.fill(0);
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), secret);
    let mut keys = SessionKeys {
        host_to_receiver_audio: [0; 32],
        receiver_to_host_audio: [0; 32],
        host_to_receiver_control: [0; 32],
        receiver_to_host_control: [0; 32],
    };
    for (key, label) in [
        (
            &mut keys.host_to_receiver_audio,
            "AnPod P5-A key host-to-receiver audio AES-256-GCM v2",
        ),
        (
            &mut keys.receiver_to_host_audio,
            "AnPod P5-A key receiver-to-host audio AES-256-GCM v2",
        ),
        (
            &mut keys.host_to_receiver_control,
            "AnPod P5-A key host-to-receiver control HMAC-SHA256 v2",
        ),
        (
            &mut keys.receiver_to_host_control,
            "AnPod P5-A key receiver-to-host control HMAC-SHA256 v2",
        ),
    ] {
        hkdf.expand(label.as_bytes(), key)
            .expect("32-byte HKDF output");
    }
    keys
}

#[cfg(all(test, windows))]
mod storage_tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn dpapi_storage_is_create_only_and_corruption_fails_closed() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("anpod-p5a-{}-{stamp}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("binding.dpapi");
        let material = BindingMaterial::generate().unwrap();
        BindingStore::create_if_absent(&path, &material).unwrap();
        let first_ciphertext = fs::read(&path).unwrap();
        BindingStore::create_or_confirm(&path, &material).unwrap();
        assert!(matches!(
            BindingStore::create_or_confirm(&path, &BindingMaterial::generate().unwrap()),
            Err(SecurityError::BindingConflict)
        ));
        assert!(!first_ciphertext
            .windows(32)
            .any(|window| window == &material.secret[..]));
        assert!(
            BindingStore::create_if_absent(&path, &BindingMaterial::generate().unwrap()).is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), first_ciphertext);
        let loaded = BindingStore::load(&path).unwrap();
        assert_eq!(loaded.host_id, material.host_id);
        assert_eq!(loaded.receiver_id, material.receiver_id);
        assert_eq!(loaded.secret, material.secret);
        drop(loaded);
        drop(material);
        let mut damaged = first_ciphertext;
        damaged[0] ^= 0x80;
        fs::write(&path, damaged).unwrap();
        assert!(BindingStore::load(&path).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dpapi_binding_removal_is_targeted_and_compare_and_delete() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "anpod-binding-reset-{}-{stamp}",
                std::process::id()
            ));
        fs::create_dir_all(directory.parent().unwrap()).unwrap();
        fs::create_dir(&directory).unwrap();
        let path = directory.join("selected.dpapi");
        let other = directory.join("other.dpapi");
        let selected = BindingMaterial::generate().unwrap();
        let unrelated = BindingMaterial::generate().unwrap();
        write_test_binding_file(&path, &selected).unwrap();
        write_test_binding_file(&other, &unrelated).unwrap();
        let wrong = [0x55; 32];
        assert!(matches!(
            BindingStore::remove_if_matches(&path, Some(&wrong)),
            Err(SecurityError::BindingConflict)
        ));
        assert!(matches!(
            BindingStore::remove_if_matches(&path, None),
            Err(SecurityError::BindingConflict)
        ));
        assert!(path.exists());
        assert!(matches!(
            BindingStore::remove_if_matches(&path, Some(&selected.commitment())),
            Ok(BindingRemoval::Removed)
        ));
        assert!(!path.exists());
        assert!(
            other.exists(),
            "a reset must not remove another saved binding"
        );
        assert!(matches!(
            BindingStore::remove_if_matches(&path, None),
            Ok(BindingRemoval::AlreadyMissing)
        ));
        assert!(matches!(
            BindingStore::remove_if_matches(&path, Some(&selected.commitment())),
            Err(SecurityError::Io(_))
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;

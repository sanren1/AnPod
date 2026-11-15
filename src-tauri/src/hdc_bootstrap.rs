//! ANHB bootstrap frames and first-time pairing for HarmonyOS wired mode.
//! Frame: "ANHB" | version 1 | kind u8 | body length u16 BE | body (<= 256).
//! Nothing here logs or formats tokens, codes or binding material.

use crate::security::BindingMaterial;
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAGIC: &[u8; 4] = b"ANHB";
const VERSION: u8 = 1;
const MAX_BODY: usize = 256;
const TABLET_HELLO: u8 = 1;
const PAIR_OFFER: u8 = 2;
const PAIR_NONCE: u8 = 3;
const PAIR_CONFIRMED: u8 = 4;
const PAIR_SAVED: u8 = 5;
const PAIR_CANCEL: u8 = 6;
const PROCEED: u8 = 7;
const CANCEL_USER: u8 = 1;
const PAIR_TIMEOUT: Duration = Duration::from_secs(120);
/// A foreign connection must show its hello quickly or it is dropped.
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn encode_anhb(kind: u8, body: &[u8]) -> Vec<u8> {
    debug_assert!(body.len() <= MAX_BODY);
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(MAGIC);
    frame.push(VERSION);
    frame.push(kind);
    frame.extend_from_slice(&(body.len() as u16).to_be_bytes());
    frame.extend_from_slice(body);
    frame
}

fn parse_header(header: &[u8; 8]) -> io::Result<(u8, usize)> {
    if &header[..4] != MAGIC || header[4] != VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "引导帧格式错误"));
    }
    let length = u16::from_be_bytes([header[6], header[7]]) as usize;
    if length > MAX_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "引导帧过长"));
    }
    Ok((header[5], length))
}

pub(crate) fn read_anhb(stream: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header)?;
    let (kind, length) = parse_header(&header)?;
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    Ok((kind, body))
}

/// read_exact under one overall deadline: the socket timeout is re-armed with the
/// remaining time before every read, so a slow-drip peer cannot extend it.
fn read_exact_until(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        stream.set_read_timeout(Some(remaining))?;
        match stream.read(&mut buf[filled..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Returns the has-binding flag only for a well-formed TabletHello carrying `token`.
fn read_hello(stream: &mut TcpStream, token: &[u8; 16], deadline: Instant) -> Option<bool> {
    let mut header = [0u8; 8];
    read_exact_until(stream, &mut header, deadline).ok()?;
    if parse_header(&header).ok()? != (TABLET_HELLO, 17) {
        return None;
    }
    let mut body = [0u8; 17];
    read_exact_until(stream, &mut body, deadline).ok()?;
    (equal(&body[..16], token) && body[16] <= 1).then_some(body[16] == 1)
}

fn equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// Waits for a connection whose TabletHello carries `token`; other connections are closed.
pub(crate) fn accept_with_token(
    listener: &TcpListener,
    token: [u8; 16],
    deadline: Instant,
    cancel: &CancellationToken,
) -> Result<(TcpStream, bool), String> {
    listener
        .set_nonblocking(true)
        .map_err(|_| "无法监听平板连接".to_string())?;
    loop {
        if cancel.is_cancelled() {
            return Err("已取消".into());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err("等待平板连接超时".into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_nodelay(true);
                if let Some(has_binding) =
                    read_hello(&mut stream, &token, (now + HELLO_TIMEOUT).min(deadline))
                {
                    return Ok((stream, has_binding));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return Err("接收平板连接失败".into()),
        }
    }
}

fn arm(stream: &TcpStream, deadline: Instant) -> Result<(), String> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or("配对超时")?;
    stream
        .set_read_timeout(Some(remaining))
        .and_then(|_| stream.set_write_timeout(Some(remaining)))
        .map_err(|_| "无法设置配对超时".to_string())
}

fn send(stream: &mut TcpStream, deadline: Instant, kind: u8, body: &[u8]) -> Result<(), String> {
    arm(stream, deadline)?;
    let mut frame = encode_anhb(kind, body);
    let result = stream.write_all(&frame);
    frame.fill(0);
    result.map_err(|_| "向平板发送配对数据失败".to_string())
}

fn receive(stream: &mut TcpStream, deadline: Instant) -> Result<(u8, Vec<u8>), String> {
    arm(stream, deadline)?;
    read_anhb(stream).map_err(|_| "平板配对响应失败或超时".to_string())
}

/// First-time pairing. The caller shows `code` and answers `confirm`; storing the new
/// material is the caller's job once this returns Ok.
pub(crate) fn host_pair(
    stream: &mut TcpStream,
    material: &BindingMaterial,
    confirm: impl FnOnce(&str) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + PAIR_TIMEOUT;
    let mut pc_nonce = [0u8; 16];
    getrandom::fill(&mut pc_nonce).map_err(|_| "生成随机数失败".to_string())?;
    let mut offer = material.encode();
    offer.0.extend_from_slice(&pc_nonce);
    send(stream, deadline, PAIR_OFFER, &offer.0)?;

    let tablet_nonce: [u8; 16] = match receive(stream, deadline)? {
        (PAIR_NONCE, body) => body
            .try_into()
            .map_err(|_| "平板配对数据无效".to_string())?,
        (PAIR_CANCEL, _) => return Err("平板已取消配对".into()),
        _ => return Err("平板配对数据无效".into()),
    };
    if !confirm(&material.pairing_code(&pc_nonce, &tablet_nonce)) {
        let _ = send(stream, deadline, PAIR_CANCEL, &[CANCEL_USER]);
        return Err("已取消配对".into());
    }
    send(stream, deadline, PAIR_CONFIRMED, &[])?;
    match receive(stream, deadline)? {
        (PAIR_SAVED, body) if equal(&body, &material.commitment()) => Ok(()),
        (PAIR_SAVED, _) => Err("平板保存的绑定与电脑不一致".into()),
        (PAIR_CANCEL, _) => Err("平板已取消配对".into()),
        _ => Err("平板配对数据无效".into()),
    }
}

pub(crate) fn host_proceed(stream: &mut TcpStream) -> io::Result<()> {
    stream.set_write_timeout(Some(HELLO_TIMEOUT))?;
    stream.write_all(&encode_anhb(PROCEED, &[]))
}

#[cfg(test)]
#[path = "hdc_bootstrap_tests.rs"]
mod tests;

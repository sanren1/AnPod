//! Process-wide application log: levelled lines with a UTC millisecond
//! timestamp, run id, module and stable event name, written through one kept
//! file handle and rotated by size so the total stays bounded.

use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const FILE_NAME: &str = "anpod.log";
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Rotated files kept beside the live one (anpod.1.log is the newest).
const KEEP_ROTATED: usize = 4;
const TIMELINE_MAX_FILE_BYTES: u64 = 1024 * 1024;
const TIMELINE_KEEP_ROTATED: usize = 2;
/// Events whose detail is built only from numbers and enum values; any other
/// event is exported without its free-text detail.
const STRUCTURED_EVENTS: &[&str] = &[
    "app.start",
    "timeline.started",
    "state.changed",
    "session.changed",
    "mode.changed",
    "format.changed",
    "loss.increased",
    "usb_direct.phase",
    "usb_direct.finished",
    "operation.cancelled",
    "connected",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }
}

const MAX_LEVEL: Level = if cfg!(debug_assertions) {
    Level::Debug
} else {
    Level::Info
};

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static TIMELINE: Mutex<Option<Sink>> = Mutex::new(None);

pub(crate) fn init(directory: &Path) {
    *SINK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Sink::new(
        directory.to_path_buf(),
        "anpod",
        "log",
        MAX_FILE_BYTES,
        KEEP_ROTATED,
    ));
    *TIMELINE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Sink::new(
        directory.to_path_buf(),
        "timeline",
        "jsonl",
        TIMELINE_MAX_FILE_BYTES,
        TIMELINE_KEEP_ROTATED,
    ));
}

/// Appends one diagnostics timeline sample (a JSON object) so it survives a
/// crash or restart.
pub(crate) fn timeline(json: &str) {
    if let Some(sink) = TIMELINE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_mut()
    {
        sink.write_line(&format!("{json}\r\n"));
    }
}

/// The newest `limit` log lines from the live and previous file, reduced to
/// header plus structured detail for a sanitised export.
pub(crate) fn recent_export_lines(limit: usize) -> Vec<String> {
    let Some(directory) = SINK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .map(|sink| sink.directory.clone())
    else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for name in ["anpod.1.log", FILE_NAME] {
        if let Ok(text) = fs::read_to_string(directory.join(name)) {
            lines.extend(text.lines().filter_map(export_line));
        }
    }
    let skip = lines.len().saturating_sub(limit);
    lines.split_off(skip)
}

/// Keeps `timestamp level run module event` and the detail only for
/// structured events; lines not written by this module are dropped.
fn export_line(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let header: Vec<&str> = parts.by_ref().take(5).collect();
    if header.len() < 5 || !header[2].starts_with("run=") || !header[0].ends_with('Z') {
        return None;
    }
    let mut output = header.join(" ");
    if STRUCTURED_EVENTS.contains(&header[4]) {
        for part in parts {
            output.push(' ');
            output.push_str(part);
        }
    }
    Some(output)
}

pub(crate) fn warn(module: &str, event: &str, detail: impl Display) {
    write(Level::Warn, module, event, detail);
}

pub(crate) fn info(module: &str, event: &str, detail: impl Display) {
    write(Level::Info, module, event, detail);
}

pub(crate) fn write(level: Level, module: &str, event: &str, detail: impl Display) {
    if level > MAX_LEVEL {
        return;
    }
    let line = format_line(level, unix_millis(), run_id(), module, event, &detail);
    if let Some(sink) = SINK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_mut()
    {
        sink.write_line(&line);
    }
}

/// Records a panic. Uses `try_lock` so a panic raised while the log lock is
/// held cannot deadlock the hook.
pub(crate) fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_default();
    let location = info
        .location()
        .map(|location| format!("{}:{}", location.file(), location.line()))
        .unwrap_or_default();
    let thread = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_owned();
    let line = format_line(
        Level::Error,
        unix_millis(),
        run_id(),
        "process",
        "panic",
        &format_args!("thread={thread} location={location} message={payload}"),
    );
    if let Ok(mut guard) = SINK.try_lock() {
        if let Some(sink) = guard.as_mut() {
            sink.write_line(&line);
        }
    }
}

fn run_id() -> &'static str {
    static RUN_ID: OnceLock<String> = OnceLock::new();
    RUN_ID.get_or_init(|| {
        let mut bytes = [0u8; 4];
        let _ = getrandom::fill(&mut bytes);
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    })
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn format_line(
    level: Level,
    unix_ms: u64,
    run: &str,
    module: &str,
    event: &str,
    detail: &dyn Display,
) -> String {
    let detail = redact_ipv4(&detail.to_string()).replace(['\r', '\n'], " | ");
    let mut line = format!(
        "{} {:<5} run={run} {module} {event}",
        utc_timestamp(unix_ms),
        level.as_str()
    );
    if !detail.is_empty() {
        line.push(' ');
        line.push_str(&detail);
    }
    line.push_str("\r\n");
    line
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` from Unix milliseconds (proleptic Gregorian).
pub(crate) fn utc_timestamp(unix_ms: u64) -> String {
    let seconds = unix_ms / 1000;
    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60,
        unix_ms % 1000
    )
}

/// Masks dotted IPv4 addresses so free-form error text cannot leak them.
fn redact_ipv4(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        let boundary =
            index == 0 || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'.');
        if boundary && bytes[index].is_ascii_digit() {
            if let Some(length) = ipv4_length(&bytes[index..]) {
                output.push_str("<ipv4>");
                index += length;
                continue;
            }
        }
        let character = text[index..].chars().next().unwrap_or_default();
        output.push(character);
        index += character.len_utf8().max(1);
    }
    output
}

fn ipv4_length(bytes: &[u8]) -> Option<usize> {
    let mut index = 0;
    for octet in 0..4 {
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() && index - start < 3 {
            index += 1;
        }
        if index == start
            || std::str::from_utf8(&bytes[start..index])
                .ok()?
                .parse::<u16>()
                .ok()?
                > 255
        {
            return None;
        }
        if octet < 3 {
            if bytes.get(index) != Some(&b'.') {
                return None;
            }
            index += 1;
        }
    }
    let trailing = bytes.get(index);
    if trailing.is_some_and(|byte| {
        byte.is_ascii_digit()
            || (*byte == b'.' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
    }) {
        return None;
    }
    Some(index)
}

struct Sink {
    directory: PathBuf,
    stem: &'static str,
    extension: &'static str,
    keep_rotated: usize,
    max_file_bytes: u64,
    file: Option<File>,
    size: u64,
}

impl Sink {
    fn new(
        directory: PathBuf,
        stem: &'static str,
        extension: &'static str,
        max_file_bytes: u64,
        keep_rotated: usize,
    ) -> Self {
        Self {
            directory,
            stem,
            extension,
            keep_rotated,
            max_file_bytes,
            file: None,
            size: 0,
        }
    }

    fn write_line(&mut self, line: &str) {
        if self.file.is_none() {
            self.open();
        }
        if self.size > 0 && self.size + line.len() as u64 > self.max_file_bytes {
            self.rotate();
            if self.file.is_none() {
                self.open();
            }
        }
        if let Some(file) = self.file.as_mut() {
            if file.write_all(line.as_bytes()).is_ok() {
                self.size += line.len() as u64;
            } else {
                self.file = None;
            }
        }
    }

    fn open(&mut self) {
        let _ = fs::create_dir_all(&self.directory);
        let path = self.live();
        self.file = OpenOptions::new().create(true).append(true).open(path).ok();
        self.size = self
            .file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |metadata| metadata.len());
    }

    fn rotate(&mut self) {
        self.file = None;
        let _ = fs::remove_file(self.rotated(self.keep_rotated));
        for index in (1..self.keep_rotated).rev() {
            let _ = fs::rename(self.rotated(index), self.rotated(index + 1));
        }
        if fs::rename(self.live(), self.rotated(1)).is_err() {
            // The live file is held elsewhere (for example open in a viewer);
            // keep appending and try again after another full file.
            self.open();
            self.size = 0;
        }
    }

    fn live(&self) -> PathBuf {
        self.directory
            .join(format!("{}.{}", self.stem, self.extension))
    }

    fn rotated(&self, index: usize) -> PathBuf {
        self.directory
            .join(format!("{}.{index}.{}", self.stem, self.extension))
    }
}

#[cfg(test)]
#[path = "applog_tests.rs"]
mod tests;

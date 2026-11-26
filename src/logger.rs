//! A small leveled logger writing to stderr.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    Off = 4,
}

/// Thread-safe logger. Cheap to share via `Arc`.
pub struct Logger {
    level: AtomicU8,
}

impl Logger {
    pub fn new(level: Level) -> Logger {
        Logger { level: AtomicU8::new(level as u8) }
    }

    pub fn set_level(&self, level: Level) {
        self.level.store(level as u8, Ordering::Relaxed);
    }

    pub fn level(&self) -> Level {
        match self.level.load(Ordering::Relaxed) {
            0 => Level::Debug,
            1 => Level::Info,
            2 => Level::Warn,
            3 => Level::Error,
            _ => Level::Off,
        }
    }

    pub fn log(&self, level: Level, msg: impl Display, fields: &[(&str, &dyn Display)]) {
        if level < self.level() || level == Level::Off {
            return;
        }
        let tag = match level {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            _ => "ERROR",
        };
        let mut line = format!("{} {:<5} {}", timestamp(), tag, msg);
        for (k, v) in fields {
            line.push_str(&format!(" {k}={v}"));
        }
        eprintln!("{line}");
    }

    pub fn debug(&self, msg: impl Display, fields: &[(&str, &dyn Display)]) {
        self.log(Level::Debug, msg, fields)
    }
    pub fn info(&self, msg: impl Display, fields: &[(&str, &dyn Display)]) {
        self.log(Level::Info, msg, fields)
    }
    pub fn warn(&self, msg: impl Display, fields: &[(&str, &dyn Display)]) {
        self.log(Level::Warn, msg, fields)
    }
    pub fn error(&self, msg: impl Display, fields: &[(&str, &dyn Display)]) {
        self.log(Level::Error, msg, fields)
    }
}

impl Default for Logger {
    fn default() -> Logger {
        Logger::new(Level::Info)
    }
}

/// `YYYY/MM/DD HH:MM:SS` in UTC.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}/{:02}/{:02} {:02}:{:02}:{:02}", y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        let l = Logger::new(Level::Warn);
        assert_eq!(l.level(), Level::Warn);
        l.set_level(Level::Debug);
        assert_eq!(l.level(), Level::Debug);
        assert_eq!(timestamp().len(), 19);
    }
}

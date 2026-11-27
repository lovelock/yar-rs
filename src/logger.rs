//! Yar logger - Rust implementation replicating C yar_log.c/h functionality.
//!
//! Provides structured logging with level filtering, multiple output targets
//! (stderr, file, pipe), and hostname tagging. Log format matches the C version:
//! `"{hostname} [{timestamp}] {LEVEL}: {message}\n"`

use std::fs::{File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::sync::Mutex;

use chrono::prelude::*;

/// Log levels matching C YAR_OKEY/YAR_DEBUG/YAR_NOTICE/YAR_WARNING/YAR_ERROR
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Ok = 0,
    Debug = 1,
    Notice = 2,
    Warning = 4,
    Error = 8,
}

impl LogLevel {
    /// Return the display string for the log level.
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Ok => "OK",
            LogLevel::Debug => "DEBUG",
            LogLevel::Notice => "NOTICE",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
        }
    }
}

/// Output target for the logger.
enum LoggerOutput {
    Stderr,
    File(File),
    #[cfg_attr(not(unix), allow(dead_code))] // pipe targets are Unix-only
    Pipe(std::process::ChildStdin),
}

impl Write for LoggerOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            LoggerOutput::Stderr => std::io::stderr().write(buf),
            LoggerOutput::File(f) => f.write(buf),
            LoggerOutput::Pipe(p) => p.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            LoggerOutput::Stderr => std::io::stderr().flush(),
            LoggerOutput::File(f) => f.flush(),
            LoggerOutput::Pipe(p) => p.flush(),
        }
    }
}

/// Yar logger replicating the C `yar_logger` struct and associated functions.
pub struct YarLogger {
    output: Mutex<LoggerOutput>,
    enabled: bool,
    level: LogLevel,
    hostname: Mutex<Option<String>>,
    /// Held to keep the child process alive when using pipe output.
    _child: Option<std::process::Child>,
}

impl YarLogger {
    /// Create a new logger.
    ///
    /// - `path = None` -> log to stderr
    /// - `path` starts with `'|'` -> spawn the remainder as a pipe command
    /// - otherwise -> open file for append (created if missing)
    ///
    /// Messages with a level below `level` (except `LogLevel::Ok`) are suppressed.
    pub fn init(path: Option<&str>, level: LogLevel) -> Result<Self, std::io::Error> {
        let (output, child) = match path {
            None => (LoggerOutput::Stderr, None),
            Some(p) if p.starts_with('|') => {
                #[cfg(unix)]
                {
                    let cmd = p.trim_start_matches('|').trim();
                    let mut c = Command::new("sh")
                        .arg("-c")
                        .arg(cmd)
                        .stdin(Stdio::piped())
                        .spawn()?;
                    let stdin = c
                        .stdin
                        .take()
                        .ok_or_else(|| std::io::Error::other("failed to open pipe stdin"))?;
                    (LoggerOutput::Pipe(stdin), Some(c))
                }
                #[cfg(not(unix))]
                {
                    let _ = p;
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "pipe logging is only supported on Unix",
                    ));
                }
            }
            Some(p) => {
                let f = OpenOptions::new().create(true).append(true).open(p)?;
                (LoggerOutput::File(f), None)
            }
        };

        Ok(YarLogger {
            output: Mutex::new(output),
            enabled: true,
            level,
            hostname: Mutex::new(None),
            _child: child,
        })
    }

    /// Set the hostname tag prepended to every log line.
    pub fn set_hostname(&self, hostname: &str) {
        if let Ok(mut h) = self.hostname.lock() {
            *h = Some(hostname.to_string());
        }
    }

    /// Write a log line if the level passes the threshold.
    ///
    /// `LogLevel::Ok` messages are always emitted regardless of the configured level.
    /// Format: `"{hostname} [{timestamp}] {LEVEL}: {message}\n"`
    pub fn log(&self, level: LogLevel, message: &str) {
        if !self.enabled {
            return;
        }

        // Suppress if level < configured threshold, but always allow Ok.
        if level != LogLevel::Ok && level < self.level {
            return;
        }

        let now = Local::now().format("%Y-%m-%d %H:%M:%S");
        let host_guard = self.hostname.lock().ok();
        let host = host_guard
            .as_ref()
            .and_then(|g| g.as_deref())
            .unwrap_or("-");
        let line = format!("{} [{}] {}: {}\n", host, now, level.as_str(), message);

        if let Ok(mut out) = self.output.lock() {
            let _ = out.write_all(line.as_bytes());
            let _ = out.flush();
        }
    }

    /// Returns `true` if the logger is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns the configured minimum log level.
    pub fn min_level(&self) -> LogLevel {
        self.level
    }
}

impl Drop for YarLogger {
    fn drop(&mut self) {
        if let Ok(mut out) = self.output.lock() {
            let _ = out.flush();
        }
    }
}

// ---------------------------------------------------------------------------
// Global logger instance
// ---------------------------------------------------------------------------

static GLOBAL_LOGGER: std::sync::OnceLock<YarLogger> = std::sync::OnceLock::new();

/// Initialise the process-wide global logger.
///
/// Returns an error if called more than once (the first call wins).
pub fn init_global(path: Option<&str>, level: LogLevel) -> Result<(), std::io::Error> {
    let logger = YarLogger::init(path, level)?;
    GLOBAL_LOGGER
        .set(logger)
        .map_err(|_| std::io::Error::other("global logger already set"))
}

/// Set the hostname on the global logger.
pub fn set_hostname(hostname: &str) {
    if let Some(logger) = GLOBAL_LOGGER.get() {
        logger.set_hostname(hostname);
    }
}

/// Return a reference to the global logger, if initialised.
pub fn global() -> Option<&'static YarLogger> {
    GLOBAL_LOGGER.get()
}

/// Log a message through the global logger.
pub fn log_global(level: LogLevel, message: &str) {
    if let Some(logger) = GLOBAL_LOGGER.get() {
        logger.log(level, message);
    }
}

/// Convenience macro for logging through the global logger.
#[macro_export]
macro_rules! yar_log {
    ($level:expr, $($arg:tt)*) => {
        $crate::logger::log_global($level, &format!($($arg)*))
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_log_levels() {
        // Logger configured at Warning level: Debug and Notice are suppressed,
        // Warning and Error pass through, Ok always passes.
        let tmp = tempfile().expect("tmpfile");
        let path = tmp.path().to_string_lossy().to_string();
        let logger = YarLogger::init(Some(&path), LogLevel::Warning).unwrap();

        logger.log(LogLevel::Debug, "debug msg");
        logger.log(LogLevel::Notice, "notice msg");
        logger.log(LogLevel::Warning, "warning msg");
        logger.log(LogLevel::Error, "error msg");
        logger.log(LogLevel::Ok, "ok msg");
        drop(logger);

        let mut contents = String::new();
        let mut f = File::open(&path).unwrap();
        f.read_to_string(&mut contents).unwrap();

        assert!(
            !contents.contains("debug msg"),
            "debug should be suppressed"
        );
        assert!(
            !contents.contains("notice msg"),
            "notice should be suppressed"
        );
        assert!(contents.contains("warning msg"), "warning should appear");
        assert!(contents.contains("error msg"), "error should appear");
        assert!(contents.contains("ok msg"), "ok should always appear");
    }

    #[test]
    fn test_log_format() {
        // Verify the format components are correct.
        let host = "test-host";
        let level = LogLevel::Error;
        let msg = "something broke";

        let formatted = format!(
            "{} [{}] {}: {}\n",
            host,
            Local::now().format("%Y-%m-%d %H:%M:%S"),
            level.as_str(),
            msg
        );

        assert!(formatted.starts_with("test-host ["));
        assert!(formatted.contains("ERROR: something broke"));
        assert!(formatted.ends_with('\n'));
    }

    #[test]
    fn test_log_format_with_hostname() {
        let tmp = tempfile().expect("tmpfile");
        let path = tmp.path().to_string_lossy().to_string();
        let logger = YarLogger::init(Some(&path), LogLevel::Debug).unwrap();
        logger.set_hostname("myhost");

        logger.log(LogLevel::Notice, "hello world");
        drop(logger);

        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        assert!(contents.starts_with("myhost ["));
        assert!(contents.contains("NOTICE: hello world"));
    }

    #[test]
    fn test_log_to_file() {
        let tmp = tempfile().expect("tmpfile");
        let path = tmp.path().to_string_lossy().to_string();

        {
            let logger = YarLogger::init(Some(&path), LogLevel::Debug).unwrap();
            logger.log(LogLevel::Debug, "line one");
            logger.log(LogLevel::Error, "line two");
        }

        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        let lines: Vec<&str> = contents.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("DEBUG: line one"));
        assert!(lines[1].contains("ERROR: line two"));
    }

    #[test]
    fn test_stderr_fallback() {
        let logger = YarLogger::init(None, LogLevel::Debug);
        assert!(logger.is_ok());
    }

    #[test]
    fn test_level_ordering() {
        assert!(LogLevel::Ok < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Notice);
        assert!(LogLevel::Notice < LogLevel::Warning);
        assert!(LogLevel::Warning < LogLevel::Error);
    }

    // Minimal tempfile helper to avoid pulling in the `tempfile` crate.
    struct TempFile {
        path: std::path::PathBuf,
    }

    impl TempFile {
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn tempfile() -> Result<TempFile, std::io::Error> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("yar_logger_test_{}_{}.log", std::process::id(), id));
        let _ = std::fs::remove_file(&path);
        File::create(&path)?;
        Ok(TempFile { path })
    }
}

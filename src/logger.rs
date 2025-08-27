use log::{Level, LevelFilter, Metadata, Record, SetLoggerError};
use std::sync::OnceLock;

#[derive(Debug)]
pub struct Logger {
    max_level: Level,
}

impl Logger {
    pub fn new(max_level: Level) -> Self {
        Self { max_level }
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.max_level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level_label = match record.level() {
                Level::Error => "error",
                Level::Warn => "warning",
                Level::Info => "note",
                Level::Debug => "debug",
                Level::Trace => "trace",
            };

            eprintln!("{level_label}: {}", record.args());
        }
    }

    fn flush(&self) {}
}

pub fn init(max_level: Level) -> Result<(), SetLoggerError> {
    static LOGGER: OnceLock<Logger> = OnceLock::new();
    let logger: &'static Logger =
        LOGGER.get_or_init(|| Logger::new(max_level));

    log::set_logger(logger)
        .map(|()| log::set_max_level(LevelFilter::Trace))
}

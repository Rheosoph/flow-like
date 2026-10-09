// Compile the exact production adapter against a separate C++ slave stack in Linux.
#[path = "../../../../packages/industrial/src/ethercat.rs"]
pub mod ethercat;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("Industrial request timed out")]
    Timeout,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn require(condition: bool, message: &str) -> Result<()> {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        struct Logger;
        impl log::Log for Logger {
            fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
                metadata.level() <= log::max_level()
            }
            fn log(&self, record: &log::Record<'_>) {
                if self.enabled(record.metadata()) {
                    eprintln!("{}: {}", record.level(), record.args());
                }
            }
            fn flush(&self) {}
        }
        static LOGGER: Logger = Logger;
        log::set_logger(&LOGGER).expect("fixture logger");
        log::set_max_level(if std::env::var_os("FLOW_LIKE_ETHERCAT_DEBUG").is_some() {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Warn
        });
    });
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

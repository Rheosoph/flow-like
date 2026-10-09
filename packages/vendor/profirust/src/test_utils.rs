use std::cell::{Cell, RefCell};

std::thread_local! {
    static LOG_TIMESTAMP: Cell<i64> = Cell::new(0);
    static ACTIVE_ADDR: Cell<crate::Address> = Cell::new(0);
    static ALLOWED_WARNINGS: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());
}

pub fn prepare_test_logger() {
    prepare_test_logger_with_warnings(vec![])
}

pub fn prepare_test_logger_with_warnings(allowed_warnings: Vec<&'static str>) {
    ALLOWED_WARNINGS.set(allowed_warnings);

    let _ = env_logger::builder()
        .is_test(true)
        .format(move |buf, record| {
            use std::io::Write;
            let level_str = match record.level() {
                log::Level::Error => "\x1b[31mERROR\x1b[0m",
                log::Level::Warn => "\x1b[33mWARN \x1b[0m",
                log::Level::Info => "\x1b[34mINFO \x1b[0m",
                log::Level::Debug => "\x1b[35mDEBUG\x1b[0m",
                log::Level::Trace => "\x1b[36mTRACE\x1b[0m",
            };

            if cfg!(test) && record.level() == log::Level::Warn {
                let message = format!("{}", record.args());
                let is_allowed = ALLOWED_WARNINGS.with_borrow(|w| w.contains(&message.trim()));

                if !is_allowed {
                    panic!(
                        "Received denied warning: [{:32} #{}] {}",
                        record.module_path().unwrap_or(""),
                        ACTIVE_ADDR.get(),
                        record.args(),
                    );
                }
            }

            writeln!(
                buf,
                "[{:16} {} {:32} #{}] {}",
                LOG_TIMESTAMP.get(),
                level_str,
                record.module_path().unwrap_or(""),
                ACTIVE_ADDR.get(),
                record.args(),
            )
        })
        .filter_level(log::LevelFilter::Debug)
        .filter_module("profirust::phy::simulator", log::LevelFilter::Trace)
        .filter_module("profirust::fdl", log::LevelFilter::Trace)
        .filter_module("profirust::dp", log::LevelFilter::Trace)
        .try_init();

    set_log_timestamp(crate::time::Instant::ZERO);
    set_active_addr(0);
}

pub fn set_log_timestamp(t: crate::time::Instant) {
    LOG_TIMESTAMP.set(t.total_micros());
}

pub fn set_active_addr(addr: u8) {
    ACTIVE_ADDR.set(addr);
}

use crate::config::Config;
use std::{fs, io::IsTerminal};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init(config: &Config) -> Option<WorkerGuard> {
    // Protect log files from the first write; directory permissions also restrict access.
    #[cfg(unix)]
    unsafe {
        libc::umask(0o177);
    }
    let filter = EnvFilter::try_new(&config.log_filter).unwrap_or_else(|error| {
        eprintln!("invalid RUST_LOG filter, using info: {error}");
        EnvFilter::new("info")
    });
    let stdout = fmt::layer()
        .with_timer(fmt::time::UtcTime::rfc_3339())
        .with_ansi(std::io::stdout().is_terminal())
        .with_writer(std::io::stdout);

    let file = create_file_writer(config);
    match file {
        Some((writer, guard)) => {
            let file_layer = fmt::layer()
                .with_timer(fmt::time::UtcTime::rfc_3339())
                .with_ansi(false)
                .with_writer(writer);
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout)
                .with(file_layer)
                .init();
            Some(guard)
        }
        None => {
            tracing_subscriber::registry()
                .with(filter)
                .with(stdout)
                .init();
            None
        }
    }
}

fn create_file_writer(
    config: &Config,
) -> Option<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    let directory = config.data_dir.join("logs");
    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!(
            "could not create log directory {}: {error}; logging to stdout only",
            directory.display()
        );
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)) {
            eprintln!(
                "could not secure log directory {}: {error}; logging to stdout only",
                directory.display()
            );
            return None;
        }
    }
    let appender = match RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("sprinter.log")
        .max_log_files(config.log_keep_days)
        .build(&directory)
    {
        Ok(appender) => appender,
        Err(error) => {
            eprintln!(
                "could not open log files in {}: {error}; logging to stdout only",
                directory.display()
            );
            return None;
        }
    };
    let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .lossy(false)
        .finish(appender);
    Some((writer, guard))
}

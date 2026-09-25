use ipnet::IpNet;
use secrecy::SecretString;
use std::{env, error::Error, net::{IpAddr, SocketAddr}, path::PathBuf, str::FromStr};
use thiserror::Error as ThisError;

#[derive(Debug)]
pub struct Config {
    pub master_password: Option<SecretString>,
    pub data_dir: PathBuf,
    pub bind_address: SocketAddr,
    pub trusted_proxies: Vec<IpNet>,
    pub backup_keep: usize,
    pub worker_threads: usize,
    pub log_filter: String,
    pub log_keep_days: usize,
    pub insecure_cookies: bool,
    pub openrouter_base_url: String,
}

#[derive(Debug, ThisError)]
enum ConfigError {
    #[error("invalid {name} value {value:?}: {source}")]
    Invalid { name: &'static str, value: String, #[source] source: Box<dyn Error + Send + Sync> },
    #[error("{0} must be greater than zero")]
    MustBePositive(&'static str),
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let bind = env::var("BIND").unwrap_or_else(|_| "0.0.0.0".to_owned());
        let port = parse_or("PORT", 8080_u16)?;
        let bind_ip = IpAddr::from_str(&bind)
            .map_err(|e| ConfigError::Invalid { name: "BIND", value: bind.clone(), source: Box::new(e) })?;
        let bind_address = SocketAddr::new(bind_ip, port);
        let trusted_proxies = env::var("TRUSTED_PROXIES").unwrap_or_default()
            .split(',').filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().parse::<IpNet>()
                .map_err(|e| ConfigError::Invalid { name: "TRUSTED_PROXIES", value: value.to_owned(), source: Box::new(e) }))
            .collect::<Result<Vec<_>, _>>()?;
        let worker_threads = parse_or("WORKER_THREADS", 2_usize)?;
        let log_keep_days = parse_or("LOG_KEEP_DAYS", 30_usize)?;
        let backup_keep = parse_or("BACKUP_KEEP", 7_usize)?;
        if worker_threads == 0 { return Err(Box::new(ConfigError::MustBePositive("WORKER_THREADS"))); }
        if log_keep_days == 0 { return Err(Box::new(ConfigError::MustBePositive("LOG_KEEP_DAYS"))); }

        Ok(Self {
            master_password: env::var("SPRINTER_PASSWORD").ok().map(SecretString::from),
            data_dir: env::var_os("DATA_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/var/lib/sprinter")),
            bind_address,
            trusted_proxies,
            backup_keep,
            worker_threads,
            log_filter: env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned()),
            log_keep_days,
            insecure_cookies: parse_or("SPRINTER_INSECURE_COOKIES", false)?,
            openrouter_base_url: env::var("OPENROUTER_BASE_URL").unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_owned()),
        })
    }
}

fn parse_or<T>(name: &'static str, default: T) -> Result<T, Box<dyn Error>>
where T: FromStr, T::Err: Error + Send + Sync + 'static {
    match env::var(name) {
        Ok(value) => value.parse().map_err(|e| Box::new(ConfigError::Invalid { name, value, source: Box::new(e) }) as Box<dyn Error>),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(Box::new(error)),
    }
}

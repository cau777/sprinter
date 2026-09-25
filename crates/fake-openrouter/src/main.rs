use std::{net::SocketAddr, str::FromStr};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fake_openrouter=info".into()),
        )
        .init();
    let addr = std::env::var("FAKE_OPENROUTER_ADDR")
        .or_else(|_| std::env::var("OPENROUTER_FAKE_ADDR"))
        .unwrap_or_else(|_| "127.0.0.1:4010".to_owned());
    fake_openrouter::serve(SocketAddr::from_str(&addr).expect("invalid FAKE_OPENROUTER_ADDR")).await
}

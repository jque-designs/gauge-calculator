use std::path::PathBuf;

use anyhow::Result;
use axum::http::Method;
use clap::Parser;
use gauge_calculator::{
    api::{router, ApiState},
    config::{default_config_path, AppConfig},
};
use tower_http::cors::{Any, CorsLayer};

#[derive(Debug, Parser)]
#[command(
    name = "gauge-api",
    version,
    about = "Gauge Calculator REST API server"
)]
struct Cli {
    #[arg(short = 'c', long = "config", default_value_os_t = default_config_path())]
    config: PathBuf,

    #[arg(short = 'r', long = "rpc")]
    rpc: Option<String>,

    #[arg(long = "live", default_value_t = false)]
    live: bool,

    #[arg(long = "host", default_value = "127.0.0.1")]
    host: String,

    #[arg(long = "port", default_value_t = 3001)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config = AppConfig::load_or_default(&cli.config)?;
    config.merge_cli_overrides(cli.rpc.clone());

    let state = ApiState::new(config, cli.live);
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);
    let app = router(state).layer(cors);

    let bind = format!("{}:{}", cli.host, cli.port);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("gauge-api listening on http://{bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

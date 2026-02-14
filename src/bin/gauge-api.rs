use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use gauge_calculator::{
    api::{router, ApiState},
    config::{default_config_path, AppConfig},
};

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

    #[arg(long = "port", default_value_t = 3000)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config = AppConfig::load_or_default(&cli.config)?;
    config.merge_cli_overrides(cli.rpc.clone());

    let state = ApiState::new(config, cli.live);
    let app = router(state);

    let bind = format!("{}:{}", cli.host, cli.port);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("gauge-api listening on http://{bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

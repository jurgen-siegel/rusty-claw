use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rustyclaw_core::config::Paths;

#[tokio::main]
async fn main() -> Result<()> {
    let script_dir = env::var("RUSTYCLAW_SCRIPT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
        });

    let paths = Arc::new(Paths::resolve(&script_dir));
    rustyclaw_discord::run(paths).await
}

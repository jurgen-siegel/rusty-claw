mod conversation;
mod invoke;
mod processor;

use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rustyclaw_core::config::Paths;

#[tokio::main]
async fn main() -> Result<()> {
    // Determine script_dir (the rusty-claw installation root)
    let script_dir = env::var("RUSTYCLAW_SCRIPT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
        });

    let paths = Arc::new(Paths::resolve(&script_dir));

    println!("Rusty Claw Queue Processor");
    println!("  Home: {}", paths.rustyclaw_home.display());
    println!("  Incoming: {}", paths.queue_incoming.display());

    processor::run_queue_processor(paths).await
}

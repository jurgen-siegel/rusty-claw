pub mod conversation;
pub mod invoke;
pub mod processor;

use std::sync::Arc;

use anyhow::Result;
use rustyclaw_core::config::Paths;

pub async fn run(paths: Arc<Paths>) -> Result<()> {
    println!("Rusty Claw Queue Processor");
    println!("  Home: {}", paths.rustyclaw_home.display());
    println!("  Incoming: {}", paths.queue_incoming.display());

    processor::run_queue_processor(paths).await
}

use std::process::Command;

use anyhow::Result;
use colored::Colorize;
use rand::Rng;

use rustyclaw_core::config::Paths;
use rustyclaw_core::types::MessageData;

/// Send a message to the queue (written as JSON to incoming/)
pub fn send_message(message: &str, paths: &Paths) -> Result<()> {
    if message.trim().is_empty() {
        println!("{}", "Message cannot be empty.".yellow());
        return Ok(());
    }

    // Ensure queue directories exist
    paths.ensure_queue_dirs()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let message_id = format!("cli-{}-{}", now, random_suffix());

    let msg = MessageData {
        channel: "cli".to_string(),
        sender: "cli-user".to_string(),
        sender_id: Some("cli".to_string()),
        message: message.to_string(),
        timestamp: now,
        message_id: message_id.clone(),
        agent: None,
        files: None,
        conversation_id: None,
        from_agent: None,
    };

    let json = serde_json::to_string_pretty(&msg)?;
    let filename = format!("{}.json", message_id);
    let filepath = paths.queue_incoming.join(&filename);
    std::fs::write(&filepath, &json)?;

    println!("{} Message queued: {}", "✓".green(), filepath.display().to_string().dimmed());
    Ok(())
}

/// View logs (tail -f)
pub fn view_logs(target: &str, paths: &Paths) -> Result<()> {
    let log_dir = paths.rustyclaw_home.join("logs");

    let log_pattern = match target {
        "queue" => log_dir.join("queue.log"),
        "discord" => log_dir.join("discord.log"),
        "telegram" => log_dir.join("telegram.log"),
        "heartbeat" => log_dir.join("heartbeat.log"),
        "all" => {
            // Tail all log files
            let pattern = log_dir.join("*.log");
            println!("Tailing all logs from {}", log_dir.display().to_string().dimmed());
            println!("{}", "(Ctrl+C to stop)".dimmed());
            println!();

            let status = Command::new("tail")
                .args(["-f", &pattern.to_string_lossy()])
                .status()?;

            if !status.success() {
                // Try with explicit file list
                let files: Vec<String> = std::fs::read_dir(&log_dir)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                e.path()
                                    .extension()
                                    .map(|ext| ext == "log")
                                    .unwrap_or(false)
                            })
                            .map(|e| e.path().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                if files.is_empty() {
                    println!("{}", "No log files found.".yellow());
                } else {
                    let mut args = vec!["-f".to_string()];
                    args.extend(files);
                    Command::new("tail").args(&args).status()?;
                }
            }
            return Ok(());
        }
        other => {
            println!(
                "{} Unknown log target '{}'. Options: queue, discord, telegram, heartbeat, all",
                "Error:".red(),
                other
            );
            return Ok(());
        }
    };

    if !log_pattern.exists() {
        println!("{} Log file not found: {}", "Warning:".yellow(), log_pattern.display());
        println!("Waiting for log output...");
    }

    println!(
        "Tailing {}",
        log_pattern.display().to_string().dimmed()
    );
    println!("{}", "(Ctrl+C to stop)".dimmed());
    println!();

    Command::new("tail")
        .args(["-f", &log_pattern.to_string_lossy()])
        .status()?;

    Ok(())
}

fn random_suffix() -> String {
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| {
            let idx = rng.gen_range(0..36u8);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}

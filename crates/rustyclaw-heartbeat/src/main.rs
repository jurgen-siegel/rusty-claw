use std::env;
use std::path::PathBuf;

use anyhow::Result;

use rustyclaw_core::config::{get_agents, get_settings, get_workspace_path, Paths};
use rustyclaw_core::logging::log;
use rustyclaw_core::types::{MessageData, ResponseData};

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

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

    let paths = Paths::resolve(&script_dir);

    // Use a separate log file for heartbeat
    let log_file = paths.rustyclaw_home.join("logs/heartbeat.log");
    if let Some(dir) = log_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::create_dir_all(&paths.queue_incoming);
    let _ = std::fs::create_dir_all(&paths.queue_outgoing);

    // Read heartbeat interval from settings (default 3600 seconds)
    let settings = get_settings(&paths.settings_file).unwrap_or_default();
    let interval_secs = settings
        .monitoring
        .as_ref()
        .and_then(|m| m.heartbeat_interval)
        .unwrap_or(3600);

    log(
        "INFO",
        &format!("Heartbeat started (interval: {}s)", interval_secs),
        &log_file,
    );

    // Set up graceful shutdown
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        // Sleep first, then check
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)) => {}
            _ = &mut shutdown => {
                log("INFO", "Heartbeat shutting down...", &log_file);
                break;
            }
        }

        log("INFO", "Heartbeat check - scanning all agents...", &log_file);

        // Reload settings each cycle
        let settings = match get_settings(&paths.settings_file) {
            Ok(s) => s,
            Err(_) => {
                log("WARN", "No settings file found, skipping heartbeat", &log_file);
                continue;
            }
        };

        let agents = get_agents(&settings);
        let workspace_path = get_workspace_path(&settings);

        let mut agent_count = 0;

        for (agent_id, agent) in &agents {
            agent_count += 1;

            // Get agent's working directory to find heartbeat.md
            let agent_dir = if agent.working_directory.is_empty() {
                workspace_path.join(agent_id)
            } else {
                let wd = PathBuf::from(&agent.working_directory);
                if wd.is_absolute() { wd } else { workspace_path.join(&agent.working_directory) }
            };

            // Read agent-specific heartbeat.md or use default prompt
            let heartbeat_file = agent_dir.join("heartbeat.md");
            let prompt = if heartbeat_file.exists() {
                match std::fs::read_to_string(&heartbeat_file) {
                    Ok(content) => {
                        log(
                            "INFO",
                            &format!("  -> Agent @{}: using custom heartbeat.md", agent_id),
                            &log_file,
                        );
                        content
                    }
                    Err(_) => {
                        log(
                            "INFO",
                            &format!("  -> Agent @{}: using default prompt", agent_id),
                            &log_file,
                        );
                        "Quick status check: Any pending tasks? Keep response brief.".to_string()
                    }
                }
            } else {
                log(
                    "INFO",
                    &format!("  -> Agent @{}: using default prompt", agent_id),
                    &log_file,
                );
                "Quick status check: Any pending tasks? Keep response brief.".to_string()
            };

            // Generate unique message ID
            let message_id = format!("heartbeat_{}_{}", agent_id, now_millis());

            // Write to queue with @agent_id routing prefix
            let message = format!("@{} {}", agent_id, prompt);
            let queue_data = MessageData {
                channel: "heartbeat".to_string(),
                sender: "System".to_string(),
                sender_id: Some(format!("heartbeat_{}", agent_id)),
                message,
                timestamp: now_millis(),
                message_id: message_id.clone(),
                agent: None,
                files: None,
                conversation_id: None,
                from_agent: None,
            };

            let queue_file = paths.queue_incoming.join(format!("{}.json", message_id));
            match serde_json::to_string_pretty(&queue_data) {
                Ok(json) => {
                    let _ = std::fs::write(&queue_file, json);
                    log(
                        "INFO",
                        &format!("  Queued for @{}: {}", agent_id, message_id),
                        &log_file,
                    );
                }
                Err(e) => {
                    log(
                        "ERROR",
                        &format!("Failed to write heartbeat for @{}: {}", agent_id, e),
                        &log_file,
                    );
                }
            }
        }

        log(
            "INFO",
            &format!("Heartbeat sent to {} agent(s)", agent_count),
            &log_file,
        );

        // Wait 10 seconds for responses
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        // Check for responses and log brief summaries
        for (agent_id, _) in &agents {
            let prefix = format!("heartbeat_{}_", agent_id);
            let entries = match std::fs::read_dir(&paths.queue_outgoing) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with(&prefix) || !name.ends_with(".json") {
                    continue;
                }

                let file_path = entry.path();
                if let Ok(raw) = std::fs::read_to_string(&file_path) {
                    if let Ok(response) = serde_json::from_str::<ResponseData>(&raw) {
                        let preview: String = response.message.chars().take(80).collect();
                        log(
                            "INFO",
                            &format!("  <- @{}: {}...", agent_id, preview),
                            &log_file,
                        );
                        // Clean up response file
                        let _ = std::fs::remove_file(&file_path);
                    }
                }
            }
        }
    }

    Ok(())
}

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use regex::Regex;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::http::Http;
use serenity::prelude::*;
use tokio::sync::Mutex;

use rustyclaw_core::config::{get_agents, get_settings, get_teams, get_workspace_path, Paths};
use rustyclaw_core::logging::log;
use rustyclaw_core::pairing::ensure_sender_paired;
use rustyclaw_core::types::{MessageData, ResponseData};

/// Track pending messages awaiting a response.
struct PendingMessage {
    channel_id: serenity::model::id::ChannelId,
    message_id: serenity::model::id::MessageId,
    timestamp: u64,
}

struct Handler {
    paths: Arc<Paths>,
    pending: Arc<Mutex<HashMap<String, PendingMessage>>>,
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn sanitize_file_name(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let cleaned: String = base
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() {
        "file.bin".to_string()
    } else {
        trimmed
    }
}

fn build_unique_file_path(dir: &Path, preferred_name: &str) -> PathBuf {
    let clean = sanitize_file_name(preferred_name);
    let ext = Path::new(&clean)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let stem = Path::new(&clean)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut candidate = dir.join(&clean);
    let mut counter = 1;
    while candidate.exists() {
        candidate = dir.join(format!("{}_{}{}", stem, counter, ext));
        counter += 1;
    }
    candidate
}

/// Split long messages to stay within Discord's 2000 char limit.
fn split_message(text: &str, max_length: usize) -> Vec<String> {
    if text.len() <= max_length {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text.to_string();

    while !remaining.is_empty() {
        if remaining.len() <= max_length {
            chunks.push(remaining);
            break;
        }

        // Try to split at newline boundary
        let mut split_index = remaining[..max_length].rfind('\n').unwrap_or(0);
        if split_index == 0 {
            split_index = remaining[..max_length].rfind(' ').unwrap_or(0);
        }
        if split_index == 0 {
            split_index = max_length;
        }

        chunks.push(remaining[..split_index].to_string());
        remaining = remaining[split_index..].trim_start_matches('\n').to_string();
    }

    chunks
}

fn pairing_message(code: &str) -> String {
    format!(
        "This sender is not paired yet.\nYour pairing code: {}\nAsk the Rusty Claw owner to approve you with:\nrustyclaw pairing approve {}",
        code, code
    )
}

fn get_agent_list_text(paths: &Paths) -> String {
    let settings = match get_settings(&paths.settings_file) {
        Ok(s) => s,
        Err(_) => return "Could not load agent configuration.".to_string(),
    };
    let agents = get_agents(&settings);
    if agents.len() <= 1 && agents.contains_key("default") {
        return "No agents configured. Using default single-agent mode.\n\nConfigure agents in `.rustyclaw/settings.json` or run `rustyclaw agent add`.".to_string();
    }
    let mut text = "**Available Agents:**\n".to_string();
    for (id, agent) in &agents {
        text += &format!("\n**@{}** - {}", id, agent.name);
        text += &format!("\n  Provider: {}/{}", agent.provider, agent.model);
        text += &format!("\n  Directory: {}", agent.working_directory);
    }
    text += "\n\nUsage: Start your message with `@agent_id` to route to a specific agent.";
    text
}

fn get_team_list_text(paths: &Paths) -> String {
    let settings = match get_settings(&paths.settings_file) {
        Ok(s) => s,
        Err(_) => return "Could not load team configuration.".to_string(),
    };
    let teams = get_teams(&settings);
    if teams.is_empty() {
        return "No teams configured.\n\nCreate a team with `rustyclaw team add`.".to_string();
    }
    let mut text = "**Available Teams:**\n".to_string();
    for (id, team) in &teams {
        text += &format!("\n**@{}** - {}", id, team.name);
        text += &format!("\n  Agents: {}", team.agents.join(", "));
        text += &format!("\n  Leader: @{}", team.leader_agent);
    }
    text += "\n\nUsage: Start your message with `@team_id` to route to a team.";
    text
}

/// Download a file from a URL to a local path using reqwest.
async fn download_file(url: &str, dest: &Path) -> Result<()> {
    let resp = reqwest::get(url).await?;
    let bytes = resp.bytes().await?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(dest, &bytes)?;
    Ok(())
}

fn random_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let suffix: String = (0..7)
        .map(|_| {
            let idx = rng.gen_range(0..36u8);
            if idx < 10 { (b'0' + idx) as char } else { (b'a' + idx - 10) as char }
        })
        .collect();
    format!("{}_{}", now_millis(), suffix)
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        log(
            "INFO",
            &format!("Discord bot connected as {}", ready.user.name),
            &self.paths.log_file,
        );
        log("INFO", "Listening for DMs...", &self.paths.log_file);
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Skip bot messages
        if msg.author.bot {
            return;
        }
        // Skip guild (server) messages - DM only
        if msg.guild_id.is_some() {
            return;
        }

        let has_attachments = !msg.attachments.is_empty();
        let has_content = !msg.content.trim().is_empty();
        if !has_content && !has_attachments {
            return;
        }

        let sender = &msg.author.name;
        let sender_id = msg.author.id.to_string();
        let message_id = random_id();

        // Download attachments
        let mut downloaded_files: Vec<String> = Vec::new();
        if has_attachments {
            for attachment in &msg.attachments {
                let att_name = if attachment.filename.is_empty() {
                    format!("discord_{}.bin", now_millis())
                } else {
                    attachment.filename.clone()
                };
                let filename = format!("discord_{}_{}", message_id, att_name);
                let local_path = build_unique_file_path(&self.paths.files_dir, &filename);

                match download_file(&attachment.url, &local_path).await {
                    Ok(_) => {
                        log(
                            "INFO",
                            &format!("Downloaded attachment: {}", local_path.file_name().unwrap_or_default().to_string_lossy()),
                            &self.paths.log_file,
                        );
                        downloaded_files.push(local_path.to_string_lossy().to_string());
                    }
                    Err(e) => {
                        log(
                            "ERROR",
                            &format!("Failed to download attachment {}: {}", att_name, e),
                            &self.paths.log_file,
                        );
                    }
                }
            }
        }

        let preview: String = msg.content.chars().take(50).collect();
        let files_note = if !downloaded_files.is_empty() {
            format!(" [+{} file(s)]", downloaded_files.len())
        } else {
            String::new()
        };
        log(
            "INFO",
            &format!("Message from {}: {}{}...", sender, preview, files_note),
            &self.paths.log_file,
        );

        // Pairing check
        let pairing = ensure_sender_paired(
            &self.paths.pairing_file,
            "discord",
            &sender_id,
            sender,
        );
        if !pairing.approved {
            if let Some(code) = &pairing.code {
                if pairing.is_new_pending == Some(true) {
                    log(
                        "INFO",
                        &format!("Blocked unpaired Discord sender {} ({}) with code {}", sender, sender_id, code),
                        &self.paths.log_file,
                    );
                    let _ = msg.reply(&ctx.http, pairing_message(code)).await;
                } else {
                    log(
                        "INFO",
                        &format!("Blocked pending Discord sender {} ({})", sender, sender_id),
                        &self.paths.log_file,
                    );
                }
            }
            return;
        }

        // Check for /agent command
        let agent_re = Regex::new(r"(?i)^[!/]agent$").unwrap();
        if agent_re.is_match(msg.content.trim()) {
            log("INFO", "Agent list command received", &self.paths.log_file);
            let text = get_agent_list_text(&self.paths);
            let _ = msg.reply(&ctx.http, text).await;
            return;
        }

        // Check for /team command
        let team_re = Regex::new(r"(?i)^[!/]team$").unwrap();
        if team_re.is_match(msg.content.trim()) {
            log("INFO", "Team list command received", &self.paths.log_file);
            let text = get_team_list_text(&self.paths);
            let _ = msg.reply(&ctx.http, text).await;
            return;
        }

        // Check for /reset command
        let reset_bare_re = Regex::new(r"(?i)^[!/]reset$").unwrap();
        if reset_bare_re.is_match(msg.content.trim()) {
            let _ = msg
                .reply(&ctx.http, "Usage: `/reset @agent_id [@agent_id2 ...]`\nSpecify which agent(s) to reset.")
                .await;
            return;
        }

        let reset_re = Regex::new(r"(?i)^[!/]reset\s+(.+)$").unwrap();
        if let Some(caps) = reset_re.captures(msg.content.trim()) {
            log("INFO", "Per-agent reset command received", &self.paths.log_file);
            let args_str = &caps[1];
            let agent_args: Vec<String> = args_str
                .split_whitespace()
                .map(|a| a.trim_start_matches('@').to_lowercase())
                .collect();

            match get_settings(&self.paths.settings_file) {
                Ok(settings) => {
                    let agents = get_agents(&settings);
                    let workspace_path = get_workspace_path(&settings);
                    let mut results = Vec::new();
                    for agent_id in &agent_args {
                        if !agents.contains_key(agent_id) {
                            results.push(format!("Agent '{}' not found.", agent_id));
                            continue;
                        }
                        let flag_dir = workspace_path.join(agent_id);
                        let _ = std::fs::create_dir_all(&flag_dir);
                        let _ = std::fs::write(flag_dir.join("reset_flag"), "reset");
                        results.push(format!(
                            "Reset @{} ({}).",
                            agent_id,
                            agents[agent_id].name
                        ));
                    }
                    let _ = msg.reply(&ctx.http, results.join("\n")).await;
                }
                Err(_) => {
                    let _ = msg.reply(&ctx.http, "Could not process reset command. Check settings.").await;
                }
            }
            return;
        }

        // Show typing indicator
        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

        // Build full message with file references
        let mut full_message = msg.content.clone();
        if !downloaded_files.is_empty() {
            let file_refs: Vec<String> = downloaded_files.iter().map(|f| format!("[file: {}]", f)).collect();
            let refs_str = file_refs.join("\n");
            if full_message.is_empty() {
                full_message = refs_str;
            } else {
                full_message = format!("{}\n\n{}", full_message, refs_str);
            }
        }

        // Write to incoming queue
        let queue_data = MessageData {
            channel: "discord".to_string(),
            sender: sender.clone(),
            sender_id: Some(sender_id),
            message: full_message,
            timestamp: now_millis(),
            message_id: message_id.clone(),
            agent: None,
            files: if downloaded_files.is_empty() { None } else { Some(downloaded_files) },
            conversation_id: None,
            from_agent: None,
        };

        let queue_file = self.paths.queue_incoming.join(format!("discord_{}.json", message_id));
        let _ = std::fs::create_dir_all(&self.paths.queue_incoming);
        match serde_json::to_string_pretty(&queue_data) {
            Ok(json) => {
                let _ = std::fs::write(&queue_file, json);
            }
            Err(e) => {
                log("ERROR", &format!("Failed to serialize queue data: {}", e), &self.paths.log_file);
                return;
            }
        }

        log("INFO", &format!("Queued message {}", message_id), &self.paths.log_file);

        // Store pending message
        self.pending.lock().await.insert(
            message_id.clone(),
            PendingMessage {
                channel_id: msg.channel_id,
                message_id: msg.id,
                timestamp: now_millis(),
            },
        );

        // Clean up old pending messages (older than 10 minutes)
        let ten_minutes_ago = now_millis().saturating_sub(10 * 60 * 1000);
        self.pending
            .lock()
            .await
            .retain(|_, v| v.timestamp >= ten_minutes_ago);
    }
}

/// Poll the outgoing queue for Discord responses.
async fn check_outgoing_queue(
    http: &Arc<Http>,
    paths: &Paths,
    pending: &Arc<Mutex<HashMap<String, PendingMessage>>>,
) {
    let outgoing = &paths.queue_outgoing;
    let entries = match std::fs::read_dir(outgoing) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let file_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("discord_") || !name.ends_with(".json") {
            continue;
        }

        let raw = match std::fs::read_to_string(&file_path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let response_data: ResponseData = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                log("ERROR", &format!("Bad response JSON {}: {}", name, e), &paths.log_file);
                continue;
            }
        };

        let msg_id = &response_data.message_id;
        let response_text = &response_data.message;

        // Find pending message
        let pending_lock = pending.lock().await;
        let channel_id = pending_lock.get(msg_id).map(|p| p.channel_id);
        drop(pending_lock);

        if let Some(channel_id) = channel_id {
            // Send attached files
            if let Some(files) = &response_data.files {
                for fp in files {
                    if !Path::new(fp).exists() {
                        continue;
                    }
                    let attachment = serenity::builder::CreateAttachment::path(fp).await;
                    if let Ok(att) = attachment {
                        let builder = serenity::builder::CreateMessage::new().add_file(att);
                        let _ = channel_id.send_message(http, builder).await;
                        log("INFO", &format!("Sent file to Discord: {}", Path::new(fp).file_name().unwrap_or_default().to_string_lossy()), &paths.log_file);
                    }
                }
            }

            // Send message chunks
            if !response_text.is_empty() {
                let chunks = split_message(response_text, 2000);
                for (i, chunk) in chunks.iter().enumerate() {
                    if i == 0 {
                        let pending_lock = pending.lock().await;
                        if let Some(p) = pending_lock.get(msg_id) {
                            let builder = serenity::builder::CreateMessage::new()
                                .content(chunk)
                                .reference_message((channel_id, p.message_id));
                            let _ = channel_id.send_message(http, builder).await;
                        } else {
                            let builder = serenity::builder::CreateMessage::new().content(chunk);
                            let _ = channel_id.send_message(http, builder).await;
                        }
                        drop(pending_lock);
                    } else {
                        let builder = serenity::builder::CreateMessage::new().content(chunk);
                        let _ = channel_id.send_message(http, builder).await;
                    }
                }
            }

            log(
                "INFO",
                &format!("Sent response to {} ({} chars)", response_data.sender, response_text.len()),
                &paths.log_file,
            );

            pending.lock().await.remove(msg_id);
            let _ = std::fs::remove_file(&file_path);
        } else {
            log("WARN", &format!("No pending message for {} and no senderId, cleaning up", msg_id), &paths.log_file);
            let _ = std::fs::remove_file(&file_path);
        }
    }
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

    let paths = Arc::new(Paths::resolve(&script_dir));

    // Ensure directories exist
    let _ = std::fs::create_dir_all(&paths.queue_incoming);
    let _ = std::fs::create_dir_all(&paths.queue_outgoing);
    let _ = std::fs::create_dir_all(&paths.files_dir);
    if let Some(dir) = paths.log_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    // Get Discord bot token
    let token = env::var("DISCORD_BOT_TOKEN")
        .expect("DISCORD_BOT_TOKEN environment variable not set");
    if token == "your_token_here" || token.is_empty() {
        eprintln!("ERROR: DISCORD_BOT_TOKEN is not configured");
        std::process::exit(1);
    }

    let pending: Arc<Mutex<HashMap<String, PendingMessage>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let handler = Handler {
        paths: Arc::clone(&paths),
        pending: Arc::clone(&pending),
    };

    log("INFO", "Starting Discord client...", &paths.log_file);

    let mut client = Client::builder(&token, intents)
        .event_handler(handler)
        .await
        .expect("Error creating Discord client");

    // Spawn outgoing queue poller (1 second interval)
    let http_poll = client.http.clone();
    let paths_poll = Arc::clone(&paths);
    let pending_poll = Arc::clone(&pending);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            check_outgoing_queue(&http_poll, &paths_poll, &pending_poll).await;
        }
    });

    // Spawn typing indicator refresh (every 8 seconds)
    let http_typing = client.http.clone();
    let pending_typing = Arc::clone(&pending);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(8));
        loop {
            interval.tick().await;
            let pending_lock = pending_typing.lock().await;
            for (_, data) in pending_lock.iter() {
                let _ = data.channel_id.broadcast_typing(&http_typing).await;
            }
        }
    });

    // Start client (this blocks)
    if let Err(e) = client.start().await {
        log("ERROR", &format!("Discord client error: {}", e), &paths.log_file);
    }

    Ok(())
}

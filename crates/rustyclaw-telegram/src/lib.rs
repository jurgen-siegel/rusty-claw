use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use regex::Regex;
use teloxide::prelude::*;
use teloxide::types::{InputFile, ReplyParameters};
use tokio::sync::Mutex;

use rustyclaw_core::config::{get_agents, get_settings, get_teams, get_workspace_path, Paths};
use rustyclaw_core::logging::log;
use rustyclaw_core::pairing::ensure_sender_paired;
use rustyclaw_core::types::{MessageData, ResponseData};

struct PendingMessage {
    chat_id: ChatId,
    message_id: teloxide::types::MessageId,
    timestamp: u64,
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
    if trimmed.is_empty() { "file.bin".to_string() } else { trimmed }
}

fn ensure_file_extension(name: &str, fallback_ext: &str) -> String {
    if Path::new(name).extension().is_some() {
        name.to_string()
    } else {
        format!("{}{}", name, fallback_ext)
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

fn ext_from_mime(mime: &str) -> &str {
    match mime {
        "image/jpeg" => ".jpg",
        "image/png" => ".png",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "audio/ogg" => ".ogg",
        "audio/mpeg" => ".mp3",
        "video/mp4" => ".mp4",
        "application/pdf" => ".pdf",
        _ => "",
    }
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
        return "No agents configured. Using default single-agent mode.\n\nConfigure agents in .rustyclaw/settings.json or run: rustyclaw agent add".to_string();
    }
    let mut text = "Available Agents:\n".to_string();
    for (id, agent) in &agents {
        text += &format!("\n@{} - {}", id, agent.name);
        text += &format!("\n  Provider: {}/{}", agent.provider, agent.model);
        text += &format!("\n  Directory: {}", agent.working_directory);
    }
    text += "\n\nUsage: Start your message with @agent_id to route to a specific agent.";
    text
}

fn get_team_list_text(paths: &Paths) -> String {
    let settings = match get_settings(&paths.settings_file) {
        Ok(s) => s,
        Err(_) => return "Could not load team configuration.".to_string(),
    };
    let teams = get_teams(&settings);
    if teams.is_empty() {
        return "No teams configured.\n\nCreate a team with: rustyclaw team add".to_string();
    }
    let mut text = "Available Teams:\n".to_string();
    for (id, team) in &teams {
        text += &format!("\n@{} - {}", id, team.name);
        text += &format!("\n  Agents: {}", team.agents.join(", "));
        text += &format!("\n  Leader: @{}", team.leader_agent);
    }
    text += "\n\nUsage: Start your message with @team_id to route to a team.";
    text
}

async fn download_file(url: &str, dest: &Path) -> Result<()> {
    let resp = reqwest::get(url).await?;
    let bytes = resp.bytes().await?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(dest, &bytes)?;
    Ok(())
}

async fn download_telegram_file(
    bot: &Bot,
    file_id: &str,
    ext: &str,
    message_id: &str,
    original_name: Option<&str>,
    files_dir: &Path,
    log_file: &Path,
) -> Option<String> {
    let file = match bot.get_file(file_id).await {
        Ok(f) => f,
        Err(e) => {
            log("ERROR", &format!("Failed to get file info: {}", e), log_file);
            return None;
        }
    };

    let tg_path = &file.path;
    let token = env::var("TELOXIDE_TOKEN")
        .or_else(|_| env::var("TELEGRAM_BOT_TOKEN"))
        .unwrap_or_default();
    let url = format!("https://api.telegram.org/file/bot{}/{}", token, tg_path);

    let tg_basename = Path::new(tg_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let source_name = original_name.unwrap_or(&tg_basename);
    let source_name = if source_name.is_empty() {
        format!("file_{}{}", now_millis(), ext)
    } else {
        source_name.to_string()
    };
    let with_ext = ensure_file_extension(&source_name, if ext.is_empty() { ".bin" } else { ext });
    let filename = format!("telegram_{}_{}", message_id, with_ext);
    let local_path = build_unique_file_path(files_dir, &filename);

    match download_file(&url, &local_path).await {
        Ok(_) => {
            log(
                "INFO",
                &format!("Downloaded file: {}", local_path.file_name().unwrap_or_default().to_string_lossy()),
                log_file,
            );
            Some(local_path.to_string_lossy().to_string())
        }
        Err(e) => {
            log("ERROR", &format!("Failed to download file: {}", e), log_file);
            None
        }
    }
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

async fn handle_message(
    bot: &Bot,
    msg: &Message,
    paths: &Paths,
    pending: &Arc<Mutex<HashMap<String, PendingMessage>>>,
) {
    if !msg.chat.is_private() {
        return;
    }

    let message_text = msg.text().or(msg.caption()).unwrap_or("").to_string();
    let mut downloaded_files: Vec<String> = Vec::new();
    let queue_message_id = random_id();

    if let Some(photos) = msg.photo() {
        if let Some(photo) = photos.last() {
            if let Some(path) = download_telegram_file(
                bot, &photo.file.id, ".jpg", &queue_message_id,
                Some(&format!("photo_{}.jpg", msg.id.0)),
                &paths.files_dir, &paths.log_file,
            ).await {
                downloaded_files.push(path);
            }
        }
    }

    if let Some(doc) = msg.document() {
        let ext = doc.file_name.as_deref()
            .and_then(|n| Path::new(n).extension())
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_else(|| {
                doc.mime_type.as_ref()
                    .map(|m| ext_from_mime(m.as_ref()).to_string())
                    .unwrap_or_default()
            });
        if let Some(path) = download_telegram_file(
            bot, &doc.file.id, &ext, &queue_message_id,
            doc.file_name.as_deref(),
            &paths.files_dir, &paths.log_file,
        ).await {
            downloaded_files.push(path);
        }
    }

    if let Some(audio) = msg.audio() {
        let ext = audio.mime_type.as_ref()
            .map(|m| ext_from_mime(m.as_ref()).to_string())
            .unwrap_or_else(|| ".mp3".to_string());
        if let Some(path) = download_telegram_file(
            bot, &audio.file.id, &ext, &queue_message_id,
            audio.file_name.as_deref(),
            &paths.files_dir, &paths.log_file,
        ).await {
            downloaded_files.push(path);
        }
    }

    if let Some(voice) = msg.voice() {
        if let Some(path) = download_telegram_file(
            bot, &voice.file.id, ".ogg", &queue_message_id,
            Some(&format!("voice_{}.ogg", msg.id.0)),
            &paths.files_dir, &paths.log_file,
        ).await {
            downloaded_files.push(path);
        }
    }

    if let Some(video) = msg.video() {
        let ext = video.mime_type.as_ref()
            .map(|m| ext_from_mime(m.as_ref()).to_string())
            .unwrap_or_else(|| ".mp4".to_string());
        if let Some(path) = download_telegram_file(
            bot, &video.file.id, &ext, &queue_message_id,
            video.file_name.as_deref(),
            &paths.files_dir, &paths.log_file,
        ).await {
            downloaded_files.push(path);
        }
    }

    if let Some(video_note) = msg.video_note() {
        if let Some(path) = download_telegram_file(
            bot, &video_note.file.id, ".mp4", &queue_message_id,
            Some(&format!("video_note_{}.mp4", msg.id.0)),
            &paths.files_dir, &paths.log_file,
        ).await {
            downloaded_files.push(path);
        }
    }

    if let Some(sticker) = msg.sticker() {
        let ext = if sticker.is_animated() {
            ".tgs"
        } else if sticker.is_video() {
            ".webm"
        } else {
            ".webp"
        };
        if let Some(path) = download_telegram_file(
            bot, &sticker.file.id, ext, &queue_message_id,
            Some(&format!("sticker_{}{}", msg.id.0, ext)),
            &paths.files_dir, &paths.log_file,
        ).await {
            downloaded_files.push(path);
        }
    }

    let mut message_text = message_text;

    if msg.sticker().is_some() && message_text.is_empty() {
        let emoji = msg.sticker().and_then(|s| s.emoji.clone()).unwrap_or_else(|| "sticker".to_string());
        message_text = format!("[Sticker: {}]", emoji);
    }

    if message_text.trim().is_empty() && downloaded_files.is_empty() {
        return;
    }

    let sender = msg
        .from
        .as_ref()
        .map(|u| {
            let mut name = u.first_name.clone();
            if let Some(ref last) = u.last_name {
                name += &format!(" {}", last);
            }
            name
        })
        .unwrap_or_else(|| "Unknown".to_string());
    let sender_id = msg.chat.id.0.to_string();

    let preview: String = message_text.chars().take(50).collect();
    let files_note = if !downloaded_files.is_empty() {
        format!(" [+{} file(s)]", downloaded_files.len())
    } else {
        String::new()
    };
    log(
        "INFO",
        &format!("Message from {}: {}{}...", sender, preview, files_note),
        &paths.log_file,
    );

    let pairing = ensure_sender_paired(&paths.pairing_file, "telegram", &sender_id, &sender);
    if !pairing.approved {
        if let Some(code) = &pairing.code {
            if pairing.is_new_pending == Some(true) {
                log(
                    "INFO",
                    &format!("Blocked unpaired Telegram sender {} ({}) with code {}", sender, sender_id, code),
                    &paths.log_file,
                );
                let _ = bot.send_message(msg.chat.id, pairing_message(code))
                    .reply_parameters(ReplyParameters::new(msg.id))
                    .await;
            } else {
                log(
                    "INFO",
                    &format!("Blocked pending Telegram sender {} ({})", sender, sender_id),
                    &paths.log_file,
                );
            }
        }
        return;
    }

    let agent_re = Regex::new(r"(?i)^[!/]agent$").unwrap();
    if agent_re.is_match(message_text.trim()) {
        log("INFO", "Agent list command received", &paths.log_file);
        let text = get_agent_list_text(paths);
        let _ = bot.send_message(msg.chat.id, text)
            .reply_parameters(ReplyParameters::new(msg.id))
            .await;
        return;
    }

    let team_re = Regex::new(r"(?i)^[!/]team$").unwrap();
    if team_re.is_match(message_text.trim()) {
        log("INFO", "Team list command received", &paths.log_file);
        let text = get_team_list_text(paths);
        let _ = bot.send_message(msg.chat.id, text)
            .reply_parameters(ReplyParameters::new(msg.id))
            .await;
        return;
    }

    let reset_bare_re = Regex::new(r"(?i)^[!/]reset$").unwrap();
    if reset_bare_re.is_match(message_text.trim()) {
        let _ = bot.send_message(msg.chat.id, "Usage: /reset @agent_id [@agent_id2 ...]\nSpecify which agent(s) to reset.")
            .reply_parameters(ReplyParameters::new(msg.id))
            .await;
        return;
    }

    let reset_re = Regex::new(r"(?i)^[!/]reset\s+(.+)$").unwrap();
    if let Some(caps) = reset_re.captures(message_text.trim()) {
        log("INFO", "Per-agent reset command received", &paths.log_file);
        let args_str = &caps[1];
        let agent_args: Vec<String> = args_str
            .split_whitespace()
            .map(|a| a.trim_start_matches('@').to_lowercase())
            .collect();

        match get_settings(&paths.settings_file) {
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
                    results.push(format!("Reset @{} ({}).", agent_id, agents[agent_id].name));
                }
                let _ = bot.send_message(msg.chat.id, results.join("\n"))
                    .reply_parameters(ReplyParameters::new(msg.id))
                    .await;
            }
            Err(_) => {
                let _ = bot.send_message(msg.chat.id, "Could not process reset command. Check settings.")
                    .reply_parameters(ReplyParameters::new(msg.id))
                    .await;
            }
        }
        return;
    }

    let _ = bot.send_chat_action(msg.chat.id, teloxide::types::ChatAction::Typing).await;

    let mut full_message = message_text;
    if !downloaded_files.is_empty() {
        let file_refs: Vec<String> = downloaded_files.iter().map(|f| format!("[file: {}]", f)).collect();
        let refs_str = file_refs.join("\n");
        if full_message.is_empty() {
            full_message = refs_str;
        } else {
            full_message = format!("{}\n\n{}", full_message, refs_str);
        }
    }

    let queue_data = MessageData {
        channel: "telegram".to_string(),
        sender: sender.clone(),
        sender_id: Some(sender_id),
        message: full_message,
        timestamp: now_millis(),
        message_id: queue_message_id.clone(),
        agent: None,
        files: if downloaded_files.is_empty() { None } else { Some(downloaded_files) },
        conversation_id: None,
        from_agent: None,
    };

    let queue_file = paths.queue_incoming.join(format!("telegram_{}.json", queue_message_id));
    let _ = std::fs::create_dir_all(&paths.queue_incoming);
    match serde_json::to_string_pretty(&queue_data) {
        Ok(json) => { let _ = std::fs::write(&queue_file, json); }
        Err(e) => {
            log("ERROR", &format!("Failed to serialize queue data: {}", e), &paths.log_file);
            return;
        }
    }

    log("INFO", &format!("Queued message {}", queue_message_id), &paths.log_file);

    pending.lock().await.insert(
        queue_message_id,
        PendingMessage {
            chat_id: msg.chat.id,
            message_id: msg.id,
            timestamp: now_millis(),
        },
    );

    let ten_minutes_ago = now_millis().saturating_sub(10 * 60 * 1000);
    pending.lock().await.retain(|_, v| v.timestamp >= ten_minutes_ago);
}

async fn check_outgoing_queue(
    bot: &Bot,
    paths: &Paths,
    pending: &Arc<Mutex<HashMap<String, PendingMessage>>>,
) {
    let outgoing = &paths.queue_outgoing;
    let entries = match std::fs::read_dir(outgoing) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("telegram_") || !name.ends_with(".json") {
            continue;
        }

        let raw = match std::fs::read_to_string(&path) {
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

        let pending_lock = pending.lock().await;
        let pending_msg = pending_lock.get(msg_id).map(|p| (p.chat_id, p.message_id));
        drop(pending_lock);

        let target_chat_id = pending_msg.map(|(cid, _)| cid);

        if let Some(chat_id) = target_chat_id {
            if let Some(files) = &response_data.files {
                for file_path_str in files {
                    let fp = Path::new(file_path_str);
                    if !fp.exists() { continue; }
                    let ext = fp.extension().unwrap_or_default().to_string_lossy().to_lowercase();
                    let input_file = InputFile::file(fp);
                    match ext.as_str() {
                        "jpg" | "jpeg" | "png" | "gif" | "webp" => {
                            let _ = bot.send_photo(chat_id, input_file).await;
                        }
                        "mp3" | "ogg" | "wav" | "m4a" => {
                            let _ = bot.send_audio(chat_id, input_file).await;
                        }
                        "mp4" | "avi" | "mov" | "webm" => {
                            let _ = bot.send_video(chat_id, input_file).await;
                        }
                        _ => {
                            let _ = bot.send_document(chat_id, input_file).await;
                        }
                    }
                    log("INFO", &format!("Sent file to Telegram: {}", fp.file_name().unwrap_or_default().to_string_lossy()), &paths.log_file);
                }
            }

            if !response_text.is_empty() {
                let chunks = split_message(response_text, 4096);
                for (i, chunk) in chunks.iter().enumerate() {
                    if i == 0 {
                        if let Some((_, reply_msg_id)) = pending_msg {
                            let _ = bot.send_message(chat_id, chunk)
                                .reply_parameters(ReplyParameters::new(reply_msg_id))
                                .await;
                        } else {
                            let _ = bot.send_message(chat_id, chunk).await;
                        }
                    } else {
                        let _ = bot.send_message(chat_id, chunk).await;
                    }
                }
            }

            log(
                "INFO",
                &format!("Sent response to {} ({} chars)", response_data.sender, response_text.len()),
                &paths.log_file,
            );

            pending.lock().await.remove(msg_id);
            let _ = std::fs::remove_file(&path);
        } else {
            log("WARN", &format!("No pending message for {} and no valid senderId, cleaning up", msg_id), &paths.log_file);
            let _ = std::fs::remove_file(&path);
        }
    }
}

pub async fn run(paths: Arc<Paths>) -> Result<()> {
    let _ = std::fs::create_dir_all(&paths.queue_incoming);
    let _ = std::fs::create_dir_all(&paths.queue_outgoing);
    let _ = std::fs::create_dir_all(&paths.files_dir);
    if let Some(dir) = paths.log_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    log("INFO", "Starting Telegram client...", &paths.log_file);

    let bot = Bot::from_env();

    match bot.get_me().await {
        Ok(me) => {
            log(
                "INFO",
                &format!("Telegram bot connected as @{}", me.username()),
                &paths.log_file,
            );
        }
        Err(e) => {
            log("ERROR", &format!("Failed to connect: {}", e), &paths.log_file);
            std::process::exit(1);
        }
    }

    log("INFO", "Listening for messages...", &paths.log_file);

    let pending: Arc<Mutex<HashMap<String, PendingMessage>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let bot_poll = bot.clone();
    let paths_poll = Arc::clone(&paths);
    let pending_poll = Arc::clone(&pending);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            check_outgoing_queue(&bot_poll, &paths_poll, &pending_poll).await;
        }
    });

    let bot_typing = bot.clone();
    let pending_typing = Arc::clone(&pending);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(4));
        loop {
            interval.tick().await;
            let pending_lock = pending_typing.lock().await;
            for (_, data) in pending_lock.iter() {
                let _ = bot_typing.send_chat_action(data.chat_id, teloxide::types::ChatAction::Typing).await;
            }
        }
    });

    let paths_handler = Arc::clone(&paths);
    let pending_handler = Arc::clone(&pending);

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let paths = Arc::clone(&paths_handler);
        let pending = Arc::clone(&pending_handler);
        async move {
            handle_message(&bot, &msg, &paths, &pending).await;
            Ok(())
        }
    })
    .await;

    Ok(())
}

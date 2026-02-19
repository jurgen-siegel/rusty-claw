use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;

use rustyclaw_core::config::Paths;
use rustyclaw_core::logging::{emit_event, log};
use rustyclaw_core::types::{
    AgentConfig, Conversation, MessageData, ResponseData, TeamContext,
};

pub const MAX_CONVERSATION_MESSAGES: u32 = 50;
pub const LONG_RESPONSE_THRESHOLD: usize = 4000;

/// If a response exceeds the threshold, save the full text as a .md file
/// and return a truncated preview with the file attached.
pub fn handle_long_response(
    response: &str,
    existing_files: &[String],
    files_dir: &Path,
    log_file: &Path,
) -> (String, Vec<String>) {
    if response.len() <= LONG_RESPONSE_THRESHOLD {
        return (response.to_string(), existing_files.to_vec());
    }

    let filename = format!(
        "response_{}.md",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let file_path = files_dir.join(&filename);
    let _ = std::fs::create_dir_all(files_dir);
    let _ = std::fs::write(&file_path, response);
    log(
        "INFO",
        &format!(
            "Long response ({} chars) saved to {}",
            response.len(),
            filename
        ),
        log_file,
    );

    let preview = format!(
        "{}\n\n_(Full response attached as file)_",
        &response[..LONG_RESPONSE_THRESHOLD]
    );
    let mut files = existing_files.to_vec();
    files.push(file_path.to_string_lossy().to_string());
    (preview, files)
}

/// Collect file references (`[send_file: path]`) from a response text.
pub fn collect_files(response: &str, file_set: &mut HashSet<String>) {
    let re = Regex::new(r"\[send_file:\s*([^\]]+)\]").unwrap();
    for caps in re.captures_iter(response) {
        let path = caps[1].trim().to_string();
        if Path::new(&path).exists() {
            file_set.insert(path);
        }
    }
}

/// Enqueue an internal (agent-to-agent) message into the incoming queue.
pub fn enqueue_internal_message(
    conversation_id: &str,
    from_agent: &str,
    target_agent: &str,
    message: &str,
    original_data: &MessageData,
    queue_incoming: &Path,
    log_file: &Path,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let mut rng = rand::thread_rng();
    let suffix: String = (0..4)
        .map(|_| {
            let idx = rand::Rng::gen_range(&mut rng, 0..36u8);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect();

    let internal_message = MessageData {
        channel: original_data.channel.clone(),
        sender: original_data.sender.clone(),
        sender_id: original_data.sender_id.clone(),
        message: message.to_string(),
        timestamp: now as u64,
        message_id: original_data.message_id.clone(),
        agent: Some(target_agent.to_string()),
        files: None,
        conversation_id: Some(conversation_id.to_string()),
        from_agent: Some(from_agent.to_string()),
    };

    let filename = format!(
        "internal_{}_{}_{}_{}. json",
        conversation_id, target_agent, now, suffix
    );
    // Remove the space in filename (formatting artifact)
    let filename = filename.replace(". json", ".json");

    let _ = std::fs::create_dir_all(queue_incoming);
    match serde_json::to_string_pretty(&internal_message) {
        Ok(json) => {
            let _ = std::fs::write(queue_incoming.join(&filename), json);
        }
        Err(e) => {
            log(
                "ERROR",
                &format!("Failed to serialize internal message: {}", e),
                log_file,
            );
            return;
        }
    }

    log(
        "INFO",
        &format!("Enqueued internal message: @{} -> @{}", from_agent, target_agent),
        log_file,
    );
}

/// Complete a conversation: aggregate responses, write to outgoing queue, save chat history.
pub fn complete_conversation(
    conv: &Conversation,
    paths: &Paths,
    agents: &HashMap<String, AgentConfig>,
) {
    log(
        "INFO",
        &format!(
            "Conversation {} complete -- {} response(s), {} total message(s)",
            conv.id,
            conv.responses.len(),
            conv.total_messages
        ),
        &paths.log_file,
    );
    let team_id = conv.team_context.as_ref().map(|tc| tc.team_id.as_str()).unwrap_or("dispatch");
    emit_event(
        "team_chain_end",
        serde_json::json!({
            "teamId": team_id,
            "totalSteps": conv.responses.len(),
            "agents": conv.responses.iter().map(|s| s.agent_id.as_str()).collect::<Vec<_>>(),
        }),
        &paths.events_dir,
    );

    // Aggregate responses
    let final_response = if conv.responses.len() == 1 {
        conv.responses[0].response.clone()
    } else {
        conv.responses
            .iter()
            .map(|step| format!("@{}: {}", step.agent_id, step.response))
            .collect::<Vec<_>>()
            .join("\n\n------\n\n")
    };

    // Save chat history (only for team conversations)
    if conv.team_context.is_some() {
        save_chat_history(conv, agents, &paths.chats_dir, &paths.log_file);
    }

    // Detect file references
    let mut final_response = final_response.trim().to_string();
    let mut outbound_files: HashSet<String> = conv.files.clone();
    collect_files(&final_response, &mut outbound_files);
    let outbound_files_vec: Vec<String> = outbound_files.into_iter().collect();

    // Remove [send_file: ...] tags
    if !outbound_files_vec.is_empty() {
        let re = Regex::new(r"\[send_file:\s*[^\]]+\]").unwrap();
        final_response = re.replace_all(&final_response, "").trim().to_string();
    }

    // Remove [@agent: ...] tags from final response
    let tag_re = Regex::new(r"\[@\S+?:\s*[\s\S]*?\]").unwrap();
    final_response = tag_re.replace_all(&final_response, "").trim().to_string();

    // Handle long responses
    let (response_message, all_files) = handle_long_response(
        &final_response,
        &outbound_files_vec,
        &paths.files_dir,
        &paths.log_file,
    );

    // Write to outgoing queue
    let response_data = ResponseData {
        channel: conv.channel.clone(),
        sender: conv.sender.clone(),
        message: response_message,
        original_message: conv.original_message.clone(),
        timestamp: now_millis(),
        message_id: conv.message_id.clone(),
        agent: None,
        files: if all_files.is_empty() {
            None
        } else {
            Some(all_files)
        },
    };

    let response_file = if conv.channel == "heartbeat" {
        paths.queue_outgoing.join(format!("{}.json", conv.message_id))
    } else {
        paths.queue_outgoing.join(format!(
            "{}_{}_{}.json",
            conv.channel,
            conv.message_id,
            now_millis()
        ))
    };

    let _ = std::fs::create_dir_all(&paths.queue_outgoing);
    match serde_json::to_string_pretty(&response_data) {
        Ok(json) => {
            let _ = std::fs::write(&response_file, json);
        }
        Err(e) => {
            log(
                "ERROR",
                &format!("Failed to serialize response: {}", e),
                &paths.log_file,
            );
        }
    }

    log(
        "INFO",
        &format!(
            "Response ready [{}] {} ({} chars)",
            conv.channel,
            conv.sender,
            final_response.len()
        ),
        &paths.log_file,
    );
    emit_event(
        "response_ready",
        serde_json::json!({
            "channel": conv.channel,
            "sender": conv.sender,
            "responseLength": final_response.len(),
            "responseText": final_response,
            "messageId": conv.message_id,
        }),
        &paths.events_dir,
    );
}

/// Save the team conversation chat history to a markdown file.
fn save_chat_history(
    conv: &Conversation,
    agents: &HashMap<String, AgentConfig>,
    chats_dir: &Path,
    log_file: &Path,
) {
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        let tc = conv.team_context.as_ref().ok_or("No team context for chat history")?;
        let team_chats_dir = chats_dir.join(&tc.team_id);
        std::fs::create_dir_all(&team_chats_dir)?;

        let mut lines = Vec::new();
        lines.push(format!(
            "# Team Conversation: {} (@{})",
            tc.team.name, tc.team_id
        ));
        lines.push(format!(
            "**Date:** {}",
            chrono::Utc::now().to_rfc3339()
        ));
        lines.push(format!(
            "**Channel:** {} | **Sender:** {}",
            conv.channel, conv.sender
        ));
        lines.push(format!("**Messages:** {}", conv.total_messages));
        lines.push(String::new());
        lines.push("------".to_string());
        lines.push(String::new());
        lines.push("## User Message".to_string());
        lines.push(String::new());
        lines.push(conv.original_message.clone());
        lines.push(String::new());

        for step in &conv.responses {
            let step_label = if let Some(agent) = agents.get(&step.agent_id) {
                format!("{} (@{})", agent.name, step.agent_id)
            } else {
                format!("@{}", step.agent_id)
            };
            lines.push("------".to_string());
            lines.push(String::new());
            lines.push(format!("## {}", step_label));
            lines.push(String::new());
            lines.push(step.response.clone());
            lines.push(String::new());
        }

        let now = chrono::Utc::now();
        let date_time = now.format("%Y-%m-%dT%H-%M-%S").to_string();
        std::fs::write(
            team_chats_dir.join(format!("{}.md", date_time)),
            lines.join("\n"),
        )?;
        log("INFO", "Chat history saved", log_file);
        Ok(())
    })();

    if let Err(e) = result {
        log(
            "ERROR",
            &format!("Failed to save chat history: {}", e),
            log_file,
        );
    }
}

/// Create a new Conversation struct.
pub fn create_conversation(
    message_id: &str,
    channel: &str,
    sender: &str,
    original_message: &str,
    team_context: Option<TeamContext>,
) -> Conversation {
    let conv_id = format!(
        "{}_{}",
        message_id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    Conversation {
        id: conv_id,
        channel: channel.to_string(),
        sender: sender.to_string(),
        original_message: original_message.to_string(),
        message_id: message_id.to_string(),
        pending: 1,
        responses: Vec::new(),
        files: HashSet::new(),
        total_messages: 0,
        max_messages: MAX_CONVERSATION_MESSAGES,
        team_context,
        start_time: now_millis(),
        outgoing_mentions: HashMap::new(),
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::types::TeamConfig;

    #[test]
    fn test_handle_long_response_short() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log_file = tmp.path().join("test.log");
        let files_dir = tmp.path().join("files");

        let (msg, files) = handle_long_response("short text", &[], &files_dir, &log_file);
        assert_eq!(msg, "short text");
        assert!(files.is_empty());
    }

    #[test]
    fn test_handle_long_response_long() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log_file = tmp.path().join("test.log");
        let files_dir = tmp.path().join("files");

        let long_text = "x".repeat(5000);
        let (msg, files) = handle_long_response(&long_text, &[], &files_dir, &log_file);
        assert!(msg.contains("Full response attached as file"));
        assert_eq!(files.len(), 1);
        assert!(Path::new(&files[0]).exists());
    }

    #[test]
    fn test_collect_files_none() {
        let mut set = HashSet::new();
        collect_files("no files here", &mut set);
        assert!(set.is_empty());
    }

    #[test]
    fn test_create_conversation() {
        let tc = TeamContext {
            team_id: "dev".to_string(),
            team: TeamConfig {
                name: "Dev Team".to_string(),
                agents: vec!["coder".to_string()],
                leader_agent: "coder".to_string(),
                description: None,
            },
        };
        let conv = create_conversation("msg1", "discord", "Alice", "hello", Some(tc));
        assert!(conv.id.starts_with("msg1_"));
        assert_eq!(conv.pending, 1);
        assert_eq!(conv.total_messages, 0);
        assert_eq!(conv.max_messages, MAX_CONVERSATION_MESSAGES);
        assert!(conv.team_context.is_some());
    }
}

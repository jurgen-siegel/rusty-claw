use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use regex::Regex;
use tokio::sync::{mpsc, Mutex};

use rustyclaw_core::compaction;
use rustyclaw_core::config::{get_agents, get_settings, get_teams, get_workspace_path, Paths};
use rustyclaw_core::logging::{emit_event, log};
use rustyclaw_core::routing::{
    extract_all_agent_mentions, extract_cross_team_mentions, extract_natural_handoffs,
    extract_teammate_mentions, find_team_for_agent, parse_agent_routing,
};
use rustyclaw_core::session;
use rustyclaw_core::transcript::{self, TranscriptEntry};
use rustyclaw_core::types::{
    ChainStep, Conversation, MessageData, QueueFile, ResponseData, TeamContext,
};

use crate::conversation::{
    collect_files, complete_conversation, create_conversation, enqueue_internal_message,
    handle_long_response,
};
use crate::invoke::{invoke_agent, invoke_agent_with_failover};

/// Maximum age for a conversation before it's considered timed out (30 minutes).
const CONVERSATION_TIMEOUT_MS: u64 = 30 * 60 * 1000;

/// Move orphaned files from processing/ back to incoming/ on startup.
pub fn recover_orphaned_files(paths: &Paths) {
    let processing_dir = &paths.queue_processing;
    if !processing_dir.exists() {
        return;
    }

    let entries = match std::fs::read_dir(processing_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let dest = paths.queue_incoming.join(entry.file_name());
            match std::fs::rename(&path, &dest) {
                Ok(_) => {
                    log(
                        "INFO",
                        &format!("Recovered orphaned file: {}", entry.file_name().to_string_lossy()),
                        &paths.log_file,
                    );
                }
                Err(e) => {
                    log(
                        "ERROR",
                        &format!(
                            "Failed to recover orphaned file {}: {}",
                            entry.file_name().to_string_lossy(),
                            e
                        ),
                        &paths.log_file,
                    );
                }
            }
        }
    }
}

/// List JSON files in the incoming queue, sorted by modification time.
pub fn list_queue_files(queue_incoming: &Path) -> Vec<QueueFile> {
    let entries = match std::fs::read_dir(queue_incoming) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut files: Vec<QueueFile> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                return None;
            }
            let meta = std::fs::metadata(&path).ok()?;
            let time = meta
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            Some(QueueFile {
                name: entry.file_name().to_string_lossy().to_string(),
                path,
                time,
            })
        })
        .collect();

    files.sort_by_key(|f| f.time);
    files
}

/// Peek at a message file to determine which agent it's routed to.
pub fn peek_agent_id(file_path: &Path, paths: &Paths) -> String {
    let result: Result<String, Box<dyn std::error::Error>> = (|| {
        let raw = std::fs::read_to_string(file_path)?;
        let message_data: MessageData = serde_json::from_str(&raw)?;

        let settings = get_settings(&paths.settings_file)?;
        let agents = get_agents(&settings);
        let teams = get_teams(&settings);

        // Check for pre-routed agent
        if let Some(ref agent) = message_data.agent {
            if agents.contains_key(agent) {
                return Ok(agent.clone());
            }
        }

        // Parse @agent_id or @team_id prefix
        let routing = parse_agent_routing(&message_data.message, &agents, &teams);
        Ok(routing.agent_id)
    })();

    result.unwrap_or_else(|_| "default".to_string())
}

/// Process a single message file. This is the heart of the queue processor.
async fn process_message_inner(
    message_file: &Path,
    paths: &Paths,
    conversations: &Arc<Mutex<HashMap<String, Conversation>>>,
) -> Result<()> {
    let processing_file = paths.queue_processing.join(
        message_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
    );

    // Move to processing
    std::fs::rename(message_file, &processing_file)?;

    // Read message
    let raw = std::fs::read_to_string(&processing_file)?;
    let message_data: MessageData = serde_json::from_str(&raw)?;

    let channel = &message_data.channel;
    let sender = &message_data.sender;
    let raw_message = &message_data.message;
    let message_id = &message_data.message_id;
    let is_internal = message_data.conversation_id.is_some();

    let preview: String = raw_message.chars().take(50).collect();
    if is_internal {
        log(
            "INFO",
            &format!(
                "Processing [internal] @{}->@{}: {}...",
                message_data.from_agent.as_deref().unwrap_or("?"),
                message_data.agent.as_deref().unwrap_or("?"),
                preview
            ),
            &paths.log_file,
        );
    } else {
        log(
            "INFO",
            &format!("Processing [{}] from {}: {}...", channel, sender, preview),
            &paths.log_file,
        );
        let msg_preview: String = raw_message.chars().take(120).collect();
        emit_event(
            "message_received",
            serde_json::json!({
                "channel": channel,
                "sender": sender,
                "message": msg_preview,
                "messageId": message_id,
            }),
            &paths.events_dir,
        );
    }

    // Get settings, agents, and teams
    let settings = get_settings(&paths.settings_file)?;
    let agents = get_agents(&settings);
    let teams = get_teams(&settings);
    let workspace_path = get_workspace_path(&settings);

    // Route message to agent (or team)
    let (mut agent_id, mut message, is_team_routed, multi_agents);

    if let Some(ref pre_routed) = message_data.agent {
        if agents.contains_key(pre_routed) {
            agent_id = pre_routed.clone();
            message = raw_message.clone();
            is_team_routed = false;
            multi_agents = Vec::new();
        } else {
            let routing = parse_agent_routing(raw_message, &agents, &teams);
            agent_id = routing.agent_id;
            message = routing.message;
            is_team_routed = routing.is_team;
            multi_agents = routing.multi_agents;
        }
    } else {
        let routing = parse_agent_routing(raw_message, &agents, &teams);
        agent_id = routing.agent_id;
        message = routing.message;
        is_team_routed = routing.is_team;
        multi_agents = routing.multi_agents;
    }

    // Multi-agent dispatch: create ad-hoc conversation and fan out to all agents
    if !is_internal && multi_agents.len() > 1 {
        log(
            "INFO",
            &format!(
                "Multi-agent dispatch: {} agents [{}]",
                multi_agents.len(),
                multi_agents.join(", ")
            ),
            &paths.log_file,
        );
        emit_event(
            "multi_dispatch",
            serde_json::json!({
                "agents": multi_agents,
                "sender": sender,
                "channel": channel,
            }),
            &paths.events_dir,
        );

        // Create an ad-hoc conversation (no team context) to aggregate responses
        let mut conv = create_conversation(
            message_id, channel, sender, raw_message, None,
        );
        conv.pending = multi_agents.len() as i32;
        let conv_id = conv.id.clone();

        {
            let mut convs = conversations.lock().await;
            convs.insert(conv_id.clone(), conv);
        }

        // Enqueue internal messages for each agent
        for target_agent in &multi_agents {
            enqueue_internal_message(
                &conv_id,
                "dispatch",
                target_agent,
                &message,
                &message_data,
                &paths.queue_incoming,
                &paths.log_file,
            );
        }

        std::fs::remove_file(&processing_file)?;
        return Ok(());
    }

    // Fall back to default if agent not found
    if !agents.contains_key(&agent_id) {
        agent_id = "default".to_string();
        message = raw_message.clone();
    }

    // Final fallback: use first available agent
    if !agents.contains_key(&agent_id) {
        if let Some(first_id) = agents.keys().next() {
            agent_id = first_id.clone();
        } else {
            anyhow::bail!("No agents configured");
        }
    }

    let agent = agents[&agent_id].clone();
    log(
        "INFO",
        &format!(
            "Routing to agent: {} ({}) [{}/{}]",
            agent.name, agent_id, agent.provider, agent.model
        ),
        &paths.log_file,
    );
    if !is_internal {
        emit_event(
            "agent_routed",
            serde_json::json!({
                "agentId": agent_id,
                "agentName": agent.name,
                "provider": agent.provider,
                "model": agent.model,
                "isTeamRouted": is_team_routed,
            }),
            &paths.events_dir,
        );
    }

    // Determine team context
    let team_context: Option<TeamContext> = if is_internal {
        let convs = conversations.lock().await;
        message_data
            .conversation_id
            .as_ref()
            .and_then(|cid| convs.get(cid))
            .and_then(|c| c.team_context.clone())
    } else {
        let mut ctx = None;
        if is_team_routed {
            for (tid, t) in &teams {
                if t.leader_agent == agent_id && t.agents.contains(&agent_id) {
                    ctx = Some(TeamContext {
                        team_id: tid.clone(),
                        team: t.clone(),
                    });
                    break;
                }
            }
        }
        if ctx.is_none() {
            ctx = find_team_for_agent(&agent_id, &teams);
        }
        ctx
    };

    // Resolve session state and determine if reset is needed
    let agent_dir = workspace_path.join(&agent_id);
    let (should_reset, _session_id) = session::resolve_should_reset(
        &agent_dir, &agent_id, &agent, channel, sender, &workspace_path,
    );

    // For internal messages: append pending response indicator
    if is_internal {
        if let Some(ref conv_id) = message_data.conversation_id {
            let convs = conversations.lock().await;
            if let Some(conv) = convs.get(conv_id) {
                let others_pending = conv.pending - 1;
                if others_pending > 0 {
                    message = format!(
                        "{}\n\n------\n\n[{} other teammate response(s) are still being processed and will be delivered when ready. Do not re-mention teammates who haven't responded yet.]",
                        message, others_pending
                    );
                }
            }
        }
    }

    // Write user transcript entry
    {
        let transcripts_dir = workspace_path.join(&agent_id).join(".rustyclaw/transcripts");
        let user_entry = TranscriptEntry {
            timestamp: message_data.timestamp,
            agent_id: agent_id.clone(),
            role: "user".to_string(),
            content: message.clone(),
            message_id: Some(message_id.clone()),
            channel: Some(channel.clone()),
            sender: Some(sender.clone()),
            response_length: None,
            entry_type: None,
            chars_before: None,
        };
        let _ = transcript::append_transcript_entry(&transcripts_dir, &user_entry);
    }

    // Invoke agent (with failover support)
    emit_event(
        "chain_step_start",
        serde_json::json!({
            "agentId": agent_id,
            "agentName": agent.name,
            "fromAgent": message_data.from_agent,
        }),
        &paths.events_dir,
    );

    let cooldowns_file = paths.script_dir.join("cooldowns.json");
    let response = match invoke_agent_with_failover(
        &agent,
        &agent_id,
        &message,
        &workspace_path,
        should_reset,
        &agents,
        &teams,
        &paths.script_dir,
        &paths.log_file,
        &cooldowns_file,
        &settings,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let provider_label = match agent.provider.as_str() {
                "openai" => "Codex",
                "opencode" => "OpenCode",
                _ => "Claude",
            };
            log(
                "ERROR",
                &format!("{} error (agent: {}): {}", provider_label, agent_id, e),
                &paths.log_file,
            );
            "Sorry, I encountered an error processing your request. Please check the queue logs."
                .to_string()
        }
    };

    emit_event(
        "chain_step_done",
        serde_json::json!({
            "agentId": agent_id,
            "agentName": agent.name,
            "responseLength": response.len(),
            "responseText": response,
        }),
        &paths.events_dir,
    );

    // Write assistant transcript entry
    {
        let transcripts_dir = workspace_path.join(&agent_id).join(".rustyclaw/transcripts");
        let assistant_entry = TranscriptEntry {
            timestamp: now_millis(),
            agent_id: agent_id.clone(),
            role: "assistant".to_string(),
            content: response.clone(),
            message_id: Some(message_id.clone()),
            channel: Some(channel.clone()),
            sender: Some(sender.clone()),
            response_length: Some(response.len()),
            entry_type: None,
            chars_before: None,
        };
        let _ = transcript::append_transcript_entry(&transcripts_dir, &assistant_entry);
    }

    // Update session state and check for compaction
    {
        let agent_dir = workspace_path.join(&agent_id);
        if let Ok(session_entry) = session::update_session(
            &agent_dir, &agent_id, channel, sender,
            message.len(), response.len(), should_reset,
        ) {
            let context_window = compaction::resolve_context_window(agent.context_window);
            if compaction::should_compact(
                session_entry.total_chars,
                context_window,
                compaction::DEFAULT_RESERVE_TOKENS,
            ) {
                log(
                    "INFO",
                    &format!(
                        "Compaction triggered for agent {} (session chars: {}, threshold: {})",
                        agent_id,
                        session_entry.total_chars,
                        compaction::compaction_threshold_chars(context_window, compaction::DEFAULT_RESERVE_TOKENS)
                    ),
                    &paths.log_file,
                );

                // Ask the agent to summarize
                let compaction_prompt = compaction::build_compaction_prompt();
                let summary = match invoke_agent(
                    &agent, &agent_id, &compaction_prompt, &workspace_path,
                    false, &agents, &teams, &paths.script_dir, &paths.log_file, &settings,
                )
                .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        log(
                            "WARN",
                            &format!("Compaction summarization failed for agent {}: {}", agent_id, e),
                            &paths.log_file,
                        );
                        format!(
                            "[Compaction summary unavailable. {} chars of context were accumulated.]",
                            session_entry.total_chars
                        )
                    }
                };

                // Write compaction entry to transcript
                let transcripts_dir = agent_dir.join(".rustyclaw/transcripts");
                let compaction_entry = TranscriptEntry {
                    timestamp: now_millis(),
                    agent_id: agent_id.clone(),
                    role: "system".to_string(),
                    content: summary.clone(),
                    message_id: None,
                    channel: Some(channel.clone()),
                    sender: Some(sender.clone()),
                    response_length: None,
                    entry_type: Some("compaction".to_string()),
                    chars_before: Some(session_entry.total_chars),
                };
                let _ = transcript::append_transcript_entry(&transcripts_dir, &compaction_entry);

                // Reset session chars and increment compaction count
                let session_key = session::resolve_session_key(&agent_id, channel, sender);
                let mut sessions = session::load_sessions(&agent_dir);
                if let Some(entry) = sessions.get_mut(&session_key) {
                    entry.total_chars = summary.len() as u64;
                    entry.compaction_count += 1;
                }
                let _ = session::save_sessions(&agent_dir, &sessions);

                log(
                    "INFO",
                    &format!(
                        "Compaction complete for agent {} (summary: {} chars, count: {})",
                        agent_id, summary.len(), session_entry.compaction_count + 1
                    ),
                    &paths.log_file,
                );

                emit_event(
                    "compaction",
                    serde_json::json!({
                        "agentId": agent_id,
                        "charsBefore": session_entry.total_chars,
                        "summaryLength": summary.len(),
                        "compactionCount": session_entry.compaction_count + 1,
                    }),
                    &paths.events_dir,
                );
            }
        }
    }

    // --- No team context: check for ad-hoc handoffs before simple response ---
    if team_context.is_none() {
        // Check for handoff mentions even without a team
        let bracket_mentions = extract_all_agent_mentions(
            &response,
            &agent_id,
            &agents,
            &HashSet::new(),
        );
        let already_mentioned: HashSet<String> =
            bracket_mentions.iter().map(|m| m.teammate_id.clone()).collect();
        let natural_mentions =
            extract_natural_handoffs(&response, &agent_id, &agents, &already_mentioned);
        let ad_hoc_mentions: Vec<_> = bracket_mentions
            .into_iter()
            .chain(natural_mentions)
            .collect();

        if !ad_hoc_mentions.is_empty() || is_internal {
            // --- Ad-hoc conversation path (handoffs without team context) ---
            let mut convs = conversations.lock().await;

            let conv_id = if is_internal {
                message_data.conversation_id.clone().unwrap_or_default()
            } else {
                String::new()
            };

            let is_existing = is_internal && convs.contains_key(&conv_id);
            let active_conv_id;

            if is_existing {
                active_conv_id = conv_id;
            } else {
                let conv = create_conversation(
                    message_id, channel, sender, raw_message, None,
                );
                active_conv_id = conv.id.clone();
                log(
                    "INFO",
                    &format!("Ad-hoc conversation started: {}", active_conv_id),
                    &paths.log_file,
                );
                convs.insert(active_conv_id.clone(), conv);
            }

            // Record response and process mentions
            {
                let conv = convs.get_mut(&active_conv_id).unwrap();
                conv.responses.push(ChainStep {
                    agent_id: agent_id.clone(),
                    response: response.clone(),
                });
                conv.total_messages += 1;
                collect_files(&response, &mut conv.files);

                // For internal messages, re-extract mentions from this agent's response
                let mentions = if is_internal {
                    let bracket = extract_all_agent_mentions(
                        &response, &agent_id, &agents, &HashSet::new(),
                    );
                    let already: HashSet<String> =
                        bracket.iter().map(|m| m.teammate_id.clone()).collect();
                    let natural = extract_natural_handoffs(
                        &response, &agent_id, &agents, &already,
                    );
                    bracket.into_iter().chain(natural).collect::<Vec<_>>()
                } else {
                    ad_hoc_mentions
                };

                if !mentions.is_empty() && conv.total_messages < conv.max_messages {
                    conv.pending += mentions.len() as i32;
                    conv.outgoing_mentions
                        .insert(agent_id.clone(), mentions.len() as u32);

                    for mention in &mentions {
                        log(
                            "INFO",
                            &format!(
                                "@{} -> @{} (ad-hoc handoff)",
                                agent_id, mention.teammate_id
                            ),
                            &paths.log_file,
                        );
                        emit_event(
                            "chain_handoff",
                            serde_json::json!({
                                "fromAgent": agent_id,
                                "toAgent": mention.teammate_id,
                            }),
                            &paths.events_dir,
                        );

                        let internal_msg = format!(
                            "[Message from @{}]:\n{}",
                            agent_id, mention.message
                        );
                        enqueue_internal_message(
                            &active_conv_id,
                            &agent_id,
                            &mention.teammate_id,
                            &internal_msg,
                            &message_data,
                            &paths.queue_incoming,
                            &paths.log_file,
                        );
                    }
                }

                conv.pending -= 1;
            }

            // Check if conversation is complete
            let should_complete = {
                let conv = convs.get(&active_conv_id).unwrap();
                conv.pending == 0
            };

            if should_complete {
                let conv = convs.remove(&active_conv_id).unwrap();
                drop(convs);
                complete_conversation(&conv, paths, &agents);
            } else {
                let conv = convs.get(&active_conv_id).unwrap();
                log(
                    "INFO",
                    &format!(
                        "Ad-hoc conversation {}: {} branch(es) still pending",
                        conv.id, conv.pending
                    ),
                    &paths.log_file,
                );
            }

            let _ = std::fs::remove_file(&processing_file);
            return Ok(());
        }

        // --- No handoffs: simple response to user ---
        let mut final_response = response.trim().to_string();

        // Detect files
        let mut outbound_files_set = std::collections::HashSet::new();
        collect_files(&final_response, &mut outbound_files_set);
        let outbound_files: Vec<String> = outbound_files_set.into_iter().collect();
        if !outbound_files.is_empty() {
            let re = Regex::new(r"\[send_file:\s*[^\]]+\]").unwrap();
            final_response = re.replace_all(&final_response, "").trim().to_string();
        }

        // Handle long responses
        let (response_message, all_files) = handle_long_response(
            &final_response,
            &outbound_files,
            &paths.files_dir,
            &paths.log_file,
        );

        let response_data = ResponseData {
            channel: channel.clone(),
            sender: sender.clone(),
            message: response_message,
            original_message: raw_message.clone(),
            timestamp: now_millis(),
            message_id: message_id.clone(),
            agent: Some(agent_id.clone()),
            files: if all_files.is_empty() {
                None
            } else {
                Some(all_files)
            },
        };

        let response_file = if channel == "heartbeat" {
            paths.queue_outgoing.join(format!("{}.json", message_id))
        } else {
            paths.queue_outgoing.join(format!(
                "{}_{}_{}.json",
                channel,
                message_id,
                now_millis()
            ))
        };

        let _ = std::fs::create_dir_all(&paths.queue_outgoing);
        let json = serde_json::to_string_pretty(&response_data)?;
        std::fs::write(&response_file, json)?;

        log(
            "INFO",
            &format!(
                "Response ready [{}] {} via agent:{} ({} chars)",
                channel,
                sender,
                agent_id,
                final_response.len()
            ),
            &paths.log_file,
        );
        emit_event(
            "response_ready",
            serde_json::json!({
                "channel": channel,
                "sender": sender,
                "agentId": agent_id,
                "responseLength": final_response.len(),
                "responseText": final_response,
                "messageId": message_id,
            }),
            &paths.events_dir,
        );

        std::fs::remove_file(&processing_file)?;
        return Ok(());
    }

    // --- Team context: conversation-based message passing ---
    let team_context = team_context.unwrap();

    let mut convs = conversations.lock().await;

    let conv_id = if is_internal {
        message_data.conversation_id.clone().unwrap_or_default()
    } else {
        String::new()
    };

    // Get or create conversation
    let is_existing = is_internal && convs.contains_key(&conv_id);
    let active_conv_id;

    if is_existing {
        active_conv_id = conv_id;
    } else {
        let conv = create_conversation(
            message_id,
            channel,
            sender,
            raw_message,
            Some(team_context.clone()),
        );
        active_conv_id = conv.id.clone();
        log(
            "INFO",
            &format!(
                "Conversation started: {} (team: {})",
                active_conv_id, team_context.team.name
            ),
            &paths.log_file,
        );
        emit_event(
            "team_chain_start",
            serde_json::json!({
                "teamId": team_context.team_id,
                "teamName": team_context.team.name,
                "agents": team_context.team.agents,
                "leader": team_context.team.leader_agent,
            }),
            &paths.events_dir,
        );
        convs.insert(active_conv_id.clone(), conv);
    }

    // Record response
    {
        let conv = convs.get_mut(&active_conv_id).unwrap();
        conv.responses.push(ChainStep {
            agent_id: agent_id.clone(),
            response: response.clone(),
        });
        conv.total_messages += 1;
        collect_files(&response, &mut conv.files);

        // Check for teammate mentions (only within team conversations)
        let team_id = conv.team_context.as_ref().map(|tc| tc.team_id.as_str()).unwrap_or("");
        let teammate_mentions = if !team_id.is_empty() {
            extract_teammate_mentions(
                &response,
                &agent_id,
                team_id,
                &teams,
                &agents,
            )
        } else {
            Vec::new()
        };

        // Collect already-mentioned agents for cross-team dedup
        let mut already_mentioned: HashSet<String> = teammate_mentions
            .iter()
            .map(|m| m.teammate_id.clone())
            .collect();

        // Check for cross-team mentions ([@!agent: msg] syntax)
        let cross_team_mentions = extract_cross_team_mentions(
            &response,
            &agent_id,
            &agents,
            &already_mentioned,
        );
        for m in &cross_team_mentions {
            already_mentioned.insert(m.teammate_id.clone());
        }

        // Check for natural @agent handoffs (bare @agent: patterns without brackets)
        let natural_mentions = extract_natural_handoffs(
            &response,
            &agent_id,
            &agents,
            &already_mentioned,
        );

        let all_mentions_count =
            teammate_mentions.len() + cross_team_mentions.len() + natural_mentions.len();
        if all_mentions_count > 0 && conv.total_messages < conv.max_messages {
            conv.pending += all_mentions_count as i32;
            conv.outgoing_mentions
                .insert(agent_id.clone(), all_mentions_count as u32);

            // Enqueue teammate mentions (within team)
            for mention in &teammate_mentions {
                log(
                    "INFO",
                    &format!("@{} -> @{} (team)", agent_id, mention.teammate_id),
                    &paths.log_file,
                );
                emit_event(
                    "chain_handoff",
                    serde_json::json!({
                        "teamId": team_id,
                        "fromAgent": agent_id,
                        "toAgent": mention.teammate_id,
                    }),
                    &paths.events_dir,
                );

                let internal_msg = format!(
                    "[Message from teammate @{}]:\n{}",
                    agent_id, mention.message
                );
                enqueue_internal_message(
                    &active_conv_id,
                    &agent_id,
                    &mention.teammate_id,
                    &internal_msg,
                    &message_data,
                    &paths.queue_incoming,
                    &paths.log_file,
                );
            }

            // Enqueue cross-team mentions
            for mention in &cross_team_mentions {
                log(
                    "INFO",
                    &format!("@{} -> @{} (cross-team)", agent_id, mention.teammate_id),
                    &paths.log_file,
                );
                emit_event(
                    "cross_team_handoff",
                    serde_json::json!({
                        "fromAgent": agent_id,
                        "toAgent": mention.teammate_id,
                        "conversationId": active_conv_id,
                    }),
                    &paths.events_dir,
                );

                let internal_msg = format!(
                    "[Cross-team message from @{}]:\n{}",
                    agent_id, mention.message
                );
                enqueue_internal_message(
                    &active_conv_id,
                    &agent_id,
                    &mention.teammate_id,
                    &internal_msg,
                    &message_data,
                    &paths.queue_incoming,
                    &paths.log_file,
                );
            }

            // Enqueue natural handoff mentions (bare @agent: patterns)
            for mention in &natural_mentions {
                log(
                    "INFO",
                    &format!("@{} -> @{} (natural handoff)", agent_id, mention.teammate_id),
                    &paths.log_file,
                );
                emit_event(
                    "chain_handoff",
                    serde_json::json!({
                        "fromAgent": agent_id,
                        "toAgent": mention.teammate_id,
                    }),
                    &paths.events_dir,
                );

                let internal_msg = format!(
                    "[Message from @{}]:\n{}",
                    agent_id, mention.message
                );
                enqueue_internal_message(
                    &active_conv_id,
                    &agent_id,
                    &mention.teammate_id,
                    &internal_msg,
                    &message_data,
                    &paths.queue_incoming,
                    &paths.log_file,
                );
            }
        } else if all_mentions_count > 0 {
            log(
                "WARN",
                &format!(
                    "Conversation {} hit max messages ({}) -- not enqueuing further mentions",
                    conv.id, conv.max_messages
                ),
                &paths.log_file,
            );
        }

        // This branch is done
        conv.pending -= 1;
    }

    // Check if conversation is complete
    let should_complete = {
        let conv = convs.get(&active_conv_id).unwrap();
        conv.pending == 0
    };

    if should_complete {
        let conv = convs.remove(&active_conv_id).unwrap();
        // Drop lock before calling complete_conversation
        drop(convs);
        complete_conversation(&conv, paths, &agents);
    } else {
        let conv = convs.get(&active_conv_id).unwrap();
        log(
            "INFO",
            &format!(
                "Conversation {}: {} branch(es) still pending",
                conv.id, conv.pending
            ),
            &paths.log_file,
        );
    }

    // Clean up processing file
    let _ = std::fs::remove_file(&processing_file);
    Ok(())
}

/// Process a single message with error recovery.
pub async fn process_message(
    message_file: PathBuf,
    paths: Arc<Paths>,
    conversations: Arc<Mutex<HashMap<String, Conversation>>>,
) {
    let processing_file = paths.queue_processing.join(
        message_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
    );

    if let Err(e) = process_message_inner(&message_file, &paths, &conversations).await {
        log(
            "ERROR",
            &format!("Processing error: {}", e),
            &paths.log_file,
        );

        // Move back to incoming for retry
        if processing_file.exists() {
            if let Err(e2) = std::fs::rename(&processing_file, &message_file) {
                log(
                    "ERROR",
                    &format!("Failed to move file back: {}", e2),
                    &paths.log_file,
                );
            }
        }
    }
}

/// Log the current agent and team configuration.
pub fn log_agent_config(paths: &Paths) {
    let settings = match get_settings(&paths.settings_file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let agents = get_agents(&settings);
    let teams = get_teams(&settings);

    log(
        "INFO",
        &format!("Loaded {} agent(s):", agents.len()),
        &paths.log_file,
    );
    for (id, agent) in &agents {
        log(
            "INFO",
            &format!(
                "  {}: {} [{}/{}] cwd={}",
                id, agent.name, agent.provider, agent.model, agent.working_directory
            ),
            &paths.log_file,
        );
    }

    if !teams.is_empty() {
        log(
            "INFO",
            &format!("Loaded {} team(s):", teams.len()),
            &paths.log_file,
        );
        for (id, team) in &teams {
            log(
                "INFO",
                &format!(
                    "  {}: {} [agents: {}] leader={}",
                    id,
                    team.name,
                    team.agents.join(", "),
                    team.leader_agent
                ),
                &paths.log_file,
            );
        }
    }
}

/// Main queue processor loop.
/// Uses per-agent mpsc channels for sequential per-agent processing,
/// with different agents running in parallel.
pub async fn run_queue_processor(paths: Arc<Paths>) -> Result<()> {
    // Ensure directories exist
    paths.ensure_queue_dirs()?;
    let _ = std::fs::create_dir_all(&paths.events_dir);
    let _ = std::fs::create_dir_all(&paths.files_dir);
    if let Some(dir) = paths.log_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    log("INFO", "Queue processor started", &paths.log_file);
    recover_orphaned_files(&paths);
    log(
        "INFO",
        &format!("Watching: {}", paths.queue_incoming.display()),
        &paths.log_file,
    );
    log_agent_config(&paths);

    // Emit startup event
    {
        let settings = get_settings(&paths.settings_file).unwrap_or_default();
        let agents = get_agents(&settings);
        let teams = get_teams(&settings);
        emit_event(
            "processor_start",
            serde_json::json!({
                "agents": agents.keys().collect::<Vec<_>>(),
                "teams": teams.keys().collect::<Vec<_>>(),
            }),
            &paths.events_dir,
        );
    }

    // Shared state
    let conversations: Arc<Mutex<HashMap<String, Conversation>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let agent_senders: Arc<Mutex<HashMap<String, mpsc::Sender<PathBuf>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Queued files set to prevent duplicate processing
    let queued_files: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));

    // Set up graceful shutdown
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    // Polling loop (1 second interval, like the TypeScript version)
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Sweep timed-out conversations
                {
                    let now = now_millis();
                    let mut convs = conversations.lock().await;
                    let timed_out: Vec<String> = convs
                        .iter()
                        .filter(|(_, c)| now.saturating_sub(c.start_time) > CONVERSATION_TIMEOUT_MS)
                        .map(|(id, _)| id.clone())
                        .collect();

                    for conv_id in timed_out {
                        if let Some(mut conv) = convs.remove(&conv_id) {
                            log(
                                "WARN",
                                &format!(
                                    "Conversation {} timed out after {}s ({} pending branches)",
                                    conv_id,
                                    (now - conv.start_time) / 1000,
                                    conv.pending
                                ),
                                &paths.log_file,
                            );
                            emit_event(
                                "conversation_timeout",
                                serde_json::json!({
                                    "conversationId": conv_id,
                                    "pending": conv.pending,
                                    "elapsed_ms": now - conv.start_time,
                                }),
                                &paths.events_dir,
                            );
                            // Force-complete the conversation with whatever responses we have
                            conv.pending = 0;
                            let settings = get_settings(&paths.settings_file).unwrap_or_default();
                            let agents = get_agents(&settings);
                            complete_conversation(&conv, &paths, &agents);
                        }
                    }
                }

                let files = list_queue_files(&paths.queue_incoming);
                if files.is_empty() {
                    continue;
                }

                log(
                    "DEBUG",
                    &format!("Found {} message(s) in queue", files.len()),
                    &paths.log_file,
                );

                for file in files {
                    // Skip files already being processed
                    {
                        let mut queued = queued_files.lock().await;
                        if queued.contains(&file.name) {
                            continue;
                        }
                        queued.insert(file.name.clone());
                    }

                    // Determine target agent
                    let target_agent_id = peek_agent_id(&file.path, &paths);

                    // Get or create channel for this agent
                    let sender = {
                        let mut senders = agent_senders.lock().await;
                        if let Some(tx) = senders.get(&target_agent_id) {
                            tx.clone()
                        } else {
                            // Create a new channel for this agent
                            let (tx, mut rx) = mpsc::channel::<PathBuf>(100);
                            let paths_clone = Arc::clone(&paths);
                            let conversations_clone = Arc::clone(&conversations);
                            let queued_clone = Arc::clone(&queued_files);

                            // Spawn agent processing task
                            tokio::spawn(async move {
                                while let Some(msg_path) = rx.recv().await {
                                    let file_name = msg_path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string();

                                    process_message(
                                        msg_path,
                                        Arc::clone(&paths_clone),
                                        Arc::clone(&conversations_clone),
                                    )
                                    .await;

                                    // Remove from queued set
                                    queued_clone.lock().await.remove(&file_name);
                                }
                            });

                            senders.insert(target_agent_id.clone(), tx.clone());
                            tx
                        }
                    };

                    // Send to agent's channel
                    if let Err(e) = sender.send(file.path).await {
                        log(
                            "ERROR",
                            &format!(
                                "Failed to send message to agent {}: {}",
                                target_agent_id, e
                            ),
                            &paths.log_file,
                        );
                        queued_files.lock().await.remove(&file.name);
                    }
                }
            }
            _ = &mut shutdown => {
                log("INFO", "Shutting down queue processor...", &paths.log_file);
                break;
            }
        }
    }

    Ok(())
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

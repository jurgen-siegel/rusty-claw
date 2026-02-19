use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

use rustyclaw_core::agent_setup::{ensure_agent_directory, update_agent_teammates};
use rustyclaw_core::context;
use rustyclaw_core::failover::{
    classify_error, clear_cooldown, cooldown_key, is_in_cooldown, load_cooldowns, record_failure,
    save_cooldowns,
};
use rustyclaw_core::logging::log;
use rustyclaw_core::models::{resolve_claude_model, resolve_codex_model, resolve_opencode_model};
use rustyclaw_core::types::{AgentConfig, Settings, SkillOverride, TeamConfig};

/// Run a command and capture stdout. Returns an error if the process exits non-zero.
pub async fn run_command(command: &str, args: &[&str], cwd: &Path) -> Result<String> {
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .with_context(|| format!("Failed to spawn command: {}", command))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let msg = if stderr.is_empty() {
            format!("Command exited with code {:?}", output.status.code())
        } else {
            stderr
        };
        Err(anyhow::anyhow!(msg))
    }
}

/// Parse Codex JSONL output — extract the final `agent_message` text.
pub fn parse_codex_output(raw: &str) -> String {
    let mut response = String::new();
    for line in raw.trim().lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if json.get("type").and_then(|t| t.as_str()) == Some("item.completed") {
                if let Some(item) = json.get("item") {
                    if item.get("type").and_then(|t| t.as_str()) == Some("agent_message") {
                        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                            response = text.to_string();
                        }
                    }
                }
            }
        }
    }
    if response.is_empty() {
        "Sorry, I could not generate a response from Codex.".to_string()
    } else {
        response
    }
}

/// Parse OpenCode JSONL output — collect `text` type events.
pub fn parse_opencode_output(raw: &str) -> String {
    let mut response = String::new();
    for line in raw.trim().lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if json.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(part) = json.get("part") {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        response = text.to_string();
                    }
                }
            }
        }
    }
    if response.is_empty() {
        "Sorry, I could not generate a response from OpenCode.".to_string()
    } else {
        response
    }
}

/// Invoke an agent with a message. Dispatches to Claude, Codex, or OpenCode CLI
/// depending on the agent's provider. Returns the raw response text.
pub async fn invoke_agent(
    agent: &AgentConfig,
    agent_id: &str,
    message: &str,
    workspace_path: &Path,
    should_reset: bool,
    agents: &HashMap<String, AgentConfig>,
    teams: &HashMap<String, TeamConfig>,
    script_dir: &Path,
    log_file: &Path,
    settings: &Settings,
) -> Result<String> {
    let agent_dir = workspace_path.join(agent_id);
    let is_new = !agent_dir.exists();
    ensure_agent_directory(&agent_dir, script_dir)?;
    if is_new {
        log(
            "INFO",
            &format!("Initialized agent directory with config files: {}", agent_dir.display()),
            log_file,
        );
    }

    // Update AGENTS.md with current teammate info
    let _ = update_agent_teammates(&agent_dir, agent_id, agents, teams);

    // Build skill discovery directories
    let skills_dir_project = script_dir.join("skills");
    let skills_dir_home = agent_dir.join(".rustyclaw/skills");
    let skill_dirs_owned: Vec<PathBuf> = vec![skills_dir_project, skills_dir_home];
    let skill_dirs: Vec<&Path> = skill_dirs_owned.iter().map(|p| p.as_path()).collect();
    let empty_overrides = HashMap::new();
    let skill_overrides: &HashMap<String, SkillOverride> = settings
        .skills
        .as_ref()
        .unwrap_or(&empty_overrides);

    // Build context preamble from bootstrap files, memory, transcripts, and skills
    let context_preamble = context::build_context_preamble(
        &agent_dir,
        agent_id,
        context::MAX_TRANSCRIPT_CONTEXT_CHARS,
        &skill_dirs,
        skill_overrides,
    );
    let enriched_message = if context_preamble.is_empty() {
        message.to_string()
    } else {
        format!("{}{}", context_preamble, message)
    };

    // Resolve working directory
    let working_dir = if agent.working_directory.is_empty() {
        agent_dir.clone()
    } else {
        let wd = PathBuf::from(&agent.working_directory);
        if wd.is_absolute() {
            wd
        } else {
            workspace_path.join(&agent.working_directory)
        }
    };

    // Ensure working directory exists
    if !working_dir.exists() {
        std::fs::create_dir_all(&working_dir)?;
    }

    let provider = if agent.provider.is_empty() {
        "anthropic"
    } else {
        &agent.provider
    };

    match provider {
        "openai" => {
            log(
                "INFO",
                &format!("Using Codex CLI (agent: {})", agent_id),
                log_file,
            );

            let should_resume = !should_reset;
            if should_reset {
                log(
                    "INFO",
                    &format!("Resetting Codex conversation for agent: {}", agent_id),
                    log_file,
                );
            }

            let model_id = resolve_codex_model(&agent.model);
            let mut args: Vec<String> = vec!["exec".to_string()];
            if should_resume {
                args.push("resume".to_string());
                args.push("--last".to_string());
            }
            if !model_id.is_empty() {
                args.push("--model".to_string());
                args.push(model_id);
            }
            args.extend([
                "--skip-git-repo-check".to_string(),
                "--dangerously-bypass-approvals-and-sandbox".to_string(),
                "--json".to_string(),
                enriched_message.clone(),
            ]);

            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let output = run_command("codex", &args_ref, &working_dir).await?;
            Ok(parse_codex_output(&output))
        }
        "opencode" => {
            let model_id = resolve_opencode_model(&agent.model);
            log(
                "INFO",
                &format!(
                    "Using OpenCode CLI (agent: {}, model: {})",
                    agent_id, model_id
                ),
                log_file,
            );

            let continue_conversation = !should_reset;
            if should_reset {
                log(
                    "INFO",
                    &format!("Resetting OpenCode conversation for agent: {}", agent_id),
                    log_file,
                );
            }

            let mut args: Vec<String> = vec![
                "run".to_string(),
                "--format".to_string(),
                "json".to_string(),
            ];
            if !model_id.is_empty() {
                args.push("--model".to_string());
                args.push(model_id);
            }
            if continue_conversation {
                args.push("-c".to_string());
            }
            args.push(enriched_message.clone());

            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let output = run_command("opencode", &args_ref, &working_dir).await?;
            Ok(parse_opencode_output(&output))
        }
        _ => {
            // Default to Claude (Anthropic)
            log(
                "INFO",
                &format!("Using Claude provider (agent: {})", agent_id),
                log_file,
            );

            let continue_conversation = !should_reset;
            if should_reset {
                log(
                    "INFO",
                    &format!("Resetting conversation for agent: {}", agent_id),
                    log_file,
                );
            }

            let model_id = resolve_claude_model(&agent.model);
            let mut args: Vec<String> = vec!["--dangerously-skip-permissions".to_string()];
            if !model_id.is_empty() {
                args.push("--model".to_string());
                args.push(model_id);
            }
            if continue_conversation {
                args.push("-c".to_string());
            }
            args.push("-p".to_string());
            args.push(enriched_message);

            let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            run_command("claude", &args_ref, &working_dir).await
        }
    }
}

/// Invoke an agent with failover support. Tries the primary model first,
/// then falls back to each model in the agent's `fallbacks` list.
/// Respects cooldown periods to avoid hammering failing providers.
pub async fn invoke_agent_with_failover(
    agent: &AgentConfig,
    agent_id: &str,
    message: &str,
    workspace_path: &Path,
    should_reset: bool,
    agents: &HashMap<String, AgentConfig>,
    teams: &HashMap<String, TeamConfig>,
    script_dir: &Path,
    log_file: &Path,
    cooldowns_file: &Path,
    settings: &Settings,
) -> Result<String> {
    let mut cooldowns = load_cooldowns(cooldowns_file);
    let primary_key = cooldown_key(&agent.provider, &agent.model);

    // Try primary model (unless in cooldown)
    if !is_in_cooldown(&cooldowns, &primary_key) {
        match invoke_agent(
            agent, agent_id, message, workspace_path, should_reset, agents, teams, script_dir,
            log_file, settings,
        )
        .await
        {
            Ok(response) => {
                clear_cooldown(&mut cooldowns, &primary_key);
                let _ = save_cooldowns(cooldowns_file, &cooldowns);
                return Ok(response);
            }
            Err(e) => {
                let reason = classify_error(&e.to_string());
                record_failure(&mut cooldowns, &primary_key, reason);
                let _ = save_cooldowns(cooldowns_file, &cooldowns);
                log(
                    "WARN",
                    &format!(
                        "Primary model {}/{} failed for agent {}: {}. Trying fallbacks...",
                        agent.provider, agent.model, agent_id, e
                    ),
                    log_file,
                );
            }
        }
    } else {
        log(
            "INFO",
            &format!(
                "Primary model {}/{} in cooldown for agent {}, trying fallbacks...",
                agent.provider, agent.model, agent_id
            ),
            log_file,
        );
    }

    // Try fallbacks
    let fallbacks = agent.fallbacks.as_deref().unwrap_or(&[]);
    if fallbacks.is_empty() {
        return Err(anyhow::anyhow!(
            "Primary model failed and no fallbacks configured for agent {}",
            agent_id
        ));
    }

    for fallback_model in fallbacks {
        let fb_key = cooldown_key(&agent.provider, fallback_model);
        if is_in_cooldown(&cooldowns, &fb_key) {
            log(
                "INFO",
                &format!("Fallback model {} in cooldown, skipping", fallback_model),
                log_file,
            );
            continue;
        }

        let mut fallback_agent = agent.clone();
        fallback_agent.model = fallback_model.clone();

        log(
            "INFO",
            &format!(
                "Trying fallback model {} for agent {}",
                fallback_model, agent_id
            ),
            log_file,
        );

        match invoke_agent(
            &fallback_agent, agent_id, message, workspace_path, should_reset, agents, teams,
            script_dir, log_file, settings,
        )
        .await
        {
            Ok(response) => {
                clear_cooldown(&mut cooldowns, &fb_key);
                let _ = save_cooldowns(cooldowns_file, &cooldowns);
                return Ok(response);
            }
            Err(e) => {
                let reason = classify_error(&e.to_string());
                record_failure(&mut cooldowns, &fb_key, reason);
                let _ = save_cooldowns(cooldowns_file, &cooldowns);
                log(
                    "WARN",
                    &format!("Fallback model {} failed: {}", fallback_model, e),
                    log_file,
                );
            }
        }
    }

    Err(anyhow::anyhow!(
        "All models failed for agent {} (primary + {} fallbacks)",
        agent_id,
        fallbacks.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_codex_output_agent_message() {
        let raw = r#"{"type":"item.started","item":{"type":"agent_message"}}
{"type":"item.completed","item":{"type":"agent_message","text":"Hello from Codex!"}}"#;
        assert_eq!(parse_codex_output(raw), "Hello from Codex!");
    }

    #[test]
    fn test_parse_codex_output_empty() {
        let raw = r#"{"type":"item.started","item":{"type":"agent_message"}}
{"type":"something_else","data":"irrelevant"}"#;
        assert_eq!(
            parse_codex_output(raw),
            "Sorry, I could not generate a response from Codex."
        );
    }

    #[test]
    fn test_parse_codex_output_last_message_wins() {
        let raw = r#"{"type":"item.completed","item":{"type":"agent_message","text":"First"}}
{"type":"item.completed","item":{"type":"agent_message","text":"Second"}}"#;
        assert_eq!(parse_codex_output(raw), "Second");
    }

    #[test]
    fn test_parse_opencode_output_text() {
        let raw = r#"{"type":"start","data":{}}
{"type":"text","part":{"text":"Hello from OpenCode!"}}
{"type":"end","data":{}}"#;
        assert_eq!(parse_opencode_output(raw), "Hello from OpenCode!");
    }

    #[test]
    fn test_parse_opencode_output_empty() {
        let raw = r#"{"type":"start","data":{}}
{"type":"end","data":{}}"#;
        assert_eq!(
            parse_opencode_output(raw),
            "Sorry, I could not generate a response from OpenCode."
        );
    }

    #[test]
    fn test_parse_codex_output_invalid_json_lines() {
        let raw = "not json at all\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Works\"}}\nmore junk";
        assert_eq!(parse_codex_output(raw), "Works");
    }
}

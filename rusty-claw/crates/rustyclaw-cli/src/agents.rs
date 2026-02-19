use std::path::PathBuf;

use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, Select};

use rustyclaw_core::agent_setup::{ensure_agent_directory, populate_agent_identity, update_agent_teammates};
use rustyclaw_core::config::{get_agents, get_settings, get_teams, get_workspace_path, Paths};
use rustyclaw_core::types::{AgentConfig, ProviderModelConfig, Settings};

/// List all configured agents
pub fn list_agents(paths: &Paths) -> Result<()> {
    let settings = get_settings(&paths.settings_file)?;
    let agents = get_agents(&settings);

    if agents.is_empty() {
        println!("{}", "No agents configured.".yellow());
        println!("Add one with: {}", "rustyclaw agent add".green());
        return Ok(());
    }

    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!("  {}", "Configured Agents".green().bold());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!();

    for (id, agent) in &agents {
        println!(
            "  {} {}",
            format!("@{}", id).bright_white().bold(),
            format!("({})", agent.name).dimmed()
        );
        println!(
            "    Provider: {}  Model: {}",
            agent.provider.bright_white(),
            agent.model.bright_white()
        );
        println!("    Directory: {}", agent.working_directory.dimmed());
        println!();
    }

    Ok(())
}

/// Show agent details
pub fn show_agent(agent_id: &str, paths: &Paths) -> Result<()> {
    let settings = get_settings(&paths.settings_file)?;
    let agents = get_agents(&settings);

    let Some(agent) = agents.get(agent_id) else {
        println!("{} Agent '{}' not found.", "Error:".red(), agent_id);
        println!("Available agents: {}", agents.keys().cloned().collect::<Vec<_>>().join(", "));
        return Ok(());
    };

    println!();
    println!("  {} {}", format!("@{}", agent_id).bright_white().bold(), format!("({})", agent.name).dimmed());
    println!("  Provider:  {}", agent.provider.bright_white());
    println!("  Model:     {}", agent.model.bright_white());
    println!("  Directory: {}", agent.working_directory.bright_white());

    // Check if part of any teams
    let teams = get_teams(&settings);
    let agent_teams: Vec<_> = teams
        .iter()
        .filter(|(_, t)| t.agents.contains(&agent_id.to_string()))
        .collect();

    if !agent_teams.is_empty() {
        println!("  Teams:");
        for (tid, team) in &agent_teams {
            let role = if team.leader_agent == agent_id { " (leader)" } else { "" };
            println!("    - {} {}{}", tid.bright_white(), team.name.dimmed(), role.green());
        }
    }

    // Check reset flag
    let reset_flag = PathBuf::from(&agent.working_directory).join(".rustyclaw/reset_flag");
    if reset_flag.exists() {
        println!("  Status:    {} (pending reset)", "⚑".yellow());
    }

    println!();
    Ok(())
}

/// Add a new agent interactively
pub fn add_agent(paths: &Paths) -> Result<()> {
    let mut settings = get_settings(&paths.settings_file)?;

    println!();
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!("  {}", "Add New Agent".green().bold());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!();

    // Agent ID
    let agent_id: String = Input::new()
        .with_prompt("Agent ID (lowercase, no spaces)")
        .interact_text()?;
    let agent_id = agent_id.trim().to_lowercase().replace(' ', "-");
    let agent_id: String = agent_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();

    if agent_id.is_empty() {
        println!("{}", "Invalid agent ID.".red());
        return Ok(());
    }

    // Check for duplicates
    let existing_agents = get_agents(&settings);
    if existing_agents.contains_key(&agent_id) {
        println!("{} Agent '{}' already exists.", "Error:".red(), agent_id);
        return Ok(());
    }

    // Display name
    let default_name = capitalize(&agent_id);
    let name: String = Input::new()
        .with_prompt("Display name")
        .default(default_name)
        .interact_text()?;

    // Provider
    let providers = &["anthropic", "openai", "opencode"];
    let provider_idx = Select::new()
        .with_prompt("Provider")
        .items(&["Anthropic (Claude)", "OpenAI (Codex/GPT)", "OpenCode"])
        .default(0)
        .interact()?;
    let provider = providers[provider_idx].to_string();

    // Model
    let model = select_model_for_provider(&provider)?;

    // Working directory
    let workspace_path = get_workspace_path(&settings);
    let default_dir = workspace_path.join(&agent_id);
    let working_directory: String = Input::new()
        .with_prompt("Working directory")
        .default(default_dir.to_string_lossy().to_string())
        .interact_text()?;

    // Role description
    println!("{}", "(e.g. \"Writes and debugs code\", \"Reviews PRs and tests\")".yellow());
    let role: String = Input::new()
        .with_prompt("What is this agent's job?")
        .default(String::new())
        .interact_text()?;

    let agent_config = AgentConfig {
        name: name.clone(),
        provider: provider.clone(),
        model: model.clone(),
        working_directory: working_directory.clone(),
        reset_policy: String::new(),
        reset_hour: None,
        idle_timeout_minutes: None,
        context_window: None,
        fallbacks: None,
        cross_team_handoffs: true,
        route_patterns: None,
        route_priority: 0,
    };

    // Save to settings
    let agents = settings.agents.get_or_insert_with(Default::default);
    agents.insert(agent_id.clone(), agent_config);
    save_settings(&paths.settings_file, &settings)?;

    // Create agent directory with templates
    let agent_dir = PathBuf::from(&working_directory);
    ensure_agent_directory(&agent_dir, &paths.script_dir)?;

    // Populate IDENTITY.md with role if provided
    if !role.trim().is_empty() {
        populate_agent_identity(&agent_dir, &name, role.trim())?;
    }

    // Update teammate info for all agents in teams
    let all_agents = get_agents(&settings);
    let all_teams = get_teams(&settings);
    for (aid, a) in &all_agents {
        let _ = update_agent_teammates(
            &PathBuf::from(&a.working_directory),
            aid,
            &all_agents,
            &all_teams,
        );
    }

    println!();
    println!("{} Agent '{}' added!", "✓".green(), agent_id.bright_white());
    println!("  Provider:  {}", provider.bright_white());
    println!("  Model:     {}", model.bright_white());
    println!("  Directory: {}", working_directory.dimmed());
    println!();

    Ok(())
}

/// Remove an agent
pub fn remove_agent(agent_id: &str, paths: &Paths) -> Result<()> {
    let mut settings = get_settings(&paths.settings_file)?;
    let agents = get_agents(&settings);

    if !agents.contains_key(agent_id) {
        println!("{} Agent '{}' not found.", "Error:".red(), agent_id);
        return Ok(());
    }

    // Check if in any teams
    let teams = get_teams(&settings);
    let in_teams: Vec<_> = teams
        .iter()
        .filter(|(_, t)| t.agents.contains(&agent_id.to_string()))
        .map(|(tid, _)| tid.clone())
        .collect();

    if !in_teams.is_empty() {
        println!(
            "{} Agent '{}' is a member of teams: {}",
            "Warning:".yellow(),
            agent_id,
            in_teams.join(", ")
        );
        println!("Remove the agent from these teams first.");
        return Ok(());
    }

    let confirm = Confirm::new()
        .with_prompt(format!("Remove agent '{}'?", agent_id))
        .default(false)
        .interact()?;

    if !confirm {
        println!("Cancelled.");
        return Ok(());
    }

    if let Some(ref mut agents_map) = settings.agents {
        agents_map.remove(agent_id);
    }
    save_settings(&paths.settings_file, &settings)?;

    println!("{} Agent '{}' removed.", "✓".green(), agent_id);
    println!("Note: The agent's working directory was not deleted.");

    Ok(())
}

/// Reset agent conversations
pub fn reset_agents(agent_ids: &[String], paths: &Paths) -> Result<()> {
    let settings = get_settings(&paths.settings_file)?;
    let agents = get_agents(&settings);

    if agent_ids.is_empty() {
        println!("{}", "No agent IDs specified.".yellow());
        println!("Usage: rustyclaw reset <agent_id> [agent_id2 ...]");
        return Ok(());
    }

    for agent_id in agent_ids {
        if let Some(agent) = agents.get(agent_id.as_str()) {
            let flag_dir = PathBuf::from(&agent.working_directory).join(".rustyclaw");
            std::fs::create_dir_all(&flag_dir)?;
            let flag_file = flag_dir.join("reset_flag");
            std::fs::write(&flag_file, "")?;
            println!("{} Agent '{}' will be reset on next message.", "✓".green(), agent_id);
        } else {
            println!("{} Agent '{}' not found.", "Warning:".yellow(), agent_id);
        }
    }

    Ok(())
}

/// Set the default provider (and optionally model)
pub fn set_provider(name: Option<&str>, model: Option<&str>, paths: &Paths) -> Result<()> {
    let mut settings = get_settings(&paths.settings_file)?;

    let Some(provider) = name else {
        // Show current provider
        let current = settings
            .models
            .as_ref()
            .and_then(|m| m.provider.as_deref())
            .unwrap_or("anthropic");
        println!("Current provider: {}", current.bright_white());
        return Ok(());
    };

    // Validate
    if !["anthropic", "openai", "opencode"].contains(&provider) {
        println!("{} Unknown provider '{}'. Valid options: anthropic, openai, opencode", "Error:".red(), provider);
        return Ok(());
    }

    let models = settings.models.get_or_insert_with(Default::default);
    models.provider = Some(provider.to_string());

    // If model specified, set it too
    if let Some(model_name) = model {
        match provider {
            "anthropic" => {
                models.anthropic = Some(ProviderModelConfig {
                    model: Some(model_name.to_string()),
                });
            }
            "openai" => {
                models.openai = Some(ProviderModelConfig {
                    model: Some(model_name.to_string()),
                });
            }
            "opencode" => {
                models.opencode = Some(ProviderModelConfig {
                    model: Some(model_name.to_string()),
                });
            }
            _ => {}
        }
    }

    save_settings(&paths.settings_file, &settings)?;
    println!("{} Provider set to: {}", "✓".green(), provider.bright_white());
    if let Some(model_name) = model {
        println!("  Model: {}", model_name.bright_white());
    }

    Ok(())
}

/// Set the default model
pub fn set_model(name: Option<&str>, paths: &Paths) -> Result<()> {
    let mut settings = get_settings(&paths.settings_file)?;

    let Some(model_name) = name else {
        // Show current model
        let models = settings.models.as_ref();
        let provider = models.and_then(|m| m.provider.as_deref()).unwrap_or("anthropic");
        let model = match provider {
            "openai" => models.and_then(|m| m.openai.as_ref()).and_then(|o| o.model.as_deref()),
            "opencode" => models.and_then(|m| m.opencode.as_ref()).and_then(|o| o.model.as_deref()),
            _ => models.and_then(|m| m.anthropic.as_ref()).and_then(|a| a.model.as_deref()),
        };
        println!("Current model: {} ({})", model.unwrap_or("sonnet").bright_white(), provider.dimmed());
        return Ok(());
    };

    let models = settings.models.get_or_insert_with(Default::default);
    let provider = models.provider.clone().unwrap_or_else(|| "anthropic".to_string());

    match provider.as_str() {
        "openai" => {
            models.openai = Some(ProviderModelConfig {
                model: Some(model_name.to_string()),
            });
        }
        "opencode" => {
            models.opencode = Some(ProviderModelConfig {
                model: Some(model_name.to_string()),
            });
        }
        _ => {
            models.anthropic = Some(ProviderModelConfig {
                model: Some(model_name.to_string()),
            });
        }
    }

    save_settings(&paths.settings_file, &settings)?;
    println!("{} Model set to: {} ({})", "✓".green(), model_name.bright_white(), provider.dimmed());

    Ok(())
}

// --- Helpers ---

fn select_model_for_provider(provider: &str) -> Result<String> {
    match provider {
        "anthropic" => {
            let models = &["sonnet", "opus"];
            let labels = &["Sonnet (fast, recommended)", "Opus (smartest)", "Custom"];
            let idx = Select::new()
                .with_prompt("Model")
                .items(labels)
                .default(0)
                .interact()?;
            if idx < 2 {
                Ok(models[idx].to_string())
            } else {
                let custom: String = Input::new()
                    .with_prompt("Enter model name")
                    .interact_text()?;
                Ok(custom)
            }
        }
        "openai" => {
            let models = &["gpt-5.3-codex", "gpt-5.2"];
            let labels = &["GPT-5.3 Codex (recommended)", "GPT-5.2", "Custom"];
            let idx = Select::new()
                .with_prompt("Model")
                .items(labels)
                .default(0)
                .interact()?;
            if idx < 2 {
                Ok(models[idx].to_string())
            } else {
                let custom: String = Input::new()
                    .with_prompt("Enter model name")
                    .interact_text()?;
                Ok(custom)
            }
        }
        "opencode" => {
            let models = &[
                "opencode/claude-sonnet-4-5",
                "opencode/claude-opus-4-6",
                "opencode/gemini-3-flash",
                "opencode/gemini-3-pro",
                "anthropic/claude-sonnet-4-5",
                "anthropic/claude-opus-4-6",
                "openai/gpt-5.3-codex",
            ];
            let labels = &[
                "opencode/claude-sonnet-4-5 (recommended)",
                "opencode/claude-opus-4-6",
                "opencode/gemini-3-flash",
                "opencode/gemini-3-pro",
                "anthropic/claude-sonnet-4-5",
                "anthropic/claude-opus-4-6",
                "openai/gpt-5.3-codex",
                "Custom",
            ];
            let idx = Select::new()
                .with_prompt("Model")
                .items(labels)
                .default(0)
                .interact()?;
            if idx < 7 {
                Ok(models[idx].to_string())
            } else {
                let custom: String = Input::new()
                    .with_prompt("Enter model name (e.g. provider/model)")
                    .interact_text()?;
                Ok(custom)
            }
        }
        _ => {
            let model: String = Input::new()
                .with_prompt("Model name")
                .interact_text()?;
            Ok(model)
        }
    }
}

fn save_settings(settings_file: &std::path::Path, settings: &Settings) -> Result<()> {
    if let Some(dir) = settings_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    let tmp = settings_file.with_extension("tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, settings_file)?;
    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

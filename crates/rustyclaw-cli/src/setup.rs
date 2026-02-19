use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{bail, Result};
use colored::Colorize;
use dialoguer::{Confirm, Input, MultiSelect, Select};

use rustyclaw_core::agent_setup::{ensure_agent_directory, populate_agent_identity, update_agent_teammates};
use rustyclaw_core::config::{get_agents, get_teams, Paths};
use rustyclaw_core::types::*;

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// Interactive setup wizard — creates settings.json and all directories.
pub fn run_setup(paths: &Paths) -> Result<()> {
    println!();
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".blue());
    println!("{}", "  Rusty Claw - Setup Wizard".green());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".blue());
    println!();

    // Check if settings already exist
    if paths.settings_file.exists() {
        let overwrite = Confirm::new()
            .with_prompt("Settings already exist. Overwrite?")
            .default(false)
            .interact()?;
        if !overwrite {
            println!("Setup cancelled.");
            return Ok(());
        }
    }

    // ─── Channel selection ─────────────────────────────────────────────
    println!("Which messaging channels do you want to enable?");
    println!();

    let mut enabled_channels: Vec<String> = Vec::new();
    let mut discord_token = String::new();
    let mut telegram_token = String::new();

    let enable_discord = Confirm::new()
        .with_prompt("  Enable Discord?")
        .default(false)
        .interact()?;
    if enable_discord {
        enabled_channels.push("discord".to_string());
        println!("    {}", "Discord enabled".green());
    }

    let enable_telegram = Confirm::new()
        .with_prompt("  Enable Telegram?")
        .default(false)
        .interact()?;
    if enable_telegram {
        enabled_channels.push("telegram".to_string());
        println!("    {}", "Telegram enabled".green());
    }
    println!();

    if enabled_channels.is_empty() {
        println!(
            "{}",
            "No channels selected. You can still use 'rustyclaw send' from the CLI.".yellow()
        );
        println!();
    }

    // Collect tokens
    if enabled_channels.contains(&"discord".to_string()) {
        println!("Enter your Discord bot token:");
        println!(
            "{}",
            "(Get one at: https://discord.com/developers/applications)".yellow()
        );
        discord_token = Input::new()
            .with_prompt("Token")
            .interact_text()?;
        if discord_token.is_empty() {
            bail!("Discord bot token is required");
        }
        println!("{}", "Discord token saved".green());
        println!();
    }

    if enabled_channels.contains(&"telegram".to_string()) {
        println!("Enter your Telegram bot token:");
        println!(
            "{}",
            "(Create a bot via @BotFather on Telegram to get a token)".yellow()
        );
        telegram_token = Input::new()
            .with_prompt("Token")
            .interact_text()?;
        if telegram_token.is_empty() {
            bail!("Telegram bot token is required");
        }
        println!("{}", "Telegram token saved".green());
        println!();
    }

    // ─── Provider selection ────────────────────────────────────────────
    let providers = &["Anthropic (Claude) - recommended", "OpenAI (Codex/GPT)", "OpenCode"];
    let provider_idx = Select::new()
        .with_prompt("Which AI provider?")
        .items(providers)
        .default(0)
        .interact()?;

    let provider = match provider_idx {
        0 => "anthropic",
        1 => "openai",
        2 => "opencode",
        _ => "anthropic",
    };
    println!("{}", format!("Provider: {}", provider).green());
    println!();

    // ─── Model selection ───────────────────────────────────────────────
    let model = select_model(provider)?;
    println!("{}", format!("Model: {}", model).green());
    println!();

    // ─── Heartbeat interval ────────────────────────────────────────────
    println!("{}", "(How often agents check in proactively)".yellow());
    let heartbeat_str: String = Input::new()
        .with_prompt("Heartbeat interval in seconds")
        .default("3600".to_string())
        .interact_text()?;
    let heartbeat_interval: u64 = heartbeat_str.parse().unwrap_or(3600);
    println!(
        "{}",
        format!("Heartbeat interval: {}s", heartbeat_interval).green()
    );
    println!();

    // ─── Workspace ─────────────────────────────────────────────────────
    println!("{}", "(Creates ~/your-workspace-name/)".yellow());
    let workspace_name: String = Input::new()
        .with_prompt("Workspace name")
        .default("rustyclaw-workspace".to_string())
        .interact_text()?;
    // Sanitize
    let workspace_name: String = workspace_name
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let workspace_path = home_dir().join(&workspace_name);
    println!(
        "{}",
        format!("Workspace: {}", workspace_path.display()).green()
    );
    println!();

    // ─── Default agent ─────────────────────────────────────────────────
    println!("{}", "(The main AI assistant you'll interact with)".yellow());
    let default_agent_name: String = Input::new()
        .with_prompt("Default agent name")
        .default("assistant".to_string())
        .interact_text()?;
    let default_agent_id: String = default_agent_name
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>()
        .to_lowercase();
    if default_agent_id.is_empty() {
        bail!("Agent name is required");
    }

    // Capitalize for display
    let display_name = capitalize(&default_agent_id);
    let default_agent_dir = workspace_path.join(&default_agent_id);

    println!(
        "{}",
        format!("Default agent: @{} ({})", default_agent_id, display_name).green()
    );

    println!("{}", "(e.g. \"Writes and debugs code\", \"Reviews PRs and tests\")".yellow());
    let default_agent_role: String = Input::new()
        .with_prompt("What is this agent's job?")
        .default(String::new())
        .interact_text()?;
    println!();

    let mut agent_roles: HashMap<String, String> = HashMap::new();
    if !default_agent_role.trim().is_empty() {
        agent_roles.insert(default_agent_id.clone(), default_agent_role.trim().to_string());
    }

    let mut agents: HashMap<String, AgentConfig> = HashMap::new();
    agents.insert(
        default_agent_id.clone(),
        AgentConfig {
            name: display_name,
            provider: provider.to_string(),
            model: model.clone(),
            working_directory: default_agent_dir.to_string_lossy().to_string(),
            reset_policy: String::new(),
            reset_hour: None,
            idle_timeout_minutes: None,
            context_window: None,
            fallbacks: None,
            cross_team_handoffs: true,
            route_patterns: None,
            route_priority: 0,
        },
    );

    // ─── Additional agents ─────────────────────────────────────────────
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".blue());
    println!("{}", "  Additional Agents (Optional)".green());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".blue());
    println!();
    println!("You can set up multiple agents with different roles, models, and working directories.");
    println!("Users route messages with '@agent_id message' in chat.");
    println!();

    loop {
        let add_more = Confirm::new()
            .with_prompt("Add another agent?")
            .default(false)
            .interact()?;
        if !add_more {
            break;
        }

        let agent_id: String = Input::new()
            .with_prompt("  Agent ID (lowercase, no spaces)")
            .interact_text()?;
        let agent_id: String = agent_id
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>()
            .to_lowercase();
        if agent_id.is_empty() {
            println!("{}", "  Invalid ID, skipping".red());
            continue;
        }
        if agents.contains_key(&agent_id) {
            println!("{}", format!("  Agent '{}' already exists, skipping", agent_id).red());
            continue;
        }

        let agent_name: String = Input::new()
            .with_prompt("  Display name")
            .default(capitalize(&agent_id))
            .interact_text()?;

        let agent_providers = &["Anthropic", "OpenAI", "OpenCode"];
        let ap_idx = Select::new()
            .with_prompt("  Provider")
            .items(agent_providers)
            .default(0)
            .interact()?;
        let agent_provider = match ap_idx {
            0 => "anthropic",
            1 => "openai",
            2 => "opencode",
            _ => "anthropic",
        };

        let agent_model = select_model(agent_provider)?;

        let agent_role: String = Input::new()
            .with_prompt("  What is this agent's job?")
            .default(String::new())
            .interact_text()?;
        if !agent_role.trim().is_empty() {
            agent_roles.insert(agent_id.clone(), agent_role.trim().to_string());
        }

        let agent_dir = workspace_path.join(&agent_id);

        agents.insert(
            agent_id.clone(),
            AgentConfig {
                name: agent_name,
                provider: agent_provider.to_string(),
                model: agent_model,
                working_directory: agent_dir.to_string_lossy().to_string(),
                reset_policy: String::new(),
                reset_hour: None,
                idle_timeout_minutes: None,
                context_window: None,
                fallbacks: None,
                cross_team_handoffs: true,
                route_patterns: None,
                route_priority: 0,
            },
        );
        println!("  {}", format!("Agent '{}' added", agent_id).green());
    }

    // ─── Teams (optional) ───────────────────────────────────────────────
    let mut teams_map: HashMap<String, TeamConfig> = HashMap::new();

    if agents.len() >= 2 {
        println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".blue());
        println!("{}", "  Teams (Optional)".green());
        println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".blue());
        println!();
        println!("Teams group agents that collaborate. Messages sent to @team-id");
        println!("go to the team leader, who can hand off to teammates.");
        println!();

        loop {
            let create_team = Confirm::new()
                .with_prompt("Create a team?")
                .default(false)
                .interact()?;
            if !create_team {
                break;
            }

            let team_id: String = Input::new()
                .with_prompt("  Team ID (lowercase, no spaces)")
                .interact_text()?;
            let team_id: String = team_id
                .replace(' ', "-")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
                .to_lowercase();
            if team_id.is_empty() {
                println!("{}", "  Invalid ID, skipping".red());
                continue;
            }
            if teams_map.contains_key(&team_id) || agents.contains_key(&team_id) {
                println!("{}", format!("  ID '{}' already in use, skipping", team_id).red());
                continue;
            }

            let default_name = capitalize(&team_id);
            let team_name: String = Input::new()
                .with_prompt("  Team name")
                .default(default_name)
                .interact_text()?;

            let description: String = Input::new()
                .with_prompt("  Description (what does this team do?)")
                .default(String::new())
                .interact_text()?;
            let description = if description.trim().is_empty() {
                None
            } else {
                Some(description.trim().to_string())
            };

            // Select team members
            let agent_ids: Vec<String> = agents.keys().cloned().collect();
            let agent_labels: Vec<String> = agent_ids
                .iter()
                .map(|id| {
                    let a = &agents[id];
                    format!("@{} ({})", id, a.name)
                })
                .collect();

            println!();
            let selected_indices = MultiSelect::new()
                .with_prompt("  Select team members (space to toggle, enter to confirm)")
                .items(&agent_labels)
                .interact()?;

            if selected_indices.len() < 2 {
                println!("{}", "  Teams require at least 2 agents, skipping".red());
                continue;
            }

            let selected_agents: Vec<String> = selected_indices
                .iter()
                .map(|&i| agent_ids[i].clone())
                .collect();

            // Select leader
            let leader_labels: Vec<String> = selected_agents
                .iter()
                .map(|id| format!("@{}", id))
                .collect();
            let leader_idx = Select::new()
                .with_prompt("  Team leader (receives messages first)")
                .items(&leader_labels)
                .default(0)
                .interact()?;
            let leader_agent = selected_agents[leader_idx].clone();

            teams_map.insert(
                team_id.clone(),
                TeamConfig {
                    name: team_name,
                    agents: selected_agents.clone(),
                    leader_agent: leader_agent.clone(),
                    description,
                },
            );

            println!(
                "  {} Team '{}' created! Leader: {}, Members: {}",
                "✓".green(),
                team_id.bright_white(),
                format!("@{}", leader_agent).green(),
                selected_agents
                    .iter()
                    .map(|id| format!("@{}", id))
                    .collect::<Vec<_>>()
                    .join(", ")
                    .bright_white()
            );
            println!();
        }
    }

    // ─── Build settings ────────────────────────────────────────────────

    let models_config = {
        let mut mc = ModelsConfig::default();
        mc.provider = Some(provider.to_string());
        match provider {
            "anthropic" => {
                mc.anthropic = Some(ProviderModelConfig {
                    model: Some(model.clone()),
                });
            }
            "openai" => {
                mc.openai = Some(ProviderModelConfig {
                    model: Some(model.clone()),
                });
            }
            "opencode" => {
                mc.opencode = Some(ProviderModelConfig {
                    model: Some(model.clone()),
                });
            }
            _ => {}
        }
        mc
    };

    let channels_config = if !enabled_channels.is_empty()
        || !discord_token.is_empty()
        || !telegram_token.is_empty()
    {
        Some(ChannelsConfig {
            enabled: if enabled_channels.is_empty() {
                None
            } else {
                Some(enabled_channels)
            },
            discord: if discord_token.is_empty() {
                None
            } else {
                Some(DiscordChannelConfig {
                    bot_token: Some(discord_token),
                })
            },
            telegram: if telegram_token.is_empty() {
                None
            } else {
                Some(TelegramChannelConfig {
                    bot_token: Some(telegram_token),
                })
            },
        })
    } else {
        None
    };

    let settings = Settings {
        workspace: Some(WorkspaceConfig {
            path: Some(workspace_path.to_string_lossy().to_string()),
            name: Some(workspace_name),
        }),
        channels: channels_config,
        models: Some(models_config),
        agents: Some(agents.clone()),
        teams: if teams_map.is_empty() {
            None
        } else {
            Some(teams_map)
        },
        monitoring: Some(MonitoringConfig {
            heartbeat_interval: Some(heartbeat_interval),
        }),
        skills: None,
    };

    // ─── Write settings and create directories ─────────────────────────

    // Ensure ~/.rustyclaw exists
    std::fs::create_dir_all(&paths.rustyclaw_home)?;
    std::fs::create_dir_all(paths.rustyclaw_home.join("logs"))?;
    std::fs::create_dir_all(&paths.files_dir)?;
    paths.ensure_queue_dirs()?;

    // Write settings.json (atomic)
    let json = serde_json::to_string_pretty(&settings)?;
    let tmp_file = paths.settings_file.with_extension("json.tmp");
    std::fs::write(&tmp_file, format!("{}\n", json))?;
    std::fs::rename(&tmp_file, &paths.settings_file)?;
    println!(
        "{}",
        format!(
            "Configuration saved to {}",
            paths.settings_file.display()
        )
        .green()
    );

    // Create workspace directory
    std::fs::create_dir_all(&workspace_path)?;
    println!(
        "{}",
        format!("Created workspace: {}", workspace_path.display()).green()
    );

    // Create agent directories with templates
    for (id, agent) in &agents {
        let agent_dir = PathBuf::from(&agent.working_directory);
        ensure_agent_directory(&agent_dir, &paths.script_dir)?;

        // Populate IDENTITY.md with role if provided
        if let Some(role) = agent_roles.get(id) {
            populate_agent_identity(&agent_dir, &agent.name, role)?;
        }

        println!(
            "{}",
            format!("Created agent directory: @{} → {}", id, agent_dir.display()).green()
        );
    }

    // Update AGENTS.md with teammate info (if teams were created)
    let all_agents = get_agents(&settings);
    let all_teams = get_teams(&settings);
    if !all_teams.is_empty() {
        for (aid, agent) in &all_agents {
            let _ = update_agent_teammates(
                &PathBuf::from(&agent.working_directory),
                aid,
                &all_agents,
                &all_teams,
            );
        }
    }

    // ─── Done ──────────────────────────────────────────────────────────
    println!();
    println!("You can manage agents later with:");
    println!("  {}    - List agents", "rustyclaw agent list".green());
    println!("  {}     - Add more agents", "rustyclaw agent add".green());
    println!("  {}     - Create a team", "rustyclaw team add".green());
    println!();
    println!("Start Rusty Claw:");
    println!("  {}", "rustyclaw start".green());
    println!();

    Ok(())
}

fn select_model(provider: &str) -> Result<String> {
    match provider {
        "anthropic" => {
            let models = &["Sonnet (fast, recommended)", "Opus (smartest)", "Custom"];
            let idx = Select::new()
                .with_prompt("Which Claude model?")
                .items(models)
                .default(0)
                .interact()?;
            match idx {
                0 => Ok("sonnet".to_string()),
                1 => Ok("opus".to_string()),
                _ => {
                    let m: String = Input::new()
                        .with_prompt("Enter model name")
                        .interact_text()?;
                    Ok(m)
                }
            }
        }
        "openai" => {
            let models = &["GPT-5.3 Codex (recommended)", "GPT-5.2", "Custom"];
            let idx = Select::new()
                .with_prompt("Which OpenAI model?")
                .items(models)
                .default(0)
                .interact()?;
            match idx {
                0 => Ok("gpt-5.3-codex".to_string()),
                1 => Ok("gpt-5.2".to_string()),
                _ => {
                    let m: String = Input::new()
                        .with_prompt("Enter model name")
                        .interact_text()?;
                    Ok(m)
                }
            }
        }
        "opencode" => {
            let models = &[
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
                .with_prompt("Which OpenCode model?")
                .items(models)
                .default(0)
                .interact()?;
            match idx {
                0 => Ok("opencode/claude-sonnet-4-5".to_string()),
                1 => Ok("opencode/claude-opus-4-6".to_string()),
                2 => Ok("opencode/gemini-3-flash".to_string()),
                3 => Ok("opencode/gemini-3-pro".to_string()),
                4 => Ok("anthropic/claude-sonnet-4-5".to_string()),
                5 => Ok("anthropic/claude-opus-4-6".to_string()),
                6 => Ok("openai/gpt-5.3-codex".to_string()),
                _ => {
                    let m: String = Input::new()
                        .with_prompt("Enter model name (e.g. provider/model)")
                        .interact_text()?;
                    Ok(m)
                }
            }
        }
        _ => Ok("sonnet".to_string()),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

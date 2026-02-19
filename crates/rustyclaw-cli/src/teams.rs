use std::path::PathBuf;

use anyhow::Result;
use colored::Colorize;
use dialoguer::{Confirm, Input, MultiSelect, Select};

use rustyclaw_core::agent_setup::update_agent_teammates;
use rustyclaw_core::config::{get_agents, get_settings, get_teams, Paths};
use rustyclaw_core::types::{Settings, TeamConfig};

/// List all configured teams
pub fn list_teams(paths: &Paths) -> Result<()> {
    let settings = get_settings(&paths.settings_file)?;
    let teams = get_teams(&settings);
    let agents = get_agents(&settings);

    if teams.is_empty() {
        println!("{}", "No teams configured.".yellow());
        println!("Create one with: {}", "rustyclaw team add".green());
        return Ok(());
    }

    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!("  {}", "Configured Teams".green().bold());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!();

    for (id, team) in &teams {
        println!(
            "  {} {}",
            format!("@{}", id).bright_white().bold(),
            format!("({})", team.name).dimmed()
        );
        if let Some(ref desc) = team.description {
            println!("    {}", desc.dimmed());
        }
        println!("    Leader: {}", format!("@{}", team.leader_agent).green());
        println!("    Agents:");
        for aid in &team.agents {
            let name = agents
                .get(aid.as_str())
                .map(|a| a.name.as_str())
                .unwrap_or("?");
            let is_leader = if aid == &team.leader_agent {
                " ★".yellow().to_string()
            } else {
                String::new()
            };
            println!("      - {} ({}){}", format!("@{}", aid).bright_white(), name.dimmed(), is_leader);
        }
        println!();
    }

    Ok(())
}

/// Show team details
pub fn show_team(team_id: &str, paths: &Paths) -> Result<()> {
    let settings = get_settings(&paths.settings_file)?;
    let teams = get_teams(&settings);
    let agents = get_agents(&settings);

    let Some(team) = teams.get(team_id) else {
        println!("{} Team '{}' not found.", "Error:".red(), team_id);
        println!("Available teams: {}", teams.keys().cloned().collect::<Vec<_>>().join(", "));
        return Ok(());
    };

    println!();
    println!("  {} {}", format!("@{}", team_id).bright_white().bold(), format!("({})", team.name).dimmed());
    if let Some(ref desc) = team.description {
        println!("  {}", desc.dimmed());
    }
    println!("  Leader: {}", format!("@{}", team.leader_agent).green());
    println!("  Members ({}):", team.agents.len());

    for aid in &team.agents {
        let agent = agents.get(aid.as_str());
        let name = agent.map(|a| a.name.as_str()).unwrap_or("?");
        let provider = agent.map(|a| a.provider.as_str()).unwrap_or("?");
        let model = agent.map(|a| a.model.as_str()).unwrap_or("?");
        let is_leader = if aid == &team.leader_agent { " ★ leader" } else { "" };
        println!(
            "    {} — {} ({}/{}){}",
            format!("@{}", aid).bright_white(),
            name,
            provider.dimmed(),
            model.dimmed(),
            is_leader.green()
        );
    }

    println!();
    Ok(())
}

/// Add a new team interactively
pub fn add_team(paths: &Paths) -> Result<()> {
    let mut settings = get_settings(&paths.settings_file)?;
    let agents = get_agents(&settings);

    if agents.len() < 2 {
        println!("{} Teams require at least 2 agents. You have {}.", "Error:".red(), agents.len());
        println!("Add agents first with: {}", "rustyclaw agent add".green());
        return Ok(());
    }

    println!();
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!("  {}", "Create New Team".green().bold());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!();

    // Team ID
    let team_id: String = Input::new()
        .with_prompt("Team ID (lowercase, no spaces)")
        .interact_text()?;
    let team_id = team_id.trim().to_lowercase().replace(' ', "-");
    let team_id: String = team_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();

    if team_id.is_empty() {
        println!("{}", "Invalid team ID.".red());
        return Ok(());
    }

    // Check for duplicates (teams and agents share namespace)
    let existing_teams = get_teams(&settings);
    if existing_teams.contains_key(&team_id) {
        println!("{} Team '{}' already exists.", "Error:".red(), team_id);
        return Ok(());
    }
    if agents.contains_key(&team_id) {
        println!("{} ID '{}' conflicts with an existing agent.", "Error:".red(), team_id);
        return Ok(());
    }

    // Display name
    let default_name = capitalize(&team_id);
    let name: String = Input::new()
        .with_prompt("Team name")
        .default(default_name)
        .interact_text()?;

    // Description
    let description: String = Input::new()
        .with_prompt("Description (what does this team do?)")
        .default(String::new())
        .interact_text()?;
    let description = if description.trim().is_empty() {
        None
    } else {
        Some(description.trim().to_string())
    };

    // Select agents
    let agent_ids: Vec<String> = agents.keys().cloned().collect();
    let agent_labels: Vec<String> = agent_ids
        .iter()
        .map(|id| {
            let a = &agents[id];
            format!("@{} — {} ({}/{})", id, a.name, a.provider, a.model)
        })
        .collect();

    println!();
    let selected_indices = MultiSelect::new()
        .with_prompt("Select team members (space to toggle, enter to confirm)")
        .items(&agent_labels)
        .interact()?;

    if selected_indices.len() < 2 {
        println!("{} Teams require at least 2 agents.", "Error:".red());
        return Ok(());
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
        .with_prompt("Select team leader (receives messages first)")
        .items(&leader_labels)
        .default(0)
        .interact()?;
    let leader_agent = selected_agents[leader_idx].clone();

    let team_config = TeamConfig {
        name: name.clone(),
        agents: selected_agents.clone(),
        leader_agent: leader_agent.clone(),
        description,
    };

    // Save
    let teams = settings.teams.get_or_insert_with(Default::default);
    teams.insert(team_id.clone(), team_config);
    save_settings(&paths.settings_file, &settings)?;

    // Update AGENTS.md for all team members
    let all_agents = get_agents(&settings);
    let all_teams = get_teams(&settings);
    for aid in &selected_agents {
        if let Some(agent) = all_agents.get(aid.as_str()) {
            let _ = update_agent_teammates(
                &PathBuf::from(&agent.working_directory),
                aid,
                &all_agents,
                &all_teams,
            );
        }
    }

    println!();
    println!("{} Team '{}' created!", "✓".green(), team_id.bright_white());
    println!("  Leader: {}", format!("@{}", leader_agent).green());
    println!("  Members: {}", selected_agents.iter().map(|id| format!("@{}", id)).collect::<Vec<_>>().join(", ").bright_white());
    println!();
    println!("Route messages to this team with: {}", format!("@{} <message>", team_id).bright_white());
    println!();

    Ok(())
}

/// Remove a team
pub fn remove_team(team_id: &str, paths: &Paths) -> Result<()> {
    let mut settings = get_settings(&paths.settings_file)?;
    let teams = get_teams(&settings);

    let Some(team) = teams.get(team_id) else {
        println!("{} Team '{}' not found.", "Error:".red(), team_id);
        return Ok(());
    };

    let members = team.agents.clone();

    let confirm = Confirm::new()
        .with_prompt(format!("Remove team '{}'?", team_id))
        .default(false)
        .interact()?;

    if !confirm {
        println!("Cancelled.");
        return Ok(());
    }

    if let Some(ref mut teams_map) = settings.teams {
        teams_map.remove(team_id);
    }
    save_settings(&paths.settings_file, &settings)?;

    // Update AGENTS.md for former team members
    let all_agents = get_agents(&settings);
    let all_teams = get_teams(&settings);
    for aid in &members {
        if let Some(agent) = all_agents.get(aid.as_str()) {
            let _ = update_agent_teammates(
                &PathBuf::from(&agent.working_directory),
                aid,
                &all_agents,
                &all_teams,
            );
        }
    }

    println!("{} Team '{}' removed.", "✓".green(), team_id);

    Ok(())
}

// --- Helpers ---

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

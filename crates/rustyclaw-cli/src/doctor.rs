use std::collections::HashSet;
use std::process::Command;

use anyhow::Result;
use colored::Colorize;

use rustyclaw_core::config::{get_agents, get_settings, get_teams, Paths};

/// Run the doctor command — check all prerequisites and configuration.
pub fn run_doctor(paths: &Paths) -> Result<()> {
    println!();
    println!("  {}", "Rusty Claw Doctor".green().bold());
    println!("  {}", "Checking your setup...".dimmed());
    println!();

    let mut issues = 0;

    // 1. tmux
    issues += check_tool("tmux", &["tmux", "-V"], &[
        ("macOS", "brew install tmux"),
        ("Ubuntu/Debian", "sudo apt install tmux"),
        ("Fedora", "sudo dnf install tmux"),
    ]);

    // 2. Settings file
    let settings_ok = if paths.settings_file.exists() {
        match get_settings(&paths.settings_file) {
            Ok(s) => {
                if s.agents.is_some() || s.models.is_some() {
                    print_ok(&format!("Settings: {}", paths.settings_file.display()));
                    Some(s)
                } else {
                    print_warn("Settings file exists but has no agents or models configured");
                    println!("         Run: {}", "rustyclaw setup".green());
                    issues += 1;
                    Some(s)
                }
            }
            Err(_) => {
                print_fail("Settings file is corrupted");
                println!("         Fix: delete and re-run {}", "rustyclaw setup".green());
                issues += 1;
                None
            }
        }
    } else {
        print_fail(&format!("Settings file not found: {}", paths.settings_file.display()));
        println!("         Run: {}", "rustyclaw setup".green());
        issues += 1;
        None
    };

    // 3. Check which AI CLI tools are needed based on configured agents
    if let Some(ref settings) = settings_ok {
        let agents = get_agents(settings);
        let providers_in_use: HashSet<&str> = agents.values().map(|a| a.provider.as_str()).collect();

        for provider in &providers_in_use {
            match *provider {
                "anthropic" => {
                    issues += check_tool("claude", &["claude", "--version"], &[
                        ("npm", "npm install -g @anthropic-ai/claude-code"),
                    ]);
                }
                "openai" => {
                    issues += check_tool("codex", &["codex", "--version"], &[
                        ("npm", "npm install -g @openai/codex"),
                    ]);
                }
                "opencode" => {
                    issues += check_tool("opencode", &["opencode", "--version"], &[
                        ("Go", "go install github.com/opencode-ai/opencode@latest"),
                    ]);
                }
                _ => {
                    print_warn(&format!("Unknown provider '{}' — can't verify CLI tool", provider));
                }
            }
        }

        // 4. Check agent working directories
        for (id, agent) in &agents {
            let dir = std::path::Path::new(&agent.working_directory);
            if dir.exists() {
                print_ok(&format!("Agent '{}' working dir: {}", id, agent.working_directory));
            } else {
                print_warn(&format!(
                    "Agent '{}' working dir missing: {} (will be created on first message)",
                    id, agent.working_directory
                ));
            }
        }

        // 5. Check teams reference valid agents
        let teams = get_teams(settings);
        for (team_id, team) in &teams {
            let mut team_ok = true;
            for member in &team.agents {
                if !agents.contains_key(member) {
                    print_fail(&format!(
                        "Team '{}' references agent '{}' which doesn't exist",
                        team_id, member
                    ));
                    team_ok = false;
                    issues += 1;
                }
            }
            if !agents.contains_key(&team.leader_agent) {
                print_fail(&format!(
                    "Team '{}' leader '{}' doesn't exist as an agent",
                    team_id, team.leader_agent
                ));
                team_ok = false;
                issues += 1;
            }
            if team_ok {
                print_ok(&format!(
                    "Team '{}': {} agents, leader @{}",
                    team_id,
                    team.agents.len(),
                    team.leader_agent
                ));
            }
        }

        // 6. Check channels
        if let Some(ref channels) = settings.channels {
            if let Some(ref enabled) = channels.enabled {
                for ch in enabled {
                    match ch.as_str() {
                        "discord" => {
                            let has_token = channels
                                .discord
                                .as_ref()
                                .and_then(|d| d.bot_token.as_ref())
                                .map(|t| !t.is_empty())
                                .unwrap_or(false);
                            if has_token {
                                print_ok("Discord: enabled with bot token");
                            } else {
                                print_fail("Discord: enabled but no bot token configured");
                                issues += 1;
                            }
                        }
                        "telegram" => {
                            let has_token = channels
                                .telegram
                                .as_ref()
                                .and_then(|t| t.bot_token.as_ref())
                                .map(|t| !t.is_empty())
                                .unwrap_or(false);
                            if has_token {
                                print_ok("Telegram: enabled with bot token");
                            } else {
                                print_fail("Telegram: enabled but no bot token configured");
                                issues += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // 7. Cooldowns
    let cooldowns_file = paths.rustyclaw_home.join("cooldowns.json");
    if cooldowns_file.exists() {
        let cooldowns = rustyclaw_core::failover::load_cooldowns(&cooldowns_file);
        let active: Vec<_> = cooldowns
            .iter()
            .filter(|(_, entry)| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                now < entry.until
            })
            .collect();
        if active.is_empty() {
            print_ok("No models in cooldown");
        } else {
            for (key, entry) in &active {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let remaining_secs = (entry.until.saturating_sub(now)) / 1000;
                let mins = remaining_secs / 60;
                let secs = remaining_secs % 60;
                print_fail(&format!(
                    "Model '{}' in cooldown ({} errors, {}m {}s remaining)",
                    key, entry.error_count, mins, secs
                ));
                issues += 1;
            }
            println!("         Fix: {}", "rustyclaw cooldown reset".green());
        }
    } else {
        print_ok("No models in cooldown");
    }

    // 8. Visualizer
    if let Some(_dir) = super::find_viz_dist_dir() {
        print_ok("Visualizer: WASM dist found");
    } else {
        print_warn("Visualizer: WASM not built (optional)");
        println!("         Build: {}", "cd crates/rustyclaw-viz && trunk build --release".dimmed());
    }

    // Summary
    println!();
    if issues == 0 {
        println!("  {} {}", "All checks passed!".green().bold(), "You're good to go.".dimmed());
    } else {
        println!(
            "  {} {}",
            format!("{} issue(s) found.", issues).yellow().bold(),
            "Fix the items above and run doctor again.".dimmed()
        );
    }
    println!();

    Ok(())
}

/// Check if a CLI tool is available. Returns 1 if missing, 0 if found.
fn check_tool(name: &str, check_cmd: &[&str], install_hints: &[(&str, &str)]) -> usize {
    let result = Command::new(check_cmd[0])
        .args(&check_cmd[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if version.is_empty() {
                print_ok(name);
            } else {
                print_ok(&format!("{} ({})", name, version.dimmed()));
            }
            0
        }
        _ => {
            print_fail(&format!("{} — not found", name));
            for (platform, cmd) in install_hints {
                println!("         {}: {}", platform, cmd.green());
            }
            1
        }
    }
}

fn print_ok(msg: &str) {
    println!("  {} {}", "✓".green(), msg);
}

fn print_fail(msg: &str) {
    println!("  {} {}", "✗".red(), msg);
}

fn print_warn(msg: &str) {
    println!("  {} {}", "!".yellow(), msg);
}

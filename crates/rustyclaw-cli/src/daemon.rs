use std::process::Command;

use anyhow::{bail, Result};
use colored::Colorize;

use rustyclaw_core::config::{get_settings, Paths};

const SESSION_NAME: &str = "rustyclaw";

/// Check if the tmux session is running
fn is_session_running() -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", SESSION_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if tmux is installed
fn ensure_tmux() -> Result<()> {
    match Command::new("tmux").arg("-V").output() {
        Ok(output) if output.status.success() => Ok(()),
        _ => {
            println!("{}", "tmux is required but not installed.".red().bold());
            println!();
            if cfg!(target_os = "macos") {
                println!("  Install with: {}", "brew install tmux".green());
            } else {
                println!("  Install with: {}", "sudo apt install tmux".green());
                println!("           or:  {}", "sudo dnf install tmux".green());
            }
            println!();
            bail!("tmux not found");
        }
    }
}

/// Start the Rusty Claw daemon (tmux session with all components)
pub fn start(paths: &Paths) -> Result<()> {
    ensure_tmux()?;

    if is_session_running() {
        println!("{}", "Rusty Claw is already running.".yellow());
        println!("Use {} to see status, {} to restart.", "rustyclaw status".green(), "rustyclaw restart".green());
        return Ok(());
    }

    // Validate settings
    let settings = get_settings(&paths.settings_file)?;
    if settings.agents.is_none() && settings.models.is_none() {
        println!("{}", "No settings found. Please run setup first.".red());
        println!("  {}", "rustyclaw setup".green());
        return Ok(());
    }

    // Ensure queue directories exist
    paths.ensure_queue_dirs()?;

    // Ensure logs directory
    if let Some(log_dir) = paths.log_file.parent() {
        std::fs::create_dir_all(log_dir)?;
    }

    // Determine enabled channels
    let enabled_channels = settings
        .channels
        .as_ref()
        .and_then(|c| c.enabled.as_ref())
        .cloned()
        .unwrap_or_default();

    // Resolve the rustyclaw binary path (all components run via subcommands)
    let self_bin = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("rustyclaw"));

    // Create tmux session with first pane (queue processor)
    let queue_cmd = format!(
        "RUSTYCLAW_HOME={} {} run queue",
        paths.rustyclaw_home.display(),
        self_bin.display()
    );
    let status = Command::new("tmux")
        .args([
            "new-session",
            "-d",
            "-s",
            SESSION_NAME,
            "-n",
            "rustyclaw",
            &queue_cmd,
        ])
        .status()?;

    if !status.success() {
        bail!("Failed to create tmux session");
    }

    // Collect channel tokens from settings
    let discord_token = settings
        .channels
        .as_ref()
        .and_then(|c| c.discord.as_ref())
        .and_then(|d| d.bot_token.as_deref())
        .unwrap_or("");
    let telegram_token = settings
        .channels
        .as_ref()
        .and_then(|c| c.telegram.as_ref())
        .and_then(|t| t.bot_token.as_deref())
        .unwrap_or("");

    // Split for channel clients
    let mut pane_index = 1;
    for channel in &enabled_channels {
        let token_env = match channel.as_str() {
            "discord" => format!("DISCORD_BOT_TOKEN={}", discord_token),
            "telegram" => format!("TELOXIDE_TOKEN={}", telegram_token),
            _ => continue,
        };

        let chan_cmd = format!(
            "{} RUSTYCLAW_HOME={} {} run {}",
            token_env,
            paths.rustyclaw_home.display(),
            self_bin.display(),
            channel
        );
        Command::new("tmux")
            .args([
                "split-window",
                "-t",
                &format!("{}:{}", SESSION_NAME, 0),
                "-v",
                &chan_cmd,
            ])
            .status()?;
        pane_index += 1;
    }

    // Split for heartbeat
    {
        let heartbeat_cmd = format!(
            "RUSTYCLAW_HOME={} {} run heartbeat",
            paths.rustyclaw_home.display(),
            self_bin.display()
        );
        Command::new("tmux")
            .args([
                "split-window",
                "-t",
                &format!("{}:{}", SESSION_NAME, 0),
                "-v",
                &heartbeat_cmd,
            ])
            .status()?;
        pane_index += 1;
    }

    // Split for log tail
    let log_dir = paths.rustyclaw_home.join("logs");
    let log_cmd = format!("tail -f {}/*.log 2>/dev/null || echo 'No logs yet. Waiting...' && sleep infinity", log_dir.display());
    Command::new("tmux")
        .args([
            "split-window",
            "-t",
            &format!("{}:{}", SESSION_NAME, 0),
            "-v",
            &log_cmd,
        ])
        .status()?;

    // Even out the pane layout
    Command::new("tmux")
        .args([
            "select-layout",
            "-t",
            &format!("{}:{}", SESSION_NAME, 0),
            "tiled",
        ])
        .status()?;

    // Select the first pane (queue processor)
    Command::new("tmux")
        .args([
            "select-pane",
            "-t",
            &format!("{}:{}.0", SESSION_NAME, 0),
        ])
        .status()?;

    println!("{}", "Rusty Claw daemon started!".green().bold());
    println!("  Queue processor:  {}", "running".green());
    for channel in &enabled_channels {
        println!("  {} client: {}", capitalize(channel), "running".green());
    }
    println!("  Heartbeat:        {}", "running".green());
    println!("  Panes:            {}", pane_index + 1);
    println!();
    println!("Use {} to view the session.", "rustyclaw attach".green());
    Ok(())
}

/// Stop the Rusty Claw daemon
pub fn stop(paths: &Paths) -> Result<()> {
    if !is_session_running() {
        println!("{}", "Rusty Claw is not running.".yellow());
        return Ok(());
    }

    let status = Command::new("tmux")
        .args(["kill-session", "-t", SESSION_NAME])
        .status()?;

    if status.success() {
        println!("{}", "Rusty Claw daemon stopped.".green());
    } else {
        println!("{}", "Failed to stop daemon.".red());
    }

    // Kill any lingering processes
    let _ = Command::new("pkill")
        .args(["-f", "rustyclaw run queue"])
        .status();
    let _ = Command::new("pkill")
        .args(["-f", "rustyclaw run discord"])
        .status();
    let _ = Command::new("pkill")
        .args(["-f", "rustyclaw run telegram"])
        .status();
    let _ = Command::new("pkill")
        .args(["-f", "rustyclaw run heartbeat"])
        .status();

    let _ = paths;
    Ok(())
}

/// Restart the daemon
pub fn restart(paths: &Paths) -> Result<()> {
    // Check if we're inside the tmux session
    if let Ok(tmux_val) = std::env::var("TMUX") {
        if tmux_val.contains(SESSION_NAME) {
            println!("{}", "Cannot restart from inside the tmux session.".yellow());
            println!("Detach first with Ctrl+B then D, then run: {}", "rustyclaw restart".green());
            return Ok(());
        }
    }

    println!("Restarting Rusty Claw...");
    stop(paths)?;
    // Brief pause to let processes exit
    std::thread::sleep(std::time::Duration::from_secs(1));
    start(paths)
}

/// Show daemon status
pub fn status(paths: &Paths) -> Result<()> {
    let running = is_session_running();
    let settings = get_settings(&paths.settings_file)?;

    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!("  {}", "Rusty Claw Status".green().bold());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!();

    // Daemon status
    if running {
        println!("  Daemon:   {} {}", "●".green(), "running".green());
    } else {
        println!("  Daemon:   {} {}", "●".red(), "stopped".red());
    }

    // Settings info
    if let Some(ref workspace) = settings.workspace {
        if let Some(ref name) = workspace.name {
            println!("  Workspace: {}", name.bright_white());
        }
    }

    // Provider / model
    let provider = settings
        .models
        .as_ref()
        .and_then(|m| m.provider.as_deref())
        .unwrap_or("anthropic");
    println!("  Provider: {}", provider.bright_white());

    // Enabled channels
    let channels = settings
        .channels
        .as_ref()
        .and_then(|c| c.enabled.as_ref())
        .cloned()
        .unwrap_or_default();
    if channels.is_empty() {
        println!("  Channels: {}", "none".yellow());
    } else {
        println!(
            "  Channels: {}",
            channels
                .iter()
                .map(|c| capitalize(c))
                .collect::<Vec<_>>()
                .join(", ")
                .bright_white()
        );
    }

    // Agents
    let agents = rustyclaw_core::config::get_agents(&settings);
    println!("  Agents:   {}", format!("{}", agents.len()).bright_white());
    for (id, agent) in &agents {
        println!(
            "            {} ({}/{})",
            id.bright_white(),
            agent.provider.dimmed(),
            agent.model.dimmed()
        );
    }

    // Teams
    let teams = rustyclaw_core::config::get_teams(&settings);
    if !teams.is_empty() {
        println!("  Teams:    {}", format!("{}", teams.len()).bright_white());
        for (id, team) in &teams {
            println!(
                "            {} ({} agents, leader: {})",
                id.bright_white(),
                team.agents.len(),
                team.leader_agent.dimmed()
            );
        }
    }

    // Queue status
    let incoming_count = count_files(&paths.queue_incoming);
    let processing_count = count_files(&paths.queue_processing);
    println!();
    println!("  Queue:    {} incoming, {} processing",
        incoming_count.to_string().bright_white(),
        processing_count.to_string().bright_white()
    );

    // Recent activity — last log line
    if paths.log_file.exists() {
        if let Ok(content) = std::fs::read_to_string(&paths.log_file) {
            if let Some(last_line) = content.lines().rev().next() {
                println!();
                println!("  Last activity:");
                println!("    {}", last_line.dimmed());
            }
        }
    }

    println!();
    if !running {
        println!("  Start with: {}", "rustyclaw start".green());
    }

    Ok(())
}

/// Attach to the tmux session
pub fn attach() -> Result<()> {
    if !is_session_running() {
        println!("{}", "Rusty Claw is not running.".yellow());
        println!("Start it first with: {}", "rustyclaw start".green());
        return Ok(());
    }

    let status = Command::new("tmux")
        .args(["attach-session", "-t", SESSION_NAME])
        .status()?;

    if !status.success() {
        bail!("Failed to attach to tmux session");
    }
    Ok(())
}

fn count_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

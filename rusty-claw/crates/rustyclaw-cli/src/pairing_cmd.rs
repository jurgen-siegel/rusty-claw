use anyhow::Result;
use colored::Colorize;

use rustyclaw_core::config::Paths;
use rustyclaw_core::pairing::{approve_pairing_code, load_pairing_state, save_pairing_state};

/// List pending pairing requests
pub fn list_pending(paths: &Paths) -> Result<()> {
    let state = load_pairing_state(&paths.pairing_file);

    if state.pending.is_empty() {
        println!("{}", "No pending pairing requests.".yellow());
        return Ok(());
    }

    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!("  {}", "Pending Pairing Requests".green().bold());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!();

    for entry in &state.pending {
        println!(
            "  Code: {}  Channel: {}  Sender: {} ({})",
            entry.code.bright_white().bold(),
            entry.channel.bright_white(),
            entry.sender.bright_white(),
            entry.sender_id.dimmed()
        );
        let created = format_timestamp(entry.created_at);
        let last_seen = format_timestamp(entry.last_seen_at);
        println!("    Created: {}  Last seen: {}", created.dimmed(), last_seen.dimmed());
        println!();
    }

    println!("Approve with: {} <code>", "rustyclaw pairing approve".green());

    Ok(())
}

/// List approved senders
pub fn list_approved(paths: &Paths) -> Result<()> {
    let state = load_pairing_state(&paths.pairing_file);

    if state.approved.is_empty() {
        println!("{}", "No approved senders.".yellow());
        return Ok(());
    }

    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!("  {}", "Approved Senders".green().bold());
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".bright_blue());
    println!();

    for entry in &state.approved {
        println!(
            "  {} — {} on {} ({})",
            entry.sender.bright_white().bold(),
            entry.sender_id.dimmed(),
            entry.channel.bright_white(),
            entry
                .approved_code
                .as_deref()
                .unwrap_or("—")
                .dimmed()
        );
        let approved = format_timestamp(entry.approved_at);
        println!("    Approved: {}", approved.dimmed());
        println!();
    }

    Ok(())
}

/// List all (pending + approved)
pub fn list_all(paths: &Paths) -> Result<()> {
    list_pending(paths)?;
    println!();
    list_approved(paths)
}

/// Approve a pairing code
pub fn approve(code: &str, paths: &Paths) -> Result<()> {
    let result = approve_pairing_code(&paths.pairing_file, code);

    if result.ok {
        if let Some(entry) = &result.entry {
            println!(
                "{} Approved {} ({}) on {}",
                "✓".green(),
                entry.sender.bright_white(),
                entry.sender_id.dimmed(),
                entry.channel.bright_white()
            );
        } else {
            println!("{} Pairing code approved.", "✓".green());
        }
    } else {
        let reason = result.reason.unwrap_or_else(|| "Unknown error".to_string());
        println!("{} {}", "Error:".red(), reason);
    }

    Ok(())
}

/// Unpair a sender by sender ID
pub fn unpair(sender_id: &str, paths: &Paths) -> Result<()> {
    let mut state = load_pairing_state(&paths.pairing_file);

    let original_len = state.approved.len();
    state.approved.retain(|e| e.sender_id != sender_id);

    if state.approved.len() == original_len {
        // Try matching by channel::sender_id format
        state.approved.retain(|e| {
            let key = format!("{}::{}", e.channel, e.sender_id);
            key != sender_id
        });
    }

    if state.approved.len() == original_len {
        println!("{} Sender '{}' not found in approved list.", "Error:".red(), sender_id);
        return Ok(());
    }

    save_pairing_state(&paths.pairing_file, &state)?;
    println!("{} Sender '{}' unpaired.", "✓".green(), sender_id);

    Ok(())
}

// --- Helpers ---

fn format_timestamp(millis: u64) -> String {
    let secs = (millis / 1000) as i64;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

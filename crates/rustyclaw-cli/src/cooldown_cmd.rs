use anyhow::Result;
use colored::Colorize;

use rustyclaw_core::config::Paths;
use rustyclaw_core::failover::{load_cooldowns, save_cooldowns};

/// Show all cooldown entries.
pub fn show_cooldowns(paths: &Paths) -> Result<()> {
    let cooldowns_file = paths.rustyclaw_home.join("cooldowns.json");
    let cooldowns = load_cooldowns(&cooldowns_file);

    if cooldowns.is_empty() {
        println!("{}", "No models in cooldown.".green());
        return Ok(());
    }

    let now = now_millis();

    println!();
    println!("  {}", "Model Cooldowns".green().bold());
    println!();

    let mut any_active = false;
    for (key, entry) in &cooldowns {
        if now < entry.until {
            let remaining_secs = (entry.until - now) / 1000;
            let mins = remaining_secs / 60;
            let secs = remaining_secs % 60;
            println!(
                "  {} {} — {} errors, {}m {}s remaining",
                "●".red(),
                key.bright_white(),
                entry.error_count,
                mins,
                secs
            );
            any_active = true;
        } else {
            println!(
                "  {} {} — {} errors (cooldown expired)",
                "●".dimmed(),
                key.dimmed(),
                entry.error_count,
            );
        }
    }

    if any_active {
        println!();
        println!("  Reset with: {}", "rustyclaw cooldown reset".green());
    }

    println!();
    Ok(())
}

/// Reset cooldowns — all or for a specific model.
pub fn reset_cooldowns(paths: &Paths, model: Option<&str>) -> Result<()> {
    let cooldowns_file = paths.rustyclaw_home.join("cooldowns.json");
    let mut cooldowns = load_cooldowns(&cooldowns_file);

    if cooldowns.is_empty() {
        println!("{}", "No cooldowns to reset.".green());
        return Ok(());
    }

    match model {
        Some(key) => {
            if cooldowns.remove(key).is_some() {
                save_cooldowns(&cooldowns_file, &cooldowns)?;
                println!("{} Cooldown reset for {}.", "✓".green(), key.bright_white());
            } else {
                // Try partial match (e.g. "opus" matches "anthropic:opus")
                let matching: Vec<String> = cooldowns
                    .keys()
                    .filter(|k| k.contains(key))
                    .cloned()
                    .collect();
                if matching.is_empty() {
                    println!("{} No cooldown found for '{}'.", "!".yellow(), key);
                    println!("  Active cooldowns:");
                    for k in cooldowns.keys() {
                        println!("    {}", k);
                    }
                } else {
                    for k in &matching {
                        cooldowns.remove(k);
                    }
                    save_cooldowns(&cooldowns_file, &cooldowns)?;
                    for k in &matching {
                        println!("{} Cooldown reset for {}.", "✓".green(), k.bright_white());
                    }
                }
            }
        }
        None => {
            let count = cooldowns.len();
            cooldowns.clear();
            save_cooldowns(&cooldowns_file, &cooldowns)?;
            println!(
                "{} All cooldowns cleared ({} entries).",
                "✓".green(),
                count
            );
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

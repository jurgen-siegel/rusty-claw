use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::types::{AgentConfig, TeamConfig};

/// Recursively copy a directory and all its contents.
pub fn copy_dir_sync(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            copy_dir_sync(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }

    Ok(())
}

/// Ensure agent directory exists with template files copied from script_dir.
/// Creates directory if it doesn't exist and copies .claude/, heartbeat.md, AGENTS.md, etc.
pub fn ensure_agent_directory(agent_dir: &Path, script_dir: &Path) -> Result<()> {
    if agent_dir.exists() {
        return Ok(()); // Directory already exists
    }

    std::fs::create_dir_all(agent_dir)?;

    // Copy .claude directory
    let source_claude_dir = script_dir.join(".claude");
    let target_claude_dir = agent_dir.join(".claude");
    if source_claude_dir.exists() {
        copy_dir_sync(&source_claude_dir, &target_claude_dir)?;
    }

    // Copy heartbeat.md
    let source_heartbeat = script_dir.join("heartbeat.md");
    let target_heartbeat = agent_dir.join("heartbeat.md");
    if source_heartbeat.exists() {
        std::fs::copy(&source_heartbeat, &target_heartbeat)?;
    }

    // Copy AGENTS.md
    let source_agents = script_dir.join("AGENTS.md");
    let target_agents = agent_dir.join("AGENTS.md");
    if source_agents.exists() {
        std::fs::copy(&source_agents, &target_agents)?;
    }

    // Copy AGENTS.md as .claude/CLAUDE.md
    if source_agents.exists() {
        std::fs::create_dir_all(agent_dir.join(".claude"))?;
        std::fs::copy(&source_agents, agent_dir.join(".claude/CLAUDE.md"))?;
    }

    // Symlink skills directory into .claude/skills
    // Prefer .agent/skills, fall back to .agents/skills
    let source_skills = if script_dir.join(".agent/skills").exists() {
        script_dir.join(".agent/skills")
    } else {
        script_dir.join(".agents/skills")
    };
    let target_claude_skills = agent_dir.join(".claude/skills");
    if source_skills.exists() && !target_claude_skills.exists() {
        std::fs::create_dir_all(agent_dir.join(".claude"))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source_skills, &target_claude_skills)?;
    }

    // Symlink .agent/skills
    let target_agent_dir = agent_dir.join(".agent");
    let target_agent_skills = target_agent_dir.join("skills");
    if !target_agent_skills.exists() && source_skills.exists() {
        std::fs::create_dir_all(&target_agent_dir)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source_skills, &target_agent_skills)?;
    }

    // Create .rustyclaw directory and copy SOUL.md
    let target_rustyclaw = agent_dir.join(".rustyclaw");
    std::fs::create_dir_all(&target_rustyclaw)?;
    let source_soul = script_dir.join("SOUL.md");
    if source_soul.exists() {
        std::fs::copy(&source_soul, target_rustyclaw.join("SOUL.md"))?;
    }

    // Copy bootstrap files (IDENTITY.md, USER.md, TOOLS.md)
    for filename in &["IDENTITY.md", "USER.md", "TOOLS.md"] {
        let source = script_dir.join(filename);
        let target = target_rustyclaw.join(filename);
        if source.exists() && !target.exists() {
            std::fs::copy(&source, &target)?;
        }
    }

    // Create empty MEMORY.md for agent to populate
    let memory_file = target_rustyclaw.join("MEMORY.md");
    if !memory_file.exists() {
        std::fs::write(
            &memory_file,
            "# Memory\n\n<!-- Long-term notes and learnings. Update as you work. -->\n",
        )?;
    }

    // Create memory and transcripts directories
    std::fs::create_dir_all(target_rustyclaw.join("memory"))?;
    std::fs::create_dir_all(target_rustyclaw.join("transcripts"))?;

    Ok(())
}

/// Populate the IDENTITY.md file in an agent's .rustyclaw directory with role info.
/// Call this after `ensure_agent_directory` to fill in the agent's role.
pub fn populate_agent_identity(agent_dir: &Path, agent_name: &str, role: &str) -> Result<()> {
    let identity_file = agent_dir.join(".rustyclaw/IDENTITY.md");
    let content = format!(
        "# Identity — {name}\n\
         \n\
         ## Role\n\
         \n\
         {role}\n\
         \n\
         ## Expertise\n\
         \n\
         <!-- Your core domains. What you're best at. Fill this in as you work. -->\n\
         \n\
         ## Working Style\n\
         \n\
         - Read before writing. Understand the codebase before changing it.\n\
         - Ask when unclear. Don't guess at requirements.\n\
         - Ship working code. Test before declaring done.\n",
        name = agent_name,
        role = role,
    );
    // Ensure the directory exists (should already from ensure_agent_directory)
    if let Some(parent) = identity_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&identity_file, content)?;
    Ok(())
}

/// Update the AGENTS.md in an agent's directory with current teammate info.
/// Replaces content between <!-- TEAMMATES_START --> and <!-- TEAMMATES_END --> markers.
pub fn update_agent_teammates(
    agent_dir: &Path,
    agent_id: &str,
    agents: &HashMap<String, AgentConfig>,
    teams: &HashMap<String, TeamConfig>,
) -> Result<()> {
    let agents_md_path = agent_dir.join("AGENTS.md");
    if !agents_md_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&agents_md_path)?;
    let start_marker = "<!-- TEAMMATES_START -->";
    let end_marker = "<!-- TEAMMATES_END -->";

    let start_idx = match content.find(start_marker) {
        Some(idx) => idx,
        None => return Ok(()),
    };
    let end_idx = match content.find(end_marker) {
        Some(idx) => idx,
        None => return Ok(()),
    };

    // Find teammates from all teams this agent belongs to
    let mut teammates: Vec<(String, String, String)> = Vec::new(); // (id, name, model)
    let mut my_team_ids: Vec<String> = Vec::new();
    for (team_id, team) in teams {
        if !team.agents.iter().any(|a| a == agent_id) {
            continue;
        }
        my_team_ids.push(team_id.clone());
        for tid in &team.agents {
            if tid == agent_id {
                continue;
            }
            if let Some(agent) = agents.get(tid) {
                if !teammates.iter().any(|(id, _, _)| id == tid) {
                    teammates.push((tid.clone(), agent.name.clone(), agent.model.clone()));
                }
            }
        }
    }

    // Find cross-team agents (agents on other teams that this agent can hand off to)
    let self_config = agents.get(agent_id);
    let can_cross_team = self_config.map(|a| a.cross_team_handoffs).unwrap_or(true);
    let mut other_agents: Vec<(String, String, String, String)> = Vec::new(); // (id, name, model, team_name)
    if can_cross_team {
        for (team_id, team) in teams {
            if my_team_ids.contains(team_id) {
                continue;
            }
            for tid in &team.agents {
                if let Some(agent) = agents.get(tid) {
                    if !other_agents.iter().any(|(id, _, _, _)| id == tid)
                        && !teammates.iter().any(|(id, _, _)| id == tid)
                    {
                        other_agents.push((
                            tid.clone(),
                            agent.name.clone(),
                            agent.model.clone(),
                            team.name.clone(),
                        ));
                    }
                }
            }
        }
    }

    let mut block = String::new();
    if let Some(self_agent) = agents.get(agent_id) {
        block += &format!(
            "\n### You\n\n- `@{}` — **{}** ({})\n",
            agent_id, self_agent.name, self_agent.model
        );
    }
    if !teammates.is_empty() {
        block += "\n### Your Teammates\n\nUse `[@agent_id: message]` to message them:\n\n";
        for (id, name, model) in &teammates {
            block += &format!("- `@{}` — **{}** ({})\n", id, name, model);
        }
    }
    if !other_agents.is_empty() {
        block += "\n### Other Agents (cross-team)\n\nUse `[@!agent_id: message]` to hand off to them:\n\n";
        for (id, name, model, team_name) in &other_agents {
            block += &format!("- `@{}` — **{}** ({}) — team: {}\n", id, name, model, team_name);
        }
    }

    let new_content = format!(
        "{}{}{}{}",
        &content[..start_idx + start_marker.len()],
        block,
        end_marker, // intentionally skip the old end_marker position
        &content[end_idx + end_marker.len()..]
    );
    std::fs::write(&agents_md_path, &new_content)?;

    // Also write to .claude/CLAUDE.md
    let claude_dir = agent_dir.join(".claude");
    std::fs::create_dir_all(&claude_dir)?;
    let claude_md_path = claude_dir.join("CLAUDE.md");

    let mut claude_content = if claude_md_path.exists() {
        std::fs::read_to_string(&claude_md_path)?
    } else {
        String::new()
    };

    let c_start = claude_content.find(start_marker);
    let c_end = claude_content.find(end_marker);

    if let (Some(cs), Some(ce)) = (c_start, c_end) {
        claude_content = format!(
            "{}{}{}{}",
            &claude_content[..cs + start_marker.len()],
            block,
            end_marker,
            &claude_content[ce + end_marker.len()..]
        );
    } else {
        // Append markers + block
        claude_content = format!(
            "{}\n\n{}{}{}",
            claude_content.trim_end(),
            start_marker,
            block,
            end_marker,
        );
        claude_content.push('\n');
    }
    std::fs::write(&claude_md_path, claude_content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_copy_dir_sync() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");

        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("file.txt"), "hello").unwrap();
        std::fs::write(src.join("sub/nested.txt"), "world").unwrap();

        copy_dir_sync(&src, &dest).unwrap();

        assert!(dest.join("file.txt").exists());
        assert!(dest.join("sub/nested.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("file.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("sub/nested.txt")).unwrap(),
            "world"
        );
    }

    #[test]
    fn test_ensure_agent_directory() {
        let tmp = TempDir::new().unwrap();
        let script_dir = tmp.path().join("scripts");
        let agent_dir = tmp.path().join("workspace/coder");

        // Set up script_dir with templates
        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::write(script_dir.join("heartbeat.md"), "status check").unwrap();
        std::fs::write(
            script_dir.join("AGENTS.md"),
            "<!-- TEAMMATES_START -->\n<!-- TEAMMATES_END -->",
        )
        .unwrap();
        std::fs::write(script_dir.join("SOUL.md"), "soul template").unwrap();

        ensure_agent_directory(&agent_dir, &script_dir).unwrap();

        assert!(agent_dir.join("heartbeat.md").exists());
        assert!(agent_dir.join("AGENTS.md").exists());
        assert!(agent_dir.join(".claude/CLAUDE.md").exists());
        assert!(agent_dir.join(".rustyclaw/SOUL.md").exists());
    }

    #[test]
    fn test_ensure_agent_directory_idempotent() {
        let tmp = TempDir::new().unwrap();
        let script_dir = tmp.path().join("scripts");
        let agent_dir = tmp.path().join("workspace/coder");

        std::fs::create_dir_all(&script_dir).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();

        // Should be a no-op since directory already exists
        ensure_agent_directory(&agent_dir, &script_dir).unwrap();
    }

    #[test]
    fn test_update_agent_teammates() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("coder");
        std::fs::create_dir_all(&agent_dir).unwrap();

        let agents_md = "# Agents\n\n<!-- TEAMMATES_START -->\n<!-- TEAMMATES_END -->\n\nFooter";
        std::fs::write(agent_dir.join("AGENTS.md"), agents_md).unwrap();

        let mut agents = HashMap::new();
        agents.insert(
            "coder".to_string(),
            AgentConfig {
                name: "Coder".to_string(),
                provider: "anthropic".to_string(),
                model: "sonnet".to_string(),
                working_directory: "/tmp/coder".to_string(),
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
        agents.insert(
            "reviewer".to_string(),
            AgentConfig {
                name: "Reviewer".to_string(),
                provider: "anthropic".to_string(),
                model: "opus".to_string(),
                working_directory: "/tmp/reviewer".to_string(),
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

        agents.insert(
            "tester".to_string(),
            AgentConfig {
                name: "Tester".to_string(),
                provider: "anthropic".to_string(),
                model: "haiku".to_string(),
                working_directory: "/tmp/tester".to_string(),
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

        let mut teams = HashMap::new();
        teams.insert(
            "dev".to_string(),
            TeamConfig {
                name: "Dev".to_string(),
                agents: vec!["coder".to_string(), "reviewer".to_string()],
                leader_agent: "coder".to_string(),
                description: None,
            },
        );
        teams.insert(
            "qa".to_string(),
            TeamConfig {
                name: "QA".to_string(),
                agents: vec!["tester".to_string()],
                leader_agent: "tester".to_string(),
                description: None,
            },
        );

        update_agent_teammates(&agent_dir, "coder", &agents, &teams).unwrap();

        let content = std::fs::read_to_string(agent_dir.join("AGENTS.md")).unwrap();
        assert!(content.contains("### You"));
        assert!(content.contains("@coder"));
        assert!(content.contains("### Your Teammates"));
        assert!(content.contains("@reviewer"));
        assert!(content.contains("### Other Agents (cross-team)"));
        assert!(content.contains("@tester"));
        assert!(content.contains("team: QA"));
        assert!(content.contains("[@!agent_id: message]"));
        assert!(content.contains("Footer")); // preserved
    }
}

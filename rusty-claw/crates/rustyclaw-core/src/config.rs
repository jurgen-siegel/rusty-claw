use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::types::{AgentConfig, Settings, TeamConfig};

/// All resolved paths for Rusty Claw directories
#[derive(Debug, Clone)]
pub struct Paths {
    /// Root of the rusty-claw installation (where templates/ lives)
    pub script_dir: PathBuf,
    /// Data directory (~/.rustyclaw or local .rustyclaw/)
    pub rustyclaw_home: PathBuf,
    pub queue_incoming: PathBuf,
    pub queue_outgoing: PathBuf,
    pub queue_processing: PathBuf,
    pub log_file: PathBuf,
    pub settings_file: PathBuf,
    pub events_dir: PathBuf,
    pub chats_dir: PathBuf,
    pub files_dir: PathBuf,
    pub pairing_file: PathBuf,
}

impl Paths {
    /// Resolve RUSTYCLAW_HOME using the same precedence as the TypeScript code:
    /// 1. RUSTYCLAW_HOME env var
    /// 2. local .rustyclaw/ if it has settings.json
    /// 3. ~/.rustyclaw/
    pub fn resolve(script_dir: &Path) -> Self {
        let rustyclaw_home = if let Ok(env_home) = std::env::var("RUSTYCLAW_HOME") {
            PathBuf::from(env_home)
        } else {
            let local = script_dir.join(".rustyclaw");
            if local.join("settings.json").exists() {
                local
            } else {
                dirs_home().join(".rustyclaw")
            }
        };

        Self {
            script_dir: script_dir.to_path_buf(),
            queue_incoming: rustyclaw_home.join("queue/incoming"),
            queue_outgoing: rustyclaw_home.join("queue/outgoing"),
            queue_processing: rustyclaw_home.join("queue/processing"),
            log_file: rustyclaw_home.join("logs/queue.log"),
            settings_file: rustyclaw_home.join("settings.json"),
            events_dir: rustyclaw_home.join("events"),
            chats_dir: rustyclaw_home.join("chats"),
            files_dir: rustyclaw_home.join("files"),
            pairing_file: rustyclaw_home.join("pairing.json"),
            rustyclaw_home,
        }
    }

    /// Ensure all queue directories exist
    pub fn ensure_queue_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(&self.queue_incoming)
            .context("Failed to create incoming queue dir")?;
        std::fs::create_dir_all(&self.queue_outgoing)
            .context("Failed to create outgoing queue dir")?;
        std::fs::create_dir_all(&self.queue_processing)
            .context("Failed to create processing queue dir")?;
        Ok(())
    }
}

/// Get user home directory
fn dirs_home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Load and parse settings.json
pub fn get_settings(settings_file: &Path) -> Result<Settings> {
    if !settings_file.exists() {
        return Ok(Settings::default());
    }

    let data =
        std::fs::read_to_string(settings_file).context("Failed to read settings.json")?;

    let mut settings: Settings = match serde_json::from_str(&data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[WARN] settings.json contains invalid JSON: {}",
                e
            );
            eprintln!("[ERROR] Could not parse settings.json — returning empty config");
            return Ok(Settings::default());
        }
    };

    // Auto-detect provider if not specified
    if let Some(ref mut models) = settings.models {
        if models.provider.is_none() {
            if models.openai.is_some() {
                models.provider = Some("openai".to_string());
            } else if models.opencode.is_some() {
                models.provider = Some("opencode".to_string());
            } else if models.anthropic.is_some() {
                models.provider = Some("anthropic".to_string());
            }
        }
    }

    Ok(settings)
}

/// Build the default agent config from the legacy models section.
/// Used when no agents are configured, for backwards compatibility.
pub fn get_default_agent_from_models(settings: &Settings) -> AgentConfig {
    let provider = settings
        .models
        .as_ref()
        .and_then(|m| m.provider.clone())
        .unwrap_or_else(|| "anthropic".to_string());

    let model = match provider.as_str() {
        "openai" => settings
            .models
            .as_ref()
            .and_then(|m| m.openai.as_ref())
            .and_then(|o| o.model.clone())
            .unwrap_or_else(|| "gpt-5.3-codex".to_string()),
        "opencode" => settings
            .models
            .as_ref()
            .and_then(|m| m.opencode.as_ref())
            .and_then(|o| o.model.clone())
            .unwrap_or_else(|| "sonnet".to_string()),
        _ => settings
            .models
            .as_ref()
            .and_then(|m| m.anthropic.as_ref())
            .and_then(|a| a.model.clone())
            .unwrap_or_else(|| "sonnet".to_string()),
    };

    let workspace_path = settings
        .workspace
        .as_ref()
        .and_then(|w| w.path.clone())
        .unwrap_or_else(|| {
            dirs_home()
                .join("rustyclaw-workspace")
                .to_string_lossy()
                .to_string()
        });
    let default_agent_dir = PathBuf::from(&workspace_path)
        .join("default")
        .to_string_lossy()
        .to_string();

    AgentConfig {
        name: "Default".to_string(),
        provider,
        model,
        working_directory: default_agent_dir,
        reset_policy: String::new(),
        reset_hour: None,
        idle_timeout_minutes: None,
        context_window: None,
        fallbacks: None,
        cross_team_handoffs: true,
        route_patterns: None,
        route_priority: 0,
    }
}

/// Get all configured agents. Falls back to a single "default" agent
/// derived from the legacy models section if no agents are configured.
pub fn get_agents(settings: &Settings) -> HashMap<String, AgentConfig> {
    if let Some(ref agents) = settings.agents {
        if !agents.is_empty() {
            return agents.clone();
        }
    }
    let mut map = HashMap::new();
    map.insert("default".to_string(), get_default_agent_from_models(settings));
    map
}

/// Get all configured teams.
pub fn get_teams(settings: &Settings) -> HashMap<String, TeamConfig> {
    settings.teams.clone().unwrap_or_default()
}

/// Get the workspace path from settings, with default fallback.
pub fn get_workspace_path(settings: &Settings) -> PathBuf {
    settings
        .workspace
        .as_ref()
        .and_then(|w| w.path.as_ref())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join("rustyclaw-workspace"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_settings(dir: &Path, content: &str) -> PathBuf {
        let file = dir.join("settings.json");
        let mut f = std::fs::File::create(&file).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn test_get_settings_missing_file() {
        let settings = get_settings(Path::new("/nonexistent/settings.json")).unwrap();
        assert!(settings.agents.is_none());
    }

    #[test]
    fn test_get_settings_empty_json() {
        let tmp = TempDir::new().unwrap();
        let file = write_settings(tmp.path(), "{}");
        let settings = get_settings(&file).unwrap();
        assert!(settings.agents.is_none());
        assert!(settings.teams.is_none());
    }

    #[test]
    fn test_get_settings_with_agents() {
        let tmp = TempDir::new().unwrap();
        let file = write_settings(
            tmp.path(),
            r#"{
                "agents": {
                    "coder": {
                        "name": "Coder",
                        "provider": "anthropic",
                        "model": "sonnet",
                        "working_directory": "/tmp/coder"
                    }
                }
            }"#,
        );
        let settings = get_settings(&file).unwrap();
        let agents = get_agents(&settings);
        assert!(agents.contains_key("coder"));
        assert_eq!(agents["coder"].name, "Coder");
    }

    #[test]
    fn test_get_agents_fallback_default() {
        let settings = Settings::default();
        let agents = get_agents(&settings);
        assert!(agents.contains_key("default"));
        assert_eq!(agents["default"].provider, "anthropic");
        assert_eq!(agents["default"].model, "sonnet");
    }

    #[test]
    fn test_auto_detect_provider() {
        let tmp = TempDir::new().unwrap();
        let file = write_settings(
            tmp.path(),
            r#"{
                "models": {
                    "openai": { "model": "gpt-5.3-codex" }
                }
            }"#,
        );
        let settings = get_settings(&file).unwrap();
        assert_eq!(
            settings.models.as_ref().unwrap().provider.as_deref(),
            Some("openai")
        );
    }

    #[test]
    fn test_get_teams_empty() {
        let settings = Settings::default();
        let teams = get_teams(&settings);
        assert!(teams.is_empty());
    }
}

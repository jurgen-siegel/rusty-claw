use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent configuration from settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    /// Provider: "anthropic", "openai", or "opencode"
    pub provider: String,
    /// Model shortname or full ID (e.g. "sonnet", "opus", "gpt-5.3-codex")
    pub model: String,
    pub working_directory: String,
    /// Session reset policy: "daily", "idle", "both" (default), or "manual"
    #[serde(default)]
    pub reset_policy: String,
    /// Hour (0-23 UTC) at which daily reset triggers. Default: 4
    #[serde(default)]
    pub reset_hour: Option<u8>,
    /// Minutes of inactivity before idle reset triggers. Default: 120
    #[serde(default)]
    pub idle_timeout_minutes: Option<u64>,
    /// Context window size in tokens. Default: 200000
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Fallback model shortnames to try on failure, in order
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallbacks: Option<Vec<String>>,
    /// Whether this agent can hand off to agents outside its team
    #[serde(default = "default_true")]
    pub cross_team_handoffs: bool,
    /// Keyword patterns for smart routing (case-insensitive word boundary matching)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_patterns: Option<Vec<String>>,
    /// Priority for smart routing tie-breaking (higher wins). Default: 0
    #[serde(default)]
    pub route_priority: u32,
}

/// Team configuration from settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    pub name: String,
    pub agents: Vec<String>,
    pub leader_agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A single agent response in a team chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStep {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub response: String,
}

/// Root settings.json structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<ChannelsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<HashMap<String, AgentConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teams: Option<HashMap<String, TeamConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitoring: Option<MonitoringConfig>,
    /// Skill overrides: enable/disable specific skills
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<HashMap<String, SkillOverride>>,
}

/// Per-skill override in settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOverride {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discord: Option<DiscordChannelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telegram: Option<TelegramChannelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordChannelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChannelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelsConfig {
    /// Provider: "anthropic", "openai", or "opencode"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<ProviderModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai: Option<ProviderModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opencode: Option<ProviderModelConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval: Option<u64>,
}

/// Queue message format — written as JSON to incoming/processing/outgoing directories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub channel: String,
    pub sender: String,
    #[serde(rename = "senderId", skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
    pub message: String,
    pub timestamp: u64,
    #[serde(rename = "messageId")]
    pub message_id: String,
    /// Pre-routed agent id from channel client
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    /// Internal: links to parent conversation (agent-to-agent)
    #[serde(rename = "conversationId", skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Internal: which agent sent this internal message
    #[serde(rename = "fromAgent", skip_serializing_if = "Option::is_none")]
    pub from_agent: Option<String>,
}

/// Outgoing response format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseData {
    pub channel: String,
    pub sender: String,
    pub message: String,
    #[serde(rename = "originalMessage")]
    pub original_message: String,
    pub timestamp: u64,
    #[serde(rename = "messageId")]
    pub message_id: String,
    /// Which agent handled this
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
}

/// Metadata for a file in the queue directory
#[derive(Debug, Clone)]
pub struct QueueFile {
    pub name: String,
    pub path: std::path::PathBuf,
    /// Modified time in milliseconds since epoch
    pub time: u64,
}

/// In-memory conversation tracker for team chains.
/// Not serialized — lives only in the queue processor's memory.
#[derive(Debug)]
pub struct Conversation {
    pub id: String,
    pub channel: String,
    pub sender: String,
    pub original_message: String,
    pub message_id: String,
    /// Number of pending agent branches
    pub pending: i32,
    pub responses: Vec<ChainStep>,
    pub files: std::collections::HashSet<String>,
    pub total_messages: u32,
    pub max_messages: u32,
    pub team_context: Option<TeamContext>,
    pub start_time: u64,
    /// Track how many mentions each agent sent out (for inbox draining)
    pub outgoing_mentions: HashMap<String, u32>,
}

/// Team context for a conversation
#[derive(Debug, Clone)]
pub struct TeamContext {
    pub team_id: String,
    pub team: TeamConfig,
}

/// Routing result from parseAgentRouting
#[derive(Debug, Clone)]
pub struct RoutingResult {
    pub agent_id: String,
    pub message: String,
    pub is_team: bool,
    /// Non-empty when multiple agents are detected for parallel dispatch
    pub multi_agents: Vec<String>,
}

/// Teammate mention extracted from agent response
#[derive(Debug, Clone)]
pub struct TeammateMention {
    pub teammate_id: String,
    pub message: String,
}

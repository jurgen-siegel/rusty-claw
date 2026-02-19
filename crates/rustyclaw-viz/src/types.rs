use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ─── Agent display status ───────────────────────────────────────────────────

#[derive(Clone, PartialEq, Debug)]
pub enum AgentStatus {
    Idle,
    Active,
    Done,
    Error,
    Waiting,
}

impl AgentStatus {
    pub fn css_class(&self) -> &str {
        match self {
            Self::Idle => "idle",
            Self::Active => "active",
            Self::Done => "done",
            Self::Error => "error",
            Self::Waiting => "waiting",
        }
    }

    pub fn icon(&self) -> &str {
        match self {
            Self::Idle => "\u{25CB}",    // ○
            Self::Active => "\u{25CF}",  // ●
            Self::Done => "\u{2713}",    // ✓
            Self::Error => "\u{2717}",   // ✗
            Self::Waiting => "\u{25D4}", // ◔
        }
    }
}

// ─── Display state for each agent ──────────────────────────────────────────

#[derive(Clone, PartialEq)]
pub struct AgentState {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    pub status: AgentStatus,
    pub last_activity: String,
    pub response_length: Option<usize>,
}

// ─── Chain arrow (handoff visualization) ────────────────────────────────────

#[derive(Clone, PartialEq)]
pub struct ChainArrow {
    pub from: String,
    pub to: String,
}

// ─── Activity log entry ─────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
pub struct LogEntry {
    pub time: String,
    pub icon: String,
    pub text: String,
    pub css_class: String,
}

// ─── Server response: settings ──────────────────────────────────────────────

#[derive(Clone, PartialEq, Default, Deserialize)]
pub struct VizSettings {
    #[serde(default)]
    pub teams: HashMap<String, VizTeam>,
    #[serde(default)]
    pub agents: HashMap<String, VizAgentConfig>,
}

#[derive(Clone, PartialEq, Deserialize)]
pub struct VizTeam {
    pub name: String,
    pub agents: Vec<String>,
    pub leader_agent: String,
}

#[derive(Clone, PartialEq, Deserialize)]
pub struct VizAgentConfig {
    pub name: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub reset_policy: Option<String>,
    #[serde(default)]
    pub fallbacks: Option<Vec<String>>,
    #[serde(default)]
    pub route_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub route_priority: Option<u32>,
    #[serde(default)]
    pub cross_team_handoffs: Option<bool>,
}

// ─── Queue messages (from /api/queue) ───────────────────────────────────────

#[derive(Clone, PartialEq, Deserialize)]
pub struct QueuedMessage {
    pub message_id: String,
    pub channel: String,
    pub sender: String,
    pub message: String,
    pub agent: Option<String>,
    pub timestamp: u64,
    pub status: String,
}

#[derive(Clone, PartialEq, Default, Deserialize)]
pub struct QueueMessagesResponse {
    #[serde(default)]
    pub incoming: Vec<QueuedMessage>,
    #[serde(default)]
    pub processing: Vec<QueuedMessage>,
}

// ─── Kanban board types ─────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Debug)]
pub enum KanbanCardStatus {
    Queued,
    Active,
    StepDone,
    Done,
}

impl KanbanCardStatus {
    pub fn css_class(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Active => "active",
            Self::StepDone => "step-done",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct HandoffStep {
    pub agent_id: String,
    pub entered_at: f64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct KanbanCard {
    pub id: String,
    pub current_agent: String,
    pub message_preview: String,
    pub channel: String,
    pub sender: String,
    pub status: KanbanCardStatus,
    pub entered_column_at: f64,
    pub created_at: f64,
    pub handoff_trail: Vec<HandoffStep>,
    pub done_at: Option<f64>,
    pub response_length: Option<usize>,
}

// ─── Tab navigation ─────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
pub enum Tab {
    Dashboard,
    Kanban,
    Queue,
    Settings,
}

// ─── Full settings types (for /api/settings/full round-tripping) ────────────

#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FullSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<FullWorkspaceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<FullChannelsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<FullModelsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<HashMap<String, FullAgentConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teams: Option<HashMap<String, FullTeamConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitoring: Option<FullMonitoringConfig>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullWorkspaceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullChannelsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discord: Option<FullDiscordConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telegram: Option<FullTelegramConfig>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullDiscordConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullTelegramConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullModelsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<FullProviderModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai: Option<FullProviderModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opencode: Option<FullProviderModelConfig>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullProviderModelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullAgentConfig {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub working_directory: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_hour: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallbacks: Option<Vec<String>>,
    #[serde(default = "default_true_option", skip_serializing_if = "Option::is_none")]
    pub cross_team_handoffs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_patterns: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_priority: Option<u32>,
}

fn default_true_option() -> Option<bool> {
    Some(true)
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullTeamConfig {
    pub name: String,
    pub agents: Vec<String>,
    pub leader_agent: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct FullMonitoringConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_interval: Option<u64>,
}

// ─── WebSocket event from viz server ────────────────────────────────────────

#[derive(Deserialize)]
pub struct VizEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub timestamp: Option<u64>,
    /// All other dynamic fields
    #[serde(flatten)]
    pub data: HashMap<String, serde_json::Value>,
}

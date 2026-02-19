use yew::prelude::*;

use crate::types::{AgentState, AgentStatus};

#[derive(Properties, PartialEq)]
pub struct AgentCardProps {
    pub agent: AgentState,
    pub is_leader: bool,
}

/// Animated dots based on current time
fn dots() -> String {
    let ms = js_sys::Date::now() as u64;
    match (ms / 400) % 4 {
        0 => String::new(),
        1 => ".".to_string(),
        2 => "..".to_string(),
        _ => "...".to_string(),
    }
}

#[function_component(AgentCard)]
pub fn agent_card(props: &AgentCardProps) -> Html {
    let agent = props.agent.clone();
    let status_class = format!("status-{}", agent.status.css_class());
    let status_icon_str = agent.status.icon().to_string();
    let status_icon_class = agent.status.css_class().to_string();

    let activity = match agent.status {
        AgentStatus::Active => {
            html! { <span class="processing">{format!("Processing{}", dots())}</span> }
        }
        AgentStatus::Done => {
            let chars = agent.response_length.unwrap_or(0);
            html! { <span class="done">{format!("\u{2713} Done ({} chars)", chars)}</span> }
        }
        AgentStatus::Error => {
            html! { <span class="error">{"\u{2717} Error"}</span> }
        }
        AgentStatus::Waiting => {
            html! { <span class="waiting">{agent.last_activity.clone()}</span> }
        }
        AgentStatus::Idle => {
            let text = if agent.last_activity.is_empty() {
                "Idle".to_string()
            } else {
                agent.last_activity.clone()
            };
            html! { <span class="idle">{text}</span> }
        }
    };

    html! {
        <div class={classes!("agent-card", status_class)}>
            <div class="agent-header">
                <span class={classes!("status-icon", status_icon_class)}>
                    {status_icon_str}
                </span>
                <span class="agent-id">{format!("@{}", agent.id)}</span>
                if props.is_leader {
                    <span class="leader-star">{"\u{2605}"}</span>
                }
            </div>
            <div class="agent-name">{agent.name.clone()}</div>
            <div class="agent-model">{format!("{}/{}", agent.provider, agent.model)}</div>
            <div class="agent-activity">
                {activity}
            </div>
        </div>
    }
}

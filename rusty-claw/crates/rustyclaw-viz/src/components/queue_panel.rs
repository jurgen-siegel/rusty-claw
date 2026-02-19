use yew::prelude::*;

use crate::types::QueuedMessage;

#[derive(Properties, PartialEq)]
pub struct QueuePanelProps {
    pub messages: Vec<QueuedMessage>,
}

#[function_component(QueuePanel)]
pub fn queue_panel(props: &QueuePanelProps) -> Html {
    if props.messages.is_empty() {
        return html! {};
    }

    html! {
        <div class="queue-panel">
            <h3>{"Queue"}</h3>
            <hr class="divider" />
            <div class="queue-entries">
                { for props.messages.iter().map(|msg| {
                    let status_class = format!("queue-status-{}", msg.status);
                    let status_label = match msg.status.as_str() {
                        "processing" => "processing",
                        _ => "incoming",
                    };
                    let agent_display = msg.agent.as_deref().unwrap_or("auto");
                    let preview = if msg.message.len() > 80 {
                        format!("{}...", &msg.message[..80])
                    } else {
                        msg.message.clone()
                    };

                    html! {
                        <div class={classes!("queue-entry", status_class.clone())}>
                            <div class="queue-entry-header">
                                <span class={classes!("queue-badge", status_class)}>
                                    {status_label}
                                </span>
                                <span class="queue-sender">
                                    {format!("{}/{}", msg.channel, msg.sender)}
                                </span>
                                <span class="queue-agent">
                                    {format!("@{}", agent_display)}
                                </span>
                            </div>
                            <div class="queue-preview">{preview}</div>
                        </div>
                    }
                })}
            </div>
        </div>
    }
}

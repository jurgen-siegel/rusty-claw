use yew::prelude::*;

use crate::types::LogEntry;

#[derive(Properties, PartialEq)]
pub struct ActivityLogProps {
    pub entries: Vec<LogEntry>,
}

#[function_component(ActivityLog)]
pub fn activity_log(props: &ActivityLogProps) -> Html {
    html! {
        <div class="activity-log">
            <h3>{"Activity"}</h3>
            <hr class="divider" />
            if props.entries.is_empty() {
                <p class="empty-log">{"Waiting for events..."}</p>
            } else {
                <div class="log-entries">
                    { for props.entries.iter().map(|entry| {
                        html! {
                            <div class={classes!("log-entry", entry.css_class.clone())}>
                                <span class="log-time">{&entry.time}</span>
                                <span class="log-icon">{" "}{&entry.icon}{" "}</span>
                                <span class="log-text">{&entry.text}</span>
                            </div>
                        }
                    })}
                </div>
            }
        </div>
    }
}

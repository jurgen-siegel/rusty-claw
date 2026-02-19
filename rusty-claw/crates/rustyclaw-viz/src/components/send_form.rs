use std::collections::HashMap;

use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;
use gloo_net::http::Request;

use crate::types::VizAgentConfig;

#[derive(Properties, PartialEq)]
pub struct SendFormProps {
    pub agents: HashMap<String, VizAgentConfig>,
}

#[function_component(SendForm)]
pub fn send_form(props: &SendFormProps) -> Html {
    let message = use_state(String::new);
    let agent = use_state(String::new);
    let sending = use_state(|| false);
    let feedback = use_state(String::new);

    let on_input = {
        let message = message.clone();
        Callback::from(move |e: InputEvent| {
            let input: web_sys::HtmlInputElement = e.target_unchecked_into();
            message.set(input.value());
        })
    };

    let on_agent_change = {
        let agent = agent.clone();
        Callback::from(move |e: Event| {
            let select: web_sys::HtmlSelectElement = e.target_unchecked_into();
            agent.set(select.value());
        })
    };

    let on_submit = {
        let message = message.clone();
        let agent = agent.clone();
        let sending = sending.clone();
        let feedback = feedback.clone();
        Callback::from(move |e: SubmitEvent| {
            e.prevent_default();
            let msg_text = (*message).clone();
            if msg_text.trim().is_empty() || *sending {
                return;
            }
            let agent_val = (*agent).clone();
            let agent_opt = if agent_val.is_empty() {
                None
            } else {
                Some(agent_val)
            };

            sending.set(true);
            feedback.set(String::new());
            let message = message.clone();
            let sending = sending.clone();
            let feedback = feedback.clone();

            spawn_local(async move {
                let body = serde_json::json!({
                    "message": msg_text,
                    "agent": agent_opt,
                });
                let result = Request::post("/api/send")
                    .header("Content-Type", "application/json")
                    .body(body.to_string())
                    .unwrap()
                    .send()
                    .await;

                sending.set(false);
                match result {
                    Ok(resp) if resp.ok() => {
                        message.set(String::new());
                        feedback.set("Queued!".to_string());
                    }
                    Ok(resp) => {
                        feedback.set(format!("Error ({})", resp.status()));
                    }
                    Err(e) => {
                        feedback.set(format!("Failed: {}", e));
                    }
                }
            });
        })
    };

    let mut agent_ids: Vec<&String> = props.agents.keys().collect();
    agent_ids.sort();

    html! {
        <div class="send-form-panel">
            <h3>{"Send Message"}</h3>
            <hr class="divider" />
            <form class="send-form" onsubmit={on_submit}>
                <input
                    type="text"
                    class="send-input"
                    placeholder="Type a message..."
                    value={(*message).clone()}
                    oninput={on_input}
                    disabled={*sending}
                />
                <select class="send-agent-select" onchange={on_agent_change}>
                    <option value="" selected=true>{"Any agent"}</option>
                    { for agent_ids.iter().map(|id| {
                        let name = props.agents.get(*id)
                            .map(|a| a.name.as_str())
                            .unwrap_or("");
                        html! {
                            <option value={(*id).clone()}>
                                {format!("@{} ({})", id, name)}
                            </option>
                        }
                    })}
                </select>
                <button type="submit" class="send-button" disabled={*sending}>
                    if *sending {
                        {"Sending..."}
                    } else {
                        {"Send"}
                    }
                </button>
                if !feedback.is_empty() {
                    <span class="send-feedback">{&*feedback}</span>
                }
            </form>
        </div>
    }
}

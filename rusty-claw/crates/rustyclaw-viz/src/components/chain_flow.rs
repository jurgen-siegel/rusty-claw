use yew::prelude::*;

use crate::types::ChainArrow;

#[derive(Properties, PartialEq)]
pub struct ChainFlowProps {
    pub arrows: Vec<ChainArrow>,
}

#[function_component(ChainFlow)]
pub fn chain_flow(props: &ChainFlowProps) -> Html {
    if props.arrows.is_empty() {
        return html! {};
    }

    html! {
        <div class="chain-flow">
            <h3>{"Message Flow"}</h3>
            <div class="chain-arrows">
                { for props.arrows.iter().enumerate().map(|(i, arrow)| {
                    html! {
                        <span class="chain-step">
                            <span class="chain-from">{format!("@{}", arrow.from)}</span>
                            <span class="chain-arrow">{" \u{2192} "}</span>
                            <span class="chain-to">{format!("@{}", arrow.to)}</span>
                            if i < props.arrows.len() - 1 {
                                <span class="chain-sep">{" \u{2502} "}</span>
                            }
                        </span>
                    }
                })}
            </div>
        </div>
    }
}

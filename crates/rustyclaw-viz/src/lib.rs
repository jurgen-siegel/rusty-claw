mod app;
mod components;
mod types;

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn run() {
    yew::Renderer::<app::App>::new().render();
}

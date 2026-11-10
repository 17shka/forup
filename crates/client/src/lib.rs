#![allow(deprecated)]

include!(concat!(env!("OUT_DIR"), "/i18n/mod.rs"));

pub mod app;
pub mod auth;
mod components;
mod game;
mod pages;
mod settings;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}

/// Entry point for static, client-side rendered builds (e.g. GitHub Pages).
#[cfg(feature = "csr")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}

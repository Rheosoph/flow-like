#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod geometry;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod model;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod status;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod text_cache;

#[cfg(target_arch = "wasm32")]
mod aura;
#[cfg(target_arch = "wasm32")]
mod painter;

#[cfg(target_arch = "wasm32")]
pub use painter::*;

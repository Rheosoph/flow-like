pub mod accessibility;
pub mod capture;
pub mod clipboard;
pub mod display;
pub mod keyboard;
pub mod mouse;
pub mod session;
pub mod wait;
pub mod window;

pub mod capture_state;
pub mod ocr;
pub mod stability;
pub mod zoom;
#[cfg(feature = "execute")]
pub mod native;

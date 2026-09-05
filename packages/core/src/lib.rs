//! Public Flow-Like API, combining the runtime with FlowScript and copilot services.
//! Catalog implementations depend on `flow-like-runtime` directly so they can compile
//! alongside editor services.

pub use flow_like_runtime::*;

#[cfg(feature = "flow-metadata")]
pub use flow_like_editor::copilot;

#[cfg(feature = "flow-metadata")]
pub mod flow {
    pub use flow_like_editor::flow::{ast, copilot};
    pub use flow_like_runtime::flow::*;
}

pub mod a2ui {
    #[cfg(feature = "flow-metadata")]
    pub use flow_like_editor::a2ui::copilot;
    #[cfg(feature = "flow-metadata")]
    pub use flow_like_editor::a2ui::copilot::*;
    pub use flow_like_runtime::a2ui::*;
}

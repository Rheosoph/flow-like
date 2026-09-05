//! Shared data handles and runtime helpers used by independent catalog packages.

pub mod remote_util;

#[path = "attachment.rs"]
mod attachment_support;
#[path = "cache.rs"]
mod cache_support;
#[path = "graph.rs"]
mod graph_support;
#[path = "path.rs"]
mod path_support;
#[path = "query.rs"]
mod query_support;
#[path = "session.rs"]
mod session_support;
#[path = "table.rs"]
mod table_support;

pub mod data {
    pub mod cache {
        pub use crate::cache_support::*;
    }
    pub mod path {
        pub use crate::path_support::*;
    }
    pub mod excel {
        pub use crate::table_support::*;
    }
    pub mod datafusion {
        pub mod session {
            pub use crate::session_support::*;
        }
        pub mod query {
            pub use crate::query_support::*;
        }
    }
    pub mod db {
        pub mod graph {
            #[cfg(feature = "execute")]
            pub use crate::graph_support::*;
        }
    }
}

pub mod events {
    pub mod chat_event {
        pub use crate::attachment_support::*;
    }
}

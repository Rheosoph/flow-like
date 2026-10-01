pub mod attach;
pub mod browser;
pub mod connection;
pub mod dialogs;
pub mod downloads;
pub mod element;
pub(crate) mod emulation;
pub mod error;
pub mod event_log;
pub mod fetch;
pub(crate) mod frames;
pub mod input;
pub mod launch;
pub(crate) mod navigation;
pub mod output;
pub mod page;
pub mod protocol;
pub mod refs;
pub mod script;
pub mod session;
pub(crate) mod settle;
pub mod snapshot;
pub(crate) mod target;
#[cfg(any(test, feature = "test-support"))]
pub mod test_hooks;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod transport;
pub mod types;
pub mod window;

pub use browser::{
    Browser, BrowserSettings, ClosePageOutcome, ConnectionKind, PageInfo, VersionInfo,
};
pub use connection::Connection;
pub use dialogs::{Dialog, DialogAction};
pub use element::{Element, ElementRect};
pub use error::{BrowserError, ErrorClass, Result};
pub use page::{Frame, Page};
pub use refs::{NodeRef, ProposedRef, RefAllocator, RefEntry, RefTable};
pub use session::Session;
pub use settle::NavigationOutcome;

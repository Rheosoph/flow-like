//! Runtime-independent package manifest data model.

#[cfg(feature = "nodes")]
mod node;
mod package;
mod permissions;

#[cfg(feature = "nodes")]
pub use node::PackageNodeEntry;
pub use package::{
    PackageAuthor, PackageManifest, PackageWidgetEntry, WasmPackageCategory, MANIFEST_VERSION,
};
pub use permissions::{
    DatabasePermissions, FileSystemPermissions, MemoryTier, NetworkPermissions,
    OAuthScopeRequirement, PackagePermissions, PackageSecurityConfig, TimeoutTier,
};

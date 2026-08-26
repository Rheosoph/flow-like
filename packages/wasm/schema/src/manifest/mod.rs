//! Runtime-independent package manifest data model.

#[cfg(feature = "nodes")]
mod node;
mod package;
mod permissions;

#[cfg(feature = "nodes")]
pub use node::PackageNodeEntry;
pub use package::{
    MANIFEST_VERSION, PackageAuthor, PackageManifest, PackageWidgetEntry, WasmPackageCategory,
};
pub use permissions::{
    FileSystemPermissions, MemoryTier, NetworkPermissions, OAuthScopeRequirement,
    PackagePermissions, PackageSecurityConfig, TimeoutTier,
};

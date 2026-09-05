//! Constructors that preserve Flow-Like's WASI isolation invariants.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;

use wasmtime_wasi::cli::{StdinStream, StdoutStream};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::sockets::SocketAddrUse;
use wasmtime_wasi::{
    Deterministic, FsPerms, HostMonotonicClock, HostWallClock, WasiCtx, WasiCtxBuilder,
};

/// A WASI context builder that cannot inherit the host process environment.
///
/// The raw Wasmtime builder remains private and this type intentionally does
/// not implement `Deref`, `AsMut`, or an inner-value escape hatch. Only
/// explicitly approved guest capabilities are exposed below.
pub struct IsolatedWasiCtxBuilder {
    inner: WasiCtxBuilder,
}

impl Default for IsolatedWasiCtxBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IsolatedWasiCtxBuilder {
    /// Start with closed stdin, discarded output, no arguments, no preopens,
    /// no network addresses, and no guest environment variables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: WasiCtxBuilder::new(),
        }
    }

    /// Forward guest stdout and stderr to the host while leaving stdin closed.
    pub fn inherit_output(&mut self) -> &mut Self {
        self.inner.inherit_stdout().inherit_stderr();
        self
    }

    pub fn stdin(&mut self, stdin: impl StdinStream + 'static) -> &mut Self {
        self.inner.stdin(stdin);
        self
    }

    pub fn stdout(&mut self, stdout: impl StdoutStream + 'static) -> &mut Self {
        self.inner.stdout(stdout);
        self
    }

    pub fn stderr(&mut self, stderr: impl StdoutStream + 'static) -> &mut Self {
        self.inner.stderr(stderr);
        self
    }

    /// Add a single value from explicit guest configuration.
    pub fn guest_env(&mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> &mut Self {
        self.inner.env(key, value);
        self
    }

    pub fn args(&mut self, args: &[impl AsRef<str>]) -> &mut Self {
        self.inner.args(args);
        self
    }

    pub fn preopened_dir(
        &mut self,
        host_path: impl AsRef<Path>,
        guest_path: impl AsRef<str>,
        perms: FsPerms,
    ) -> wasmtime::Result<&mut Self> {
        self.inner.preopened_dir(host_path, guest_path, perms)?;
        Ok(self)
    }

    /// Allow host network addresses. TCP, UDP, and name lookup each require
    /// their own explicit protocol grant.
    pub fn inherit_network(&mut self) -> &mut Self {
        self.inner.inherit_network();
        self
    }

    pub fn socket_addr_check<F>(&mut self, check: F) -> &mut Self
    where
        F: Fn(SocketAddr, SocketAddrUse) -> Pin<Box<dyn Future<Output = bool> + Send + Sync>>
            + Send
            + Sync
            + 'static,
    {
        self.inner.socket_addr_check(check);
        self
    }

    pub fn allow_ip_name_lookup(&mut self, enable: bool) -> &mut Self {
        self.inner.allow_ip_name_lookup(enable);
        self
    }

    pub fn allow_udp(&mut self, enable: bool) -> &mut Self {
        self.inner.allow_udp(enable);
        self
    }

    pub fn allow_tcp(&mut self, enable: bool) -> &mut Self {
        self.inner.allow_tcp(enable);
        self
    }

    /// Strip the two ambient sources a guest can read without any capability:
    /// `wasi:clocks` and `wasi:random`.
    ///
    /// Components always get a real WASI context, so a guest asked only to
    /// report its node definitions could otherwise consult the clock or draw
    /// entropy and answer differently on every call. Running extraction
    /// against this makes the same module bytes yield the same definitions.
    pub fn make_deterministic(&mut self) -> &mut Self {
        self.inner
            .wall_clock(FrozenClock)
            .monotonic_clock(FrozenClock)
            .secure_random(fixed_entropy())
            .insecure_random(fixed_entropy())
            .insecure_random_seed(0);
        self
    }

    pub fn build(&mut self) -> WasiCtx {
        self.inner.build()
    }

    pub fn build_p1(&mut self) -> WasiP1Ctx {
        self.inner.build_p1()
    }
}

/// Start a WASI context with no ambient host environment.
#[must_use]
pub fn isolated_wasi_ctx_builder() -> IsolatedWasiCtxBuilder {
    IsolatedWasiCtxBuilder::new()
}

/// A fixed byte cycle standing in for the host entropy source.
fn fixed_entropy() -> Deterministic {
    Deterministic::new(vec![0x9e, 0x37, 0x79, 0xb9, 0x7f, 0x4a, 0x7c, 0x15])
}

/// A clock stopped at the epoch, reporting one-second resolution so guests
/// that divide by it stay well-defined.
struct FrozenClock;

impl HostWallClock for FrozenClock {
    fn resolution(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }

    fn now(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

impl HostMonotonicClock for FrozenClock {
    fn resolution(&self) -> u64 {
        1_000_000_000
    }

    fn now(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::component::{Resource, ResourceTable};
    use wasmtime_wasi::filesystem::WasiFilesystemCtxView;
    use wasmtime_wasi::p2::bindings::filesystem::{
        preopens,
        types::{DescriptorFlags, ErrorCode, HostDescriptor, OpenFlags, PathFlags},
    };

    #[tokio::test]
    async fn preopen_permissions_allow_reads_and_only_grant_writes_when_requested() {
        for (perms, writable) in [(FsPerms::ReadOnly, false), (FsPerms::ReadWrite, true)] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("existing.txt"), "saved").unwrap();
            let mut builder = isolated_wasi_ctx_builder();
            builder.preopened_dir(root.path(), "/flow", perms).unwrap();
            let mut ctx = builder.build();
            let mut table = ResourceTable::new();
            let mut filesystem = WasiFilesystemCtxView {
                ctx: ctx.filesystem(),
                table: &mut table,
            };
            let (directory, path) = preopens::Host::get_directories(&mut filesystem)
                .unwrap()
                .pop()
                .unwrap();
            assert_eq!(path, "/flow");
            let file = HostDescriptor::open_at(
                &mut filesystem,
                Resource::new_borrow(directory.rep()),
                PathFlags::empty(),
                "existing.txt".into(),
                OpenFlags::empty(),
                DescriptorFlags::READ,
            )
            .await
            .unwrap();
            let (contents, _) = HostDescriptor::read(&mut filesystem, file, 16, 0)
                .await
                .unwrap();
            assert_eq!(contents, b"saved");
            let created = HostDescriptor::open_at(
                &mut filesystem,
                Resource::new_borrow(directory.rep()),
                PathFlags::empty(),
                "new.txt".into(),
                OpenFlags::CREATE,
                DescriptorFlags::WRITE,
            )
            .await;
            if writable {
                let file = created.expect("a read-write preopen should allow file creation");
                HostDescriptor::write(&mut filesystem, file, b"new".to_vec(), 0)
                    .await
                    .unwrap();
                assert_eq!(std::fs::read(root.path().join("new.txt")).unwrap(), b"new");
            } else {
                assert_eq!(
                    created.unwrap_err().downcast().unwrap(),
                    ErrorCode::NotPermitted
                );
                assert!(!root.path().join("new.txt").exists());
                let truncate = HostDescriptor::open_at(
                    &mut filesystem,
                    Resource::new_borrow(directory.rep()),
                    PathFlags::empty(),
                    "existing.txt".into(),
                    OpenFlags::TRUNCATE,
                    DescriptorFlags::WRITE,
                )
                .await;
                assert_eq!(
                    truncate.unwrap_err().downcast().unwrap(),
                    ErrorCode::NotPermitted
                );
                assert_eq!(
                    std::fs::read(root.path().join("existing.txt")).unwrap(),
                    b"saved"
                );
            }
        }
    }
}

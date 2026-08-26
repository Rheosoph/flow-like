//! Backwards-compatible manifest surface for the Wasmtime host.
//!
//! Wire types live in `flow-like-wasm-schema`; only the conversion into the
//! host's execution security configuration is implemented here.

pub use flow_like_wasm_schema::manifest::*;

impl PackageSecurityConfig for crate::limits::WasmSecurityConfig {
    fn from_package_permissions(permissions: &PackagePermissions) -> Self {
        Self {
            limits: permissions.to_limits(),
            capabilities: permissions.to_capabilities(),
            allow_wasi: false,
            // Individual network capabilities are enforced independently.
            // This flag is reserved for an explicit grant-all override.
            allow_wasi_network: false,
            allowed_hosts: if permissions.network.allowed_hosts.is_empty() {
                None
            } else {
                Some(permissions.network.allowed_hosts.clone())
            },
            execution_environment: Default::default(),
            deterministic: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_convert_to_the_host_security_config() {
        let permissions = PackagePermissions {
            network: NetworkPermissions {
                http_enabled: true,
                allowed_hosts: vec!["api.example.com".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let security: crate::WasmSecurityConfig = permissions.to_security_config();

        assert!(security
            .capabilities
            .contains(crate::WasmCapabilities::HTTP_ALL));
        assert_eq!(
            security.allowed_hosts.as_deref(),
            Some(&["api.example.com".to_string()][..])
        );
    }
}

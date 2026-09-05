use execution_manager::{
    CommonConfig, Error, Result,
    config::positive,
    server::{self, ServerState},
};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let result = positive("EXECUTION_MANAGER_WORKER_THREADS", 2, 64).and_then(|workers| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(workers as usize)
            .max_blocking_threads(16)
            .build()?;
        runtime.block_on(run())
    });
    if let Err(error) = result {
        // SDK errors can contain URLs and credentials. Diagnostics must remain
        // structural; operators inspect backend readiness and lifecycle metrics.
        if let Error::Invalid(message) = error {
            eprintln!("Execution supervisor configuration: {message}");
        } else {
            eprintln!("Execution supervisor failed; check configuration and infrastructure health");
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let target = arguments.next().unwrap_or_else(|| "docker".into());
    if target == "healthcheck" {
        let endpoint = arguments
            .next()
            .unwrap_or_else(|| "http://127.0.0.1:9000/ready".into());
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()?
            .get(endpoint)
            .send()
            .await?
            .error_for_status()?;
        return Ok(());
    }
    if !matches!(target.as_str(), "docker" | "kubernetes") {
        return Err(Error::invalid("Expected docker, kubernetes or healthcheck"));
    }
    let config = Arc::new(CommonConfig::from_env(target == "kubernetes")?);
    let backend = if target == "docker" {
        execution_manager::docker::from_env(config.clone()).await?
    } else {
        execution_manager::kubernetes::from_env(config.clone()).await?
    };
    backend.clone().prepare().await?;
    let listen = listen_address(
        target == "kubernetes",
        std::env::var("POD_IP").ok().as_deref(),
    )?;
    let listener =
        tokio::net::TcpListener::bind((listen, positive("PORT", 9000, 65535)? as u16)).await?;
    let stop = CancellationToken::new();
    let shutdown = stop.clone();
    tokio::spawn(async move {
        server::shutdown_signal().await;
        shutdown.cancel();
    });
    server::serve(listener, ServerState::new(backend, config), stop).await
}

fn listen_address(kubernetes: bool, pod_ip: Option<&str>) -> Result<IpAddr> {
    if kubernetes && let Some(address) = pod_ip {
        let address: IpAddr = address
            .parse()
            .map_err(|_| Error::invalid("POD_IP must be a valid IP address"))?;
        if address.is_ipv6() {
            return Ok(Ipv6Addr::UNSPECIFIED.into());
        }
    }
    Ok(Ipv4Addr::UNSPECIFIED.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_listens_on_its_kubernetes_pod_address_family() {
        assert_eq!(
            listen_address(true, Some("fd00::10")).unwrap(),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        );
        assert_eq!(
            listen_address(true, Some("10.0.0.10")).unwrap(),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
        assert!(listen_address(true, Some("not-an-ip")).is_err());
        assert_eq!(
            listen_address(false, Some("fd00::10")).unwrap(),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        );
    }
}

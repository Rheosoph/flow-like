use std::{
    io::Cursor,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use flow_like_runtime::flow::execution::service::{
    BoxedServiceIo, ServiceTlsFuture, ServiceTlsProvider,
};
use tokio::{net::TcpStream, sync::Mutex};

use crate::{
    certificates::CertificateIdentity,
    ipc::{ChildBroker, TlsIdentityUpdate},
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

struct CachedIdentity {
    revision: u64,
    config: Arc<rustls::ServerConfig>,
    not_before: i64,
    not_after: i64,
    checked_at: Instant,
}

/// Only the supervisor chooses which identity a workload may receive.
pub struct ManagedTls {
    broker: Arc<ChildBroker>,
    cached: Mutex<Option<CachedIdentity>>,
}

impl ManagedTls {
    pub fn new(broker: Arc<ChildBroker>) -> Arc<Self> {
        Arc::new(Self {
            broker,
            cached: Mutex::new(None),
        })
    }

    async fn configuration(&self) -> Result<Arc<rustls::ServerConfig>> {
        let mut cached = self.cached.lock().await;
        if cached
            .as_ref()
            .is_none_or(|entry| entry.checked_at.elapsed() >= REFRESH_INTERVAL)
        {
            let update = async {
                match cached.as_ref() {
                    Some(current) => tokio::time::timeout(
                        HANDSHAKE_TIMEOUT,
                        self.broker.try_tls_identity(current.revision),
                    )
                    .await
                    .context("Placement TLS broker timed out")?,
                    None => Ok(TlsIdentityUpdate::Changed(
                        tokio::time::timeout(HANDSHAKE_TIMEOUT, self.broker.tls_identity(None))
                            .await
                            .context("Placement TLS broker timed out")??
                            .context("Missing initial placement TLS identity")?,
                    )),
                }
            }
            .await;
            match update {
                Err(error) => {
                    // A later busy request must never revive a denied identity.
                    *cached = None;
                    return Err(error);
                }
                Ok(TlsIdentityUpdate::Changed(identity)) => {
                    let config = match server_config(&identity) {
                        Ok(config) => config,
                        Err(error) => {
                            *cached = None;
                            return Err(error);
                        }
                    };
                    *cached = Some(CachedIdentity {
                        config,
                        revision: identity.revision,
                        not_before: identity.not_before,
                        not_after: identity.not_after,
                        checked_at: Instant::now(),
                    });
                }
                Ok(TlsIdentityUpdate::Unchanged) => {
                    cached
                        .as_mut()
                        .context("Missing initial placement TLS identity")?
                        .checked_at = Instant::now();
                }
                Ok(TlsIdentityUpdate::Busy) => {}
            }
        }
        let identity = cached.as_ref().context("Missing placement TLS identity")?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        ensure!(
            now >= identity.not_before && now < identity.not_after,
            "Placement TLS certificate is outside its validity period"
        );
        Ok(identity.config.clone())
    }
}

fn server_config(identity: &CertificateIdentity) -> Result<Arc<rustls::ServerConfig>> {
    let certs = rustls_pemfile::certs(&mut Cursor::new(
        identity.certificate_chain_pem.0.as_bytes(),
    ))
    .collect::<std::io::Result<Vec<_>>>()
    .context("Invalid placement certificate chain")?;
    let key = rustls_pemfile::private_key(&mut Cursor::new(identity.private_key_pem.0.as_bytes()))
        .context("Invalid placement private key")?
        .context("Missing placement private key")?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .context("Invalid placement TLS identity")?;
    Ok(Arc::new(config))
}

impl ServiceTlsProvider for ManagedTls {
    fn validate(&self) -> ServiceTlsFuture<'_, ()> {
        Box::pin(async move { self.configuration().await.map(|_| ()) })
    }

    fn accept(&self, stream: TcpStream) -> ServiceTlsFuture<'_, BoxedServiceIo> {
        Box::pin(async move {
            let acceptor = tokio_rustls::TlsAcceptor::from(self.configuration().await?);
            let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream))
                .await
                .context("Placement TLS handshake timed out")?
                .context("Placement TLS handshake failed")?;
            Ok(Box::new(stream) as BoxedServiceIo)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::PlacementConfig,
        ipc::{self, ChildBootstrap},
        state::{DesiredState, ObservedState, StateStore},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn live_tls_rotation_preserves_connections_and_fences_stopped_workloads() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let id = uuid::Uuid::new_v4().to_string();
        let first = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let second = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let mut store = StateStore::open(&root.join("management.sqlite"))?;
        crate::certificates::put(
            &store,
            root,
            &id,
            "Service",
            0,
            &first.cert.pem(),
            &first.signing_key.serialize_pem(),
            crate::enrollment::unix_time()?,
        )?;
        let config: PlacementConfig = serde_json::from_value(serde_json::json!({
            "id":"placement","project_id":"project","deployment_id":"deployment","revision":"one",
            "source":"offline","project_path":root,"tls_certificate_id":id,
            "events":[{"event_id":"daemon","event_version":[1,0,0],"board_version":[1,0,0]}]
        }))?;
        store.upsert_placement(
            "placement",
            &serde_json::to_value(&config)?,
            DesiredState::Running,
        )?;
        store.claim_replica("placement", 0, 1, 1)?;
        store.record_replica(
            "placement",
            0,
            1,
            1,
            ObservedState::Starting,
            Some(42),
            None,
        )?;
        let bootstrap = ChildBootstrap {
            config,
            data_root: None,
            replica_slot: 0,
            inherited_listener: false,
            config_revision: 1,
            intent_revision: 1,
            parent_pid: 1,
            api_base_url: None,
            workload_identity: None,
        };
        let stop = CancellationToken::new();
        let (parent, child) = tokio::net::UnixStream::pair()?;
        let broker_task = tokio::spawn(ipc::serve(
            parent,
            bootstrap,
            root.into(),
            42,
            None,
            stop.clone(),
        ));
        let (_, broker) = ChildBroker::connect(child).await?;
        let tls = ManagedTls::new(broker.clone());
        tls.validate().await?;
        assert!(
            broker.tls_identity(Some(1)).await?.is_none(),
            "unchanged identities do not retransmit private keys"
        );
        {
            // A cloud request can occupy the shared broker during an outage.
            let _busy = broker.lock_channel_for_test().await;
            tls.cached.lock().await.as_mut().unwrap().checked_at =
                Instant::now() - REFRESH_INTERVAL;
            tokio::time::timeout(Duration::from_millis(100), tls.validate()).await??;
        }
        let mut trust = rustls::RootCertStore::empty();
        trust.add(first.cert.der().clone())?;
        trust.add(second.cert.der().clone())?;
        let client = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(trust)
        .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (client, server) = tokio::join!(
            async {
                connector
                    .connect("localhost".try_into()?, TcpStream::connect(address).await?)
                    .await
                    .map_err(anyhow::Error::from)
            },
            async {
                let (stream, _) = listener.accept().await?;
                tls.accept(stream).await
            }
        );
        let mut client = client?;
        let mut server = server?;
        assert_eq!(
            client.get_ref().1.peer_certificates().unwrap()[0],
            *first.cert.der()
        );
        crate::certificates::put(
            &store,
            root,
            &id,
            "Service",
            1,
            &second.cert.pem(),
            &second.signing_key.serialize_pem(),
            crate::enrollment::unix_time()?,
        )?;
        tls.cached.lock().await.as_mut().unwrap().checked_at = Instant::now() - REFRESH_INTERVAL;
        let (next_client, next_server) = tokio::join!(
            async {
                connector
                    .connect("localhost".try_into()?, TcpStream::connect(address).await?)
                    .await
                    .map_err(anyhow::Error::from)
            },
            async {
                let (stream, _) = listener.accept().await?;
                tls.accept(stream).await
            }
        );
        let next_client = next_client?;
        let _next_server = next_server?;
        assert_eq!(
            next_client.get_ref().1.peer_certificates().unwrap()[0],
            *second.cert.der()
        );
        server.write_all(b"still running").await?;
        let mut response = [0; 13];
        client.read_exact(&mut response).await?;
        assert_eq!(&response, b"still running");
        let mut plain = TcpStream::connect(address).await?;
        plain
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await?;
        let (stream, _) = listener.accept().await?;
        assert!(
            tls.accept(stream).await.is_err(),
            "managed TLS never accepts plaintext HTTP"
        );
        tls.cached.lock().await.as_mut().unwrap().not_after = crate::enrollment::unix_time()?;
        assert!(
            tls.validate().await.is_err(),
            "expiry is enforced even inside the refresh interval"
        );
        tls.cached.lock().await.as_mut().unwrap().checked_at = Instant::now() - REFRESH_INTERVAL;
        store.set_desired_state("placement", DesiredState::Stopped)?;
        assert!(
            tls.validate().await.is_err(),
            "a stopped placement cannot continue fetching keys"
        );
        assert!(
            tls.cached.lock().await.is_none(),
            "denied material cannot reappear during later broker contention"
        );
        stop.cancel();
        broker_task.await??;
        Ok(())
    }
}

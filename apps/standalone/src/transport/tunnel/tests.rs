use super::*;
use crate::{
    enrollment::{DeviceSession, unix_time},
    state::{DesiredState, ObservedState, StateStore},
};
use flow_like_device_protocol::{
    ArtifactTransferStatus, ControllerCertificate, ManagementCommand, ManagementRequest,
    ProjectArtifactFile, ProjectArtifactManifest, ProjectArtifactSource, SigningKey,
    TunnelDataOpen, TunnelHello, TunnelOpen, TunnelRenewStart, artifact_sha256,
    sign_controller_certificate,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct Client {
    input: mpsc::Sender<Vec<u8>>,
    output: mpsc::Receiver<Vec<u8>>,
    session: noise::Session,
    sent: u64,
    received: u64,
}

impl Client {
    async fn renew(&mut self, signer: &SigningKey) -> Result<()> {
        let cert = certificate(signer, "renewed", [8; 32], unix_time()? + 300)?;
        let mut initiator = noise::Handshake::tunnel_initiator(
            &[8; 32],
            x25519_dalek::x25519([42; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
            "device",
            "renewed",
        )?;
        self.send(
            0,
            TunnelFrameBody::RenewStart(TunnelRenewStart {
                certificate_jws: cert,
                data: wire::encode(&initiator.write()?),
            }),
        )
        .await?;
        let TunnelFrameBody::RenewReply(reply) = self.read().await?.body else {
            anyhow::bail!("No renewal reply")
        };
        initiator.read(&reply)?;
        self.send(0, TunnelFrameBody::RenewFinish(initiator.write()?))
            .await?;
        self.session = initiator.finish()?;
        ensure!(
            matches!(self.read().await?.body, TunnelFrameBody::Renewed(_)),
            "No renewed authorization"
        );
        Ok(())
    }
    async fn open_data(&mut self, id: u32, open: TunnelDataOpen) -> Result<()> {
        self.send(id, TunnelFrameBody::OpenData(open)).await?;
        let frame = self.read().await?;
        ensure!(
            frame.stream_id == id && matches!(frame.body, TunnelFrameBody::Opened),
            "Internal stream was not opened: {:?}",
            frame.body
        );
        Ok(())
    }

    async fn json_response(&mut self, id: u32) -> Result<Vec<u8>> {
        let mut response = Vec::new();
        loop {
            let frame = self.read().await?;
            ensure!(frame.stream_id == id, "Unexpected stream response");
            match frame.body {
                TunnelFrameBody::Data(bytes) => {
                    self.send(id, TunnelFrameBody::Window(bytes.len() as u32))
                        .await?;
                    response.extend(bytes);
                }
                TunnelFrameBody::Window(_) => {}
                TunnelFrameBody::Fin => return Ok(response),
                body => anyhow::bail!("Unexpected internal response: {body:?}"),
            }
        }
    }

    async fn upload(
        &mut self,
        id: u32,
        transfer_id: &str,
        index: Option<u32>,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ArtifactTransferStatus> {
        self.open_data(
            id,
            TunnelDataOpen::Artifact {
                project_id: "project".into(),
                transfer_id: transfer_id.into(),
                file_index: index,
                offset,
            },
        )
        .await?;
        let mut credit = INITIAL_WINDOW as usize;
        let mut sent = 0;
        while sent < bytes.len() {
            while credit > 0 && sent < bytes.len() {
                let count = (bytes.len() - sent).min(MAX_DATA).min(credit);
                self.send(id, TunnelFrameBody::Data(bytes[sent..sent + count].into()))
                    .await?;
                sent += count;
                credit -= count;
            }
            if sent < bytes.len() {
                let frame = self.read().await?;
                ensure!(frame.stream_id == id, "Wrong upload stream");
                let TunnelFrameBody::Window(delta) = frame.body else {
                    anyhow::bail!("Upload lost its window: {:?}", frame.body)
                };
                credit += delta as usize;
            }
        }
        self.send(id, TunnelFrameBody::Fin).await?;
        Ok(serde_json::from_slice(&self.json_response(id).await?)?)
    }

    async fn send(&mut self, stream_id: u32, body: TunnelFrameBody) -> Result<()> {
        let plaintext = TunnelFrame {
            sequence: self.sent,
            stream_id,
            body,
        }
        .encode()?;
        self.sent += 1;
        let data = self.session.encrypt(&plaintext)?;
        self.envelope(TunnelEnvelopeBody::Message(data)).await
    }
    async fn envelope(&self, body: TunnelEnvelopeBody) -> Result<()> {
        self.input
            .send(
                TunnelEnvelope {
                    session_id: "tunnel".into(),
                    body,
                }
                .encode()?,
            )
            .await?;
        Ok(())
    }
    async fn read(&mut self) -> Result<TunnelFrame> {
        let bytes = tokio::time::timeout(Duration::from_secs(5), self.output.recv())
            .await?
            .context("Tunnel closed")?;
        let TunnelEnvelopeBody::Message(data) = TunnelEnvelope::decode(&bytes)?.body else {
            anyhow::bail!("Expected encrypted tunnel frame")
        };
        let frame = TunnelFrame::decode(&self.session.decrypt(&data)?)?;
        ensure!(frame.sequence == self.received, "Device sequence mismatch");
        self.received += 1;
        Ok(frame)
    }
    async fn echo(&mut self, id: u32, bytes: &[u8]) -> Result<()> {
        self.send(id, TunnelFrameBody::Data(bytes.into())).await?;
        let mut echoed = Vec::new();
        let mut credited = 0;
        while echoed.len() < bytes.len() || credited < bytes.len() {
            let frame = self.read().await?;
            ensure!(frame.stream_id == id, "Unexpected echo stream");
            match frame.body {
                TunnelFrameBody::Data(data) => {
                    self.send(id, TunnelFrameBody::Window(data.len() as u32))
                        .await?;
                    echoed.extend(data);
                }
                TunnelFrameBody::Window(delta) => credited += delta as usize,
                body => anyhow::bail!("Unexpected echo frame: {body:?}"),
            }
        }
        ensure!(echoed == bytes, "Echo was changed");
        Ok(())
    }
}

#[tokio::test]
async fn internal_streams_read_management_and_upload_raw_files_with_resume_and_empty_files()
-> Result<()> {
    use crate::project_artifacts as artifacts;
    let mut fixture = fixture_with_lifetime(1, 3).await?;
    let root = fixture._directory.path();
    let store = StateStore::open(&root.join("management.sqlite"))?;
    let bytes: Vec<u8> = (0..700_000).map(|index| (index % 251) as u8).collect();
    let manifest = ProjectArtifactManifest {
        version: 1,
        project_id: "project".into(),
        source: ProjectArtifactSource::Offline,
        files: vec![
            ProjectArtifactFile {
                path: "apps/project/empty.bin".into(),
                size: 0,
                sha256: artifact_sha256(&[]),
            },
            ProjectArtifactFile {
                path: "apps/project/manifest.app".into(),
                size: bytes.len() as u64,
                sha256: artifact_sha256(&bytes),
            },
        ],
        bit_pins: vec![],
        package_pins: vec![],
    };
    let transfer_id = uuid::Uuid::new_v4().to_string();
    artifacts::begin(
        &store,
        root,
        &transfer_id,
        "owner:owner",
        &manifest.descriptor()?,
    )?;
    let status = fixture
        .client
        .upload(1, &transfer_id, None, 0, &manifest.canonical_bytes()?)
        .await?;
    assert!(status.manifest_ready);
    let status = fixture
        .client
        .upload(3, &transfer_id, Some(0), 0, &[])
        .await?;
    assert!(status.complete);
    // A cancelled upload leaves only persisted bytes for the ordinary status command.
    fixture
        .client
        .open_data(
            5,
            TunnelDataOpen::Artifact {
                project_id: "project".into(),
                transfer_id: transfer_id.clone(),
                file_index: Some(1),
                offset: 0,
            },
        )
        .await?;
    fixture
        .client
        .send(5, TunnelFrameBody::Data(bytes[..MAX_DATA].into()))
        .await?;
    let credited = fixture.client.read().await?;
    assert!(matches!(credited.body, TunnelFrameBody::Window(delta) if delta == MAX_DATA as u32));
    fixture.client.renew(&fixture.signer).await?;
    tokio::time::sleep(Duration::from_secs(4)).await;
    fixture
        .client
        .send(
            5,
            TunnelFrameBody::Data(bytes[MAX_DATA..2 * MAX_DATA].into()),
        )
        .await?;
    assert!(
        matches!(fixture.client.read().await?.body, TunnelFrameBody::Window(delta) if delta == MAX_DATA as u32)
    );
    fixture
        .client
        .send(
            5,
            TunnelFrameBody::Reset(TunnelReset {
                code: "cancelled".into(),
                message: "Resume in another stream".into(),
            }),
        )
        .await?;
    let status = artifacts::status(
        &store,
        root,
        "project",
        &transfer_id,
        "owner:owner",
        Some(1),
    )?;
    assert_eq!(status.offset, 2 * MAX_DATA as u64);
    let status = fixture
        .client
        .upload(
            7,
            &transfer_id,
            Some(1),
            status.offset,
            &bytes[2 * MAX_DATA..],
        )
        .await?;
    assert_eq!(status.offset, bytes.len() as u64);
    assert!(status.complete);
    let committed = artifacts::commit(&store, root, "project", &transfer_id, "owner:owner")?;
    let path = std::path::PathBuf::from(committed.project_path.context("Missing committed path")?);
    assert_eq!(
        std::fs::read(path.join("apps/project/manifest.app"))?,
        bytes
    );
    let now = unix_time()?;
    fixture
        .client
        .open_data(
            9,
            TunnelDataOpen::Request {
                request: ManagementRequest {
                    operation_id: "bulk-inspect".into(),
                    device_id: "device".into(),
                    issued_at: now,
                    expires_at: now + 60,
                    command: ManagementCommand::InspectPage {
                        after: None,
                        limit: 2,
                    },
                },
            },
        )
        .await?;
    fixture.client.send(9, TunnelFrameBody::Fin).await?;
    let response: serde_json::Value =
        serde_json::from_slice(&fixture.client.json_response(9).await?)?;
    assert_eq!(response["operation_id"], "bulk-inspect");
    assert_eq!(response["state"], "completed");
    fixture.cancel.cancel();
    fixture.serving.await??;
    Ok(())
}

struct Fixture {
    _directory: tempfile::TempDir,
    signer: SigningKey,
    client: Client,
    cancel: CancellationToken,
    serving: tokio::task::JoinHandle<Result<()>>,
}

fn certificate(signer: &SigningKey, id: &str, secret: [u8; 32], expires_at: i64) -> Result<String> {
    Ok(sign_controller_certificate(
        &ControllerCertificate {
            version: 1,
            device_id: "device".into(),
            grant_id: "owner".into(),
            session_id: id.into(),
            management_key: x25519_dalek::x25519(secret, x25519_dalek::X25519_BASEPOINT_BYTES),
            issued_at: unix_time()?,
            expires_at,
        },
        signer,
    )?)
}

async fn fixture(port: u16) -> Result<Fixture> {
    fixture_with_lifetime(port, 120).await
}

async fn fixture_with_lifetime(port: u16, lifetime: i64) -> Result<Fixture> {
    let directory = tempfile::tempdir()?;
    crate::supervisor::prepare_state_dir(directory.path())?;
    let signer = SigningKey::generate();
    let device = Arc::new(DeviceSession::test_management_session(
        "http://127.0.0.1:1/api/v1".into(),
        "device".into(),
        SigningKey::generate(),
        signer.public_key(),
    ));
    let service = ManagementService::new(directory.path().into(), device, "boot".into());
    let now = unix_time()?;
    service.refresh_authority(now + 300)?;
    let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
    store.upsert_placement("placement", &serde_json::json!({"id":"placement","project_id":"project","deployment_id":"deployment","revision":"v1","source":"offline","project_path":directory.path(),"events":[],"tunnel_services":[{"id":"database","host":"127.0.0.1","port":port,"protocol":"tcp"}],"hosting":{"host":"127.0.0.1","port":port,"max_in_flight":16,"request_timeout_secs":30,"auth_secret":"service"}}), DesiredState::Running)?;
    store.record_observed("placement", ObservedState::Running, None, None, Some(1))?;
    store.connection.execute("INSERT INTO placement_replicas(placement_id,slot,config_revision,intent_revision,observed_state,applied_revision) VALUES('placement',0,1,1,'running',1)", [])?;
    let cert = certificate(&signer, "tunnel", [7; 32], now + lifetime)?;
    let connection = Connection::new(&service, "tunnel", "owner", &cert)?;
    let (input, incoming) = mpsc::channel(32);
    let (output, mut outgoing) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    let serving = tokio::spawn(connection.serve(incoming, output, cancel.clone()));
    let mut initiator = noise::Handshake::tunnel_initiator(
        &[7; 32],
        x25519_dalek::x25519([42; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
        "device",
        "tunnel",
    )?;
    input
        .send(
            TunnelEnvelope {
                session_id: "tunnel".into(),
                body: TunnelEnvelopeBody::Hello(TunnelHello {
                    grant_id: "owner".into(),
                    certificate_jws: cert,
                    data: wire::encode(&initiator.write()?),
                }),
            }
            .encode()?,
        )
        .await?;
    let answer = tokio::time::timeout(Duration::from_secs(5), outgoing.recv())
        .await?
        .context("No handshake reply")?;
    let TunnelEnvelopeBody::Handshake(answer) = TunnelEnvelope::decode(&answer)?.body else {
        anyhow::bail!("No handshake reply")
    };
    initiator.read(&answer)?;
    input
        .send(
            TunnelEnvelope {
                session_id: "tunnel".into(),
                body: TunnelEnvelopeBody::Handshake(initiator.write()?),
            }
            .encode()?,
        )
        .await?;
    let mut client = Client {
        input,
        output: outgoing,
        session: initiator.finish()?,
        sent: 0,
        received: 0,
    };
    assert!(matches!(
        client.read().await?.body,
        TunnelFrameBody::Renewed(_)
    ));
    Ok(Fixture {
        _directory: directory,
        signer,
        client,
        cancel,
        serving,
    })
}

#[tokio::test]
async fn tunnel_http_mode_authenticates_hosting_and_named_https_listeners() -> Result<()> {
    use sha2::{Digest, Sha256};
    let identity = rcgen::generate_simple_self_signed(vec!["api.localhost".into()])?;
    let fingerprint = format!("{:x}", Sha256::digest(identity.cert.der().as_ref()));
    let config =
        rustls::ServerConfig::builder_with_provider(Arc::new(crate::crypto::tls_provider()))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![identity.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der())
                    .into(),
            )?;
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let request = b"GET /health HTTP/1.1\r\nHost: api.localhost\r\nConnection: close\r\n\r\n";
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    let server = tokio::spawn(async move {
        let mut accepted = 0;
        for _ in 0..4 {
            let (socket, _) = listener.accept().await?;
            if let Ok(mut socket) = acceptor.accept(socket).await {
                let mut bytes = vec![0; request.len()];
                socket.read_exact(&mut bytes).await?;
                ensure!(bytes == request, "Unexpected upstream HTTP request");
                socket.write_all(response).await?;
                socket.shutdown().await?;
                accepted += 1;
            }
        }
        Ok::<_, anyhow::Error>(accepted)
    });
    let mut fixture = fixture(port).await?;
    let root = fixture._directory.path();
    let mut store = StateStore::open(&root.join("management.sqlite"))?;
    let certificate_id = uuid::Uuid::new_v4().to_string();
    crate::certificates::put(
        &mut store,
        root,
        &certificate_id,
        "hosting",
        0,
        &identity.cert.pem(),
        &identity.signing_key.serialize_pem(),
        unix_time()?,
    )?;
    let mut config = store
        .get_placement("placement")?
        .context("Missing placement")?
        .config;
    config["tls_certificate_id"] = certificate_id.into();
    config["tunnel_services"] = serde_json::json!([
        {"id":"api","host":"127.0.0.1","port":port,"protocol":"https","tls_server_name":"api.localhost","tls_sha256_fingerprint":fingerprint},
        {"id":"bad-pin","host":"127.0.0.1","port":port,"protocol":"https","tls_server_name":"api.localhost","tls_sha256_fingerprint":"0".repeat(64)},
        {"id":"bad-name","host":"127.0.0.1","port":port,"protocol":"https","tls_server_name":"other.localhost","tls_sha256_fingerprint":fingerprint}
    ]);
    store.upsert_placement("placement", &config, DesiredState::Running)?;
    store.record_observed("placement", ObservedState::Running, None, None, Some(2))?;
    store.connection.execute("UPDATE placement_replicas SET config_revision=2,applied_revision=2 WHERE placement_id='placement'", [])?;
    for (id, service) in [(1, "hosting"), (3, "api"), (5, "bad-pin"), (7, "bad-name")] {
        fixture
            .client
            .send(
                id,
                TunnelFrameBody::Open(TunnelOpen {
                    placement_id: "placement".into(),
                    service_id: service.into(),
                    mode: flow_like_device_protocol::TunnelMode::Http,
                    target: flow_like_device_protocol::TunnelTarget::Service,
                }),
            )
            .await?;
        let opened = fixture.client.read().await?;
        assert_eq!(opened.stream_id, id);
        if id >= 5 {
            assert!(matches!(opened.body, TunnelFrameBody::Reset(_)));
            continue;
        }
        assert!(matches!(opened.body, TunnelFrameBody::Opened));
        fixture
            .client
            .send(id, TunnelFrameBody::Data(request.to_vec()))
            .await?;
        fixture.client.send(id, TunnelFrameBody::Fin).await?;
        assert_eq!(fixture.client.json_response(id).await?, response);
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), server).await???,
        2
    );
    fixture.cancel.cancel();
    fixture.serving.await??;
    Ok(())
}

#[tokio::test]
async fn tunnel_concurrent_streams_backpressure_half_close_and_live_rekey() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await?;
        let bulk = tokio::spawn(async move {
            first
                .write_all(&vec![7; INITIAL_WINDOW as usize + 10_000])
                .await?;
            first.shutdown().await?;
            let mut tail = Vec::new();
            first.read_to_end(&mut tail).await?;
            Ok::<_, anyhow::Error>(())
        });
        let (mut second, _) = listener.accept().await?;
        let (mut read, mut write) = second.split();
        tokio::io::copy(&mut read, &mut write).await?;
        bulk.await??;
        Ok::<_, anyhow::Error>(())
    });
    let mut fixture = fixture(port).await?;
    let client = &mut fixture.client;
    client
        .send(
            1,
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                service_id: "hosting".into(),
                mode: flow_like_device_protocol::TunnelMode::Tcp,
                target: flow_like_device_protocol::TunnelTarget::Service,
            }),
        )
        .await?;
    assert!(matches!(client.read().await?.body, TunnelFrameBody::Opened));
    let mut bulk = 0;
    while bulk < INITIAL_WINDOW as usize {
        let frame = client.read().await?;
        assert_eq!(frame.stream_id, 1);
        let TunnelFrameBody::Data(data) = frame.body else {
            anyhow::bail!("Expected bulk bytes")
        };
        assert!(data.iter().all(|byte| *byte == 7));
        bulk += data.len();
    }
    assert_eq!(bulk, INITIAL_WINDOW as usize);
    assert!(
        tokio::time::timeout(Duration::from_millis(80), client.read())
            .await
            .is_err()
    );
    client
        .send(
            3,
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                service_id: "database".into(),
                mode: flow_like_device_protocol::TunnelMode::Tcp,
                target: flow_like_device_protocol::TunnelTarget::Service,
            }),
        )
        .await?;
    let opened = client.read().await?;
    assert_eq!(opened.stream_id, 3);
    assert!(matches!(opened.body, TunnelFrameBody::Opened));
    client
        .echo(3, b"second stream progresses while first has no credit")
        .await?;
    client
        .send(1, TunnelFrameBody::Window(INITIAL_WINDOW))
        .await?;
    loop {
        let frame = client.read().await?;
        assert_eq!(frame.stream_id, 1);
        match frame.body {
            TunnelFrameBody::Data(data) => bulk += data.len(),
            TunnelFrameBody::Fin => break,
            body => anyhow::bail!("Unexpected bulk frame: {body:?}"),
        }
    }
    assert_eq!(bulk, INITIAL_WINDOW as usize + 10_000);
    client.send(1, TunnelFrameBody::Fin).await?;
    client.send(1, TunnelFrameBody::Window(10_000)).await?;

    let now = unix_time()?;
    let cert = certificate(&fixture.signer, "renewed", [8; 32], now + 300)?;
    let mut initiator = noise::Handshake::tunnel_initiator(
        &[8; 32],
        x25519_dalek::x25519([42; 32], x25519_dalek::X25519_BASEPOINT_BYTES),
        "device",
        "renewed",
    )?;
    client
        .send(
            0,
            TunnelFrameBody::RenewStart(TunnelRenewStart {
                certificate_jws: cert,
                data: wire::encode(&initiator.write()?),
            }),
        )
        .await?;
    let TunnelFrameBody::RenewReply(reply) = client.read().await?.body else {
        anyhow::bail!("No renewal reply")
    };
    initiator.read(&reply)?;
    // A heartbeat that crosses renewal waits for the new epoch's confirmation.
    client.send(0, TunnelFrameBody::Ping([9; 8])).await?;
    client
        .send(0, TunnelFrameBody::RenewFinish(initiator.write()?))
        .await?;
    client.session = initiator.finish()?;
    assert!(matches!(
        client.read().await?.body,
        TunnelFrameBody::Renewed(_)
    ));
    assert!(matches!(
        client.read().await?.body,
        TunnelFrameBody::Pong([9, 9, 9, 9, 9, 9, 9, 9])
    ));
    client
        .echo(
            3,
            b"same TCP connection after fresh authenticated Noise keys",
        )
        .await?;
    client
        .send(
            5,
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                service_id: "arbitrary-port".into(),
                mode: flow_like_device_protocol::TunnelMode::Tcp,
                target: flow_like_device_protocol::TunnelTarget::Service,
            }),
        )
        .await?;
    assert!(matches!(
        client.read().await?.body,
        TunnelFrameBody::Reset(_)
    ));
    fixture.cancel.cancel();
    fixture.serving.await??;
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    Ok(())
}

#[tokio::test]
async fn tunnel_sequence_loss_closes_every_stream_and_policy_stop_revokes_access() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mut fixture = fixture(listener.local_addr()?.port()).await?;
    fixture
        .client
        .send(
            1,
            TunnelFrameBody::Open(TunnelOpen {
                placement_id: "placement".into(),
                service_id: "hosting".into(),
                mode: flow_like_device_protocol::TunnelMode::Tcp,
                target: flow_like_device_protocol::TunnelTarget::Service,
            }),
        )
        .await?;
    let (mut socket, _) = listener.accept().await?;
    assert!(matches!(
        fixture.client.read().await?.body,
        TunnelFrameBody::Opened
    ));
    let store = StateStore::open(&fixture._directory.path().join("management.sqlite"))?;
    store.connection.execute(
        "UPDATE placements SET desired_state='stopped' WHERE id='placement'",
        [],
    )?;
    assert!(matches!(
        fixture.client.read().await?.body,
        TunnelFrameBody::Reset(_)
    ));
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte)).await??,
        0
    );
    fixture.client.sent += 1;
    fixture
        .client
        .send(0, TunnelFrameBody::Ping([1; 8]))
        .await?;
    assert!(fixture.serving.await?.is_err());
    assert!(fixture.client.output.recv().await.is_none());
    Ok(())
}

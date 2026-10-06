use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_crypto::noise;
use flow_like_device_protocol::{
    Ed25519PublicKey, TunnelFrame, TunnelFrameBody, TunnelRenewed, verify_controller_certificate,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
};

struct Peer(Child);
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn receive(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    assert_ne!(
        reader.read_line(&mut line).unwrap(),
        0,
        "Browser peer closed"
    );
    serde_json::from_str(&line).unwrap()
}

fn send(writer: &mut impl Write, value: Value) {
    writeln!(writer, "{value}").unwrap();
    writer.flush().unwrap();
}

fn bytes(value: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(value["data"].as_str().unwrap())
        .unwrap()
}

#[test]
#[ignore = "requires node and generated browser WASM from build-web.mjs"]
fn browser_and_native_rotate_tunnel_keys_without_resetting_stream_sequences() {
    let mut peer = Peer(
        Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/tunnel-peer.mjs"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut reader = BufReader::new(peer.0.stdout.take().unwrap());
    let mut writer = peer.0.stdin.take().unwrap();
    let controller_key: Ed25519PublicKey = serde_json::from_value(receive(&mut reader)).unwrap();
    let secret = [42; 32];
    let public = x25519_dalek::x25519(secret, x25519_dalek::X25519_BASEPOINT_BYTES);
    send(&mut writer, json!({ "management_key": public }));

    let hello = receive(&mut reader);
    let certificate =
        verify_controller_certificate(hello["certificate"].as_str().unwrap(), &controller_key, 100)
            .unwrap();
    let mut handshake = noise::Handshake::tunnel_responder(
        &secret,
        certificate.management_key,
        "device-tunnel",
        &certificate.session_id,
    )
    .unwrap();
    handshake.read(&bytes(&hello)).unwrap();
    send(
        &mut writer,
        json!({ "data": URL_SAFE_NO_PAD.encode(handshake.write().unwrap()) }),
    );
    handshake.read(&bytes(&receive(&mut reader))).unwrap();
    let mut session = handshake.finish().unwrap();
    let first = session.decrypt(&bytes(&receive(&mut reader))).unwrap();
    let frame = TunnelFrame::decode(&first).unwrap();
    assert_eq!(frame.sequence, 0);
    assert_eq!(frame.stream_id, 1);
    assert!(matches!(frame.body, TunnelFrameBody::Data(data) if data == b"epoch one"));
    send(
        &mut writer,
        json!({ "data": URL_SAFE_NO_PAD.encode(session.encrypt(&first).unwrap()) }),
    );

    let renewal = session.decrypt(&bytes(&receive(&mut reader))).unwrap();
    let frame = TunnelFrame::decode(&renewal).unwrap();
    assert_eq!(frame.sequence, 1);
    let TunnelFrameBody::RenewStart(start) = frame.body else {
        panic!("Expected renewal")
    };
    let renewed =
        verify_controller_certificate(&start.certificate_jws, &controller_key, 340).unwrap();
    assert_ne!(renewed.session_id, certificate.session_id);
    assert_ne!(renewed.management_key, certificate.management_key);
    let mut handshake = noise::Handshake::tunnel_responder(
        &secret,
        renewed.management_key,
        "device-tunnel",
        &renewed.session_id,
    )
    .unwrap();
    handshake
        .read(&URL_SAFE_NO_PAD.decode(start.data).unwrap())
        .unwrap();
    let reply = TunnelFrame {
        sequence: 1,
        stream_id: 0,
        body: TunnelFrameBody::RenewReply(handshake.write().unwrap()),
    }
    .encode()
    .unwrap();
    send(
        &mut writer,
        json!({ "data": URL_SAFE_NO_PAD.encode(session.encrypt(&reply).unwrap()) }),
    );
    let finish = session.decrypt(&bytes(&receive(&mut reader))).unwrap();
    let frame = TunnelFrame::decode(&finish).unwrap();
    assert_eq!(frame.sequence, 2);
    let TunnelFrameBody::RenewFinish(finish) = frame.body else {
        panic!("Expected renewal finish")
    };
    handshake.read(&finish).unwrap();
    let mut session = handshake.finish().unwrap();
    let ready = TunnelFrame {
        sequence: 2,
        stream_id: 0,
        body: TunnelFrameBody::Renewed(TunnelRenewed {
            expires_at: renewed.expires_at,
        }),
    }
    .encode()
    .unwrap();
    send(
        &mut writer,
        json!({ "data": URL_SAFE_NO_PAD.encode(session.encrypt(&ready).unwrap()) }),
    );
    let after = session.decrypt(&bytes(&receive(&mut reader))).unwrap();
    let frame = TunnelFrame::decode(&after).unwrap();
    assert_eq!(frame.sequence, 3);
    assert_eq!(frame.stream_id, 1);
    let TunnelFrameBody::Data(data) = frame.body else {
        panic!("Expected service data")
    };
    assert_eq!(data.len(), flow_like_device_protocol::TUNNEL_MAX_DATA);
    send(
        &mut writer,
        json!({ "data": URL_SAFE_NO_PAD.encode(session.encrypt(&after).unwrap()) }),
    );

    let _lost = receive(&mut reader);
    let next = bytes(&receive(&mut reader));
    assert!(session.decrypt(&next).is_err());
    assert!(matches!(
        session.decrypt(&next),
        Err(flow_like_device_crypto::CryptoError::SessionUnavailable)
    ));
    send(&mut writer, json!({ "closed": true }));
    drop(writer);
    assert!(peer.0.wait().unwrap().success());
}

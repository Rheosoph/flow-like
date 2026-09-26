use super::{CryptoError, Result};

const PATTERN: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const MAX_HANDSHAKE: usize = 128;
pub const MAX_PLAINTEXT: usize = 16 * 1024;
const TAG_LEN: usize = 16;

/// One ordered, reliable transport connection. A reconnect needs a fresh handshake.
/// Trusted keys must come from verified grants, never from signaling discovery.
pub struct Handshake {
    state: Option<snow::HandshakeState>,
    expected_peer: [u8; 32],
}

pub struct Session {
    state: Option<snow::TransportState>,
}

impl Handshake {
    pub fn initiator(
        private_key: &[u8; 32],
        expected_peer: [u8; 32],
        device_id: &str,
        grant_id: &str,
    ) -> Result<Self> {
        Self::new(private_key, expected_peer, device_id, grant_id, true)
    }

    pub fn responder(
        private_key: &[u8; 32],
        expected_peer: [u8; 32],
        device_id: &str,
        grant_id: &str,
    ) -> Result<Self> {
        Self::new(private_key, expected_peer, device_id, grant_id, false)
    }

    fn new(
        private_key: &[u8; 32],
        expected_peer: [u8; 32],
        device_id: &str,
        grant_id: &str,
        initiator: bool,
    ) -> Result<Self> {
        if x25519_dalek::x25519([42; 32], expected_peer) == [0; 32] {
            return Err(CryptoError::InvalidInput("low-order Noise peer key"));
        }
        let mut prologue = b"flow-like/standalone/control/v1".to_vec();
        for field in [device_id, grant_id] {
            if field.is_empty() || field.len() > 256 {
                return Err(CryptoError::InvalidInput("session scope"));
            }
            prologue.extend_from_slice(&(field.len() as u16).to_be_bytes());
            prologue.extend_from_slice(field.as_bytes());
        }
        let params = PATTERN
            .parse()
            .map_err(|_| CryptoError::Operation("Noise parameters"))?;
        let builder = snow::Builder::new(params)
            .local_private_key(private_key)
            .prologue(&prologue);
        let state = if initiator {
            builder.build_initiator()
        } else {
            builder.build_responder()
        }
        .map_err(|_| CryptoError::Operation("Noise initialization"))?;
        Ok(Self {
            state: Some(state),
            expected_peer,
        })
    }

    pub fn write(&mut self) -> Result<Vec<u8>> {
        let mut state = self.state.take().ok_or(CryptoError::SessionUnavailable)?;
        let mut message = vec![0; MAX_HANDSHAKE];
        let len = state
            .write_message(&[], &mut message)
            .map_err(|_| CryptoError::Operation("Noise handshake write"))?;
        message.truncate(len);
        self.state = Some(state);
        Ok(message)
    }

    pub fn read(&mut self, message: &[u8]) -> Result<()> {
        let mut state = self.state.take().ok_or(CryptoError::SessionUnavailable)?;
        if message.len() > MAX_HANDSHAKE {
            return Err(CryptoError::InvalidInput("handshake size"));
        }
        let mut payload = [0; MAX_HANDSHAKE];
        let len = state
            .read_message(message, &mut payload)
            .map_err(|_| CryptoError::Operation("Noise handshake read"))?;
        if len != 0 {
            return Err(CryptoError::InvalidInput("unexpected handshake payload"));
        }
        if let Some(remote) = state.get_remote_static()
            && remote != self.expected_peer
        {
            return Err(CryptoError::UntrustedPeer);
        }
        self.state = Some(state);
        Ok(())
    }

    pub fn finish(mut self) -> Result<Session> {
        let state = self.state.take().ok_or(CryptoError::SessionUnavailable)?;
        if !state.is_handshake_finished() {
            return Err(CryptoError::SessionUnavailable);
        }
        if state.get_remote_static() != Some(self.expected_peer.as_slice()) {
            return Err(CryptoError::UntrustedPeer);
        }
        let state = state
            .into_transport_mode()
            .map_err(|_| CryptoError::Operation("Noise transport initialization"))?;
        Ok(Session { state: Some(state) })
    }
}

impl Session {
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(CryptoError::InvalidInput("plaintext size"));
        }
        let mut state = self.state.take().ok_or(CryptoError::SessionUnavailable)?;
        let mut ciphertext = vec![0; plaintext.len() + TAG_LEN];
        let len = state
            .write_message(plaintext, &mut ciphertext)
            .map_err(|_| CryptoError::Operation("Noise transport write"))?;
        ciphertext.truncate(len);
        self.state = Some(state);
        Ok(ciphertext)
    }

    /// Authentication failures close the session. Retrying requires a fresh handshake.
    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut state = self.state.take().ok_or(CryptoError::SessionUnavailable)?;
        if !(TAG_LEN..=MAX_PLAINTEXT + TAG_LEN).contains(&ciphertext.len()) {
            return Err(CryptoError::InvalidInput("ciphertext size"));
        }
        let mut plaintext = vec![0; ciphertext.len() - TAG_LEN];
        let len = state
            .read_message(ciphertext, &mut plaintext)
            .map_err(|_| CryptoError::Operation("Noise transport read"))?;
        plaintext.truncate(len);
        self.state = Some(state);
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair() -> ([u8; 32], [u8; 32]) {
        let pair = snow::Builder::new(PATTERN.parse().unwrap())
            .generate_keypair()
            .unwrap();
        (
            pair.private.try_into().unwrap(),
            pair.public.try_into().unwrap(),
        )
    }

    fn handshakes() -> (Handshake, Handshake) {
        let (alice_secret, alice_public) = keypair();
        let (bob_secret, bob_public) = keypair();
        (
            Handshake::initiator(&alice_secret, bob_public, "device", "grant").unwrap(),
            Handshake::responder(&bob_secret, alice_public, "device", "grant").unwrap(),
        )
    }

    fn connect() -> (Session, Session) {
        let (mut alice, mut bob) = handshakes();
        bob.read(&alice.write().unwrap()).unwrap();
        alice.read(&bob.write().unwrap()).unwrap();
        bob.read(&alice.write().unwrap()).unwrap();
        (alice.finish().unwrap(), bob.finish().unwrap())
    }

    #[test]
    fn bidirectional_encryption_and_replay_rejection() {
        let (mut alice, mut bob) = connect();
        let request = alice.encrypt(b"restart placement").unwrap();
        assert_eq!(bob.decrypt(&request).unwrap(), b"restart placement");
        assert_eq!(
            alice.decrypt(&bob.encrypt(b"accepted").unwrap()).unwrap(),
            b"accepted"
        );
        assert!(bob.decrypt(&request).is_err());
        assert!(matches!(
            bob.decrypt(&request),
            Err(CryptoError::SessionUnavailable)
        ));
    }

    #[test]
    fn wrong_peer_is_rejected_before_transport() {
        let (mut alice, mut bob) = handshakes();
        alice.expected_peer = keypair().1;
        bob.read(&alice.write().unwrap()).unwrap();
        assert!(matches!(
            alice.read(&bob.write().unwrap()),
            Err(CryptoError::UntrustedPeer)
        ));
        assert!(alice.finish().is_err());
    }

    #[test]
    fn scope_mismatch_and_tampering_fail_closed() {
        let (alice_secret, alice_public) = keypair();
        let (bob_secret, bob_public) = keypair();
        let mut alice =
            Handshake::initiator(&alice_secret, bob_public, "device-a", "grant").unwrap();
        let mut bob = Handshake::responder(&bob_secret, alice_public, "device-b", "grant").unwrap();
        bob.read(&alice.write().unwrap()).unwrap();
        assert!(alice.read(&bob.write().unwrap()).is_err());
        let (mut alice, mut bob) = connect();
        let mut ciphertext = alice.encrypt(b"logs").unwrap();
        ciphertext[0] ^= 1;
        assert!(bob.decrypt(&ciphertext).is_err());
        assert!(bob.decrypt(&alice.encrypt(b"next").unwrap()).is_err());
    }

    #[test]
    fn incomplete_handshake_and_oversized_messages_are_rejected() {
        let (alice, _) = handshakes();
        assert!(alice.finish().is_err());
        let (mut alice, _) = connect();
        assert!(alice.encrypt(&vec![0; MAX_PLAINTEXT + 1]).is_err());
    }

    #[test]
    fn low_order_peer_keys_are_rejected_before_handshake() {
        let (secret, _) = keypair();
        let mut low_order = [0; 32];
        assert!(Handshake::initiator(&secret, low_order, "device", "grant").is_err());
        low_order[0] = 1;
        assert!(Handshake::responder(&secret, low_order, "device", "grant").is_err());
    }

    #[test]
    fn old_connection_ciphertext_is_not_valid_in_a_new_session() {
        let (alice_secret, alice_public) = keypair();
        let (bob_secret, bob_public) = keypair();
        let connect_same_keys = || {
            let mut alice =
                Handshake::initiator(&alice_secret, bob_public, "device", "grant").unwrap();
            let mut bob =
                Handshake::responder(&bob_secret, alice_public, "device", "grant").unwrap();
            bob.read(&alice.write().unwrap()).unwrap();
            alice.read(&bob.write().unwrap()).unwrap();
            bob.read(&alice.write().unwrap()).unwrap();
            (alice.finish().unwrap(), bob.finish().unwrap())
        };
        let (mut old_alice, _) = connect_same_keys();
        let ciphertext = old_alice.encrypt(b"previous command").unwrap();
        let (_, mut bob) = connect_same_keys();
        assert!(bob.decrypt(&ciphertext).is_err());
    }
}

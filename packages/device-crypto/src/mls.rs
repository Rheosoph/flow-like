use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_traits::OpenMlsProvider;
use tls_codec::Deserialize as _;

use super::{CryptoError, Result};

const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519;
const MAX_WIRE_SIZE: usize = 1024 * 1024;
const MAX_APPLICATION_SIZE: usize = 64 * 1024;

/// A key pinned by the caller's verified membership policy. Account names alone
/// do not authenticate an MLS credential.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberIdentity {
    id: Vec<u8>,
    signature_key: [u8; 32],
}

impl MemberIdentity {
    pub fn new(id: Vec<u8>, signature_key: [u8; 32]) -> Result<Self> {
        if id.is_empty() || id.len() > 256 {
            return Err(CryptoError::InvalidInput("member identity"));
        }
        flow_like_device_protocol::Ed25519PublicKey::from_bytes(signature_key)
            .map_err(|_| CryptoError::InvalidInput("member signature key"))?;
        Ok(Self { id, signature_key })
    }

    pub fn id(&self) -> &[u8] {
        &self.id
    }

    pub fn signature_key(&self) -> &[u8; 32] {
        &self.signature_key
    }

    fn matches(&self, credential: &Credential, key: &[u8]) -> bool {
        credential.credential_type() == CredentialType::Basic
            && credential.serialized_content() == self.id
            && key == self.signature_key
    }

    fn matches_leaf(&self, leaf: &LeafNode) -> bool {
        self.matches(leaf.credential(), leaf.signature_key().as_slice())
    }
}

pub struct Identity {
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    public: MemberIdentity,
}

impl Identity {
    pub fn from_signing_key(
        provider: &impl OpenMlsProvider,
        id: Vec<u8>,
        key: &flow_like_device_protocol::SigningKey,
    ) -> Result<Self> {
        let public = MemberIdentity::new(
            id.clone(),
            key.public_key()
                .to_bytes()
                .map_err(|_| CryptoError::InvalidInput("member signature key"))?,
        )?;
        let private = zeroize::Zeroizing::new(key.to_bytes());
        let signer = SignatureKeyPair::from_raw(
            CIPHERSUITE.signature_algorithm(),
            private.to_vec(),
            public.signature_key.to_vec(),
        );
        signer
            .store(provider.storage())
            .map_err(|_| CryptoError::Operation("MLS signing key storage"))?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(id).into(),
            signature_key: signer.public().into(),
        };
        Ok(Self {
            signer,
            credential,
            public,
        })
    }

    pub fn generate(provider: &impl OpenMlsProvider, id: Vec<u8>) -> Result<Self> {
        if id.is_empty() || id.len() > 256 {
            return Err(CryptoError::InvalidInput("member identity"));
        }
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(|_| CryptoError::Operation("MLS signing key generation"))?;
        signer
            .store(provider.storage())
            .map_err(|_| CryptoError::Operation("MLS signing key storage"))?;
        let public = MemberIdentity::new(
            id.clone(),
            signer
                .public()
                .try_into()
                .map_err(|_| CryptoError::Operation("MLS signing key size"))?,
        )?;
        let credential = CredentialWithKey {
            credential: BasicCredential::new(id).into(),
            signature_key: signer.public().into(),
        };
        Ok(Self {
            signer,
            credential,
            public,
        })
    }

    pub fn public(&self) -> &MemberIdentity {
        &self.public
    }

    pub fn load(provider: &impl OpenMlsProvider, public: MemberIdentity) -> Result<Self> {
        let signer = SignatureKeyPair::read(
            provider.storage(),
            public.signature_key(),
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or(CryptoError::Operation("MLS signing key restoration"))?;
        if signer.public() != public.signature_key() {
            return Err(CryptoError::UntrustedPeer);
        }
        let credential = CredentialWithKey {
            credential: BasicCredential::new(public.id.clone()).into(),
            signature_key: signer.public().into(),
        };
        Ok(Self {
            signer,
            credential,
            public,
        })
    }

    /// The provider retains the private bundle. Publish only these public bytes.
    pub fn key_package(&self, provider: &impl OpenMlsProvider) -> Result<Vec<u8>> {
        let bundle = KeyPackage::builder()
            .build(CIPHERSUITE, provider, &self.signer, self.credential.clone())
            .map_err(|_| CryptoError::Operation("MLS key package generation"))?;
        encode(bundle.key_package())
    }
}

/// A device-authored telemetry group. Readers cannot author accepted telemetry or
/// membership changes. The caller validates signed grants before approving keys.
/// Use a protected persistent provider in the agent; memory storage is for tests.
/// Keep exactly one active wrapper per stored group to avoid ratchet reuse.
pub struct TelemetryGroup {
    group: MlsGroup,
    publisher: MemberIdentity,
}

pub struct AddMessages {
    pub commit: Vec<u8>,
    pub welcome: Vec<u8>,
}

pub struct MembershipMessages {
    pub commit: Vec<u8>,
    pub welcome: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    Application(Vec<u8>),
    EpochChanged(u64),
}

impl TelemetryGroup {
    pub fn roster(&self) -> Result<Vec<MemberIdentity>> {
        self.group
            .members()
            .map(|member| {
                if member.credential.credential_type() != CredentialType::Basic {
                    return Err(CryptoError::UntrustedPeer);
                }
                MemberIdentity::new(
                    member.credential.serialized_content().to_vec(),
                    member
                        .signature_key
                        .as_slice()
                        .try_into()
                        .map_err(|_| CryptoError::UntrustedPeer)?,
                )
            })
            .collect()
    }

    pub fn is_active(&self) -> bool {
        self.group.is_active()
    }

    /// Build one ordered commit for the complete approved roster. The caller
    /// persists the resulting state and messages within its storage transaction.
    pub fn prepare_membership(
        &mut self,
        provider: &impl OpenMlsProvider,
        identity: &Identity,
        approved: &[MemberIdentity],
        packages: &[(MemberIdentity, Vec<u8>)],
    ) -> Result<MembershipMessages> {
        self.require_publisher(identity)?;
        if !approved.contains(&self.publisher)
            || approved.is_empty()
            || approved.len() > flow_like_device_protocol::MAX_TELEMETRY_MEMBERS
        {
            return Err(CryptoError::UntrustedPeer);
        }
        for (index, member) in approved.iter().enumerate() {
            MemberIdentity::new(member.id.clone(), member.signature_key)?;
            if approved[..index]
                .iter()
                .any(|other| other.id == member.id || other.signature_key == member.signature_key)
            {
                return Err(CryptoError::InvalidInput("duplicate MLS policy member"));
            }
        }
        let current = self.roster()?;
        let added: Vec<_> = approved
            .iter()
            .filter(|member| !current.contains(member))
            .collect();
        if packages.len() != added.len() {
            return Err(CryptoError::InvalidInput(
                "MLS admission packages do not match policy",
            ));
        }
        let mut parsed = Vec::with_capacity(added.len());
        for member in added {
            let matching: Vec<_> = packages
                .iter()
                .filter(|(identity, _)| identity == member)
                .collect();
            if matching.len() != 1 {
                return Err(CryptoError::InvalidInput("MLS admission package identity"));
            }
            let bytes = &matching[0].1;
            check_wire(bytes)?;
            let package = KeyPackageIn::tls_deserialize_exact(bytes)
                .map_err(|_| CryptoError::InvalidInput("MLS key package encoding"))?
                .validate(provider.crypto(), ProtocolVersion::Mls10)
                .map_err(|_| CryptoError::Operation("MLS key package verification"))?;
            if !member.matches_leaf(package.leaf_node()) {
                return Err(CryptoError::UntrustedPeer);
            }
            parsed.push(package);
        }
        let removed: Vec<_> = self
            .group
            .members()
            .filter(|member| {
                !approved
                    .iter()
                    .any(|identity| identity.matches(&member.credential, &member.signature_key))
            })
            .map(|member| member.index)
            .collect();
        let bundle = self
            .group
            .commit_builder()
            .consume_proposal_store(false)
            .propose_adds(parsed)
            .propose_removals(removed)
            .force_self_update(true)
            .load_psks(provider.storage())
            .map_err(|_| CryptoError::Operation("MLS membership preparation"))?
            .build(provider.rand(), provider.crypto(), &identity.signer, |_| {
                true
            })
            .map_err(|_| CryptoError::Operation("MLS membership commit"))?
            .stage_commit(provider)
            .map_err(|_| CryptoError::Operation("MLS membership staging"))?;
        let (commit, welcome, _) = bundle.into_messages();
        Ok(MembershipMessages {
            commit: encode(&commit)?,
            welcome: welcome.as_ref().map(encode).transpose()?,
        })
    }

    /// Remove an inactive leaf's state before a fresh admission under a new grant.
    /// Active state must never be deleted to bypass ordered membership changes.
    pub fn delete_retired(provider: &impl OpenMlsProvider, scope: &str) -> Result<()> {
        let Some(mut group) = MlsGroup::load(provider.storage(), &group_id(scope)?)
            .map_err(|_| CryptoError::Operation("MLS retired group lookup"))?
        else {
            return Ok(());
        };
        if group.is_active() {
            return Err(CryptoError::InvalidInput("MLS group is still active"));
        }
        group
            .delete(provider.storage())
            .map_err(|_| CryptoError::Operation("MLS retired group deletion"))
    }

    pub fn load(
        provider: &impl OpenMlsProvider,
        scope: &str,
        publisher: MemberIdentity,
        approved_members: &[MemberIdentity],
    ) -> Result<Self> {
        let group = MlsGroup::load(provider.storage(), &group_id(scope)?)
            .map_err(|_| CryptoError::Operation("MLS group restoration"))?
            .ok_or(CryptoError::SessionUnavailable)?;
        if group.ciphersuite() != CIPHERSUITE || !group.is_active() {
            return Err(CryptoError::SessionUnavailable);
        }
        let mut publisher_present = false;
        for member in group.members() {
            publisher_present |= publisher.matches(&member.credential, &member.signature_key);
            if !approved_members
                .iter()
                .any(|approved| approved.matches(&member.credential, &member.signature_key))
            {
                return Err(CryptoError::UntrustedPeer);
            }
        }
        if !publisher_present {
            return Err(CryptoError::UntrustedPeer);
        }
        Ok(Self { group, publisher })
    }

    pub fn create(
        provider: &impl OpenMlsProvider,
        identity: &Identity,
        scope: &str,
    ) -> Result<Self> {
        let id = group_id(scope)?;
        if MlsGroup::load(provider.storage(), &id)
            .map_err(|_| CryptoError::Operation("MLS existing group lookup"))?
            .is_some()
        {
            return Err(CryptoError::InvalidInput("MLS group already exists"));
        }
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .use_ratchet_tree_extension(true)
            .max_past_epochs(0)
            .build();
        let group = MlsGroup::new_with_group_id(
            provider,
            &identity.signer,
            &config,
            id,
            identity.credential.clone(),
        )
        .map_err(|_| CryptoError::Operation("MLS group creation"))?;
        Ok(Self {
            group,
            publisher: identity.public.clone(),
        })
    }

    /// Verify the signed invitation envelope before calling this method. OpenMLS
    /// consumes the one-use key package while staging a welcome, including a
    /// welcome rejected by the subsequent application identity checks.
    pub fn join(
        provider: &impl OpenMlsProvider,
        scope: &str,
        welcome: &[u8],
        publisher: MemberIdentity,
        approved_members: &[MemberIdentity],
    ) -> Result<Self> {
        let expected_group_id = group_id(scope)?;
        if MlsGroup::load(provider.storage(), &expected_group_id)
            .map_err(|_| CryptoError::Operation("MLS existing group lookup"))?
            .is_some()
        {
            return Err(CryptoError::InvalidInput("MLS group already exists"));
        }
        let message = decode(welcome)?;
        let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
            return Err(CryptoError::InvalidInput("expected MLS welcome"));
        };
        let config = MlsGroupJoinConfig::builder().max_past_epochs(0).build();
        let staged = StagedWelcome::new_from_welcome(provider, &config, welcome, None)
            .map_err(|_| CryptoError::Operation("MLS welcome verification"))?;
        if staged.group_context().group_id() != &expected_group_id {
            return Err(CryptoError::WrongScope);
        }
        if staged.group_context().ciphersuite() != CIPHERSUITE {
            return Err(CryptoError::InvalidInput("MLS ciphersuite"));
        }
        if !publisher.matches_leaf(
            staged
                .welcome_sender()
                .map_err(|_| CryptoError::Operation("MLS welcome signer"))?,
        ) {
            return Err(CryptoError::UntrustedPeer);
        }
        for member in staged.members() {
            if !approved_members
                .iter()
                .any(|approved| approved.matches(&member.credential, &member.signature_key))
            {
                return Err(CryptoError::UntrustedPeer);
            }
        }
        let group = staged
            .into_group(provider)
            .map_err(|_| CryptoError::Operation("MLS welcome installation"))?;
        Ok(Self { group, publisher })
    }

    /// Keep the commit pending until the caller durably records its ordered outbox.
    pub fn prepare_add(
        &mut self,
        provider: &impl OpenMlsProvider,
        identity: &Identity,
        public_package: &[u8],
        approved_member: &MemberIdentity,
    ) -> Result<AddMessages> {
        self.require_publisher(identity)?;
        check_wire(public_package)?;
        let package = KeyPackageIn::tls_deserialize_exact(public_package)
            .map_err(|_| CryptoError::InvalidInput("MLS key package encoding"))?
            .validate(provider.crypto(), ProtocolVersion::Mls10)
            .map_err(|_| CryptoError::Operation("MLS key package verification"))?;
        if !approved_member.matches_leaf(package.leaf_node()) {
            return Err(CryptoError::UntrustedPeer);
        }
        if self
            .group
            .members()
            .any(|member| member.credential.serialized_content() == approved_member.id)
        {
            return Err(CryptoError::InvalidInput("duplicate MLS member identity"));
        }
        let (commit, welcome, _) = self
            .group
            .add_members(provider, &identity.signer, &[package])
            .map_err(|_| CryptoError::Operation("MLS add member"))?;
        let encoded = (|| {
            Ok(AddMessages {
                commit: encode(&commit)?,
                welcome: encode(&welcome)?,
            })
        })();
        if encoded.is_err() {
            self.group
                .clear_pending_commit(provider.storage())
                .map_err(|_| CryptoError::Operation("MLS failed commit cleanup"))?;
        }
        encoded
    }

    pub fn prepare_remove(
        &mut self,
        provider: &impl OpenMlsProvider,
        identity: &Identity,
        member: &MemberIdentity,
    ) -> Result<Vec<u8>> {
        self.require_publisher(identity)?;
        if member == &self.publisher {
            return Err(CryptoError::InvalidInput("publisher removal"));
        }
        let index = self
            .group
            .members()
            .find(|current| member.matches(&current.credential, &current.signature_key))
            .ok_or(CryptoError::UntrustedPeer)?
            .index;
        let (commit, _, _) = self
            .group
            .remove_members(provider, &identity.signer, &[index])
            .map_err(|_| CryptoError::Operation("MLS remove member"))?;
        let encoded = encode(&commit);
        if encoded.is_err() {
            self.group
                .clear_pending_commit(provider.storage())
                .map_err(|_| CryptoError::Operation("MLS failed commit cleanup"))?;
        }
        encoded
    }

    pub fn merge_pending(&mut self, provider: &impl OpenMlsProvider) -> Result<()> {
        self.group
            .merge_pending_commit(provider)
            .map_err(|_| CryptoError::Operation("MLS pending commit installation"))
    }

    pub fn encrypt(
        &mut self,
        provider: &impl OpenMlsProvider,
        identity: &Identity,
        plaintext: &[u8],
        approved_members: &[MemberIdentity],
    ) -> Result<Vec<u8>> {
        self.require_publisher(identity)?;
        self.validate_current_roster(approved_members)?;
        if plaintext.len() > MAX_APPLICATION_SIZE {
            return Err(CryptoError::InvalidInput("MLS application size"));
        }
        let message = self
            .group
            .create_message(provider, &identity.signer, plaintext)
            .map_err(|_| CryptoError::Operation("MLS application encryption"))?;
        encode(&message)
    }

    /// The approved list must come from the current verified membership policy.
    /// Fetch and validate the associated policy envelope before calling this
    /// method. OpenMLS consumes receive ratchets during processing, so retrying
    /// the same ciphertext after refreshing a missing policy is unsafe. Queue
    /// messages whose policy cannot yet be verified outside this primitive.
    /// Provider writes and delivery acknowledgments require a durable transaction
    /// boundary in the eventual transport adapter.
    pub fn process(
        &mut self,
        provider: &impl OpenMlsProvider,
        wire: &[u8],
        approved_members: &[MemberIdentity],
    ) -> Result<Received> {
        let protocol_message = decode(wire)?
            .try_into_protocol_message()
            .map_err(|_| CryptoError::InvalidInput("expected MLS protocol message"))?;
        let processed = self
            .group
            .process_message(provider, protocol_message)
            .map_err(|_| CryptoError::Operation("MLS message verification"))?;
        let Sender::Member(index) = processed.sender() else {
            return Err(CryptoError::UntrustedPeer);
        };
        let sender_index = *index;
        let sender = self
            .group
            .member_at(*index)
            .ok_or(CryptoError::UntrustedPeer)?;
        if !self
            .publisher
            .matches(&sender.credential, &sender.signature_key)
        {
            return Err(CryptoError::UntrustedPeer);
        }
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(message) => {
                self.validate_current_roster(approved_members)?;
                let plaintext = message.into_bytes();
                if plaintext.len() > MAX_APPLICATION_SIZE {
                    return Err(CryptoError::InvalidInput("MLS application size"));
                }
                Ok(Received::Application(plaintext))
            }
            ProcessedMessageContent::StagedCommitMessage(commit) => {
                for addition in commit.add_proposals() {
                    approve_leaf(
                        addition.add_proposal().key_package().leaf_node(),
                        approved_members,
                    )?;
                }
                for update in commit.update_proposals() {
                    approve_leaf(update.update_proposal().leaf_node(), approved_members)?;
                }
                if let Some(leaf) = commit.update_path_leaf_node() {
                    approve_leaf(leaf, approved_members)?;
                    if !self.publisher.matches_leaf(leaf) {
                        return Err(CryptoError::UntrustedPeer);
                    }
                }
                for member in self.group.members() {
                    let removed = commit
                        .remove_proposals()
                        .any(|proposal| proposal.remove_proposal().removed() == member.index);
                    if removed {
                        if member.index == sender_index {
                            return Err(CryptoError::UntrustedPeer);
                        }
                        continue;
                    }
                    let updated = commit.update_proposals().any(|proposal| {
                        matches!(proposal.sender(), Sender::Member(index) if *index == member.index)
                    });
                    if updated
                        || (member.index == sender_index
                            && commit.update_path_leaf_node().is_some())
                    {
                        continue;
                    }
                    if !approved_members
                        .iter()
                        .any(|approved| approved.matches(&member.credential, &member.signature_key))
                    {
                        return Err(CryptoError::UntrustedPeer);
                    }
                }
                self.group
                    .merge_staged_commit(provider, *commit)
                    .map_err(|_| CryptoError::Operation("MLS commit installation"))?;
                Ok(Received::EpochChanged(self.group.epoch().as_u64()))
            }
            _ => Err(CryptoError::InvalidInput("unsupported MLS proposal")),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.group.epoch().as_u64()
    }

    fn require_publisher(&self, identity: &Identity) -> Result<()> {
        if identity.public != self.publisher {
            return Err(CryptoError::UntrustedPeer);
        }
        Ok(())
    }

    fn validate_current_roster(&self, approved: &[MemberIdentity]) -> Result<()> {
        for member in self.group.members() {
            if !approved
                .iter()
                .any(|identity| identity.matches(&member.credential, &member.signature_key))
            {
                return Err(CryptoError::UntrustedPeer);
            }
        }
        Ok(())
    }
}

fn approve_leaf(leaf: &LeafNode, approved: &[MemberIdentity]) -> Result<()> {
    if approved.iter().any(|identity| identity.matches_leaf(leaf)) {
        return Ok(());
    }
    Err(CryptoError::UntrustedPeer)
}

fn group_id(scope: &str) -> Result<GroupId> {
    if scope.is_empty() || scope.len() > 256 {
        return Err(CryptoError::InvalidInput("MLS group scope"));
    }
    let mut id = b"flow-like/standalone/telemetry/v1/".to_vec();
    id.extend_from_slice(scope.as_bytes());
    Ok(GroupId::from_slice(&id))
}

fn encode(value: &impl tls_codec::Serialize) -> Result<Vec<u8>> {
    let bytes = value
        .tls_serialize_detached()
        .map_err(|_| CryptoError::Operation("MLS serialization"))?;
    check_wire(&bytes)?;
    Ok(bytes)
}

fn decode(wire: &[u8]) -> Result<MlsMessageIn> {
    check_wire(wire)?;
    MlsMessageIn::tls_deserialize_exact(wire)
        .map_err(|_| CryptoError::InvalidInput("MLS message encoding"))
}

fn check_wire(wire: &[u8]) -> Result<()> {
    if wire.is_empty() || wire.len() > MAX_WIRE_SIZE {
        return Err(CryptoError::InvalidInput("MLS wire size"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmls_rust_crypto::OpenMlsRustCrypto;

    const SCOPE: &str = "device/placement/readers";

    #[test]
    fn membership_epochs_exclude_future_readers_and_removed_members() {
        let device_provider = OpenMlsRustCrypto::default();
        let reader_provider = OpenMlsRustCrypto::default();
        let new_provider = OpenMlsRustCrypto::default();
        let device = Identity::generate(&device_provider, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&reader_provider, b"reader".to_vec()).unwrap();
        let new_reader = Identity::generate(&new_provider, b"new-reader".to_vec()).unwrap();
        let approved = vec![
            device.public.clone(),
            reader.public.clone(),
            new_reader.public.clone(),
        ];
        let mut publisher = TelemetryGroup::create(&device_provider, &device, SCOPE).unwrap();
        let add = publisher
            .prepare_add(
                &device_provider,
                &device,
                &reader.key_package(&reader_provider).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&device_provider).unwrap();
        let mut reader_group = TelemetryGroup::join(
            &reader_provider,
            SCOPE,
            &add.welcome,
            device.public.clone(),
            &approved,
        )
        .unwrap();
        let old_message = publisher
            .encrypt(&device_provider, &device, b"before new reader", &approved)
            .unwrap();
        assert_eq!(
            reader_group
                .process(&reader_provider, &old_message, &approved)
                .unwrap(),
            Received::Application(b"before new reader".to_vec())
        );
        assert!(
            reader_group
                .process(&reader_provider, &old_message, &approved)
                .is_err()
        );
        let add = publisher
            .prepare_add(
                &device_provider,
                &device,
                &new_reader.key_package(&new_provider).unwrap(),
                new_reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&device_provider).unwrap();
        reader_group
            .process(&reader_provider, &add.commit, &approved)
            .unwrap();
        let mut new_group = TelemetryGroup::join(
            &new_provider,
            SCOPE,
            &add.welcome,
            device.public.clone(),
            &approved,
        )
        .unwrap();
        assert!(
            new_group
                .process(&new_provider, &old_message, &approved)
                .is_err()
        );
        let remove = publisher
            .prepare_remove(&device_provider, &device, reader.public())
            .unwrap();
        publisher.merge_pending(&device_provider).unwrap();
        let after_removal_policy = vec![device.public.clone(), new_reader.public.clone()];
        new_group
            .process(&new_provider, &remove, &after_removal_policy)
            .unwrap();
        reader_group
            .process(&reader_provider, &remove, &after_removal_policy)
            .unwrap();
        let message = publisher
            .encrypt(&device_provider, &device, b"after removal", &approved)
            .unwrap();
        assert_eq!(
            new_group
                .process(&new_provider, &message, &approved)
                .unwrap(),
            Received::Application(b"after removal".to_vec())
        );
        assert!(
            reader_group
                .process(&reader_provider, &message, &approved)
                .is_err()
        );
        drop(reader_group);
        TelemetryGroup::delete_retired(&reader_provider, SCOPE).unwrap();
        let add = publisher
            .prepare_add(
                &device_provider,
                &device,
                &reader.key_package(&reader_provider).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&device_provider).unwrap();
        let mut rejoined = TelemetryGroup::join(
            &reader_provider,
            SCOPE,
            &add.welcome,
            device.public.clone(),
            &approved,
        )
        .unwrap();
        assert!(
            rejoined
                .process(&reader_provider, &message, &approved)
                .is_err()
        );
        let after_rejoin = publisher
            .encrypt(&device_provider, &device, b"after rejoin", &approved)
            .unwrap();
        assert_eq!(
            rejoined
                .process(&reader_provider, &after_rejoin, &approved)
                .unwrap(),
            Received::Application(b"after rejoin".to_vec())
        );
    }

    #[test]
    fn key_substitution_and_wrong_scope_are_rejected() {
        let a = OpenMlsRustCrypto::default();
        let b = OpenMlsRustCrypto::default();
        let c = OpenMlsRustCrypto::default();
        let device = Identity::generate(&a, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&b, b"reader".to_vec()).unwrap();
        let impostor = Identity::generate(&c, b"reader".to_vec()).unwrap();
        let mut publisher = TelemetryGroup::create(&a, &device, SCOPE).unwrap();
        assert!(matches!(
            publisher.prepare_add(
                &a,
                &device,
                &impostor.key_package(&c).unwrap(),
                reader.public()
            ),
            Err(CryptoError::UntrustedPeer)
        ));
        let add = publisher
            .prepare_add(
                &a,
                &device,
                &reader.key_package(&b).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        let approved = vec![device.public.clone(), reader.public.clone()];
        assert!(matches!(
            TelemetryGroup::join(
                &b,
                "another-placement",
                &add.welcome,
                device.public.clone(),
                &approved
            ),
            Err(CryptoError::WrongScope)
        ));
    }

    #[test]
    fn readers_cannot_publish_or_add_members() {
        let a = OpenMlsRustCrypto::default();
        let b = OpenMlsRustCrypto::default();
        let device = Identity::generate(&a, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&b, b"reader".to_vec()).unwrap();
        let mut publisher = TelemetryGroup::create(&a, &device, SCOPE).unwrap();
        assert!(
            publisher
                .encrypt(
                    &a,
                    &reader,
                    b"forged telemetry",
                    std::slice::from_ref(&device.public),
                )
                .is_err()
        );
        assert!(TelemetryGroup::delete_retired(&a, SCOPE).is_err());
        assert!(
            publisher
                .prepare_add(
                    &a,
                    &reader,
                    &reader.key_package(&b).unwrap(),
                    reader.public()
                )
                .is_err()
        );
        assert!(decode(&vec![0; MAX_WIRE_SIZE + 1]).is_err());
    }

    #[test]
    fn correctly_encrypted_reader_messages_are_rejected_as_telemetry() {
        let a = OpenMlsRustCrypto::default();
        let b = OpenMlsRustCrypto::default();
        let device = Identity::generate(&a, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&b, b"reader".to_vec()).unwrap();
        let approved = vec![device.public.clone(), reader.public.clone()];
        let mut publisher = TelemetryGroup::create(&a, &device, SCOPE).unwrap();
        let add = publisher
            .prepare_add(
                &a,
                &device,
                &reader.key_package(&b).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        let mut reader_group =
            TelemetryGroup::join(&b, SCOPE, &add.welcome, device.public.clone(), &approved)
                .unwrap();
        let forged = reader_group
            .group
            .create_message(&b, &reader.signer, b"forged telemetry")
            .unwrap();
        assert!(matches!(
            publisher.process(&a, &encode(&forged).unwrap(), &approved),
            Err(CryptoError::UntrustedPeer)
        ));
        let mut trailing = publisher.encrypt(&a, &device, b"valid", &approved).unwrap();
        trailing.push(0);
        assert!(reader_group.process(&b, &trailing, &approved).is_err());
        let mut corrupt = publisher.encrypt(&a, &device, b"valid", &approved).unwrap();
        let last = corrupt.last_mut().unwrap();
        *last ^= 1;
        assert!(reader_group.process(&b, &corrupt, &approved).is_err());
    }

    #[test]
    fn restored_group_preserves_receive_ratchets_and_signing_identity() {
        let a = OpenMlsRustCrypto::default();
        let b = OpenMlsRustCrypto::default();
        let device = Identity::generate(&a, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&b, b"reader".to_vec()).unwrap();
        let approved = vec![device.public.clone(), reader.public.clone()];
        let mut publisher = TelemetryGroup::create(&a, &device, SCOPE).unwrap();
        let add = publisher
            .prepare_add(
                &a,
                &device,
                &reader.key_package(&b).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        let mut reader_group =
            TelemetryGroup::join(&b, SCOPE, &add.welcome, device.public.clone(), &approved)
                .unwrap();
        let message = publisher.encrypt(&a, &device, b"first", &approved).unwrap();
        reader_group.process(&b, &message, &approved).unwrap();
        drop(reader_group);
        drop(publisher);
        let public = device.public.clone();
        drop(device);
        let device = Identity::load(&a, public.clone()).unwrap();
        let mut publisher = TelemetryGroup::load(&a, SCOPE, public.clone(), &approved).unwrap();
        let mut reader_group = TelemetryGroup::load(&b, SCOPE, public, &approved).unwrap();
        assert!(reader_group.process(&b, &message, &approved).is_err());
        let next = publisher.encrypt(&a, &device, b"next", &approved).unwrap();
        assert_eq!(
            reader_group.process(&b, &next, &approved).unwrap(),
            Received::Application(b"next".to_vec())
        );
    }

    #[test]
    fn inbound_unapproved_membership_is_not_merged() {
        let a = OpenMlsRustCrypto::default();
        let b = OpenMlsRustCrypto::default();
        let c = OpenMlsRustCrypto::default();
        let device = Identity::generate(&a, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&b, b"reader".to_vec()).unwrap();
        let pending = Identity::generate(&c, b"pending-reader".to_vec()).unwrap();
        let approved = vec![device.public.clone(), reader.public.clone()];
        let mut publisher = TelemetryGroup::create(&a, &device, SCOPE).unwrap();
        let add = publisher
            .prepare_add(
                &a,
                &device,
                &reader.key_package(&b).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        let mut reader_group =
            TelemetryGroup::join(&b, SCOPE, &add.welcome, device.public.clone(), &approved)
                .unwrap();
        let epoch = reader_group.epoch();
        let add = publisher
            .prepare_add(
                &a,
                &device,
                &pending.key_package(&c).unwrap(),
                pending.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        assert!(matches!(
            reader_group.process(&b, &add.commit, &approved),
            Err(CryptoError::UntrustedPeer)
        ));
        assert_eq!(reader_group.epoch(), epoch);
        let mut pending_policy = approved.clone();
        pending_policy.push(pending.public.clone());
        let message = publisher
            .encrypt(&a, &device, b"unapproved epoch", &pending_policy)
            .unwrap();
        assert!(reader_group.process(&b, &message, &approved).is_err());
    }

    #[test]
    fn revoked_existing_roster_blocks_publication_until_removal() {
        let a = OpenMlsRustCrypto::default();
        let b = OpenMlsRustCrypto::default();
        let device = Identity::generate(&a, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&b, b"reader".to_vec()).unwrap();
        let mut publisher = TelemetryGroup::create(&a, &device, SCOPE).unwrap();
        publisher
            .prepare_add(
                &a,
                &device,
                &reader.key_package(&b).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        let current_policy = vec![device.public.clone()];
        assert!(matches!(
            publisher.encrypt(&a, &device, b"secret after revocation", &current_policy),
            Err(CryptoError::UntrustedPeer)
        ));
        publisher
            .prepare_remove(&a, &device, reader.public())
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        assert!(
            publisher
                .encrypt(&a, &device, b"secret after removal", &current_policy)
                .is_ok()
        );
    }

    #[test]
    fn welcome_cannot_replace_an_existing_group_or_its_ratchets() {
        let a = OpenMlsRustCrypto::default();
        let b = OpenMlsRustCrypto::default();
        let device = Identity::generate(&a, b"device".to_vec()).unwrap();
        let reader = Identity::generate(&b, b"reader".to_vec()).unwrap();
        let another_leaf = Identity::generate(&b, b"another-leaf".to_vec()).unwrap();
        let approved = vec![
            device.public.clone(),
            reader.public.clone(),
            another_leaf.public.clone(),
        ];
        let mut publisher = TelemetryGroup::create(&a, &device, SCOPE).unwrap();
        let add = publisher
            .prepare_add(
                &a,
                &device,
                &reader.key_package(&b).unwrap(),
                reader.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        let mut reader_group =
            TelemetryGroup::join(&b, SCOPE, &add.welcome, device.public.clone(), &approved)
                .unwrap();
        let original = publisher
            .encrypt(&a, &device, b"before attempted replacement", &approved)
            .unwrap();
        reader_group.process(&b, &original, &approved).unwrap();
        let add = publisher
            .prepare_add(
                &a,
                &device,
                &another_leaf.key_package(&b).unwrap(),
                another_leaf.public(),
            )
            .unwrap();
        publisher.merge_pending(&a).unwrap();
        assert!(matches!(
            TelemetryGroup::join(&b, SCOPE, &add.welcome, device.public.clone(), &approved),
            Err(CryptoError::InvalidInput("MLS group already exists"))
        ));
        assert!(reader_group.process(&b, &original, &approved).is_err());
        reader_group.process(&b, &add.commit, &approved).unwrap();
        let current = publisher
            .encrypt(&a, &device, b"original group continues", &approved)
            .unwrap();
        assert_eq!(
            reader_group.process(&b, &current, &approved).unwrap(),
            Received::Application(b"original group continues".to_vec())
        );
    }
}

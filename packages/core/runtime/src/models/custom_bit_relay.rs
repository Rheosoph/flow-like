//! Private Bits on server runs. A run holds no provider key for them: it calls the API's model
//! relay under its executor JWT, and the API calls the provider with the owner's key. Media
//! travels as the run holds it, links unchanged and bytes inline, so the run downloads nothing
//! for such a call; the API resolves each link the way a direct call with the Bit would.

use std::{collections::HashSet, sync::Arc};

use flow_like_model_provider::{
    llm::{ModelLogic, external::ExternalProvider, relay::RelayModel},
    systemone::SystemOneClient,
};
use flow_like_types::{Result, bail};

use crate::{bit::Bit, state::FlowLikeState};

/// Media nodes call providers with the Bit's own key, and the relay has no media route.
pub const MEDIA_RUNS_ON_DESKTOP: &str =
    "Image, audio and video generation with a private model's key runs on desktop only";

/// Where a server run calls the private Bits of its dispatched profile: the relay at
/// `{callback}/api/v1/execution/apps/{app_id}/models`.
pub struct CustomBitRelay {
    base: String,
    token: String,
    ids: HashSet<String>,
}

impl CustomBitRelay {
    /// `ids` are the relayable Bits of the run's dispatched profile; `token` is its executor JWT.
    pub fn new(
        callback_url: &str,
        app_id: &str,
        token: &str,
        ids: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            base: format!(
                "{}/api/v1/execution/apps/{app_id}/models",
                callback_url.trim().trim_end_matches('/')
            ),
            token: token.to_string(),
            ids: ids.into_iter().collect(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// A Bit of the dispatched profile whose provider the relay calls. A Bit built on the board
    /// has a content-hash id and its own key, so it keeps calling its provider directly.
    pub fn serves(&self, bit: &Bit) -> bool {
        self.ids.contains(&bit.id) && relayable(bit)
    }

    pub fn chat_model(&self, bit: &Bit) -> Result<Arc<dyn ModelLogic>> {
        Ok(Arc::new(RelayModel::new(&self.base, &self.token, &bit.id)?))
    }

    pub fn systemone_model(&self, bit: &Bit) -> Result<SystemOneClient> {
        SystemOneClient::new(
            &format!("{}/systemone", self.base),
            bit.id.as_str(),
            self.token.as_str(),
        )
    }
}

/// A `custom:*` provider whose key the hub holds. Subscription providers sign in on the owner's
/// desktop and run nowhere else.
pub fn relayable_provider(provider_name: &str) -> bool {
    let name = provider_name.trim().to_ascii_lowercase();
    name.starts_with("custom:") && ExternalProvider::from_provider_name(&name).is_none()
}

/// Whether `bit` names a relayable provider, whatever its model type.
pub fn relayable(bit: &Bit) -> bool {
    bit.parameters
        .pointer("/provider/provider_name")
        .and_then(|name| name.as_str())
        .is_some_and(relayable_provider)
}

/// Refuses a private Bit that this run reaches only through the relay, before a media node
/// looks for the key it does not carry.
pub fn refuse_relayed_media(state: &FlowLikeState, bit: &Bit) -> Result<()> {
    if state
        .custom_bit_relay
        .as_ref()
        .is_some_and(|relay| relay.serves(bit))
    {
        bail!(MEDIA_RUNS_ON_DESKTOP);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bit::BitTypes, state::FlowLikeConfig, utils::http::HTTPClient};
    use flow_like_storage::files::store::FlowLikeStore;
    use flow_like_types::json::json;

    fn bit(id: &str, bit_type: BitTypes, provider_name: &str) -> Bit {
        Bit {
            id: id.to_string(),
            bit_type,
            parameters: json!({
                "context_length": 8192,
                "provider": {"provider_name": provider_name, "model_id": "upstream", "params": {}}
            }),
            ..Bit::default()
        }
    }

    fn relay() -> CustomBitRelay {
        CustomBitRelay::new(
            "https://api.example.test/",
            "app-1",
            "exec-jwt",
            ["my-claude", "my-codex", "my-image"].map(String::from),
        )
    }

    fn state(relay: Option<CustomBitRelay>) -> FlowLikeState {
        let mut state = FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(Arc::new(
                flow_like_storage::object_store::memory::InMemory::new(),
            ))),
            HTTPClient::new_without_refetch(),
        );
        state.custom_bit_relay = relay.map(Arc::new);
        state
    }

    #[test]
    fn only_custom_providers_outside_the_desktop_subscriptions_are_relayable() {
        for (provider, relayable) in [
            ("custom:openai", true),
            (" CUSTOM:Anthropic ", true),
            ("custom:typesafe", true),
            ("custom:codex", false),
            ("custom:github-copilot", false),
            ("custom:microsoft-copilot", false),
            ("custom:claude-code", false),
            ("hosted:openai", false),
            ("openai", false),
            ("local", false),
        ] {
            assert_eq!(relayable_provider(provider), relayable, "{provider}");
        }
    }

    #[test]
    fn the_relay_serves_only_relayable_bits_of_the_dispatched_profile() {
        let relay = relay();
        assert_eq!(
            relay.base_url(),
            "https://api.example.test/api/v1/execution/apps/app-1/models"
        );
        for (bit, served) in [
            (bit("my-claude", BitTypes::Llm, "custom:anthropic"), true),
            (
                bit("my-claude", BitTypes::SystemOne, "custom:typesafe"),
                true,
            ),
            (
                bit("my-image", BitTypes::ImageGeneration, "custom:openai"),
                true,
            ),
            (bit("board-built", BitTypes::Llm, "custom:anthropic"), false),
            (bit("my-codex", BitTypes::Llm, "custom:codex"), false),
            (bit("my-claude", BitTypes::Llm, "hosted:openai"), false),
        ] {
            assert_eq!(
                relay.serves(&bit),
                served,
                "{} {:?}",
                bit.id,
                bit.parameters
            );
        }
    }

    #[test]
    fn media_nodes_refuse_only_bits_behind_the_relay() {
        let private = bit("my-image", BitTypes::ImageGeneration, "custom:openai");
        let error = refuse_relayed_media(&state(Some(relay())), &private)
            .expect_err("a relayed Bit carries no key for media");
        assert_eq!(error.to_string(), MEDIA_RUNS_ON_DESKTOP);

        let board_built = bit("board-built", BitTypes::Llm, "custom:openai");
        assert!(refuse_relayed_media(&state(Some(relay())), &board_built).is_ok());
        assert!(refuse_relayed_media(&state(None), &private).is_ok());
    }

    #[test]
    fn every_run_of_a_state_keeps_its_relay_and_egress_mode() {
        let mut host = state(Some(relay()));
        host.model_egress = flow_like_model_provider::egress::ProviderEgress::Private;

        let run = host.for_execution_run();

        assert!(Arc::ptr_eq(
            run.custom_bit_relay
                .as_ref()
                .expect("the run keeps the relay"),
            host.custom_bit_relay
                .as_ref()
                .expect("the host has a relay"),
        ));
        assert_eq!(run.model_egress, host.model_egress);
    }
}

//! Azure Web PubSub transport.
//!
//! The waiter holds one `json.webpubsub.azure.v1` WebSocket per channel, joined to the group
//! `run:{channel_id}`, and receives every client push as a group message. The API mints
//! HS256 client access tokens with the hub access key (each side gets exactly one literal
//! per-group role) and forwards fallback pushes with the data-plane REST `:send` call.

mod channel;
mod forwarder;
mod protocol;
mod router;
mod token;

pub use channel::AzureWebPubSubChannel;
pub use forwarder::AzureWebPubSubForwarder;
pub use protocol::SUBPROTOCOL;
pub use token::{
    DATA_PLANE_API_VERSION, client_access_token, client_audience, client_roles, client_ws_url,
    executor_roles, group_for, normalize_endpoint, rest_token, send_to_group_url,
};

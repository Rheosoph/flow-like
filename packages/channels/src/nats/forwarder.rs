use super::{
    ACK_TIMEOUT, Acknowledgement, Envelope, MAX_PAYLOAD_BYTES, Role, mint_user_token, subject_for,
    valid_delivery_id, validate_account_seed, validate_url,
};
use crate::ChannelForwarder;
use flow_like_types::channel::{ChannelPush, new_request_id};
use flow_like_types::{Result, anyhow, async_trait, bail};
use futures_util::StreamExt;

pub struct NatsForwarder {
    url: String,
    account_seed: String,
}

impl NatsForwarder {
    pub fn new(url: String, account_seed: String) -> Result<Self> {
        validate_url(&url)?;
        validate_account_seed(&account_seed)?;
        Ok(Self { url, account_seed })
    }
}

#[async_trait]
impl ChannelForwarder for NatsForwarder {
    fn transport(&self) -> &'static str {
        "nats"
    }
    async fn forward(&self, push: &ChannelPush) -> Result<()> {
        self.forward_with_id(push, &new_request_id()).await
    }
    async fn forward_with_id(&self, push: &ChannelPush, id: &str) -> Result<()> {
        let token = mint_user_token(&self.account_seed, &push.channel_id, Role::Client, 30)?;
        let client = super::connect(&self.url, &token).await?;
        let result = publish_confirmed(&client, &subject_for(&push.channel_id)?, push, id).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), client.drain()).await;
        result
    }
}

/// Confirms receiver acceptance. Repeated attempts keep one delivery id so a lost ack cannot
/// enqueue the same steering message twice within the receiver's recent receipt window.
pub async fn publish_confirmed(
    client: &async_nats::Client,
    subject: &str,
    push: &ChannelPush,
    id: &str,
) -> Result<()> {
    if !valid_delivery_id(id) {
        bail!("invalid NATS delivery id");
    }
    if subject != subject_for(&push.channel_id)? {
        bail!("NATS subject does not match channel");
    }
    let payload = serde_json::to_vec(&Envelope {
        id: id.into(),
        push: push.clone(),
    })?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        bail!("NATS channel message exceeds 1 MiB");
    }
    let reply = format!("{subject}.ack.{id}");
    let mut acknowledgements = client
        .subscribe(reply.clone())
        .await
        .map_err(|_| anyhow!("NATS acknowledgement subscription failed"))?;
    let result = async {
        tokio::time::timeout(super::CONNECT_TIMEOUT, client.flush())
            .await
            .map_err(|_| anyhow!("NATS acknowledgement subscription timed out"))?
            .map_err(|_| anyhow!("NATS acknowledgement subscription failed"))?;
        for _ in 0..3 {
            client
                .publish_with_reply(subject.to_owned(), reply.clone(), payload.clone().into())
                .await
                .map_err(|_| anyhow!("NATS channel publication failed"))?;
            match tokio::time::timeout(ACK_TIMEOUT, acknowledgements.next()).await {
                Ok(Some(message)) => {
                    if message.status == Some(async_nats::StatusCode::NO_RESPONDERS) {
                        bail!("NATS channel has no connected waiter");
                    }
                    let ack: Acknowledgement = serde_json::from_slice(&message.payload)
                        .map_err(|_| anyhow!("invalid NATS delivery acknowledgement"))?;
                    if ack.accepted {
                        return Ok(());
                    }
                    bail!(
                        "{}",
                        ack.error
                            .unwrap_or_else(|| "NATS waiter rejected the message".into())
                    );
                }
                Ok(None) => bail!("NATS acknowledgement subscription closed"),
                Err(_) => {}
            }
        }
        bail!("NATS waiter did not acknowledge the message")
    }
    .await;
    let _ = acknowledgements.unsubscribe().await;
    result
}

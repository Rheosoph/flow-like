use super::{MAX_WAKE_BYTES, ready, valid_wakeup};
use crate::aws::presign::{Presigner, mqtt_wss_url};
use flow_like_types::{Result, anyhow, bail, channel::AwsTemporaryCredentials};
use rumqttc::{
    AsyncClient, Event, MqttOptions, Packet, QoS, SubscribeReasonCode, TlsConfiguration, Transport,
};
use std::time::{Duration, SystemTime};
use tokio::sync::{Notify, watch};

pub(super) async fn listen(
    endpoint: &str,
    region: &str,
    client_id: &str,
    topic: &str,
    credentials: &AwsTemporaryCredentials,
    wake: &Notify,
    healthy: &watch::Sender<bool>,
) -> Result<()> {
    crate::aws::validate_topic(topic)?;
    if client_id.is_empty() || credentials.expiration <= chrono::Utc::now().timestamp() {
        bail!("Realtime AWS grant is invalid or expired");
    }
    let signer = Presigner::new(endpoint, region, credentials);
    signer.presign(SystemTime::now())?;
    let mut options = MqttOptions::new(client_id, mqtt_wss_url(endpoint), 443);
    options
        .set_transport(Transport::wss_with_config(TlsConfiguration::Rustls(
            crate::tls::client_config()?,
        )))
        .set_keep_alive(Duration::from_secs(30))
        .set_clean_session(true)
        .set_max_packet_size(MAX_WAKE_BYTES * 2, MAX_WAKE_BYTES * 2);
    options.set_request_modifier(move |request| std::future::ready(signer.apply(request)));
    let (client, mut event_loop) = AsyncClient::new(options, 4);
    let mut subscribed = false;
    loop {
        let event = tokio::time::timeout(
            Duration::from_secs(if subscribed { 90 } else { 20 }),
            event_loop.poll(),
        )
        .await
        .map_err(|_| anyhow!("Realtime AWS subscription timed out"))?
        .map_err(|_| anyhow!("Realtime AWS connection closed"))?;
        match event {
            Event::Incoming(Packet::ConnAck(_)) => {
                client
                    .subscribe(topic, QoS::AtLeastOnce)
                    .await
                    .map_err(|_| anyhow!("Realtime AWS subscription failed"))?;
            }
            Event::Incoming(Packet::SubAck(ack)) => {
                if ack.return_codes.len() != 1
                    || ack.return_codes.contains(&SubscribeReasonCode::Failure)
                {
                    bail!("Realtime AWS subscription was denied");
                }
                subscribed = true;
                ready(healthy, wake);
            }
            Event::Incoming(Packet::Publish(message)) if message.topic == topic => {
                if valid_wakeup(&message.payload) {
                    wake.notify_one();
                }
            }
            _ => {}
        }
    }
}

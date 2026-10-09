use super::{MqttConfig, MqttConnection, MqttLastWill, MqttQoS, MqttSubscription};
use flow_like::flow::execution::ExecutionEnvironment;
use flow_like_types::{json::json, tokio_util::sync::CancellationToken};
use rumqttc::{Publish, QoS};
use std::time::Duration;

fn config(client_id: String) -> MqttConfig {
    let host = std::env::var("FLOW_LIKE_MQTT_HOST")
        .expect("set FLOW_LIKE_MQTT_HOST to the Mosquitto fixture host");
    let port = std::env::var("FLOW_LIKE_MQTT_PORT")
        .expect("set FLOW_LIKE_MQTT_PORT to the Mosquitto fixture port")
        .parse::<u16>()
        .expect("FLOW_LIKE_MQTT_PORT must be a TCP port");
    flow_like_types::json::from_value(json!({
        "host": host, "port": port, "client_id": client_id,
        "keep_alive_seconds": 5, "connect_timeout_seconds": 5
    }))
    .unwrap()
}

async fn connect(config: &MqttConfig) -> std::sync::Arc<MqttConnection> {
    MqttConnection::connect(config, ExecutionEnvironment::Local, None)
        .await
        .unwrap()
}

async fn receive(subscription: &mut MqttSubscription) -> Publish {
    tokio::time::timeout(Duration::from_secs(5), subscription.recv())
        .await
        .expect("Mosquitto publication deadline")
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the real Mosquitto Docker fixture and FLOW_LIKE_MQTT_HOST/PORT"]
async fn mosquitto_qos_binary_retained_unicode_and_system_topics() {
    let id = uuid::Uuid::new_v4();
    let topic = format!("温度/{id}/測定");
    let publisher = connect(&config(format!("publisher-{id}"))).await;
    let subscriber = connect(&config(format!("subscriber-{id}"))).await;
    let mut messages = subscriber
        .subscribe(format!("温度/{id}/+"), QoS::ExactlyOnce)
        .await
        .unwrap();

    for qos in [QoS::AtLeastOnce, QoS::ExactlyOnce] {
        publisher
            .publish(topic.clone(), vec![0, 255, 128, qos as u8], qos, true)
            .await
            .unwrap();
        let message = receive(&mut messages).await;
        assert_eq!(message.topic, topic);
        assert_eq!(message.payload.as_ref(), &[0, 255, 128, qos as u8]);
        assert_eq!(message.qos, qos);
        assert!(!message.retain);
    }

    let late = connect(&config(format!("retained-{id}"))).await;
    let mut retained = late
        .subscribe(topic.clone(), QoS::AtLeastOnce)
        .await
        .unwrap();
    let message = receive(&mut retained).await;
    assert_eq!(message.topic, topic);
    assert_eq!(message.payload.as_ref(), &[0, 255, 128, 2]);
    assert_eq!(message.qos, QoS::AtLeastOnce);
    assert!(message.retain);

    let mut system = late
        .subscribe("$SYS/broker/version".into(), QoS::AtMostOnce)
        .await
        .unwrap();
    let message = receive(&mut system).await;
    assert_eq!(message.topic, "$SYS/broker/version");
    assert!(
        std::str::from_utf8(&message.payload)
            .unwrap()
            .contains("mosquitto")
    );

    // A zero-length retained publish removes the stored value for future subscribers.
    publisher
        .publish(topic.clone(), vec![], QoS::AtLeastOnce, true)
        .await
        .unwrap();
    assert!(receive(&mut retained).await.payload.is_empty());
    retained.close().await.unwrap();
    let mut cleared = late.subscribe(topic, QoS::AtLeastOnce).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(300), cleared.recv())
            .await
            .is_err()
    );
    cleared.close().await.unwrap();
    system.close().await.unwrap();
    messages.close().await.unwrap();
    late.disconnect().await.unwrap();
    subscriber.disconnect().await.unwrap();
    publisher.disconnect().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the real Mosquitto Docker fixture and FLOW_LIKE_MQTT_HOST/PORT"]
async fn mosquitto_persistent_replay_reaches_later_listeners_for_each_filter() {
    let id = uuid::Uuid::new_v4();
    let prefix = format!("flow-like/{id}");
    let mut persistent_config = config(format!("persistent-{id}"));
    persistent_config.clean_session = false;
    let session = connect(&persistent_config).await;
    let first = session
        .subscribe(format!("{prefix}/a/+"), QoS::AtLeastOnce)
        .await
        .unwrap();
    let second = session
        .subscribe(format!("{prefix}/b/+"), QoS::AtLeastOnce)
        .await
        .unwrap();
    session.disconnect().await.unwrap();
    // Disconnect before dropping the workflow listeners preserves broker subscriptions.
    drop((first, second, session));

    let publisher = connect(&config(format!("publisher-{id}"))).await;
    for (suffix, value) in [("a/温度", 1), ("b/pressure", 2)] {
        publisher
            .publish(
                format!("{prefix}/{suffix}"),
                vec![value, 255],
                QoS::AtLeastOnce,
                false,
            )
            .await
            .unwrap();
    }
    let resumed = connect(&persistent_config).await;
    resumed.wait_for_pending_replay(2).await;
    let mut first = resumed
        .subscribe(format!("{prefix}/a/+"), QoS::AtLeastOnce)
        .await
        .unwrap();
    assert_eq!(receive(&mut first).await.payload.as_ref(), &[1, 255]);
    publisher
        .publish(
            format!("{prefix}/b/pressure"),
            vec![3, 255],
            QoS::AtLeastOnce,
            false,
        )
        .await
        .unwrap();
    resumed.wait_for_pending_replay(2).await;
    let mut second = resumed
        .subscribe(format!("{prefix}/b/+"), QoS::AtLeastOnce)
        .await
        .unwrap();
    for expected in [2, 3] {
        let message = receive(&mut second).await;
        assert_eq!(message.topic, format!("{prefix}/b/pressure"));
        assert_eq!(message.payload.as_ref(), &[expected, 255]);
        assert!(!message.retain);
    }
    publisher
        .publish(
            format!("{prefix}/b/pressure"),
            vec![4],
            QoS::AtLeastOnce,
            false,
        )
        .await
        .unwrap();
    publisher
        .publish(format!("{prefix}/a/温度"), vec![5], QoS::AtLeastOnce, false)
        .await
        .unwrap();
    assert_eq!(receive(&mut first).await.payload.as_ref(), &[5]);
    assert_eq!(receive(&mut second).await.payload.as_ref(), &[4]);
    first.close().await.unwrap();
    second.close().await.unwrap();
    resumed.disconnect().await.unwrap();
    publisher.disconnect().await.unwrap();
    // Reconnecting cleanly clears the test's persistent session from the broker.
    persistent_config.clean_session = true;
    connect(&persistent_config)
        .await
        .disconnect()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires the real Mosquitto Docker fixture and FLOW_LIKE_MQTT_HOST/PORT"]
async fn mosquitto_last_will_on_cancel_and_no_will_after_graceful_disconnect() {
    let id = uuid::Uuid::new_v4();
    let topic = format!("flow-like/{id}/will");
    let observer = connect(&config(format!("observer-{id}"))).await;
    let mut messages = observer
        .subscribe(topic.clone(), QoS::AtLeastOnce)
        .await
        .unwrap();
    let mut will_config = config(format!("will-{id}"));
    will_config.last_will = Some(MqttLastWill {
        topic: topic.clone(),
        payload: vec![0, 255],
        qos: MqttQoS::AtLeastOnce,
        retain: false,
    });
    let cancellation = CancellationToken::new();
    let client = MqttConnection::connect(
        &will_config,
        ExecutionEnvironment::Local,
        Some(cancellation.clone()),
    )
    .await
    .unwrap();
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), client.closed())
        .await
        .unwrap()
        .unwrap();
    let will = receive(&mut messages).await;
    assert_eq!(will.topic, topic);
    assert_eq!(will.payload.as_ref(), &[0, 255]);
    assert_eq!(will.qos, QoS::AtLeastOnce);
    connect(&will_config).await.disconnect().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(500), messages.recv())
            .await
            .is_err()
    );
    messages.close().await.unwrap();
    observer.disconnect().await.unwrap();
}

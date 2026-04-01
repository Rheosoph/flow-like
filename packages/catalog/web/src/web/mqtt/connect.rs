#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;

use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;

#[cfg(feature = "execute")]
use flow_like_types::Cacheable;
#[cfg(feature = "execute")]
use std::sync::Arc;

use super::{MqttConfig, MqttSession};

#[cfg(feature = "execute")]
use super::CachedMqttConnection;

#[crate::register_node]
#[derive(Default)]
pub struct MqttConnectNode {}

impl MqttConnectNode {
    pub fn new() -> Self {
        MqttConnectNode {}
    }
}

#[async_trait]
impl NodeLogic for MqttConnectNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mqtt_connect",
            "MQTT Connect",
            "Connects to an MQTT broker and returns a session reference for use with \
             Publish, Subscribe, and Disconnect nodes.",
            "Web/MQTT",
        );
        node.add_icon("/flow/icons/web.svg");
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate the MQTT connection",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "MQTT connection configuration (host, port, client_id, optional credentials, TLS)",
            VariableType::Struct,
        )
        .set_schema::<MqttConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the connection is established",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session",
            "Session",
            "MQTT session reference for use with Publish/Subscribe/Disconnect nodes",
            VariableType::Struct,
        )
        .set_schema::<MqttSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the connection fails to establish",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use rumqttc::MqttOptions;

        context.deactivate_exec_pin("exec_out").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: MqttConfig = context.evaluate_pin("config").await?;

        let mut options = MqttOptions::new(&config.client_id, &config.host, config.port);
        options.set_keep_alive(std::time::Duration::from_secs(config.keep_alive_seconds));

        if let (Some(user), Some(pass)) = (&config.username, &config.password) {
            options.set_credentials(user, pass);
        }

        if config.use_tls {
            let transport = rumqttc::Transport::tls_with_default_config();
            options.set_transport(transport);
        }

        let (client, eventloop) = rumqttc::AsyncClient::new(options, 100);

        let ref_id = format!("mqtt_{}", flow_like_types::create_id());
        let close_notify = Arc::new(tokio::sync::Notify::new());

        let cached = CachedMqttConnection {
            client: Arc::new(tokio::sync::Mutex::new(client)),
            event_loop: Arc::new(tokio::sync::Mutex::new(eventloop)),
            close_notify: close_notify.clone(),
        };
        let cacheable: Arc<dyn Cacheable> = Arc::new(cached);
        context.set_cache(&ref_id, cacheable).await;

        let session = MqttSession {
            ref_id,
            client_id: config.client_id.clone(),
        };
        context.set_pin_value("session", json!(session)).await?;

        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "MQTT requires the 'execute' feature"
        ))
    }
}

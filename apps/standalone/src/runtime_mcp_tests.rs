use super::{
    tests::{ListeningProbe, connect, fixture},
    *,
};
use flow_like_runtime::flow::{board::Board, node::NodeLogic};
use serde_json::json;
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
enum Authentication {
    Public,
    Bearer,
    ApiKey,
}

fn authenticated(
    request: reqwest::RequestBuilder,
    credential: &Option<(&'static str, String)>,
) -> reqwest::RequestBuilder {
    match credential {
        Some((header, value)) => {
            let mut value = reqwest::header::HeaderValue::from_str(value).unwrap();
            value.set_sensitive(true);
            request.header(*header, value)
        }
        None => request,
    }
}

#[tokio::test]
async fn mcp_catalog_binds_with_placement_variables_and_stops_on_cancel() {
    exercise_catalog_service(Authentication::Public).await;
}

#[tokio::test]
async fn mcp_catalog_requires_private_bearer_override_for_every_session_request() {
    exercise_catalog_service(Authentication::Bearer).await;
}

#[tokio::test]
async fn mcp_catalog_requires_private_api_key_override_for_every_session_request() {
    exercise_catalog_service(Authentication::ApiKey).await;
}

async fn exercise_catalog_service(authentication: Authentication) {
    let directory = tempfile::tempdir().unwrap();
    let (mut config, state, _started, _starts) = fixture(directory.path()).await;
    let app = App::load(config.project_id.clone(), state.clone())
        .await
        .unwrap();
    let mut board = Board::new(
        Some("mcp-board".into()),
        StorePath::from("apps/project"),
        state.clone(),
    );
    // The pinned port stays occupied, so an ignored placement override cannot pass.
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pinned_port = occupied.local_addr().unwrap().port();
    for (id, data_type, default) in [
        (
            "mcp-host",
            VariableType::String,
            json!("invalid-bind-host.invalid"),
        ),
        ("mcp-port", VariableType::Integer, json!(pinned_port)),
        ("mcp-path", VariableType::String, json!("/pinned-mcp")),
    ] {
        let mut variable = Variable::new(id, data_type.clone(), ValueType::Normal);
        variable.id = id.into();
        variable.exposed = true;
        variable.set_default_value(default);
        board.variables.insert(id.into(), variable);
        let registry = state.node_registry.read().await;
        let definition = registry.get_node("variable_get").unwrap();
        let mut node = registry.instantiate(&definition).unwrap().get_node();
        node.id = format!("get-{id}");
        node.get_pin_mut_by_name("var_ref")
            .unwrap()
            .set_default_value(Some(json!(id)));
        node.get_pin_mut_by_name("value_ref").unwrap().data_type = data_type;
        board.nodes.insert(node.id.clone(), node);
    }
    for (catalog_name, id) in [("mcp_server_config", "config"), ("mcp_server", "server")] {
        let registry = state.node_registry.read().await;
        let definition = registry.get_node(catalog_name).unwrap();
        let mut node = registry.instantiate(&definition).unwrap().get_node();
        node.id = id.into();
        board.nodes.insert(node.id.clone(), node);
    }
    let (sender, receiver) = oneshot::channel();
    let probe = Arc::new(ListeningProbe(std::sync::Mutex::new(Some(sender))));
    let mut node = probe.get_node();
    node.id = "listening".into();
    board.nodes.insert(node.id.clone(), node);
    state.node_registry.write().await.push_node(probe);
    for (id, input) in [
        ("mcp-host", "host"),
        ("mcp-port", "port"),
        ("mcp-path", "path"),
    ] {
        connect(
            &mut board,
            &format!("get-{id}"),
            "value_ref",
            "config",
            input,
        );
    }
    let credential = format!("mcp-test-{}", uuid::Uuid::new_v4());
    let header = match authentication {
        Authentication::Public => {
            connect(&mut board, "config", "config", "server", "config");
            None
        }
        Authentication::Bearer | Authentication::ApiKey => {
            let mut variable =
                Variable::new("MCP credential", VariableType::String, ValueType::Normal);
            variable.id = "mcp-credential".into();
            variable.secret = true;
            variable.runtime_configured = true;
            board.variables.insert(variable.id.clone(), variable);
            let (auth_node, credential_pin, header, value) = match authentication {
                Authentication::Bearer => (
                    "bearer_token_auth",
                    "token",
                    "authorization",
                    format!("Bearer {credential}"),
                ),
                Authentication::ApiKey => (
                    "api_key_auth",
                    "key",
                    "x-standalone-mcp-key",
                    credential.clone(),
                ),
                Authentication::Public => unreachable!(),
            };
            for (catalog_name, id) in [
                ("variable_get", "get-credential"),
                (auth_node, "authentication"),
                ("mcp_register_auth", "register-auth"),
            ] {
                let registry = state.node_registry.read().await;
                let definition = registry.get_node(catalog_name).unwrap();
                let mut node = registry.instantiate(&definition).unwrap().get_node();
                node.id = id.into();
                if id == "get-credential" {
                    node.get_pin_mut_by_name("var_ref")
                        .unwrap()
                        .set_default_value(Some(json!("mcp-credential")));
                    node.get_pin_mut_by_name("value_ref").unwrap().data_type = VariableType::String;
                }
                if id == "authentication" && matches!(authentication, Authentication::ApiKey) {
                    node.get_pin_mut_by_name("header")
                        .unwrap()
                        .set_default_value(Some(json!(header)));
                }
                board.nodes.insert(node.id.clone(), node);
            }
            connect(
                &mut board,
                "get-credential",
                "value_ref",
                "authentication",
                credential_pin,
            );
            connect(
                &mut board,
                "authentication",
                "auth",
                "register-auth",
                "auth",
            );
            connect(&mut board, "config", "config", "register-auth", "config_in");
            connect(
                &mut board,
                "register-auth",
                "config_out",
                "server",
                "config",
            );
            config
                .secret_overrides
                .insert("mcp-credential".into(), "mcp-listener-credential".into());
            crate::secrets::install(
                &config,
                "mcp-listener-credential",
                &serde_json::to_vec(&credential).unwrap(),
            )
            .unwrap();
            assert!(
                !serde_json::to_string(&config)
                    .unwrap()
                    .contains(&credential)
            );
            Some((header, value))
        }
    };
    connect(&mut board, "server", "local_addr", "listening", "address");
    connect(&mut board, "server", "on_listening", "listening", "exec");
    board.snapshot_at_version((1, 0, 0), None).await.unwrap();
    let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
    event.board_id = board.id;
    event.node_id = "server".into();
    event.event_type = "mcp".into();
    event.event_version = (2, 0, 0);
    event.save(&app, Some((2, 0, 0))).await.unwrap();
    config.events[0].event_version = [2, 0, 0];
    config
        .variables
        .insert("mcp-host".into(), json!("127.0.0.1"));
    config.variables.insert("mcp-port".into(), json!(0));
    config
        .variables
        .insert("mcp-path".into(), json!("/services/test-mcp"));

    let stop = CancellationToken::new();
    let _stop_on_drop = stop.clone().drop_guard();
    let runtime_stop = stop.clone();
    validate_rollout_with_state(&config, state.clone())
        .await
        .unwrap();
    let (ready_sender, ready_receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        run_with_state_ready(&config, state, runtime_stop, Profile::default(), || async {
            ready_sender.send(()).unwrap();
            Ok(())
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(10), ready_receiver)
        .await
        .unwrap()
        .unwrap();
    let address: std::net::SocketAddr = tokio::time::timeout(Duration::from_secs(10), receiver)
        .await
        .unwrap()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(address.ip(), std::net::Ipv4Addr::LOCALHOST);
    assert_ne!(address.port(), 0);
    assert_ne!(address.port(), pinned_port);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();
    let endpoint = format!("http://{address}/services/test-mcp");
    for path in ["/mcp", "/pinned-mcp"] {
        let response = client
            .get(format!("http://{address}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }
    let initialize = json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"standalone-test","version":"1"}
    }});
    if let Some((name, _)) = &header {
        for supplied in [None, Some("Bearer invalid-credential")] {
            let mut request = client
                .post(&endpoint)
                .header("accept", "application/json")
                .json(&initialize);
            if let Some(value) = supplied {
                request = request.header(*name, value);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
            assert!(response.headers().get("mcp-session-id").is_none());
            assert_eq!(response.text().await.unwrap(), "Unauthorized");
            assert!(!task.is_finished());
        }
    }
    // Authentication does not remove MCP's separate session requirement.
    let response = authenticated(
        client
            .post(&endpoint)
            .header("accept", "application/json")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"ping"})),
        &header,
    )
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32600);
    assert_eq!(body["error"]["message"], "Missing Mcp-Session-Id header");
    let response = authenticated(
        client
            .post(&endpoint)
            .header("accept", "application/json")
            .json(&initialize),
        &header,
    )
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let session_id = response
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(!session_id.is_empty());
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["id"], 2);
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    let response = authenticated(
        client
            .post(&endpoint)
            .header("accept", "application/json")
            .header("mcp-session-id", &session_id)
            .header("mcp-protocol-version", "2025-06-18")
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})),
        &header,
    )
    .send()
    .await
    .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    if let Some((name, _)) = &header {
        // Possession of a valid session ID never substitutes for the configured credential.
        for supplied in [None, Some("Bearer invalid-credential")] {
            let mut request = client
                .post(&endpoint)
                .header("accept", "application/json")
                .header("mcp-session-id", &session_id)
                .header("mcp-protocol-version", "2025-06-18")
                .json(&json!({"jsonrpc":"2.0","id":10,"method":"ping"}));
            if let Some(value) = supplied {
                request = request.header(*name, value);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
            assert_eq!(response.text().await.unwrap(), "Unauthorized");
        }
    }
    for id in 3..6 {
        let response = authenticated(
            client
                .post(&endpoint)
                .header("accept", "application/json")
                .header("mcp-session-id", &session_id)
                .header("mcp-protocol-version", "2025-06-18")
                .json(&json!({"jsonrpc":"2.0","id":id,"method":"ping"})),
            &header,
        )
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap(),
            json!({"jsonrpc":"2.0","id":id,"result":{}})
        );
        assert!(
            !task.is_finished(),
            "MCP listener exited between session requests"
        );
    }
    stop.cancel();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}

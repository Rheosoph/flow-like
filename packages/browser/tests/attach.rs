use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use flow_like_browser::attach::{
    AttachEndpoint, WebDriverProbe, is_loopback, probe_webdriver, read_devtools_active_port,
    resolve_endpoint_with,
};
use flow_like_browser::connection::ConnectionOptions;
use flow_like_browser::launch::BrowserKind;
use flow_like_browser::testing::default_auto_reply;
use flow_like_browser::testing::mock_server::{MockCdpServer, MockJsonVersion, MockServerOptions};
use flow_like_browser::transport::memory::SentCommand;
use flow_like_browser::transport::ws::{ConnectOptions, WsTransport};
use flow_like_browser::{Browser, BrowserError, Connection, ConnectionKind};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const DIRECT_TIMEOUT: Duration = Duration::from_secs(30);
const RECEIVE_WAIT: Duration = Duration::from_secs(5);

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn mock(json_version: MockJsonVersion) -> MockCdpServer {
    MockCdpServer::start(MockServerOptions {
        json_version,
        expect_header: None,
    })
    .await
}

fn connect_message(result: flow_like_browser::Result<AttachEndpoint>) -> String {
    match result {
        Err(BrowserError::Connect { message }) => message,
        other => panic!("expected BrowserError::Connect, got {other:?}"),
    }
}

async fn resolve_error(address: &str, kind: BrowserKind, profile: Option<&Path>) -> String {
    connect_message(resolve_endpoint_with(address, kind, profile).await)
}

struct HttpStub {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl HttpStub {
    async fn start(status: &str, body: String) -> HttpStub {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("stub binds");
        let address = listener.local_addr().expect("stub address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let status = status.to_owned();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(answer_once(
                    stream,
                    status.clone(),
                    body.clone(),
                    seen.clone(),
                ));
            }
        });
        HttpStub { address, requests }
    }

    fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn request_lines(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

async fn answer_once(
    mut stream: TcpStream,
    status: String,
    body: String,
    seen: Arc<Mutex<Vec<String>>>,
) {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
    let request_line = String::from_utf8_lossy(&head)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request_line);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn silent_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("silent server binds");
    let address = listener.local_addr().expect("silent server address");
    let task = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    (address, task)
}

async fn closed_port() -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("probe port binds");
    listener.local_addr().expect("probe port address")
}

async fn next_message(client: &mut Client) -> Message {
    tokio::time::timeout(RECEIVE_WAIT, client.next())
        .await
        .expect("a frame arrives within 5 s")
        .expect("the socket is still open")
        .expect("the frame is readable")
}

async fn next_json(client: &mut Client) -> Value {
    match next_message(client).await {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("text frame is JSON"),
        other => panic!("expected a text frame, got {other:?}"),
    }
}

fn http_status(result: Result<(Client, impl std::fmt::Debug), WsError>) -> u16 {
    match result {
        Err(WsError::Http(response)) => response.status().as_u16(),
        Ok(_) => panic!("the upgrade was accepted"),
        Err(other) => panic!("expected an HTTP rejection, got {other}"),
    }
}

#[tokio::test]
async fn json_version_200_gives_port_mode_with_the_requested_host() {
    let server = mock(MockJsonVersion::Ok).await;
    let port = server.address.port();

    let exact = resolve_endpoint_with(&format!("127.0.0.1:{port}"), BrowserKind::Chrome, None)
        .await
        .expect("port mode resolves");
    assert_eq!(
        exact,
        AttachEndpoint::PortMode {
            ws_url: server.ws_url()
        }
    );
    assert!(!exact.needs_approval());

    let rewritten = resolve_endpoint_with(&format!("localhost:{port}"), BrowserKind::Edge, None)
        .await
        .expect("localhost resolves");
    assert_eq!(
        rewritten.ws_url(),
        format!("ws://localhost:{port}/devtools/browser/mock")
    );

    let with_scheme = resolve_endpoint_with(
        &format!(" http://127.0.0.1:{port}/ "),
        BrowserKind::Chrome,
        None,
    )
    .await
    .expect("an http address resolves");
    assert_eq!(with_scheme.ws_url(), server.ws_url());
    assert!(server.control.commands_seen().is_empty());
}

#[tokio::test]
async fn json_version_host_and_port_are_rewritten_to_the_address_asked() {
    let reported = json!({
        "Browser": "Chrome/154.0.8037.92",
        "webSocketDebuggerUrl": "ws://10.9.8.7:1/devtools/browser/abc",
    });
    let stub = HttpStub::start("200 OK", reported.to_string()).await;

    let endpoint = resolve_endpoint_with(&stub.address.to_string(), BrowserKind::Chrome, None)
        .await
        .expect("port mode resolves");

    assert_eq!(
        endpoint,
        AttachEndpoint::PortMode {
            ws_url: format!("ws://{}/devtools/browser/abc", stub.address)
        }
    );
    assert_eq!(stub.request_lines(), ["GET /json/version HTTP/1.1"]);
}

#[tokio::test]
async fn json_version_404_is_the_consent_endpoint_and_fails_fast() {
    let server = mock(MockJsonVersion::NotFound).await;
    let address = server.address.to_string();

    assert_eq!(
        resolve_error(&address, BrowserKind::Chrome, None).await,
        format!(
            "{address} is the remote-debugging consent endpoint of your everyday Chrome profile, not a dedicated debugging browser. To let this flow control your everyday Chrome, clear Debugger Address."
        )
    );
    assert_eq!(
        resolve_error(&address, BrowserKind::Edge, None).await,
        format!(
            "{address} is the remote-debugging consent endpoint of your everyday Edge profile, not a dedicated debugging browser. To let this flow control your everyday Edge, clear Debugger Address."
        )
    );
    assert!(
        server.control.commands_seen().is_empty(),
        "no WebSocket may be opened after a 404"
    );
}

#[tokio::test]
async fn refused_port_explains_how_to_start_a_debugging_browser() {
    let server = mock(MockJsonVersion::Refuse).await;
    let address = server.address.to_string();

    assert_eq!(
        resolve_error(&address, BrowserKind::Chrome, None).await,
        format!(
            "Nothing is listening on {address}. Start Chrome with --remote-debugging-port and its own --user-data-dir, or clear Debugger Address to use your everyday Chrome after enabling chrome://inspect/#remote-debugging."
        )
    );
    let http_address = format!("http://{address}");
    assert_eq!(
        resolve_error(&http_address, BrowserKind::Edge, None).await,
        format!(
            "Nothing is listening on {http_address}. Start Edge with --remote-debugging-port and its own --user-data-dir, or clear Debugger Address to use your everyday Edge after enabling chrome://inspect/#remote-debugging."
        )
    );
}

#[tokio::test]
async fn unexpected_json_version_replies_name_the_problem() {
    let no_url = HttpStub::start("200 OK", json!({"Browser": "Chrome/154"}).to_string()).await;
    let message = resolve_error(&no_url.address.to_string(), BrowserKind::Chrome, None).await;
    assert!(message.contains("has no webSocketDebuggerUrl"), "{message}");

    let not_json = HttpStub::start("200 OK", "<html>".to_owned()).await;
    let message = resolve_error(&not_json.address.to_string(), BrowserKind::Chrome, None).await;
    assert!(message.contains("is not JSON"), "{message}");

    let host_check = "Host header is specified and is not an IP address or localhost.";
    let failing = HttpStub::start("500 Internal Server Error", host_check.to_owned()).await;
    let message = resolve_error(&failing.address.to_string(), BrowserKind::Chrome, None).await;
    assert!(message.contains("answered HTTP 500"), "{message}");
    assert!(message.contains(host_check), "{message}");

    let message = resolve_error("ftp://127.0.0.1:9222", BrowserKind::Chrome, None).await;
    assert!(message.contains("is not host:port"), "{message}");
}

#[tokio::test]
async fn empty_address_reads_devtools_active_port_from_the_given_profile() {
    let profile = tempfile::tempdir().expect("temp profile");
    let file = profile.path().join("DevToolsActivePort");
    std::fs::write(
        &file,
        "50397\n/devtools/browser/6730c76a-94bf-48a2-be91-b9f367985480",
    )
    .expect("write DevToolsActivePort");

    let endpoint = resolve_endpoint_with("  ", BrowserKind::Chrome, Some(profile.path()))
        .await
        .expect("approval mode resolves");

    assert_eq!(
        endpoint,
        AttachEndpoint::ApprovalMode {
            ws_url: "ws://127.0.0.1:50397/devtools/browser/6730c76a-94bf-48a2-be91-b9f367985480"
                .to_owned(),
            handshake_timeout: APPROVAL_TIMEOUT,
        }
    );
    assert!(endpoint.needs_approval());
    assert!(file.exists(), "DevToolsActivePort is never deleted");
}

#[tokio::test]
async fn empty_address_without_active_port_asks_to_enable_remote_debugging() {
    let profile = tempfile::tempdir().expect("temp profile");
    let chrome = "Remote debugging is not enabled in Chrome. Open chrome://inspect/#remote-debugging and turn on 'Allow remote debugging for this browser instance', then run again.";

    assert_eq!(
        resolve_error("", BrowserKind::Chrome, Some(profile.path())).await,
        chrome
    );
    assert_eq!(resolve_error("", BrowserKind::Chrome, None).await, chrome);
    assert_eq!(
        resolve_error("", BrowserKind::Edge, None).await,
        chrome.replace("in Chrome.", "in Edge.")
    );

    std::fs::write(profile.path().join("DevToolsActivePort"), "").expect("write empty file");
    assert_eq!(
        resolve_error("", BrowserKind::Chrome, Some(profile.path())).await,
        chrome
    );
}

#[test]
fn devtools_active_port_is_parsed_strictly() {
    let profile = tempfile::tempdir().expect("temp profile");
    let file = profile.path().join("DevToolsActivePort");
    assert_eq!(
        read_devtools_active_port(profile.path()).expect("readable"),
        None
    );

    std::fs::write(&file, "9222\n/devtools/browser/abc\n").expect("write");
    assert_eq!(
        read_devtools_active_port(profile.path()).expect("valid file"),
        Some((9222, "/devtools/browser/abc".to_owned()))
    );

    for malformed in [
        "9222",
        "port\n/devtools/browser/abc",
        "9222\n/devtools/page/abc",
    ] {
        std::fs::write(&file, malformed).expect("write");
        match read_devtools_active_port(profile.path()) {
            Err(BrowserError::Connect { message }) => {
                assert!(message.contains(&file.display().to_string()), "{message}");
            }
            other => panic!("{malformed:?} must be rejected, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn websocket_addresses_pick_approval_or_direct_mode() {
    let approval = [
        "ws://127.0.0.1:9222/devtools/browser/abc",
        "ws://localhost:9222/devtools/browser",
        "ws://[::1]:9222/devtools/browser/abc",
        "WS://127.0.0.2:9222/devtools/browser/abc",
        "wss://localhost:9222/devtools/browser/abc",
    ];
    for address in approval {
        let endpoint = resolve_endpoint_with(address, BrowserKind::Chrome, None)
            .await
            .expect("ws address resolves");
        assert_eq!(
            endpoint,
            AttachEndpoint::ApprovalMode {
                ws_url: address.to_owned(),
                handshake_timeout: APPROVAL_TIMEOUT,
            },
            "{address}"
        );
    }

    let direct = [
        "ws://127.0.0.1:9222/devtools/page/ABC",
        "ws://192.168.1.20:9222/devtools/browser/abc",
        "wss://cdp.example.com/devtools/browser/abc?token=secret",
        "wss://cdp.example.com?token=secret",
    ];
    for address in direct {
        let endpoint = resolve_endpoint_with(address, BrowserKind::Edge, None)
            .await
            .expect("ws address resolves");
        assert_eq!(
            endpoint,
            AttachEndpoint::DirectWs {
                ws_url: address.to_owned(),
                handshake_timeout: DIRECT_TIMEOUT,
            },
            "{address}"
        );
        assert!(!endpoint.needs_approval());
    }

    let message = resolve_error("ws://", BrowserKind::Chrome, None).await;
    assert!(message.contains("not a valid WebSocket URL"), "{message}");
}

#[test]
fn loopback_covers_empty_localhost_127_8_and_ipv6_one() {
    for url in [
        "",
        "  ",
        "http://localhost:9515",
        "http://LOCALHOST:9515/wd/hub",
        "localhost:9515",
        "http://127.0.0.1:9515",
        "http://127.10.20.30:4444",
        "http://[::1]:9515",
        "https://localhost",
    ] {
        assert!(is_loopback(url), "{url:?} is loopback");
    }
    for url in [
        "http://192.168.1.10:9515",
        "http://selenium-hub:4444/wd/hub",
        "http://[::2]:9515",
        "http://localhost.example.com:9515",
        "http://128.0.0.1:9515",
        "::not a url::",
    ] {
        assert!(!is_loopback(url), "{url:?} is not loopback");
    }
}

#[tokio::test]
async fn probe_recognises_chromedriver_and_msedgedriver() {
    let bodies = [
        json!({"value": {"build": {"version": "154.0.8037.92"}, "message": "ChromeDriver ready for new sessions.", "os": {"name": "Mac OS X"}, "ready": true}}),
        json!({"value": {"message": "msedgedriver ready for new sessions.", "ready": true}}),
        json!({"value": {"message": "Microsoft Edge WebDriver ready for new sessions.", "ready": false}}),
    ];
    for body in bodies {
        let stub = HttpStub::start("200 OK", body.to_string()).await;
        assert_eq!(
            probe_webdriver(&stub.url()).await,
            WebDriverProbe::Chromedriver
        );
        assert_eq!(stub.request_lines(), ["GET /status HTTP/1.1"]);
    }
}

#[tokio::test]
async fn probe_names_other_webdriver_servers() {
    let grid = json!({"value": {"ready": true, "message": "Selenium Grid ready.", "nodes": []}});
    let stub = HttpStub::start("200 OK", grid.to_string()).await;
    assert_eq!(
        probe_webdriver(&format!("{}/wd/hub/", stub.url())).await,
        WebDriverProbe::Other {
            name: "Selenium Grid ready.".to_owned()
        }
    );
    assert_eq!(stub.request_lines(), ["GET /wd/hub/status HTTP/1.1"]);

    let gecko = json!({"value": {"message": "", "ready": true}});
    let stub = HttpStub::start("200 OK", gecko.to_string()).await;
    assert_eq!(
        probe_webdriver(&stub.address.to_string()).await,
        WebDriverProbe::Other {
            name: "WebDriver".to_owned()
        }
    );

    let busy = json!({"value": {"message": "Session already started", "ready": false}});
    let stub = HttpStub::start("500 Internal Server Error", busy.to_string()).await;
    assert_eq!(
        probe_webdriver(&stub.url()).await,
        WebDriverProbe::Other {
            name: "Session already started".to_owned()
        }
    );
}

#[tokio::test]
async fn probe_treats_refusal_silence_and_non_webdriver_replies_as_not_running() {
    assert_eq!(probe_webdriver("").await, WebDriverProbe::NotRunning);

    let refused = closed_port().await;
    assert_eq!(
        probe_webdriver(&format!("http://{refused}")).await,
        WebDriverProbe::NotRunning
    );

    let (silent, silent_task) = silent_server().await;
    let started = tokio::time::Instant::now();
    assert_eq!(
        probe_webdriver(&format!("http://{silent}")).await,
        WebDriverProbe::NotRunning
    );
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the probe gives up after about 1 s, took {:?}",
        started.elapsed()
    );
    silent_task.abort();

    let html = HttpStub::start("200 OK", "<html>not json</html>".to_owned()).await;
    assert_eq!(
        probe_webdriver(&html.url()).await,
        WebDriverProbe::NotRunning
    );

    let other_json = HttpStub::start("200 OK", json!({"status": "ok"}).to_string()).await;
    assert_eq!(
        probe_webdriver(&other_json.url()).await,
        WebDriverProbe::NotRunning
    );
}

async fn connected_mock() -> (MockCdpServer, Client) {
    let server = mock(MockJsonVersion::Ok).await;
    let (client, _) = tokio_tungstenite::connect_async(server.ws_url())
        .await
        .expect("the mock accepts the upgrade");
    (server, client)
}

async fn send_json(client: &mut Client, frame: Value) {
    client
        .send(Message::text(frame.to_string()))
        .await
        .expect("send a text frame");
}

#[tokio::test]
async fn mock_server_bridges_commands_and_replies() {
    let (mut server, mut client) = connected_mock().await;

    send_json(
        &mut client,
        json!({"id": 1, "method": "Browser.getVersion", "params": {}}),
    )
    .await;
    let command = server.control.expect("Browser.getVersion").await;
    assert_eq!(command.id, 1);
    server
        .control
        .reply(&command, json!({"product": "Chrome/154.0.8037.92"}));
    assert_eq!(
        next_json(&mut client).await,
        json!({"id": 1, "result": {"product": "Chrome/154.0.8037.92"}})
    );

    server.control.set_auto_reply(default_auto_reply);
    send_json(
        &mut client,
        json!({"id": 2, "method": "Page.enable", "sessionId": "S1"}),
    )
    .await;
    assert_eq!(
        next_json(&mut client).await,
        json!({"id": 2, "result": {}, "sessionId": "S1"})
    );
    assert_eq!(server.control.commands_seen().len(), 2);
}

#[tokio::test]
async fn mock_server_forwards_events_binary_frames_and_close() {
    let (server, mut client) = connected_mock().await;

    let created = json!({"targetInfo": {"targetId": "T1"}});
    server
        .control
        .emit("Target.targetCreated", None, created.clone());
    assert_eq!(
        next_json(&mut client).await,
        json!({"method": "Target.targetCreated", "params": created})
    );

    server.control.send_invalid_utf8(&[0xff, 0xfe]);
    match next_message(&mut client).await {
        Message::Binary(bytes) => assert_eq!(bytes.to_vec(), vec![0xff, 0xfe]),
        other => panic!("expected a binary frame, got {other:?}"),
    }

    server.control.close("browser went away");
    match next_message(&mut client).await {
        Message::Close(Some(frame)) => {
            assert_eq!(frame.code, CloseCode::Normal);
            assert_eq!(frame.reason.as_str(), "browser went away");
        }
        other => panic!("expected a close frame, got {other:?}"),
    }
}

#[tokio::test]
async fn mock_server_bridges_one_connection_on_browser_paths_only() {
    let server = mock(MockJsonVersion::Ok).await;
    let page_path = format!("ws://{}/devtools/page/ABC", server.address);
    assert_eq!(
        http_status(tokio_tungstenite::connect_async(page_path).await),
        403
    );

    let first = tokio_tungstenite::connect_async(server.ws_url())
        .await
        .expect("the first connection is bridged");
    assert_eq!(
        http_status(tokio_tungstenite::connect_async(server.ws_url()).await),
        409
    );
    drop(first);
}

#[tokio::test]
async fn mock_server_checks_the_expected_header() {
    let server = MockCdpServer::start(MockServerOptions {
        json_version: MockJsonVersion::NotFound,
        expect_header: Some(("Authorization".to_owned(), "Bearer mock-token".to_owned())),
    })
    .await;

    assert_eq!(
        http_status(tokio_tungstenite::connect_async(server.ws_url()).await),
        401
    );

    let mut request = server.ws_url().into_client_request().expect("request");
    request.headers_mut().insert(
        "authorization",
        "Bearer wrong".parse().expect("header value"),
    );
    assert_eq!(
        http_status(tokio_tungstenite::connect_async(request).await),
        401
    );

    let mut request = server.ws_url().into_client_request().expect("request");
    request.headers_mut().insert(
        "authorization",
        "Bearer mock-token".parse().expect("header value"),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .expect("the expected header is accepted");
}

#[tokio::test]
async fn ws_transport_and_connection_drive_the_scripted_browser() {
    let mut server = MockCdpServer::start(MockServerOptions {
        json_version: MockJsonVersion::Ok,
        expect_header: Some(("Authorization".to_owned(), "Bearer mock-token".to_owned())),
    })
    .await;
    let options = ConnectOptions {
        headers: vec![("Authorization".to_owned(), "Bearer mock-token".to_owned())],
        ..ConnectOptions::new(server.ws_url())
    };
    let transport = WsTransport::connect(&options)
        .await
        .expect("WsTransport passes the header check and upgrades");
    let connection = Connection::start(Box::new(transport), ConnectionOptions::default());

    let request = tokio::spawn({
        let connection = connection.clone();
        async move {
            connection
                .send_raw("Browser.getVersion", json!({}), None)
                .await
        }
    });
    let command = server.control.expect("Browser.getVersion").await;
    server
        .control
        .reply(&command, json!({"product": "Chrome/154.0.8037.92"}));
    let result = request
        .await
        .expect("request task joins")
        .expect("the scripted reply arrives through the WebSocket");
    assert_eq!(result, json!({"product": "Chrome/154.0.8037.92"}));

    server.control.close("browser went away");
    tokio::time::timeout(RECEIVE_WAIT, connection.closed())
        .await
        .expect("the connection sees the close within 5 s");
    let reason = connection.closed_reason().unwrap_or_default();
    assert!(reason.contains("browser went away"), "{reason}");
}

fn page_target() -> Value {
    json!({
        "targetId": "T1",
        "type": "page",
        "title": "Mock tab",
        "url": "about:blank",
        "attached": false,
        "browserContextId": "C1",
    })
}

fn scripted_browser_reply(command: &SentCommand) -> Option<Value> {
    match command.method.as_str() {
        "Browser.getVersion" => Some(json!({
            "protocolVersion": "1.3",
            "product": "Chrome/154.0.8037.92",
            "revision": "@334b65d254ccc35df4fca82706d1753227b01039",
            "userAgent": "Mozilla/5.0 HeadlessChrome/154.0.0.0",
            "jsVersion": "15.4.80.19",
        })),
        "Target.getTargets" => Some(json!({"targetInfos": [page_target()]})),
        _ => default_auto_reply(command),
    }
}

fn assert_attach_left_tabs_alone(seen: &[SentCommand]) {
    let sent = |method: &str| seen.iter().any(|command| command.method == method);
    assert!(
        sent("Browser.getVersion"),
        "construction asks for the version"
    );
    assert!(
        !sent("Target.attachToTarget"),
        "attached browsers never bulk-attach tabs"
    );
    assert!(
        !seen
            .iter()
            .any(|command| command.method == "Target.setAutoAttach" && command.session.is_none()),
        "no browser-level auto-attach"
    );
}

#[tokio::test]
async fn browser_attach_in_port_mode_completes_construction_over_one_websocket() {
    let mut server = mock(MockJsonVersion::Ok).await;
    server.control.set_auto_reply(scripted_browser_reply);

    let endpoint = resolve_endpoint_with(&server.address.to_string(), BrowserKind::Chrome, None)
        .await
        .expect("port mode resolves");
    let attach = tokio::spawn(Browser::attach(endpoint));
    let discover = server
        .control
        .wait_for("Target.setDiscoverTargets", None)
        .await;
    server.control.emit(
        "Target.targetCreated",
        None,
        json!({"targetInfo": page_target()}),
    );
    server.control.reply(&discover, json!({}));

    let browser = tokio::time::timeout(Duration::from_secs(10), attach)
        .await
        .expect("attach finishes within 10 s")
        .expect("attach task joins")
        .expect("attach succeeds over the single bridged WebSocket");

    assert_eq!(browser.kind(), ConnectionKind::AttachedPort);
    assert!(!browser.is_owned());
    assert_eq!(browser.version().product, "Chrome/154.0.8037.92");
    let pages = browser.pages();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].target_id.as_str(), "T1");
    assert_attach_left_tabs_alone(&server.control.commands_seen());
    browser.close().await.expect("close disconnects");
}

async fn attach_error(endpoint: AttachEndpoint) -> String {
    match Browser::attach(endpoint).await {
        Err(BrowserError::Connect { message }) => message,
        Err(other) => panic!("expected BrowserError::Connect, got {other:?}"),
        Ok(_) => panic!("attach succeeded on a path the browser refuses"),
    }
}

#[tokio::test]
async fn an_approval_handshake_refused_with_403_says_allow_was_not_clicked() {
    let server = mock(MockJsonVersion::NotFound).await;
    let refused = format!("ws://{}/devtools/page/ABC", server.address);

    let approval = attach_error(AttachEndpoint::ApprovalMode {
        ws_url: refused.clone(),
        handshake_timeout: RECEIVE_WAIT,
    })
    .await;
    assert_eq!(
        approval,
        "The browser refused the connection (Allow was not clicked, or no browser window was open)"
    );

    let direct = attach_error(AttachEndpoint::DirectWs {
        ws_url: refused,
        handshake_timeout: RECEIVE_WAIT,
    })
    .await;
    assert!(direct.contains("rejected: HTTP 403"), "{direct}");
}

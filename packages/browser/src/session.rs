use crate::connection::{Connection, PendingReply, Reply, Request};
use crate::transport::WriteTicket;
use crate::types::SessionId;

#[derive(Clone)]
pub struct Session {
    connection: Connection,
    id: Option<SessionId>,
}

impl Session {
    pub fn new(connection: Connection, id: Option<SessionId>) -> Self {
        Self { connection, id }
    }

    pub fn id(&self) -> Option<&SessionId> {
        self.id.as_ref()
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn enqueue(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Option<std::time::Duration>,
    ) -> crate::Result<PendingReply> {
        self.connection.enqueue(Request {
            method: method.to_owned().into(),
            params,
            session: self.id.clone(),
            timeout,
        })
    }

    pub async fn send(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> crate::Result<serde_json::Value> {
        self.connection
            .send_raw(method, params, self.id.as_ref())
            .await
    }

    pub async fn send_with_timeout(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: std::time::Duration,
    ) -> crate::Result<serde_json::Value> {
        Ok(self.enqueue(method, params, Some(timeout))?.await?.result)
    }

    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Option<std::time::Duration>,
    ) -> crate::Result<Reply> {
        self.enqueue(method, params, timeout)?.await
    }

    pub fn send_nowait(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> crate::Result<WriteTicket> {
        self.connection
            .send_nowait(method, params, self.id.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ConnectionOptions;
    use crate::error::BrowserError;
    use crate::transport::memory::InMemoryTransport;
    use serde_json::json;
    use std::time::Duration;

    #[tokio::test]
    async fn session_commands_carry_the_session_id_and_the_reply_cursor() {
        let (transport, mut control) = InMemoryTransport::new();
        let connection = Connection::start(Box::new(transport), ConnectionOptions::default());
        let page = connection.session(Some(SessionId::from("S1")));
        assert_eq!(page.id().map(SessionId::as_str), Some("S1"));
        let cursor = connection.events().cursor();
        let call = tokio::spawn({
            let page = page.clone();
            async move { page.request("Page.getFrameTree", json!({}), None).await }
        });
        let command = control.expect("Page.getFrameTree").await;
        assert_eq!(command.session.as_deref(), Some("S1"));
        control.emit(
            "Page.frameNavigated",
            Some("S1"),
            json!({"type": "Navigation"}),
        );
        control.reply(&command, json!({"frameTree": {}}));
        let reply = call.await.unwrap().unwrap();
        assert_eq!(reply.result, json!({"frameTree": {}}));
        assert_eq!(reply.cursor, cursor);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let navigated = connection
            .events()
            .wait_for(reply.cursor, deadline, |event| {
                &*event.method == "Page.frameNavigated"
            })
            .await
            .unwrap()
            .unwrap();
        assert!(navigated.seq >= reply.cursor.0);
    }

    #[tokio::test]
    async fn root_sessions_send_without_a_session_id_and_honour_timeouts() {
        let (transport, mut control) = InMemoryTransport::new();
        let connection = Connection::start(Box::new(transport), ConnectionOptions::default());
        let root = connection.session(None);
        assert!(root.id().is_none());
        assert!(!root.connection().is_closed());
        let timed = root
            .send_with_timeout("Browser.getVersion", json!({}), Duration::from_millis(20))
            .await;
        assert!(matches!(
            timed,
            Err(BrowserError::Timeout { method, timeout_ms: 20 }) if method == "Browser.getVersion"
        ));
        assert_eq!(control.next_command().await.session, None);
        root.send_nowait("Target.setDiscoverTargets", json!({"discover": true}))
            .unwrap();
        let discover = control.expect("Target.setDiscoverTargets").await;
        assert_eq!(discover.params["discover"], true);
        let call = tokio::spawn({
            let root = root.clone();
            async move { root.send("Target.getTargets", json!({})).await }
        });
        let command = control.expect("Target.getTargets").await;
        control.reply(&command, json!({"targetInfos": []}));
        assert_eq!(call.await.unwrap().unwrap()["targetInfos"], json!([]));
    }
}

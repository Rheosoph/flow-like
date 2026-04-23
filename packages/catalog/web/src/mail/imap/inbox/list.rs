use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use chrono::{DateTime, Utc};
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{
    async_trait,
    json::{from_slice, json},
};
#[cfg(feature = "execute")]
use futures::TryStreamExt;
#[cfg(feature = "execute")]
use mail_parser::{MessageParser, MimeHeaders};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::mail::imap::{ImapConnection, inbox::ImapInbox};

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[cfg(feature = "execute")]
fn parse_mail_addresses(addresses: &mail_parser::Address<'_>) -> Vec<MailAddress> {
    addresses
        .iter()
        .map(|addr: &mail_parser::Addr<'_>| MailAddress {
            name: addr.name().map(|s| s.to_string()),
            email: addr.address().unwrap_or_default().to_string(),
        })
        .collect()
}

#[cfg(feature = "execute")]
fn format_imap_search_date(date: DateTime<Utc>) -> String {
    date.format("%d-%b-%Y").to_string()
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct MailAddress {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct EmailRef {
    pub connection: ImapConnection,
    pub inbox: ImapInbox,
    pub uid: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Email {
    pub connection: ImapConnection,
    pub inbox: ImapInbox,
    pub uid: u32,

    pub from: Option<MailAddress>,
    #[serde(default)]
    pub sender: Vec<MailAddress>,
    #[serde(default)]
    pub to: Vec<MailAddress>,
    #[serde(default)]
    pub cc: Vec<MailAddress>,
    #[serde(default)]
    pub bcc: Vec<MailAddress>,
    pub subject: Option<String>,
    pub date: Option<String>,

    pub plain: Option<String>,
    pub html: Option<String>,

    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

impl EmailRef {
    pub fn new(connection: ImapConnection, inbox: ImapInbox, uid: u32) -> Self {
        EmailRef {
            connection,
            inbox,
            uid,
        }
    }

    #[cfg(feature = "execute")]
    pub async fn fetch(&self, context: &mut ExecutionContext) -> flow_like_types::Result<Email> {
        let inbox = self.inbox.clone();
        let session_arc = self.connection.to_session(context).await?;
        let mut session = session_arc.lock().await;
        session.select(&inbox.name).await.map_err(|e| anyhow!(e))?;

        let fetch = session
            .uid_fetch(self.uid.to_string(), "BODY.PEEK[]")
            .await
            .map_err(|e| anyhow!(e))?
            .try_collect::<Vec<_>>()
            .await?;

        let msg = fetch.first().ok_or_else(|| anyhow!("No message fetched"))?;
        let bytes = msg.body().ok_or_else(|| anyhow!("No body in fetch"))?;

        let mail = MessageParser::default()
            .parse(bytes)
            .ok_or_else(|| anyhow!("Failed to parse email body, possibly invalid MIME format"))?;

        let mail = Email {
            connection: self.connection.clone(),
            inbox: inbox.clone(),
            uid: self.uid,
            from: mail
                .from()
                .iter()
                .flat_map(|addr| parse_mail_addresses(addr))
                .next(),
            sender: mail
                .sender()
                .iter()
                .flat_map(|addr| parse_mail_addresses(addr))
                .collect(),
            to: mail
                .to()
                .iter()
                .flat_map(|addr| parse_mail_addresses(addr))
                .collect(),
            cc: mail
                .cc()
                .iter()
                .flat_map(|addr| parse_mail_addresses(addr))
                .collect(),
            bcc: mail
                .bcc()
                .iter()
                .flat_map(|addr| parse_mail_addresses(addr))
                .collect(),
            subject: mail.subject().map(|s| s.to_string()),
            date: mail.date().map(|d| d.to_rfc3339()).map(|s| s.to_string()),
            plain: mail.body_text(0).map(|s| s.to_string()),
            html: mail.body_html(0).map(|s| s.to_string()),
            attachments: mail
                .attachments()
                .map(|part: &mail_parser::MessagePart<'_>| {
                    let filename = part.attachment_name().map(|s| s.to_string());

                    let content_type = part
                        .content_type()
                        .map(|ct| match ct.subtype() {
                            Some(sub) => format!("{}/{}", ct.ctype(), sub),
                            None => ct.ctype().to_string(),
                        })
                        .unwrap_or_else(|| "application/octet-stream".to_string());

                    let data = part.contents().to_vec();

                    Attachment {
                        filename,
                        content_type,
                        data,
                    }
                })
                .collect::<Vec<_>>(),
        };

        Ok(mail)
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListMailsNode;

impl ListMailsNode {
    pub fn new() -> Self {
        ListMailsNode
    }
}

#[async_trait]
impl NodeLogic for ListMailsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_list",
            "List Mails",
            "Lists email UIDs for a mailbox page with selectable filters",
            "Email/IMAP",
        );
        node.add_icon("/flow/icons/mail.svg");
        node.set_version(2);

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin("inbox", "Inbox", "Mailbox name", VariableType::Struct)
            .set_schema::<ImapInbox>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Search filter selection
        node.add_input_pin(
            "filter",
            "Filter",
            "IMAP search filter",
            VariableType::String,
        )
        .set_default_value(Some(json!("NEW")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "ALL".to_string(),
                    "NEW".to_string(),
                    "UNSEEN".to_string(),
                    "SEEN".to_string(),
                    "SINCE".to_string(),
                    "BEFORE".to_string(),
                    "FROM".to_string(),
                    "TO".to_string(),
                    "SUBJECT".to_string(),
                    "BODY".to_string(),
                ])
                .build(),
        );

        // Emails output
        node.add_output_pin(
            "emails",
            "Email References",
            "List of email references",
            VariableType::Struct,
        )
        .set_schema::<Vec<EmailRef>>()
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let inbox: ImapInbox = context.evaluate_pin("inbox").await?;
        let connection = inbox.connection.clone();
        let filter: String = context.evaluate_pin("filter").await?;

        // Build search criteria
        let mut search_criteria = filter.clone();
        match filter.as_str() {
            "SINCE" => {
                let since_date: DateTime<Utc> = context.evaluate_pin("since_date").await?;
                search_criteria = format!("SINCE {}", format_imap_search_date(since_date));
            }
            "BEFORE" => {
                let before_date: DateTime<Utc> = context.evaluate_pin("before_date").await?;
                search_criteria =
                    format!("BEFORE {}", format_imap_search_date(before_date));
            }
            "FROM" => {
                let from_addr: String = context.evaluate_pin("from").await?;
                search_criteria = format!("FROM \"{}\"", from_addr);
            }
            "TO" => {
                let to_addr: String = context.evaluate_pin("to").await?;
                search_criteria = format!("TO \"{}\"", to_addr);
            }
            "SUBJECT" => {
                let subj: String = context.evaluate_pin("subject").await?;
                search_criteria = format!("SUBJECT \"{}\"", subj);
            }
            "BODY" => {
                let body_text: String = context.evaluate_pin("body").await?;
                search_criteria = format!("BODY \"{}\"", body_text);
            }
            _ => {}
        }
        let session_arc = connection.to_session(context).await?;
        let mut session = session_arc.lock().await;
        session.select(&inbox.name).await.map_err(|e| anyhow!(e))?;
        let uids = session
            .uid_search(&search_criteria)
            .await
            .map_err(|e| anyhow!(e))?;

        let emails = uids
            .iter()
            .map(|&uid| EmailRef::new(connection.clone(), inbox.clone(), uid))
            .collect::<Vec<_>>();

        context.set_pin_value("emails", json!(emails)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;

        let filter = node
            .get_pin_by_name("filter")
            .and_then(|p| p.default_value.clone())
            .and_then(|v| from_slice::<String>(&v).ok())
            .unwrap_or_else(|| "NEW".to_string());

        let dynamic_map = [
            (
                "SINCE",
                "since_date",
                "Since Date",
                "Search emails since this date; IMAP will receive it as 21-Apr-2026",
                VariableType::Date,
            ),
            (
                "BEFORE",
                "before_date",
                "Before Date",
                "Search emails before this date; IMAP will receive it as 21-Apr-2026",
                VariableType::Date,
            ),
            (
                "FROM",
                "from",
                "From",
                "Search emails from this sender",
                VariableType::String,
            ),
            (
                "TO",
                "to",
                "To",
                "Search emails to this recipient",
                VariableType::String,
            ),
            (
                "SUBJECT",
                "subject",
                "Subject",
                "Search emails with this subject",
                VariableType::String,
            ),
            (
                "BODY",
                "body",
                "Body",
                "Search emails with this text in the body",
                VariableType::String,
            ),
        ];

        // Compute sets of needed vs existing dynamic pins
        let needed: std::collections::HashSet<&str> = dynamic_map
            .iter()
            .filter(|&&(f, _, _, _, _)| f == filter)
            .map(|&(_, name, _, _, _)| name)
            .collect();
        let existing: std::collections::HashSet<String> = node
            .pins
            .iter()
            .filter_map(|(_, pin)| {
                dynamic_map.iter().find_map(|&(_, name, _, _, _)| {
                    if pin.name == name {
                        Some(name.to_string())
                    } else {
                        None
                    }
                })
            })
            .collect();

        // Remove pins no longer needed
        for obsolete in existing.difference(&needed.iter().map(|s| s.to_string()).collect()) {
            node.pins.retain(|_, pin| pin.name != *obsolete);
        }

        // Add any missing pins
        for &name in needed.difference(&existing.iter().map(AsRef::as_ref).collect()) {
            if let Some((_, _, label, description, data_type)) =
                dynamic_map.iter().find(|(_, n, _, _, _)| *n == name)
            {
                node.add_input_pin(name, label, description, data_type.clone());
            }
        }
    }
}

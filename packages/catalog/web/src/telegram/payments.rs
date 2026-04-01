//! Telegram payment operations

use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    json::{from_str, json},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::{PreCheckoutQueryId, ShippingQueryId, TelegramTransactionId, UserId};

/// Labeled price for invoices
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LabeledPrice {
    /// Portion label
    pub label: String,
    /// Price amount in smallest units (e.g., cents)
    pub amount: i64,
}

/// Star transaction information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StarTransaction {
    /// Unique identifier of the transaction
    pub id: String,
    /// Number of Telegram Stars transferred
    pub amount: i64,
    /// Date of the transaction in Unix timestamp
    pub date: i64,
    /// Source of incoming transaction (optional)
    pub source: Option<String>,
    /// Receiver of outgoing transaction (optional)
    pub receiver: Option<String>,
}

/// Invoice link result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InvoiceLink {
    /// The created invoice link URL
    pub url: String,
}

// ============================================================================
// Send Invoice Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendInvoiceNode;

impl SendInvoiceNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendInvoiceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_invoice",
            "Send Invoice",
            "Sends an invoice for payment to a chat",
            "Telegram/Payments",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "title",
            "Title",
            "Product name (1-32 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "description",
            "Description",
            "Product description (1-255 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "payload",
            "Payload",
            "Bot-defined invoice payload (1-128 bytes)",
            VariableType::String,
        );

        node.add_input_pin(
            "provider_token",
            "Provider Token",
            "Payment provider token (empty string for payments in Telegram Stars)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "currency",
            "Currency",
            "Three-letter ISO 4217 currency code (e.g., USD, EUR, or XTR for Telegram Stars)",
            VariableType::String,
        )
        .set_default_value(Some(json!("XTR")));

        node.add_input_pin(
            "prices",
            "Prices",
            "JSON array of price portions (LabeledPrice objects)",
            VariableType::Struct,
        )
        .set_schema::<Vec<LabeledPrice>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "max_tip_amount",
            "Max Tip Amount",
            "Maximum accepted tip amount in smallest currency units",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "suggested_tip_amounts",
            "Suggested Tip Amounts",
            "JSON array of suggested tip amounts in smallest currency units",
            VariableType::String,
        )
        .set_default_value(Some(json!("[]")));

        node.add_input_pin(
            "start_parameter",
            "Start Parameter",
            "Deep-linking parameter for sharing the invoice",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "provider_data",
            "Provider Data",
            "JSON data about the invoice for the payment provider",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "photo_url",
            "Photo URL",
            "URL of the product photo",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "photo_size",
            "Photo Size",
            "Photo size in bytes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "photo_width",
            "Photo Width",
            "Photo width",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "photo_height",
            "Photo Height",
            "Photo height",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "need_name",
            "Need Name",
            "Request user's full name",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "need_phone_number",
            "Need Phone Number",
            "Request user's phone number",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "need_email",
            "Need Email",
            "Request user's email",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "need_shipping_address",
            "Need Shipping Address",
            "Request user's shipping address",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "send_phone_number_to_provider",
            "Send Phone to Provider",
            "Send user's phone number to provider",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "send_email_to_provider",
            "Send Email to Provider",
            "Send user's email to provider",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "is_flexible",
            "Is Flexible",
            "Pass true if final price depends on shipping method",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after invoice is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "message_id",
            "Message ID",
            "ID of the sent invoice message",
            VariableType::Integer,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the invoice was sent successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let title: String = context.evaluate_pin("title").await?;
        let description: String = context.evaluate_pin("description").await?;
        let payload: String = context.evaluate_pin("payload").await?;
        let provider_token: String = context
            .evaluate_pin::<String>("provider_token")
            .await
            .unwrap_or_default();
        let currency: String = context
            .evaluate_pin::<String>("currency")
            .await
            .unwrap_or_else(|_| "XTR".to_string());
        let prices: Vec<LabeledPrice> = context.evaluate_pin("prices").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let teloxide_prices: Vec<teloxide::types::LabeledPrice> = prices
            .iter()
            .map(|p| teloxide::types::LabeledPrice {
                label: p.label.clone(),
                amount: p.amount as u32,
            })
            .collect();

        let mut request = bot.bot.send_invoice(
            chat_id,
            title,
            description,
            payload,
            currency,
            teloxide_prices,
        );

        if !provider_token.is_empty() {
            request = request.provider_token(provider_token);
        }

        let max_tip: i64 = context
            .evaluate_pin::<i64>("max_tip_amount")
            .await
            .unwrap_or(0);
        if max_tip > 0 {
            request = request.max_tip_amount(max_tip as u32);
        }

        let suggested_tips: String = context
            .evaluate_pin::<String>("suggested_tip_amounts")
            .await
            .unwrap_or_else(|_| "[]".to_string());
        if let Ok(tips) = from_str::<Vec<u32>>(&suggested_tips)
            && !tips.is_empty()
        {
            request = request.suggested_tip_amounts(tips);
        }

        let start_param: String = context
            .evaluate_pin::<String>("start_parameter")
            .await
            .unwrap_or_default();
        if !start_param.is_empty() {
            request = request.start_parameter(start_param);
        }

        let provider_data: String = context
            .evaluate_pin::<String>("provider_data")
            .await
            .unwrap_or_default();
        if !provider_data.is_empty() {
            request = request.provider_data(provider_data);
        }

        let photo_url: String = context
            .evaluate_pin::<String>("photo_url")
            .await
            .unwrap_or_default();
        if !photo_url.is_empty() {
            request = request.photo_url(photo_url.parse().unwrap());
        }

        let photo_size: i64 = context.evaluate_pin::<i64>("photo_size").await.unwrap_or(0);
        if photo_size > 0 {
            request = request.photo_size(photo_size as u32);
        }

        let photo_width: i64 = context
            .evaluate_pin::<i64>("photo_width")
            .await
            .unwrap_or(0);
        if photo_width > 0 {
            request = request.photo_width(photo_width as u32);
        }

        let photo_height: i64 = context
            .evaluate_pin::<i64>("photo_height")
            .await
            .unwrap_or(0);
        if photo_height > 0 {
            request = request.photo_height(photo_height as u32);
        }

        let need_name: bool = context
            .evaluate_pin::<bool>("need_name")
            .await
            .unwrap_or(false);
        request = request.need_name(need_name);

        let need_phone: bool = context
            .evaluate_pin::<bool>("need_phone_number")
            .await
            .unwrap_or(false);
        request = request.need_phone_number(need_phone);

        let need_email: bool = context
            .evaluate_pin::<bool>("need_email")
            .await
            .unwrap_or(false);
        request = request.need_email(need_email);

        let need_shipping: bool = context
            .evaluate_pin::<bool>("need_shipping_address")
            .await
            .unwrap_or(false);
        request = request.need_shipping_address(need_shipping);

        let send_phone: bool = context
            .evaluate_pin::<bool>("send_phone_number_to_provider")
            .await
            .unwrap_or(false);
        request = request.send_phone_number_to_provider(send_phone);

        let send_email: bool = context
            .evaluate_pin::<bool>("send_email_to_provider")
            .await
            .unwrap_or(false);
        request = request.send_email_to_provider(send_email);

        let is_flexible: bool = context
            .evaluate_pin::<bool>("is_flexible")
            .await
            .unwrap_or(false);
        request = request.is_flexible(is_flexible);

        let result = request.await;

        match result {
            Ok(message) => {
                context
                    .set_pin_value("message_id", json!(message.id.0))
                    .await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(_) => {
                context.set_pin_value("message_id", json!(0)).await?;
                context.set_pin_value("success", json!(false)).await?;
            }
        }

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Create Invoice Link Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct CreateInvoiceLinkNode;

impl CreateInvoiceLinkNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CreateInvoiceLinkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_create_invoice_link",
            "Create Invoice Link",
            "Creates a link for an invoice",
            "Telegram/Payments",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "title",
            "Title",
            "Product name (1-32 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "description",
            "Description",
            "Product description (1-255 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "payload",
            "Payload",
            "Bot-defined invoice payload (1-128 bytes)",
            VariableType::String,
        );

        node.add_input_pin(
            "provider_token",
            "Provider Token",
            "Payment provider token (empty string for payments in Telegram Stars)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "currency",
            "Currency",
            "Three-letter ISO 4217 currency code (e.g., USD, EUR, or XTR for Telegram Stars)",
            VariableType::String,
        )
        .set_default_value(Some(json!("XTR")));

        node.add_input_pin(
            "prices",
            "Prices",
            "JSON array of price portions (LabeledPrice objects)",
            VariableType::Struct,
        )
        .set_schema::<Vec<LabeledPrice>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after invoice link is created",
            VariableType::Execution,
        );

        node.add_output_pin(
            "invoice_link",
            "Invoice Link",
            "The created invoice link",
            VariableType::Struct,
        )
        .set_schema::<InvoiceLink>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "url",
            "URL",
            "The invoice link URL as string",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the link was created successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let title: String = context.evaluate_pin("title").await?;
        let description: String = context.evaluate_pin("description").await?;
        let payload: String = context.evaluate_pin("payload").await?;
        let provider_token: String = context
            .evaluate_pin::<String>("provider_token")
            .await
            .unwrap_or_default();
        let currency: String = context
            .evaluate_pin::<String>("currency")
            .await
            .unwrap_or_else(|_| "XTR".to_string());
        let prices: Vec<LabeledPrice> = context.evaluate_pin("prices").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let teloxide_prices: Vec<teloxide::types::LabeledPrice> = prices
            .iter()
            .map(|p| teloxide::types::LabeledPrice {
                label: p.label.clone(),
                amount: p.amount as u32,
            })
            .collect();

        let mut request =
            bot.bot
                .create_invoice_link(title, description, payload, currency, teloxide_prices);

        if !provider_token.is_empty() {
            request = request.provider_token(provider_token);
        }

        let result = request.await;

        match result {
            Ok(url) => {
                let link = InvoiceLink { url: url.clone() };
                context.set_pin_value("invoice_link", json!(link)).await?;
                context.set_pin_value("url", json!(url)).await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(_) => {
                let link = InvoiceLink { url: String::new() };
                context.set_pin_value("invoice_link", json!(link)).await?;
                context.set_pin_value("url", json!("")).await?;
                context.set_pin_value("success", json!(false)).await?;
            }
        }

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Answer Shipping Query Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct AnswerShippingQueryNode;

impl AnswerShippingQueryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AnswerShippingQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_answer_shipping_query",
            "Answer Shipping Query",
            "Replies to shipping queries with available shipping options or an error",
            "Telegram/Payments",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "shipping_query_id",
            "Shipping Query ID",
            "Unique identifier for the query to be answered",
            VariableType::String,
        );

        node.add_input_pin(
            "ok",
            "OK",
            "True if delivery to the specified address is possible",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "shipping_options",
            "Shipping Options",
            "JSON array of available shipping options (required if ok is true)",
            VariableType::String,
        )
        .set_default_value(Some(json!("[]")));

        node.add_input_pin(
            "error_message",
            "Error Message",
            "Error message if delivery is not possible (required if ok is false)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after shipping query is answered",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the query was answered successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let shipping_query_id: String = context.evaluate_pin("shipping_query_id").await?;
        let ok: bool = context.evaluate_pin::<bool>("ok").await.unwrap_or(true);
        let shipping_options: String = context
            .evaluate_pin::<String>("shipping_options")
            .await
            .unwrap_or_else(|_| "[]".to_string());
        let error_message: String = context
            .evaluate_pin::<String>("error_message")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let options: Vec<teloxide::types::ShippingOption> =
            from_str(&shipping_options).unwrap_or_default();

        let result = if ok {
            bot.bot
                .answer_shipping_query(ShippingQueryId(shipping_query_id.clone()), ok)
                .shipping_options(options)
                .await
        } else {
            bot.bot
                .answer_shipping_query(ShippingQueryId(shipping_query_id.clone()), ok)
                .error_message(error_message)
                .await
        };

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Answer Pre-Checkout Query Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct AnswerPreCheckoutQueryNode;

impl AnswerPreCheckoutQueryNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AnswerPreCheckoutQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_answer_pre_checkout_query",
            "Answer Pre-Checkout Query",
            "Responds to pre-checkout query (must respond within 10 seconds)",
            "Telegram/Payments",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "pre_checkout_query_id",
            "Pre-Checkout Query ID",
            "Unique identifier for the query to be answered",
            VariableType::String,
        );

        node.add_input_pin(
            "ok",
            "OK",
            "True if checkout can proceed",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "error_message",
            "Error Message",
            "Error message if checkout cannot proceed (required if ok is false)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after pre-checkout query is answered",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the query was answered successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let pre_checkout_query_id: String = context.evaluate_pin("pre_checkout_query_id").await?;
        let ok: bool = context.evaluate_pin::<bool>("ok").await.unwrap_or(true);
        let error_message: String = context
            .evaluate_pin::<String>("error_message")
            .await
            .unwrap_or_default();

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = if ok {
            bot.bot
                .answer_pre_checkout_query(PreCheckoutQueryId(pre_checkout_query_id.clone()), true)
                .await
        } else {
            bot.bot
                .answer_pre_checkout_query(PreCheckoutQueryId(pre_checkout_query_id.clone()), false)
                .error_message(error_message)
                .await
        };

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Get Star Transactions Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct GetStarTransactionsNode;

impl GetStarTransactionsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetStarTransactionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_get_star_transactions",
            "Get Star Transactions",
            "Returns the bot's Telegram Star transactions",
            "Telegram/Payments",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "offset",
            "Offset",
            "Number of transactions to skip",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of transactions to return (1-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after transactions are retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "transactions",
            "Transactions",
            "List of star transactions",
            VariableType::Struct,
        )
        .set_schema::<Vec<StarTransaction>>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of transactions returned",
            VariableType::Integer,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the transactions were retrieved successfully",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let offset: i64 = context.evaluate_pin::<i64>("offset").await.unwrap_or(0);
        let limit: i64 = context.evaluate_pin::<i64>("limit").await.unwrap_or(100);

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let mut request = bot.bot.get_star_transactions();

        if offset > 0 {
            request = request.offset(offset as u32);
        }

        request = request.limit(limit.clamp(1, 100) as u8);

        let result = request.await;

        match result {
            Ok(star_transactions) => {
                let transactions: Vec<StarTransaction> = star_transactions
                    .transactions
                    .iter()
                    .map(|t| StarTransaction {
                        id: t.id.to_string(),
                        amount: t.amount as i64,
                        date: t.date.timestamp(),
                        source: t.source.as_ref().map(|_| "incoming".to_string()),
                        receiver: t.receiver.as_ref().map(|_| "outgoing".to_string()),
                    })
                    .collect();

                let count = transactions.len() as i64;
                context
                    .set_pin_value("transactions", json!(transactions))
                    .await?;
                context.set_pin_value("count", json!(count)).await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Err(_) => {
                context
                    .set_pin_value("transactions", json!(Vec::<StarTransaction>::new()))
                    .await?;
                context.set_pin_value("count", json!(0)).await?;
                context.set_pin_value("success", json!(false)).await?;
            }
        }

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Refund Star Payment Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct RefundStarPaymentNode;

impl RefundStarPaymentNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RefundStarPaymentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_refund_star_payment",
            "Refund Star Payment",
            "Refunds a successful payment in Telegram Stars",
            "Telegram/Payments",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "user_id",
            "User ID",
            "Identifier of the user whose payment will be refunded",
            VariableType::Integer,
        );

        node.add_input_pin(
            "telegram_payment_charge_id",
            "Telegram Payment Charge ID",
            "Telegram payment identifier from successful_payment",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after refund is processed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the refund was successful",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;
        let user_id: i64 = context.evaluate_pin("user_id").await?;
        let telegram_payment_charge_id: String =
            context.evaluate_pin("telegram_payment_charge_id").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;

        let result = bot
            .bot
            .refund_star_payment(
                UserId(user_id as u64),
                TelegramTransactionId(telegram_payment_charge_id),
            )
            .await;

        context
            .set_pin_value("success", json!(result.is_ok()))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

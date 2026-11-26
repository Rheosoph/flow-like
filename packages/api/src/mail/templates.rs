//! The platform's emails, each composed from [`super::layout`] blocks: one look everywhere and a
//! plain-text part that says everything the HTML part says. Every function returns
//! `(html, text)`, except the Cognito templates, which Cognito sends as HTML only.

use chrono::{DateTime, FixedOffset};

use super::layout::{Badge, Block, Email, Tone, Value, plain, strong};

const SOLUTION: &str = "24-Hour Solution";

fn hero(eyebrow: &str, title: impl Into<String>, subtitle: Option<String>) -> Block {
    Block::Hero {
        eyebrow: Some(eyebrow.to_string()),
        title: title.into(),
        subtitle,
    }
}

fn text(line: impl Into<String>) -> Block {
    Block::Text(vec![plain(line)])
}

fn callout(tone: Tone, label: &str, body: &str) -> Block {
    Block::Callout {
        tone,
        label: label.to_string(),
        body: body.to_string(),
    }
}

fn button(label: impl Into<String>, url: &str) -> Block {
    Block::Button {
        label: label.into(),
        url: url.to_string(),
    }
}

fn row(label: &str, value: Value) -> (String, Value) {
    (label.to_string(), value)
}

fn present(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// A status as words: `IN_PROGRESS` reads "In progress".
fn humanize(status: &str) -> String {
    let words = status.replace('_', " ").to_lowercase();
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn solution_status(status: &str) -> Badge {
    let tone = match status.to_uppercase().as_str() {
        "PENDING_REVIEW" | "IN_QUEUE" => Tone::Info,
        "ONBOARDING_DONE" | "DELIVERED" | "PAID" => Tone::Success,
        "AWAITING_DEPOSIT" | "IN_PROGRESS" | "AWAITING_PAYMENT" => Tone::Warning,
        "CANCELLED" => Tone::Danger,
        _ => Tone::Neutral,
    };
    let label = match status.to_uppercase().as_str() {
        "ONBOARDING_DONE" => "Onboarding complete".to_string(),
        _ => humanize(status),
    };
    Badge::new(tone, label)
}

/// A review decision, named by its action (`approve`) or its resulting state (`APPROVED`).
fn review_decision(action: &str) -> Badge {
    match action.to_uppercase().as_str() {
        "APPROVE" | "APPROVED" | "ACCEPT" | "ACCEPTED" | "ACTIVE" => {
            Badge::new(Tone::Success, "Approved")
        }
        "REJECT" | "REJECTED" => Badge::new(Tone::Danger, "Rejected"),
        "REQUEST_CHANGES" => Badge::new(Tone::Warning, "Changes requested"),
        "PENDING_REVIEW" | "PENDING" => Badge::new(Tone::Info, "Pending review"),
        "HOLD" | "ON_HOLD" => Badge::new(Tone::Warning, "On hold"),
        "FLAG" | "FLAGGED" => Badge::new(Tone::Danger, "Flagged"),
        _ => Badge::new(Tone::Neutral, humanize(action)),
    }
}

pub fn solution_submission_confirmation(
    company_name: &str,
    tracking_url: &str,
    tracking_token: &str,
    pricing_tier: &str,
    is_priority: bool,
) -> (String, String) {
    let tier = match pricing_tier {
        "standard" => "Standard",
        "appstore" => "App Store",
        other => other,
    };
    let plan = if is_priority {
        format!("{tier} · Priority")
    } else {
        tier.to_string()
    };
    Email {
        preview: format!("We received the {SOLUTION} request for {company_name}."),
        footer: format!("You’re receiving this because you submitted a {SOLUTION} request."),
        blocks: vec![
            hero(
                SOLUTION,
                "Request received",
                Some("Your request is on its way.".into()),
            ),
            Block::Text(vec![
                plain("Thank you for submitting your request for "),
                strong(company_name),
                plain(". We’re excited to help bring your automation vision to life."),
            ]),
            Block::Details {
                title: Some("Request details".into()),
                rows: vec![
                    row("Plan", Value::Text(plan)),
                    row("Tracking token", Value::Code(tracking_token.to_string())),
                ],
            },
            button("Track your request", tracking_url),
            Block::Steps {
                title: "What happens next".into(),
                steps: [
                    (
                        "Review",
                        "Our team reviews your request within a few hours.",
                    ),
                    ("Onboarding call", "A quick call to clarify any details."),
                    ("Development", "Your solution is built within 24 hours."),
                    ("Delivery", "You receive your complete, working solution."),
                ]
                .into_iter()
                .map(|(title, line)| (title.to_string(), line.to_string()))
                .collect(),
            },
        ],
    }
    .render()
}

pub fn solution_status_update(
    company_name: &str,
    tracking_url: &str,
    old_status: &str,
    new_status: &str,
    message: Option<&str>,
) -> (String, String) {
    let (headline, description) = match new_status.to_uppercase().as_str() {
        "IN_QUEUE" => (
            "You’re in the queue",
            "Your request has been approved and added to our development queue.",
        ),
        "ONBOARDING_DONE" => (
            "Onboarding complete",
            "We’ve completed onboarding and are ready to start building your solution.",
        ),
        "IN_PROGRESS" => (
            "Work has started",
            "Our team is working on your solution. We’ll let you know when it’s ready.",
        ),
        "DELIVERED" => (
            "Your solution is ready",
            "Your solution has been completed and delivered. Your tracking page has the details.",
        ),
        "AWAITING_PAYMENT" => (
            "Payment required",
            "Your solution is ready. Complete the final payment to receive access.",
        ),
        "PAID" => (
            "Payment received",
            "Thank you for your payment. Your solution is fully complete.",
        ),
        "CANCELLED" => (
            "Request cancelled",
            "Your request has been cancelled. If you have questions, write to us.",
        ),
        "REFUNDED" => (
            "Refund processed",
            "Your refund has been processed. It should reach your account within 5 to 10 business days.",
        ),
        _ => ("Status update", "Your request status has been updated."),
    };
    let mut blocks = vec![
        hero(
            SOLUTION,
            headline,
            Some(format!("Update for {company_name}")),
        ),
        Block::Change {
            label: "Status".into(),
            from: solution_status(old_status),
            to: solution_status(new_status),
        },
        text(description),
    ];
    if let Some(message) = present(message) {
        blocks.push(callout(Tone::Info, "Message from our team", message));
    }
    blocks.push(button("View the full status", tracking_url));
    Email {
        preview: format!("{headline}: {description}"),
        footer: format!(
            "You’re receiving this because the status of your {SOLUTION} request changed."
        ),
        blocks,
    }
    .render()
}

pub fn solution_log_added(
    company_name: &str,
    tracking_url: &str,
    action: &str,
    details: Option<&str>,
    current_status: &str,
) -> (String, String) {
    let mut blocks = vec![
        hero(SOLUTION, "New update", Some(format!("For {company_name}"))),
        callout(Tone::Info, "Update", action),
    ];
    if let Some(details) = present(details) {
        blocks.push(callout(Tone::Neutral, "Details", details));
    }
    blocks.push(Block::Details {
        title: None,
        rows: vec![row(
            "Current status",
            Value::Badge(solution_status(current_status)),
        )],
    });
    blocks.push(button("View the full history", tracking_url));
    Email {
        preview: format!("New update for {company_name}: {action}"),
        footer: format!(
            "You’re receiving this because there’s a new update on your {SOLUTION} request."
        ),
        blocks,
    }
    .render()
}

pub fn solution_delivered(
    company_name: &str,
    tracking_url: &str,
    delivery_notes: Option<&str>,
) -> (String, String) {
    let mut blocks = vec![
        hero(
            SOLUTION,
            "Your solution is ready",
            Some(format!("Congratulations, {company_name}!")),
        ),
        callout(
            Tone::Success,
            "Delivered",
            "Your 24-Hour Solution has been completed and delivered. Your tracking page has all the details.",
        ),
    ];
    if let Some(notes) = present(delivery_notes) {
        blocks.push(callout(Tone::Neutral, "Delivery notes", notes));
    }
    blocks.push(button("Access your solution", tracking_url));
    blocks.push(Block::Note(
        "Thank you for choosing Flow-Like. We hope this solution transforms your workflow.".into(),
    ));
    Email {
        preview: format!("Your {SOLUTION} for {company_name} has been delivered."),
        footer: format!("You’re receiving this because your {SOLUTION} was delivered."),
        blocks,
    }
    .render()
}

/// Email sent to the package owner when an admin reviews their package submission.
pub fn package_review_update(
    package_name: &str,
    package_url: &str,
    action: &str,
    reviewer_comment: Option<&str>,
) -> (String, String) {
    let (headline, description, tone) = match action.to_lowercase().as_str() {
        "approve" => (
            "Your package is approved",
            "Your package passed review and is now live on the registry. People can discover and install it right away.",
            Tone::Success,
        ),
        "reject" => (
            "Your package was not approved",
            "Your package did not pass review. Check the reviewer’s feedback and resubmit when you’re ready.",
            Tone::Danger,
        ),
        "request_changes" => (
            "Changes requested",
            "The reviewer asked for some changes before your package can be approved.",
            Tone::Warning,
        ),
        "flag" => (
            "Your package was flagged",
            "Your package was flagged for an additional review. Our team will follow up shortly.",
            Tone::Danger,
        ),
        _ => (
            "Package review update",
            "There’s an update on your package review.",
            Tone::Info,
        ),
    };
    let mut blocks = vec![
        hero("Package review", headline, Some(package_name.to_string())),
        text(description),
        Block::Details {
            title: None,
            rows: vec![
                row("Package", Value::Text(package_name.to_string())),
                row("Decision", Value::Badge(review_decision(action))),
            ],
        },
    ];
    if let Some(comment) = present(reviewer_comment) {
        blocks.push(callout(tone, "Reviewer feedback", comment));
    }
    blocks.push(button("View the package", package_url));
    Email {
        preview: format!("{headline}: {package_name}"),
        footer: "You’re receiving this because you published a package on Flow-Like.".into(),
        blocks,
    }
    .render()
}

/// Email sent to the app owner when their publication request is reviewed.
pub fn app_publication_update(
    app_name: &str,
    app_url: &str,
    action: &str,
    target_visibility: &str,
    reviewer_message: Option<&str>,
) -> (String, String) {
    publication_update(
        "App",
        app_name,
        app_url,
        action,
        target_visibility,
        reviewer_message,
    )
}

/// Publication decision email. `entity_label` is the capitalised noun for the
/// reviewed thing ("App" or "Suite") so apps and suites share one template.
pub fn publication_update(
    entity_label: &str,
    app_name: &str,
    app_url: &str,
    action: &str,
    target_visibility: &str,
    reviewer_message: Option<&str>,
) -> (String, String) {
    let entity = entity_label.to_lowercase();
    let visibility = target_visibility.replace('_', " ");
    let (headline, description, tone) = match action.to_lowercase().as_str() {
        "approve" | "accept" => (
            format!("Your {entity} is published"),
            vec![
                plain(format!("Your {entity} was approved and is now visible as ")),
                strong(visibility.clone()),
                plain(". People can discover it in the store."),
            ],
            Tone::Success,
        ),
        "reject" => (
            "Publication request rejected".to_string(),
            vec![plain(
                "Your publication request was not approved. Review the feedback and try again when you’re ready.",
            )],
            Tone::Danger,
        ),
        "hold" => (
            "Publication request on hold".to_string(),
            vec![plain(
                "Your publication request is on hold. The review team will follow up with more details.",
            )],
            Tone::Warning,
        ),
        _ => (
            "Publication request update".to_string(),
            vec![plain("There’s an update on your publication request.")],
            Tone::Info,
        ),
    };
    let mut blocks = vec![
        hero(
            "Publication review",
            headline.clone(),
            Some(app_name.to_string()),
        ),
        Block::Text(description),
        Block::Details {
            title: None,
            rows: vec![
                row(entity_label, Value::Text(app_name.to_string())),
                row("Decision", Value::Badge(review_decision(action))),
                row("Target visibility", Value::Text(visibility)),
            ],
        },
    ];
    if let Some(message) = present(reviewer_message) {
        blocks.push(callout(tone, "Message from the reviewer", message));
    }
    blocks.push(button(format!("View the {entity}"), app_url));
    Email {
        preview: format!("{headline}: {app_name}"),
        footer: format!(
            "You’re receiving this because you requested publication for your {entity} on Flow-Like."
        ),
        blocks,
    }
    .render()
}

/// Email sent to the buyer after a successful WASM package purchase.
pub fn purchase_confirmation(
    package_name: &str,
    package_url: &str,
    price_display: &str,
    currency: &str,
    stripe_receipt_url: Option<&str>,
) -> (String, String) {
    let mut blocks = vec![
        hero(
            "Purchase",
            "Purchase confirmed",
            Some(format!("You now have access to {package_name}.")),
        ),
        Block::Details {
            title: Some("Order summary".into()),
            rows: vec![
                row("Package", Value::Text(package_name.to_string())),
                row("Amount", Value::Text(format!("{price_display} {currency}"))),
                row("Status", Value::Badge(Badge::new(Tone::Success, "Paid"))),
            ],
        },
        callout(
            Tone::Success,
            "Access granted",
            "The package is ready to use. Install it from the store or add it to your apps.",
        ),
        button("Go to the package", package_url),
    ];
    if let Some(receipt) = present(stripe_receipt_url) {
        blocks.push(Block::SecondaryButton {
            label: "View the receipt on Stripe".into(),
            url: receipt.to_string(),
        });
    }
    Email {
        preview: format!("You now have access to {package_name}."),
        footer: "You’re receiving this because you made a purchase on Flow-Like.".into(),
        blocks,
    }
    .render()
}

/// What a payment confirmation records: the order, the terms accepted with it, and for a
/// withdrawal what was declared.
pub struct OrderConfirmation<'a> {
    pub withdrawn: bool,
    pub intro: &'a str,
    /// When and what was confirmed or declared, as label and value.
    pub event: Vec<(String, String)>,
    pub order_id: &'a str,
    pub product: &'a str,
    pub amount: &'a str,
    pub email: &'a str,
    pub terms_version: &'a str,
    pub waiver_accepted: bool,
    pub terms: &'a str,
}

/// The confirmation of a purchase or a withdrawal on a durable medium: everything is in the
/// plain-text part as well, the accepted terms included.
pub fn order_confirmation(order: &OrderConfirmation) -> (String, String) {
    let (eyebrow, title) = if order.withdrawn {
        ("Withdrawal", "Your withdrawal confirmation")
    } else {
        ("Purchase", "Your purchase confirmation")
    };
    let rows = order
        .event
        .iter()
        .map(|(label, value)| row(label, Value::Text(value.clone())))
        .chain([
            row("Order", Value::Code(order.order_id.to_string())),
            row("Product", Value::Text(order.product.to_string())),
            row("Amount", Value::Text(order.amount.to_string())),
            row("Confirmation email", Value::Text(order.email.to_string())),
            row(
                "Purchase terms version",
                Value::Text(order.terms_version.to_string()),
            ),
            row(
                "Immediate delivery withdrawal waiver accepted",
                Value::Text(if order.waiver_accepted { "Yes" } else { "No" }.into()),
            ),
        ])
        .collect();
    Email {
        preview: order.intro.to_string(),
        footer: "Keep this email as your record of the order and the terms you accepted.".into(),
        blocks: vec![
            hero(eyebrow, title, Some(order.product.to_string())),
            text(order.intro),
            Block::Details {
                title: Some("Record".into()),
                rows,
            },
            callout(Tone::Neutral, "Accepted purchase terms", order.terms),
        ],
    }
    .render()
}

/// A notice about the reader's devices, with the page in Flow-Like it is about.
pub fn device_notice(title: &str, description: &str, url: &str) -> (String, String) {
    Email {
        preview: description.to_string(),
        footer: "You’re receiving this because you manage devices on Flow-Like.".into(),
        blocks: vec![
            hero("Devices", title, None),
            text(description),
            button("Open in Flow-Like", url),
        ],
    }
    .render()
}

/// A bot's invitation of a person into a group chat, by email.
pub struct RoomInvitationMail<'a> {
    pub bot: &'a str,
    pub owner: &'a str,
    /// The room's title; `None` for an untitled room.
    pub room: Option<&'a str>,
    /// The reason the owner approved, in the bot's words.
    pub reason: Option<&'a str>,
    /// The single-use link for someone without an account, or the reader's invitations.
    pub url: &'a str,
    /// Set for someone without an account: when the link stops working.
    pub link_expires: Option<DateTime<FixedOffset>>,
}

pub fn room_invitation(mail: &RoomInvitationMail) -> (String, String) {
    let title = mail.room.map_or_else(
        || "Join a group chat".to_string(),
        |room| format!("Join “{room}”"),
    );
    let inviter = format!("{}, {}’s bot", mail.bot, mail.owner);
    let mut blocks = vec![hero(
        "Group chat invitation",
        title,
        Some(format!(
            "{inviter}, invited you to this group chat on Flow-Like."
        )),
    )];
    if let Some(reason) = present(mail.reason) {
        blocks.push(callout(Tone::Info, "Why you’re invited", reason));
    }
    let footer = match mail.link_expires {
        Some(expires) => {
            blocks.push(button("Join the chat", mail.url));
            blocks.push(Block::Note(format!(
                "Joining takes a free Flow-Like account, which you create on the way. The link works once, until {}.",
                expires.format("%B %-d, %Y")
            )));
            blocks.push(Block::Note(
                "Not expecting this? You can ignore this email: you only join if you open the link."
                    .into(),
            ));
            format!(
                "You’re receiving this because {} approved their bot’s invitation of this address on Flow-Like.",
                mail.owner
            )
        }
        None => {
            blocks.push(button("Open your invitations", mail.url));
            blocks.push(Block::Note(
                "Accept or decline it in Flow-Like. Nobody in the chat reads anything from you before you accept."
                    .into(),
            ));
            "You’re receiving this by email because you haven’t opened Flow-Like for a week."
                .to_string()
        }
    };
    Email {
        preview: format!("{inviter}, invited you to a group chat on Flow-Like."),
        footer,
        blocks,
    }
    .render()
}

/// Cognito replaces this with the code or the temporary password.
pub const COGNITO_CODE: &str = "{####}";
/// Cognito replaces this with the username of an account an administrator created.
pub const COGNITO_USERNAME: &str = "{username}";

/// A Cognito email template: Cognito sends the HTML as the only body.
pub struct CognitoMail {
    pub subject: &'static str,
    pub html: String,
}

/// The code Cognito sends to confirm an address and to reset a password.
pub fn cognito_verification() -> CognitoMail {
    CognitoMail {
        subject: "Your Flow-Like code",
        html: Email {
            preview: "Use this code to continue on Flow-Like.".into(),
            footer: "You’re receiving this because this address was entered on Flow-Like.".into(),
            blocks: vec![
                hero(
                    "Verification",
                    "Your Flow-Like code",
                    Some("Enter this code in Flow-Like to continue.".into()),
                ),
                Block::Code(COGNITO_CODE.into()),
                Block::Note(
                    "It confirms this email address, or lets you choose a new password, and works only for a short time."
                        .into(),
                ),
                Block::Note(
                    "Didn’t ask for a code? You can ignore this email: nothing changes without it."
                        .into(),
                ),
            ],
        }
        .html(),
    }
}

/// The sign-in details Cognito sends when an administrator creates an account.
pub fn cognito_invitation() -> CognitoMail {
    CognitoMail {
        subject: "Your Flow-Like account is ready",
        html: Email {
            preview: "Sign in to Flow-Like and choose your own password.".into(),
            footer: "You’re receiving this because an administrator created a Flow-Like account for this address."
                .into(),
            blocks: vec![
                hero(
                    "Welcome",
                    "Your Flow-Like account is ready",
                    Some("Sign in with these details, then choose your own password.".into()),
                ),
                Block::Details {
                    title: Some("Sign-in details".into()),
                    rows: vec![
                        row("Username", Value::Code(COGNITO_USERNAME.into())),
                        row("Temporary password", Value::Code(COGNITO_CODE.into())),
                    ],
                },
                Block::Note(
                    "The temporary password works once and only for a limited time.".into(),
                ),
            ],
        }
        .html(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solution_confirmation_escapes_html_but_preserves_plain_text() {
        let company = "</strong><a href=\"https://evil.example\">Click</a>";
        let (html, text) = solution_submission_confirmation(
            company,
            "https://example.com/track?a=1&b=2",
            "<token>",
            "<tier>",
            false,
        );
        assert!(!html.contains(company));
        assert!(html.contains("&lt;/strong&gt;"));
        assert!(html.contains("&lt;token&gt;"));
        assert!(html.contains("&lt;tier&gt;"));
        assert!(html.contains("https://example.com/track?a=1&amp;b=2"));
        assert!(text.contains(company));
        assert!(text.contains("Track your request:\nhttps://example.com/track?a=1&b=2"));
    }

    #[test]
    fn user_written_words_never_reach_the_html_unescaped() {
        let evil = "<img src=x onerror=alert(1)>";
        let parts = [
            purchase_confirmation(evil, "https://example.com/store", "9.99", "EUR", Some(evil)),
            package_review_update(evil, "https://example.com/p", "approve", Some(evil)),
            publication_update(
                "App",
                evil,
                "https://example.com/a",
                "approve",
                evil,
                Some(evil),
            ),
            solution_status_update(evil, "https://example.com/t", "IN_QUEUE", evil, Some(evil)),
            solution_log_added(evil, "https://example.com/t", evil, Some(evil), evil),
            solution_delivered(evil, "https://example.com/t", Some(evil)),
            device_notice(evil, evil, "https://example.com/d"),
            room_invitation(&RoomInvitationMail {
                bot: evil,
                owner: evil,
                room: Some(evil),
                reason: Some(evil),
                url: "https://example.com/join?room=t",
                link_expires: None,
            }),
        ];
        for (html, text) in parts {
            assert!(!html.contains(evil), "{html}");
            assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
            assert!(text.contains(evil));
        }
    }

    #[test]
    fn review_decisions_read_the_same_whether_named_by_action_or_state() {
        assert_eq!(review_decision("approve"), review_decision("APPROVED"));
        assert_eq!(review_decision("hold").label, "On hold");
        assert_eq!(review_decision("request_changes").tone, Tone::Warning);
        assert_eq!(
            solution_status("ONBOARDING_DONE").label,
            "Onboarding complete"
        );
        assert_eq!(solution_status("IN_PROGRESS").label, "In progress");
    }

    #[test]
    fn a_newcomer_reads_who_invited_why_and_until_when_the_link_works() {
        let expires = DateTime::parse_from_rfc3339("2026-12-26T10:00:00+00:00").unwrap();
        let (html, text) = room_invitation(&RoomInvitationMail {
            bot: "Flux",
            owner: "Felix",
            room: Some("Launch"),
            reason: Some("You run the venue, and the room plans the launch party."),
            url: "https://api.example.com/join?room=abc",
            link_expires: Some(expires),
        });
        for part in [&html, &text] {
            assert!(part.contains("Flux, Felix’s bot"), "{part}");
            assert!(part.contains("You run the venue"), "{part}");
            assert!(part.contains("until December 26, 2026"), "{part}");
        }
        assert!(text.contains("Join the chat:\nhttps://api.example.com/join?room=abc"));
        assert!(text.contains("GROUP CHAT INVITATION\nJoin “Launch”"));
        let (_, away) = room_invitation(&RoomInvitationMail {
            link_expires: None,
            room: None,
            reason: None,
            url: "https://app.example.com/chat?invitations=1",
            bot: "Flux",
            owner: "Felix",
        });
        assert!(away.contains("Join a group chat"));
        assert!(
            away.contains("Open your invitations:\nhttps://app.example.com/chat?invitations=1")
        );
        assert!(away.contains("haven’t opened Flow-Like for a week"));
        assert!(!away.contains("WHY YOU’RE INVITED"));
    }

    #[test]
    fn an_order_confirmation_carries_the_whole_record_in_both_parts() {
        let (html, text) = order_confirmation(&OrderConfirmation {
            withdrawn: true,
            intro: "Your withdrawal request was received.",
            event: vec![("Received at".into(), "2026-11-26T10:00:00+00:00".into())],
            order_id: "order-1",
            product: "Tools",
            amount: "9.99 EUR",
            email: "buyer@example.com",
            terms_version: "v3",
            waiver_accepted: false,
            terms: "§1 Terms\n§2 More & <terms>",
        });
        for kept in [
            "Received at: 2026-11-26T10:00:00+00:00",
            "Order: order-1",
            "Immediate delivery withdrawal waiver accepted: No",
            "ACCEPTED PURCHASE TERMS\n§1 Terms\n§2 More & <terms>",
        ] {
            assert!(text.contains(kept), "{kept}\n{text}");
        }
        assert!(html.contains("§1 Terms<br>§2 More &amp; &lt;terms&gt;"));
        assert!(html.contains("Your withdrawal confirmation"));
    }

    /// `MAIL_PREVIEW_DIR=<dir> cargo test -p flow-like-api --lib mail_previews` writes every email
    /// there as HTML and text, to look at in browsers and mail clients.
    #[test]
    fn mail_previews() {
        let Some(dir) = std::env::var_os("MAIL_PREVIEW_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        let expires = DateTime::parse_from_rfc3339("2026-12-26T10:00:00+00:00").unwrap();
        let invitation = |link_expires, url| {
            room_invitation(&RoomInvitationMail {
                bot: "Flux",
                owner: "Felix Schultz",
                room: Some("Launch planning"),
                reason: Some(
                    "You run the venue, and we’re planning the launch party for December 12. Your take on capacity and catering would help us decide this week.",
                ),
                url,
                link_expires,
            })
        };
        let previews = [
            (
                "room-invitation-newcomer",
                invitation(Some(expires), "https://api.flow-like.com/join?room=example"),
            ),
            (
                "room-invitation-away",
                invitation(None, "https://app.flow-like.com/chat?invitations=1"),
            ),
            (
                "purchase",
                purchase_confirmation(
                    "Weather Toolkit",
                    "https://app.flow-like.com/store/packages/weather",
                    "9.99",
                    "EUR",
                    Some("https://pay.stripe.com/receipts/example"),
                ),
            ),
            (
                "solution-submitted",
                solution_submission_confirmation(
                    "Acme GmbH",
                    "https://flow-like.com/track/abc",
                    "TRK-12345",
                    "standard",
                    true,
                ),
            ),
            (
                "solution-status",
                solution_status_update(
                    "Acme GmbH",
                    "https://flow-like.com/track/abc",
                    "IN_QUEUE",
                    "IN_PROGRESS",
                    Some("We started on the CRM sync today."),
                ),
            ),
            (
                "package-review",
                package_review_update(
                    "Weather Toolkit",
                    "https://app.flow-like.com/store/packages/weather",
                    "request_changes",
                    Some("Please describe the forecast pins.\nThe icon also needs a dark variant."),
                ),
            ),
            (
                "order-confirmation",
                order_confirmation(&OrderConfirmation {
                    withdrawn: false,
                    intro: "Your payment was confirmed. Keep this message as a record of your purchase and the terms you accepted.",
                    event: vec![(
                        "Purchase confirmed at".into(),
                        "2026-11-26T10:00:00+00:00".into(),
                    )],
                    order_id: "po_8f2a91",
                    product: "Inventory Planner",
                    amount: "49.00 EUR",
                    email: "buyer@example.com",
                    terms_version: "2026-10",
                    waiver_accepted: true,
                    terms: "1. Scope\nThe seller grants you access to the app.\n\n2. Withdrawal\nYou agreed to immediate delivery.",
                }),
            ),
            (
                "device-notice",
                device_notice(
                    "A device certificate expires soon",
                    "The certificate of “Line 3 controller” expires in 7 days. Renew it to keep the device connected.",
                    "https://app.flow-like.com/devices",
                ),
            ),
        ];
        for (name, (html, text)) in previews {
            std::fs::write(dir.join(format!("{name}.html")), html).unwrap();
            std::fs::write(dir.join(format!("{name}.txt")), text).unwrap();
        }
        for (name, mail) in [
            ("cognito-verification", cognito_verification()),
            ("cognito-invitation", cognito_invitation()),
        ] {
            std::fs::write(dir.join(format!("{name}.html")), mail.html).unwrap();
        }
    }

    fn cognito_templates_json() -> String {
        let template =
            |mail: CognitoMail| serde_json::json!({ "subject": mail.subject, "html": mail.html });
        let mut json = serde_json::to_string_pretty(&serde_json::json!({
            "verification": template(cognito_verification()),
            "invitation": template(cognito_invitation()),
        }))
        .unwrap();
        json.push('\n');
        json
    }

    /// Cognito keeps its own copy of these templates; `tools/cognito-email/apply.py` sends the
    /// checked-in one, so it has to match the layout.
    #[test]
    fn the_checked_in_cognito_templates_match_the_layout() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/cognito-email/templates.json");
        let rendered = cognito_templates_json();
        if std::env::var_os("UPDATE_COGNITO_TEMPLATES").is_some() {
            std::fs::write(&path, &rendered).unwrap();
        }
        let stored = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            stored == rendered,
            "tools/cognito-email/templates.json is out of date: run `UPDATE_COGNITO_TEMPLATES=1 cargo test -p flow-like-api --lib cognito`, then `python3 -I tools/cognito-email/apply.py --apply`"
        );
    }

    #[test]
    fn cognito_templates_hold_their_placeholders_within_cognitos_limit() {
        let verification = cognito_verification();
        assert_eq!(verification.html.matches(COGNITO_CODE).count(), 1);
        let invitation = cognito_invitation();
        assert_eq!(invitation.html.matches(COGNITO_CODE).count(), 1);
        assert_eq!(invitation.html.matches(COGNITO_USERNAME).count(), 1);
        for template in [&verification, &invitation] {
            assert!(template.html.chars().count() <= 20_000);
            assert!(template.subject.chars().count() <= 140);
        }
    }
}

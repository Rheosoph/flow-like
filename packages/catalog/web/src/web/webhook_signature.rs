use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(any(feature = "execute", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum WebhookAlgorithm {
    Sha256,
    Sha1,
}

#[cfg(any(feature = "execute", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SignatureEncoding {
    Hex,
    Base64,
}

#[cfg(any(feature = "execute", test))]
impl WebhookAlgorithm {
    fn parse(value: &str) -> flow_like_types::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "sha256" => Ok(Self::Sha256),
            "sha1" => Ok(Self::Sha1),
            other => Err(flow_like_types::anyhow!(
                "Unsupported webhook signature algorithm '{}'; use sha256 or sha1",
                other
            )),
        }
    }
}

#[cfg(any(feature = "execute", test))]
impl SignatureEncoding {
    fn parse(value: &str) -> flow_like_types::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "hex" => Ok(Self::Hex),
            "base64" => Ok(Self::Base64),
            other => Err(flow_like_types::anyhow!(
                "Unsupported webhook signature encoding '{}'; use hex or base64",
                other
            )),
        }
    }
}

#[cfg(any(feature = "execute", test))]
fn decode_hex(value: &str) -> Option<Vec<u8>> {
    let (pairs, rest) = value.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    pairs
        .iter()
        .map(|[high, low]| {
            let high = (*high as char).to_digit(16)?;
            let low = (*low as char).to_digit(16)?;
            Some((high * 16 + low) as u8)
        })
        .collect()
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn decode_signature(
    signature: &str,
    prefix: &str,
    encoding: SignatureEncoding,
) -> Option<Vec<u8>> {
    let signature = signature.trim();
    let signature = if prefix.is_empty() {
        signature
    } else {
        signature.strip_prefix(prefix).unwrap_or(signature)
    };
    match encoding {
        SignatureEncoding::Hex => decode_hex(signature),
        SignatureEncoding::Base64 => {
            use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
            STANDARD.decode(signature).ok()
        }
    }
}

/// Recomputes the HMAC of `body` and compares it with the received signature in constant time.
#[cfg(feature = "execute")]
pub(crate) fn verify_webhook_signature(
    body: &[u8],
    signature: &str,
    secret: &str,
    algorithm: WebhookAlgorithm,
    prefix: &str,
    encoding: SignatureEncoding,
) -> bool {
    use hmac::{Hmac, Mac};

    let Some(expected) = decode_signature(signature, prefix, encoding) else {
        return false;
    };
    match algorithm {
        WebhookAlgorithm::Sha256 => Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes())
            .map(|mut mac| {
                mac.update(body);
                mac.verify_slice(&expected).is_ok()
            })
            .unwrap_or(false),
        WebhookAlgorithm::Sha1 => Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes())
            .map(|mut mac| {
                mac.update(body);
                mac.verify_slice(&expected).is_ok()
            })
            .unwrap_or(false),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct VerifyWebhookSignatureNode {}

impl VerifyWebhookSignatureNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for VerifyWebhookSignatureNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "verify_webhook_signature",
            "Verify Webhook Signature",
            "Checks the HMAC signature of an incoming webhook, such as GitHub's X-Hub-Signature-256 or Stripe-style headers. The body must be the raw request body exactly as received; a re-serialized JSON payload produces a different signature.",
            "Web/Auth",
        );
        node.set_flowscript_name("auth", "verifyWebhookSignature");
        node.add_icon("/flow/icons/web.svg");

        node.add_input_pin(
            "body",
            "Body",
            "Raw request body as received",
            VariableType::String,
        );

        node.add_input_pin(
            "signature",
            "Signature",
            "Signature header value, e.g. sha256=5d61…",
            VariableType::String,
        );

        node.add_input_pin(
            "secret",
            "Secret",
            "Shared webhook secret",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "algorithm",
            "Algorithm",
            "HMAC hash algorithm",
            VariableType::String,
        )
        .set_default_value(Some(json!("sha256")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["sha256".to_string(), "sha1".to_string()])
                .build(),
        );

        node.add_input_pin(
            "prefix",
            "Prefix",
            "Prefix removed from the signature before decoding. Empty for none",
            VariableType::String,
        )
        .set_default_value(Some(json!("sha256=")));

        node.add_input_pin(
            "encoding",
            "Encoding",
            "Encoding of the signature",
            VariableType::String,
        )
        .set_default_value(Some(json!("hex")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["hex".to_string(), "base64".to_string()])
                .build(),
        );

        node.add_output_pin(
            "valid",
            "Valid",
            "True when the signature matches the body and secret",
            VariableType::Boolean,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(9)
                .set_performance(10)
                .set_governance(8)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let body: String = context.evaluate_pin("body").await?;
        let signature: String = context.evaluate_pin("signature").await.unwrap_or_default();
        let secret: String = context.evaluate_pin("secret").await?;
        let algorithm: String = context
            .evaluate_pin("algorithm")
            .await
            .unwrap_or_else(|_| "sha256".to_string());
        let prefix: String = context
            .evaluate_pin("prefix")
            .await
            .unwrap_or_else(|_| "sha256=".to_string());
        let encoding: String = context
            .evaluate_pin("encoding")
            .await
            .unwrap_or_else(|_| "hex".to_string());

        if secret.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Webhook secret is required to verify a signature"
            ));
        }

        let valid = verify_webhook_signature(
            body.as_bytes(),
            &signature,
            &secret,
            WebhookAlgorithm::parse(&algorithm)?,
            &prefix,
            SignatureEncoding::parse(&encoding)?,
        );
        context.set_pin_value("valid", json!(valid)).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Verify Webhook Signature requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_base64_signatures_decode_after_prefix() {
        assert_eq!(
            decode_signature("sha256=0aff", "sha256=", SignatureEncoding::Hex),
            Some(vec![0x0a, 0xff])
        );
        assert_eq!(
            decode_signature("0AFF", "sha256=", SignatureEncoding::Hex),
            Some(vec![0x0a, 0xff])
        );
        assert_eq!(decode_signature("abc", "", SignatureEncoding::Hex), None);
        assert_eq!(decode_signature("zz", "", SignatureEncoding::Hex), None);
        assert_eq!(
            decode_signature("v1=Cv8=", "v1=", SignatureEncoding::Base64),
            Some(vec![0x0a, 0xff])
        );
    }

    #[test]
    fn options_parse_and_reject_unknown_values() {
        assert_eq!(
            WebhookAlgorithm::parse("SHA1").unwrap(),
            WebhookAlgorithm::Sha1
        );
        assert_eq!(
            WebhookAlgorithm::parse("").unwrap(),
            WebhookAlgorithm::Sha256
        );
        assert!(WebhookAlgorithm::parse("md5").is_err());
        assert_eq!(
            SignatureEncoding::parse("Base64").unwrap(),
            SignatureEncoding::Base64
        );
        assert!(SignatureEncoding::parse("binary").is_err());
    }

    #[cfg(feature = "execute")]
    mod hmac_verification {
        use super::super::*;

        // Example from GitHub's "Validating webhook deliveries" documentation.
        const GITHUB_SECRET: &str = "It's a Secret to Everybody";
        const GITHUB_BODY: &[u8] = b"Hello, World!";
        const GITHUB_SIGNATURE: &str =
            "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";

        #[test]
        fn accepts_githubs_documented_example() {
            assert!(verify_webhook_signature(
                GITHUB_BODY,
                GITHUB_SIGNATURE,
                GITHUB_SECRET,
                WebhookAlgorithm::Sha256,
                "sha256=",
                SignatureEncoding::Hex,
            ));
        }

        #[test]
        fn rejects_tampered_body_wrong_secret_and_garbage() {
            let verify = |body: &[u8], signature: &str, secret: &str| {
                verify_webhook_signature(
                    body,
                    signature,
                    secret,
                    WebhookAlgorithm::Sha256,
                    "sha256=",
                    SignatureEncoding::Hex,
                )
            };
            assert!(!verify(b"Hello, World?", GITHUB_SIGNATURE, GITHUB_SECRET));
            assert!(!verify(GITHUB_BODY, GITHUB_SIGNATURE, "wrong"));
            assert!(!verify(GITHUB_BODY, "sha256=nothex", GITHUB_SECRET));
            assert!(!verify(GITHUB_BODY, "", GITHUB_SECRET));
            assert!(!verify(GITHUB_BODY, &GITHUB_SIGNATURE[..20], GITHUB_SECRET));
        }

        #[test]
        fn supports_sha1_and_base64() {
            use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
            use hmac::{Hmac, Mac};

            let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"s3cret").unwrap();
            mac.update(b"payload");
            let raw = mac.finalize().into_bytes();
            let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();

            assert!(verify_webhook_signature(
                b"payload",
                &format!("sha1={hex}"),
                "s3cret",
                WebhookAlgorithm::Sha1,
                "sha1=",
                SignatureEncoding::Hex,
            ));
            assert!(verify_webhook_signature(
                b"payload",
                &STANDARD.encode(raw),
                "s3cret",
                WebhookAlgorithm::Sha1,
                "",
                SignatureEncoding::Base64,
            ));
            assert!(!verify_webhook_signature(
                b"payload",
                &format!("sha1={hex}"),
                "s3cret",
                WebhookAlgorithm::Sha256,
                "sha1=",
                SignatureEncoding::Hex,
            ));
        }
    }
}

//! A CAPTCHA solving service the owner signed up with: 2Captcha, CapSolver, or another service
//! that speaks 2Captcha's task API at its own address. The owner stores the service's key as
//! the secret `CAPTCHA_SOLVER_KEY` and, for anything but 2Captcha, names the service in
//! `CAPTCHA_SOLVER`. The service gets the page address and the widget's site key, nothing else.

use std::time::Duration;

use flow_like_types::reqwest;
use serde_json::{Value, json};

use super::backend::Challenge;
use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};

pub const SOLVER_KEY_SECRET: &str = "CAPTCHA_SOLVER_KEY";
pub const SOLVER_SECRET: &str = "CAPTCHA_SOLVER";
const PATIENCE: Duration = Duration::from_secs(150);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Api {
    TwoCaptcha,
    CapSolver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Service {
    api: Api,
    base: String,
}

impl Service {
    /// The service `CAPTCHA_SOLVER` names: `2captcha` (also when unset), `capsolver`, or the
    /// address of a service with 2Captcha's API.
    pub(crate) fn named(name: Option<&str>) -> ToolResult<Self> {
        let name = name.map(str::trim).filter(|name| !name.is_empty());
        let service = |api, base: &str| {
            Ok(Self {
                api,
                base: base.trim_end_matches('/').to_owned(),
            })
        };
        match name.map(str::to_ascii_lowercase).as_deref() {
            None | Some("2captcha") => service(Api::TwoCaptcha, "https://api.2captcha.com"),
            Some("capsolver") => service(Api::CapSolver, "https://api.capsolver.com"),
            Some(address) if address.starts_with("https://") || address.starts_with("http://") => {
                service(Api::TwoCaptcha, name.unwrap_or_default())
            }
            Some(other) => Err(ToolError::invalid(format!(
                "The secret {SOLVER_SECRET} says '{other}'; write 2captcha, capsolver or the address of a service with 2Captcha's API"
            ))),
        }
    }

    fn poll_every(&self) -> Duration {
        match self.api {
            Api::TwoCaptcha => Duration::from_secs(5),
            Api::CapSolver => Duration::from_secs(3),
        }
    }

    /// The service's task for `challenge`, or why it cannot take it.
    pub(crate) fn task(&self, challenge: &Challenge) -> ToolResult<Value> {
        let key = challenge
            .site_key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                ToolError::invalid(
                    "The check on this page shows no site key, which a solving service needs",
                )
            })?;
        let mut task = json!({"websiteURL": challenge.url, "websiteKey": key});
        let kind = match (challenge.kind.as_str(), self.api) {
            ("recaptcha", Api::TwoCaptcha) => "RecaptchaV2TaskProxyless",
            ("recaptcha", Api::CapSolver) => "ReCaptchaV2TaskProxyLess",
            ("turnstile" | "cloudflare_page", Api::TwoCaptcha) => {
                if let Some(action) = &challenge.action {
                    task["action"] = json!(action);
                }
                if let Some(data) = &challenge.data {
                    task["data"] = json!(data);
                }
                "TurnstileTaskProxyless"
            }
            ("turnstile" | "cloudflare_page", Api::CapSolver) => {
                let mut metadata = json!({});
                if let Some(action) = &challenge.action {
                    metadata["action"] = json!(action);
                }
                if let Some(data) = &challenge.data {
                    metadata["cdata"] = json!(data);
                }
                task["metadata"] = metadata;
                "AntiTurnstileTaskProxyLess"
            }
            (other, _) => {
                return Err(ToolError::invalid(format!(
                    "A solving service answers Turnstile and reCAPTCHA checks, not '{other}'"
                )));
            }
        };
        task["type"] = json!(kind);
        Ok(task)
    }
}

async fn secret(env: &dyn ToolEnv, name: &str) -> ToolResult<Option<String>> {
    Ok(env
        .secret(name)
        .await
        .map_err(ToolError::from)?
        .filter(|value| !value.trim().is_empty()))
}

/// Sends the check to the owner's service and waits for its token.
pub(crate) async fn solve(env: &dyn ToolEnv, challenge: &Challenge) -> ToolResult<String> {
    let key = secret(env, SOLVER_KEY_SECRET).await?.ok_or_else(|| {
        ToolError::new(
            code::NOT_FOUND,
            format!(
                "No solving service is set up. The owner stores the service's API key as the secret {SOLVER_KEY_SECRET} and, for CapSolver, writes capsolver into {SOLVER_SECRET}; ask with secret_request"
            ),
        )
    })?;
    let service = Service::named(secret(env, SOLVER_SECRET).await?.as_deref())?;
    let task = service.task(challenge)?;
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| ToolError::new(code::FAILED, format!("No HTTP client: {error}")))?;
    let created = call(
        &client,
        &service,
        "createTask",
        json!({"clientKey": key, "task": task}),
    )
    .await?;
    if created["status"] == "ready" {
        return token_of(&created);
    }
    let task_id = created
        .get("taskId")
        .filter(|id| !id.is_null())
        .cloned()
        .ok_or_else(|| {
            ToolError::new(
                code::FAILED,
                "The solving service answered without a task id",
            )
        })?;
    let deadline = flow_like_types::tokio::time::Instant::now() + PATIENCE;
    loop {
        flow_like_types::tokio::time::sleep(service.poll_every()).await;
        let result = call(
            &client,
            &service,
            "getTaskResult",
            json!({"clientKey": key, "taskId": task_id}),
        )
        .await?;
        if result["status"] == "ready" {
            return token_of(&result);
        }
        if flow_like_types::tokio::time::Instant::now() >= deadline {
            return Err(ToolError::new(
                code::FAILED,
                format!(
                    "The solving service did not answer within {} s",
                    PATIENCE.as_secs()
                ),
            ));
        }
    }
}

async fn call(
    client: &reqwest::Client,
    service: &Service,
    method: &str,
    body: Value,
) -> ToolResult<Value> {
    let unreachable = |error: String| {
        ToolError::new(
            code::FAILED,
            format!("The solving service could not be reached ({method}): {error}"),
        )
    };
    let response = client
        .post(format!("{}/{method}", service.base))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|error| unreachable(error.to_string()))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| unreachable(error.to_string()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| unreachable(format!("the answer is not JSON ({error})")))?;
    if value["errorId"].as_i64().unwrap_or(0) != 0 {
        let name = value["errorCode"].as_str().unwrap_or("an error");
        let description = value["errorDescription"].as_str().unwrap_or_default();
        return Err(ToolError::new(
            code::FAILED,
            format!("The solving service refused ({method}): {name} {description}")
                .trim_end()
                .to_owned(),
        ));
    }
    Ok(value)
}

fn token_of(result: &Value) -> ToolResult<String> {
    let solution = &result["solution"];
    solution["gRecaptchaResponse"]
        .as_str()
        .or_else(|| solution["token"].as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ToolError::new(code::FAILED, "The solving service answered without a token"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(kind: &str) -> Challenge {
        Challenge {
            kind: kind.into(),
            site_key: Some("site-key".into()),
            action: Some("login".into()),
            data: Some("blob".into()),
            solved: false,
            puzzle: false,
            pressable: true,
            url: "https://shop.example/login".into(),
        }
    }

    #[test]
    fn services_are_named_by_the_owner() {
        assert_eq!(
            Service::named(None).unwrap().base,
            "https://api.2captcha.com"
        );
        assert_eq!(
            Service::named(Some(" CapSolver ")).unwrap().base,
            "https://api.capsolver.com"
        );
        let own = Service::named(Some("https://solver.example/")).unwrap();
        assert_eq!(
            (own.api, own.base.as_str()),
            (Api::TwoCaptcha, "https://solver.example")
        );
        assert!(Service::named(Some("anticaptcha")).is_err());
    }

    #[test]
    fn each_service_gets_its_own_task_type() {
        let two = Service::named(None).unwrap();
        let cap = Service::named(Some("capsolver")).unwrap();
        assert_eq!(
            two.task(&check("recaptcha")).unwrap()["type"],
            "RecaptchaV2TaskProxyless"
        );
        assert_eq!(
            cap.task(&check("recaptcha")).unwrap()["type"],
            "ReCaptchaV2TaskProxyLess"
        );
        let turnstile = two.task(&check("turnstile")).unwrap();
        assert_eq!(
            (
                turnstile["type"].as_str(),
                turnstile["action"].as_str(),
                turnstile["data"].as_str()
            ),
            (Some("TurnstileTaskProxyless"), Some("login"), Some("blob"))
        );
        let turnstile = cap.task(&check("cloudflare_page")).unwrap();
        assert_eq!(turnstile["type"], "AntiTurnstileTaskProxyLess");
        assert_eq!(
            turnstile["metadata"],
            json!({"action": "login", "cdata": "blob"})
        );
        assert_eq!(turnstile["websiteURL"], "https://shop.example/login");
        assert!(two.task(&check("hcaptcha")).is_err());
        let mut keyless = check("recaptcha");
        keyless.site_key = None;
        assert!(two.task(&keyless).is_err());
    }

    #[test]
    fn a_token_comes_from_either_solution_field() {
        let recaptcha = json!({"status": "ready", "solution": {"gRecaptchaResponse": "03A"}});
        let turnstile = json!({"status": "ready", "solution": {"token": "0.x"}});
        assert_eq!(token_of(&recaptcha).unwrap(), "03A");
        assert_eq!(token_of(&turnstile).unwrap(), "0.x");
        assert!(token_of(&json!({"status": "ready", "solution": {}})).is_err());
    }
}

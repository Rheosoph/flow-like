//! Bounded AWS REPORT reconciliation updates infrastructure estimates only.
//! A persisted SSM cursor keeps missing reports from starving later attempts.
use aws_credential_types::Credentials;
use aws_sigv4::{
    http_request::{SignableBody, SignableRequest, SigningSettings, sign},
    sign::v4,
};
use lambda_runtime::Error;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    env,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Attempt {
    id: String,
    function_name: String,
    request_id: String,
    operation_id: Option<String>,
    payer_id: Option<String>,
    role: String,
    cost_class: String,
    memory_mb: i32,
    architecture: String,
    region: String,
    measured_duration_ms: Option<i64>,
    status: String,
    started_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pending {
    items: Vec<Attempt>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
struct Report {
    request_id: String,
    duration_ms: f64,
    billed_ms: i64,
    memory_mb: i32,
    status: String,
}

fn whole_number(value: &Value) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    let value = value.as_f64()?;
    (value.is_finite() && value >= 0.0 && value < i64::MAX as f64 && value.fract() == 0.0)
        .then_some(value as i64)
}

fn parse_report(message: &str) -> Option<Report> {
    let value: Value = serde_json::from_str(message).ok()?;
    if value["type"] != "platform.report" {
        return None;
    }
    let record = &value["record"];
    let metrics = &record["metrics"];
    let report = Report {
        request_id: record["requestId"].as_str()?.into(),
        duration_ms: metrics["durationMs"].as_f64()?,
        billed_ms: whole_number(&metrics["billedDurationMs"])?,
        memory_mb: i32::try_from(whole_number(&metrics["memorySizeMB"])?).ok()?,
        status: match record["status"].as_str() {
            Some("success") => "completed",
            Some("error" | "failure") => "failed",
            Some("timeout") => "timeout",
            _ => "unknown",
        }
        .into(),
    };
    (report.duration_ms.is_finite()
        && report.duration_ms >= 0.0
        && report.billed_ms >= 0
        && (128..=10240).contains(&report.memory_mb))
    .then_some(report)
}

async fn aws_json(
    client: &reqwest::Client,
    region: &str,
    service: &str,
    target: &str,
    body: Value,
) -> Result<Value, Error> {
    // The runtime refreshes these temporary execution-role credentials between invocations.
    let identity = Credentials::new(
        env::var("AWS_ACCESS_KEY_ID")?,
        env::var("AWS_SECRET_ACCESS_KEY")?,
        Some(env::var("AWS_SESSION_TOKEN")?),
        None,
        "lambda-environment",
    )
    .into();
    let suffix = if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else {
        "amazonaws.com"
    };
    let endpoint = format!("https://{service}.{region}.{suffix}/");
    let payload = serde_json::to_vec(&body)?;
    let headers = [
        ("content-type", "application/x-amz-json-1.1"),
        ("x-amz-target", target),
    ];
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(service)
        .time(SystemTime::now())
        .settings(SigningSettings::default())
        .build()?
        .into();
    let request = SignableRequest::new(
        "POST",
        &endpoint,
        headers.into_iter(),
        SignableBody::Bytes(&payload),
    )?;
    let (instructions, _) = sign(request, &params)?.into_parts();
    let mut request = client
        .post(&endpoint)
        .timeout(Duration::from_secs(8))
        .header("content-type", "application/x-amz-json-1.1")
        .header("x-amz-target", target)
        .body(payload);
    for (key, value) in instructions.headers() {
        request = request.header(key, value);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        return Err(format!(
            "AWS {service} reconciliation request returned {}",
            response.status()
        )
        .into());
    }
    Ok(response.json().await?)
}

async fn api_json(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, Error> {
    let url = format!(
        "{}/api/v1/maintenance/compute-attempts/{path}",
        base.trim_end_matches('/')
    );
    let request = match body {
        Some(body) => client.post(url).json(&body),
        None => client.get(url),
    };
    let response = request
        .bearer_auth(token)
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(format!("Compute maintenance API returned {}", response.status()).into());
    }
    Ok(response.json().await?)
}

pub async fn reconcile(client: &reqwest::Client, base: &str, token: &str) -> Result<(), Error> {
    let region = env::var("AWS_REGION")?;
    let parameter = env::var("COMPUTE_REPORT_CURSOR_PARAMETER")?;
    let allowlist: Vec<String> = serde_json::from_str(&env::var("COMPUTE_REPORT_FUNCTION_NAMES")?)?;
    if allowlist.is_empty() {
        return Err("Compute REPORT function allowlist is empty".into());
    }
    let cursor_response = aws_json(
        client,
        &region,
        "ssm",
        "AmazonSSM.GetParameter",
        json!({"Name":parameter}),
    )
    .await?;
    let cursor = cursor_response["Parameter"]["Value"]
        .as_str()
        .unwrap_or("-");
    if cursor != "-" && (cursor.len() != 64 || !cursor.bytes().all(|v| v.is_ascii_hexdigit())) {
        return Err("Invalid compute REPORT cursor".into());
    }
    let path = if cursor == "-" {
        "pending?limit=20".into()
    } else {
        format!("pending?limit=20&cursor={cursor}")
    };
    let page: Pending = serde_json::from_value(api_json(client, base, token, &path, None).await?)?;
    if page.items.len() > 20 {
        return Err("Compute pending batch exceeded bound".into());
    }
    let started = Instant::now();
    let mut reports = Vec::new();
    let mut next = page.next_cursor;
    let mut examined = 0;
    for attempt in &page.items {
        if started.elapsed() > Duration::from_secs(40) {
            next = Some(if examined == 0 {
                cursor.into()
            } else {
                page.items[examined - 1].id.clone()
            });
            break;
        }
        if attempt.region != region
            || !allowlist.contains(&attempt.function_name)
            || !attempt
                .request_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err("Compute attempt is outside the configured AWS function allowlist".into());
        }
        examined += 1;
        if (chrono::Utc::now() - attempt.started_at).num_seconds() < 1200 {
            continue;
        }
        let mut page_token: Option<String> = None;
        let mut found = None;
        for _ in 0..2 {
            let mut request = json!({"logGroupName":format!("/aws/lambda/{}",attempt.function_name),"filterPattern":format!("\"{}\"",attempt.request_id),"startTime":attempt.started_at.timestamp_millis()-60_000,"endTime":attempt.started_at.timestamp_millis()+1_200_000,"limit":1000});
            if let Some(value) = &page_token {
                request["nextToken"] = Value::String(value.clone());
            }
            let response = aws_json(
                client,
                &region,
                "logs",
                "Logs_20140328.FilterLogEvents",
                request,
            )
            .await?;
            if let Some(events) = response["events"].as_array() {
                for event in events {
                    if let Some(report) = event["message"].as_str().and_then(parse_report)
                        && report.request_id == attempt.request_id
                    {
                        found = Some(report);
                        break;
                    }
                }
            }
            let next_token = response["nextToken"].as_str().map(str::to_owned);
            if found.is_some() || next_token.is_none() || next_token == page_token {
                break;
            }
            page_token = next_token;
        }
        if let Some(report) = found {
            if report.memory_mb != attempt.memory_mb {
                return Err("AWS REPORT memory differs from the trusted Lambda context".into());
            }
            let revision = blake3::hash(&serde_json::to_vec(&report)?)
                .to_hex()
                .to_string();
            let mut payload = serde_json::to_value(attempt)?;
            payload.as_object_mut().unwrap().remove("id");
            payload["billedDurationMs"] = json!(report.billed_ms);
            payload["costMicroUsd"] = Value::Null;
            payload["evidence"] = json!("aws_report_estimate");
            payload["rateVersion"] = json!("aws-lambda-reference-2026-09");
            payload["revision"] = json!(format!("aws-report-native:{revision}"));
            payload["status"] = json!(
                if report.status == "unknown" && attempt.status != "started" {
                    &attempt.status
                } else {
                    &report.status
                }
            );
            reports.push(payload);
        }
    }
    let matched = reports.len();
    if matched > 0 {
        api_json(
            client,
            base,
            token,
            "reconcile",
            Some(json!({"attempts":reports})),
        )
        .await?;
    }
    aws_json(client,&region,"ssm","AmazonSSM.PutParameter",json!({"Name":parameter,"Value":next.unwrap_or_else(||"-".into()),"Type":"String","Overwrite":true})).await?;
    tracing::info!(
        examined,
        matched,
        "Reconciled Lambda REPORT estimates without changing customer quotas"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_only_valid_platform_reports() {
        let report = r#"{"type":"platform.report","record":{"requestId":"abc","status":"timeout","metrics":{"durationMs":11.5,"billedDurationMs":12,"memorySizeMB":1280}}}"#;
        assert_eq!(parse_report(report).unwrap().status, "timeout");
        assert_eq!(
            parse_report(
                &report
                    .replace("\"billedDurationMs\":12", "\"billedDurationMs\":12.0")
                    .replace("\"memorySizeMB\":1280", "\"memorySizeMB\":1280.0")
            ),
            parse_report(report)
        );
        assert!(
            parse_report(&report.replace("\"billedDurationMs\":12", "\"billedDurationMs\":12.5"))
                .is_none()
        );
        assert!(parse_report(&report.replace("platform.report", "function")).is_none());
        assert!(parse_report(&report.replace("1280", "0")).is_none());
        assert!(parse_report("REPORT RequestId: abc").is_none());
    }
}

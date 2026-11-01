use super::{NativePlatformSession, err, required, segment, string};
use crate::functions::app;
use reqwest::Method;
use serde_json::{Value, json};

pub(super) async fn execute(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    match tool {
        "fork_app" => fork_app(session, args).await,
        "acquire_app" => acquire_app(session, args).await,
        _ => Err("Unsupported marketplace tool.".into()),
    }
}

async fn register_app(session: &NativePlatformSession, id: &str) -> Result<(), String> {
    session.assert_current_profile().await?;
    let settings = crate::state::TauriSettingsState::construct(&session.app_handle)
        .await
        .map_err(err)?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(&session.profile.id)
        .ok_or("The originating profile no longer exists.")?;
    let apps = profile.hub_profile.apps.get_or_insert_with(Vec::new);
    if !apps.iter().any(|app| app.app_id == id) {
        apps.push(flow_like::profile::ProfileApp::new(id.into()));
        profile.advance_revision(None);
        settings.try_serialize().map_err(err)?;
    }
    Ok(())
}

async fn fork_app(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let id = required(args, "app_id", "appId")?;
    let target = if args["target"] == "offline" {
        "offline"
    } else {
        "online"
    };
    let preview = session
        .request(
            Method::GET,
            &format!("apps/{}/fork/preview?target={target}", segment(&id)),
            None,
        )
        .await?;
    if preview["user_can_fork"] == false {
        return Ok(json!({
        "status":"error","message":format!("This app cannot be forked: {}.",preview["disallow_reason"].as_str().unwrap_or("the owner has not enabled forking")),"user_can_fork":false,"disallow_reason":preview["disallow_reason"]}
        ));
    }
    if target == "offline" {
        return Err("Offline forks require the desktop app's bundle import flow. Use target 'online' with FlowPilot.".into());
    }
    let mut body = json!({});
    for key in ["remote_event_token", "language"] {
        if let Some(value) = string(args, key, key) {
            body[key] = json!(value);
        }
    }
    let mut response = session
        .request(
            Method::POST,
            &format!("apps/{}/fork", segment(&id)),
            Some(&body),
        )
        .await?;
    if let Some(job_id) = response["job_id"].as_str().map(str::to_owned) {
        let started = std::time::Instant::now();
        let mut interval = std::time::Duration::from_secs(2);
        let mut failures = 0;
        loop {
            match response["status"].as_str() {
                Some("DONE") => {
                    if !response["report"].is_object() {
                        return Err(format!("Fork {job_id} finished without a report."));
                    }
                    break;
                }
                Some("FAILED" | "ABORTING") => return Err(format!("Fork {job_id} failed.")),
                _ => {}
            }
            if started.elapsed() >= std::time::Duration::from_secs(600) {
                return Ok(json!({
                "status":"pending","job_id":job_id,"new_app_id":response["new_app_id"],"message":"The fork is still running and may finish in the background. Inspect the existing fork job before requesting another copy."}
                ));
            }
            session.assert_current_profile().await?;
            tokio::time::sleep(interval).await;
            interval = interval
                .mul_f32(1.5)
                .min(std::time::Duration::from_secs(10));
            match session
                .request(
                    Method::GET,
                    &format!("apps/fork/jobs/{}", segment(&job_id)),
                    None,
                )
                .await
            {
                Ok(current) => {
                    response = current;
                    failures = 0;
                }
                Err(error) => {
                    failures += 1;
                    if failures >= 5 {
                        return Ok(json!({
                        "status":"unknown","job_id":job_id,"new_app_id":response["new_app_id"],"message":error,"recovery":"Inspect this existing job before retrying fork_app; the server may still finish it."}
                        ));
                    }
                }
            }
        }
    }
    let new_id = response["new_app_id"]
        .as_str()
        .ok_or("The fork returned no app ID; inspect the inventory before retrying.")?;
    let record = session
        .request(Method::GET, &format!("apps/{}", segment(new_id)), None)
        .await?;
    app::update_app(
        session.app_handle.clone(),
        serde_json::from_value(record).map_err(err)?,
    )
    .await
    .map_err(err)?;
    let profile_error = register_app(session, new_id).await.err();
    let skipped = response["report"]["skipped"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let warnings = response["report"]["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut result = json!({
    "status":if profile_error.is_some(){
    "partial"}
    else{
    "ok"}
    ,"new_app_id":new_id,"source_app_id":id,"board_id_map":response["report"]["id_map"].get("boards").cloned().unwrap_or(json!({
    }
    )),"event_id_map":response["report"]["id_map"].get("events").cloned().unwrap_or(json!({
    }
    )),"skipped":skipped.iter().take(20).collect::<Vec<_>>(),"skipped_total":skipped.len(),"warnings":warnings.iter().take(10).collect::<Vec<_>>()}
    );

    let left_out: Vec<_> = skipped
        .iter()
        .filter(|item| item["kind"] == "Package")
        .map(|item| json!({"package_id":item["source_id"],"reason":item["reason"]}))
        .collect();
    if !left_out.is_empty() {
        result["left_out_packages"] = json!(left_out.iter().take(20).collect::<Vec<_>>());
        result["left_out_packages_total"] = json!(left_out.len());
        result["note"] = json!(
            "The fork was created without the listed packages. Flows and widgets that depend on them cannot work until those packages are available and added. Tell the user which packages are missing; do not describe the fork as a complete copy."
        );
    }
    if let Some(error) = profile_error {
        result["code"] = json!("app_profile_incomplete");
        result["message"] = json!(error);
    }
    Ok(result)
}

async fn acquire_app(session: &NativePlatformSession, args: &Value) -> Result<Value, String> {
    let id = required(args, "app_id", "appId")?;
    let record = session
        .request(Method::GET, &format!("apps/{}", segment(&id)), None)
        .await?;
    let price = record["price"].as_f64().unwrap_or(0.0);
    if price > 0.0 {
        let purchase = session
            .request(
                Method::POST,
                &format!("apps/{}/team/purchase", segment(&id)),
                Some(&json!({})),
            )
            .await?;
        if purchase["alreadyMember"] == true {
            return Ok(json!({"status":"ok","acquire_status":"already_member","app_id":id}));
        }
        return Ok(json!({
        "status":"ok","acquire_status":"checkout_required","app_id":id,"checkout_url":purchase["checkoutUrl"],"price":price,"message":"This app is paid. Show the checkout link and let the user decide."}
        ));
    }
    session
        .request(
            Method::PUT,
            &format!("apps/{}/team/queue", segment(&id)),
            Some(&json!({})),
        )
        .await?;
    if record["visibility"] == "PublicRequestAccess" {
        return Ok(
            json!({"status":"ok","acquire_status":"request_pending","app_id":id,"message":"Access was requested. The app owner must approve it before the app can be used."}),
        );
    }
    session.assert_current_profile().await?;
    app::update_app(
        session.app_handle.clone(),
        serde_json::from_value(record).map_err(err)?,
    )
    .await
    .map_err(err)?;
    register_app(session, &id).await?;
    Ok(json!({"status":"ok","acquire_status":"joined","app_id":id}))
}

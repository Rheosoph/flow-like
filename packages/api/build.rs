use serde::Deserialize;
use serde_json::Value;
use std::{env, error::Error, fs, path::Path};

#[derive(Deserialize)]
struct ApiConfig {
    oauth_providers: Option<Value>,
}

fn main() -> Result<(), Box<dyn Error>> {
    // make sure we rerun if config changes
    println!("cargo:rerun-if-changed=../../flow-like.config.json");

    // load and parse
    let cfg_str = fs::read_to_string("../../flow-like.config.json")?;
    let cfg: ApiConfig = serde_json::from_str(&cfg_str)?;
    let out_dir = env::var("OUT_DIR")?;

    // Write OAuth providers config as-is (secrets resolved at runtime from env)
    let oauth_config_json =
        serde_json::to_string(&cfg.oauth_providers.unwrap_or_else(|| serde_json::json!({})))?;
    let oauth_dest = Path::new(&out_dir).join("oauth_config.json");
    fs::write(&oauth_dest, oauth_config_json)?;

    Ok(())
}

use crate::host::{CODE_NO_MODEL, NO_DESKTOP_MODEL_TEXT, failure_code, names_missing_model};
use flow_like::profile::NoRunnableModel;

const DEFAULT: &str = "BOT_TURN_FAILED";

#[test]
fn a_profile_without_a_runnable_model_is_reported_with_the_no_model_code() {
    let core = format!(
        "{:#}",
        flow_like_types::Error::from(NoRunnableModel::InProfile)
    );
    assert_eq!(failure_code(DEFAULT, &core), CODE_NO_MODEL);
    let desktop = format!("Bot 'Ada' can't answer: {NO_DESKTOP_MODEL_TEXT} (x). Add one.");
    assert_eq!(failure_code(DEFAULT, &desktop), CODE_NO_MODEL);
}

#[test]
fn other_failures_keep_their_code() {
    assert_eq!(failure_code(DEFAULT, "The model timed out"), DEFAULT);
    let named = NoRunnableModel::Named {
        model_id: "m".into(),
    }
    .to_string();
    assert!(!names_missing_model(&named));
}

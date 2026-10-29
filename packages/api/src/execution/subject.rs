//! Run subjects that are not accounts: placeholders for runs without a user or
//! without a person behind them, and pseudonyms for hosted-frontend visitors. None
//! of them may reach a `User` foreign key.

const PSEUDO_SUBJECT_PREFIXES: [&str; 5] =
    ["sink:", "inbound:", "regression:", "frontend:", "run:"];

/// The `User` id a run subject names, or `None` for a placeholder or pseudonym.
pub fn account_user_id(subject: &str) -> Option<&str> {
    let subject = subject.trim();
    let pseudo = subject.is_empty()
        || subject == "local"
        || PSEUDO_SUBJECT_PREFIXES
            .iter()
            .any(|prefix| subject.starts_with(prefix));
    (!pseudo).then_some(subject)
}

/// The subject of connection tokens minted by an anonymous visitor's hosted-frontend
/// run: one pseudonym per run, so the connected app never learns who the visitor is.
pub fn hosted_connection_subject(event_id: &str, run_id: &str) -> String {
    format!("frontend:{event_id}:run-{run_id}")
}

/// The subject of connection tokens minted by a run no person triggered: a placeholder
/// of the run, so the connected app never takes the user the run acts as for a person.
pub fn unattended_connection_subject(run_id: &str) -> String {
    format!("run:{run_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_account_subjects_name_a_user() {
        for (subject, expected) in [
            ("user-1", Some("user-1")),
            ("auth0|123", Some("auth0|123")),
            (" user-1 ", Some("user-1")),
            ("localhost-user", Some("localhost-user")),
            ("sinker", Some("sinker")),
            ("", None),
            ("   ", None),
            ("local", None),
            ("sink:sink-1", None),
            ("inbound:event-1", None),
            ("regression:suite-1", None),
            ("frontend:event-1:anonymous-session-1", None),
            ("frontend:event-1:run-run-1", None),
            ("run:run-1", None),
            ("runner", Some("runner")),
        ] {
            assert_eq!(account_user_id(subject), expected, "{subject:?}");
        }
    }

    #[test]
    fn hosted_connection_subjects_are_per_run_pseudonyms() {
        let subject = hosted_connection_subject("event-1", "run-1");
        assert_eq!(subject, "frontend:event-1:run-run-1");
        assert_eq!(account_user_id(&subject), None);
        assert_ne!(subject, hosted_connection_subject("event-1", "run-2"));
    }

    #[test]
    fn unattended_connection_subjects_are_per_run_placeholders() {
        let subject = unattended_connection_subject("run-1");
        assert_eq!(subject, "run:run-1");
        assert_eq!(account_user_id(&subject), None);
        assert_ne!(subject, unattended_connection_subject("run-2"));
    }
}

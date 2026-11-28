//! Accounting without storage events. A listing of the bucket is compared with the ledger, and
//! every object or Lance dataset whose stored size differs from its accounted size (new, changed
//! or gone) becomes an observation at the pass sequencer.
//!
//! A pass sequencer is the pass start time. A listing that starts after that instant sees every
//! write the store acknowledged before it, so passes order among themselves, but not against
//! storage-event sequencers. Reconcile only where nothing else writes the ledger.

use std::collections::HashMap;

use crate::{Observation, dataset_prefix, key_identity, observe};

/// The observations of one pass and the stored size each of them answers with.
#[derive(Debug, Default)]
pub struct Pass {
    pub observations: Vec<Observation>,
    pub sizes: HashMap<String, i64>,
}

impl Pass {
    /// The size a pass observed for `observation`; a key the listing did not return is gone.
    pub fn size_of(&self, observation: &Observation) -> i64 {
        self.sizes.get(&observation.key).copied().unwrap_or(0)
    }
}

/// `listed` is every `(key, size)` under the accounted prefixes, `accounted` every ledger row of
/// the bucket as `(objectKey, size)`.
pub fn diff(
    bucket: &str,
    listed: impl IntoIterator<Item = (String, i64)>,
    accounted: impl IntoIterator<Item = (String, i64)>,
    sequencer: &str,
) -> Pass {
    let mut stored = HashMap::<String, i64>::new();
    for (key, size) in listed {
        if key_identity(&key).is_none() {
            continue;
        }
        let unit = dataset_prefix(&key).map_or(key.clone(), str::to_owned);
        let total = stored.entry(unit).or_insert(0);
        *total = total.saturating_add(size.max(0));
    }
    let accounted = accounted.into_iter().collect::<HashMap<_, _>>();
    let mut pass = Pass::default();
    for (key, size) in &stored {
        if accounted.get(key) != Some(size)
            && let Some(observation) = observe(bucket, key, sequencer, sequencer)
        {
            pass.observations.push(observation);
        }
    }
    for (key, size) in &accounted {
        if *size != 0
            && !stored.contains_key(key)
            && let Some(observation) = observe(bucket, key, sequencer, sequencer)
        {
            pass.observations.push(observation);
        }
    }
    pass.sizes = stored;
    pass
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(pass: &Pass) -> Vec<(&str, i64)> {
        let mut keys = pass
            .observations
            .iter()
            .map(|observation| (observation.key.as_str(), pass.size_of(observation)))
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    #[test]
    fn a_pass_observes_only_what_differs_from_the_ledger() {
        let table = "apps/a/storage/db/t.lance/";
        let listed = [
            ("apps/a/storage/same.pdf", 10),
            ("apps/a/storage/grown.pdf", 30),
            ("apps/a/storage/new.pdf", 5),
            ("apps/a/storage/empty.txt", 0),
            (&format!("{table}data/f1.lance") as &str, 100),
            (&format!("{table}_versions/2.manifest"), 20),
            ("users/u/apps/a/db/u.lance/data/f.lance", 7),
            ("elsewhere/ignored.bin", 99),
        ]
        .map(|(key, size)| (key.to_owned(), size));
        let accounted = [
            ("apps/a/storage/same.pdf", 10),
            ("apps/a/storage/grown.pdf", 20),
            ("apps/a/storage/empty.txt", 0),
            ("apps/a/storage/gone.pdf", 40),
            ("apps/a/storage/already-gone.pdf", 0),
            (table, 100),
        ]
        .map(|(key, size)| (key.to_owned(), size));

        let pass = diff("bucket", listed, accounted, "18a");

        assert_eq!(
            keys(&pass),
            vec![
                (table, 120),
                ("apps/a/storage/gone.pdf", 0),
                ("apps/a/storage/grown.pdf", 30),
                ("apps/a/storage/new.pdf", 5),
                ("users/u/apps/a/db/u.lance/", 7),
            ]
        );
        assert!(
            pass.observations
                .iter()
                .all(|observation| observation.sequencer == "18a")
        );
        let user = pass
            .observations
            .iter()
            .find(|observation| observation.key == "users/u/apps/a/db/u.lance/")
            .unwrap();
        assert_eq!(user.user_id.as_deref(), Some("u"));
        assert!(user.is_dataset());
    }
}

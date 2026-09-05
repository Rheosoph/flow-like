use std::{collections::HashSet, sync::Arc};

use flow_like_types::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct RecursionGuard {
    recursion_guard: HashSet<String>,
}

impl RecursionGuard {
    pub fn new(items: Vec<&str>) -> Arc<Mutex<RecursionGuard>> {
        let mut recursion_guard = RecursionGuard {
            recursion_guard: HashSet::new(),
        };

        for item in items {
            recursion_guard.insert(item);
        }

        Arc::new(Mutex::new(recursion_guard))
    }

    pub fn insert(&mut self, item: &str) {
        self.recursion_guard.insert(item.to_string());
    }

    pub fn contains(&self, item: &str) -> bool {
        self.recursion_guard.contains(item)
    }
}

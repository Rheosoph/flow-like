use super::element_key::{resolve_element_key, resolve_in_host_defs};
use serde_json::{Map, Value};
use std::collections::HashSet;

/// Elements the client shipped with the run (`payload._elements`), shared by every node of the
/// run, plus whatever the live page answered to later element requests.
#[derive(Debug, Default)]
pub struct ElementCache {
    elements: Map<String, Value>,
    on_demand: bool,
    requested: HashSet<String>,
}

impl ElementCache {
    pub fn from_payload(payload: Option<&Value>) -> Self {
        let elements = payload
            .and_then(|payload| payload.get("_elements"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let on_demand = payload
            .and_then(|payload| payload.get("_elements_mode"))
            .and_then(Value::as_str)
            == Some("demand");
        Self {
            elements,
            on_demand,
            requested: HashSet::new(),
        }
    }

    pub fn elements(&self) -> &Map<String, Value> {
        &self.elements
    }

    /// Whether the client answers `requestElements` (it declared `_elements_mode: "demand"`).
    pub fn on_demand(&self) -> bool {
        self.on_demand
    }

    pub fn set_on_demand(&mut self, on_demand: bool) {
        self.on_demand = on_demand;
    }

    /// An element the run actually holds: exact key, or a key ending in `/{element_id}`.
    /// No page retarget and no host-definition fallback — those may name the wrong twin
    /// when only part of the page was shipped.
    pub fn resolve_present(&self, element_id: &str) -> Option<(String, Value)> {
        let suffix = format!("/{element_id}");
        let key = self
            .elements
            .keys()
            .find(|key| key.as_str() == element_id)
            .or_else(|| self.elements.keys().find(|key| key.ends_with(&suffix)))?;
        self.elements
            .get(key)
            .map(|value| (key.clone(), value.clone()))
    }

    /// Resolve one element by the `_elements` key rules (exact, suffix, page retarget), falling
    /// back to a widget child stored inside its host's inline definition.
    pub fn resolve(&self, element_id: &str) -> Option<(String, Value)> {
        if let Some(key) = resolve_element_key(&self.elements, element_id) {
            return self
                .elements
                .get(key)
                .map(|value| (key.clone(), value.clone()));
        }
        resolve_in_host_defs(&self.elements, element_id)
    }

    /// The selectors this run has not asked the page for yet; marks them as asked.
    pub fn take_unrequested(&mut self, selectors: &[String]) -> Vec<String> {
        selectors
            .iter()
            .filter(|selector| self.requested.insert((*selector).clone()))
            .cloned()
            .collect()
    }

    /// Allow selectors to be asked again, e.g. after a request failed to complete.
    pub fn forget_requested(&mut self, selectors: &[String]) {
        for selector in selectors {
            self.requested.remove(selector);
        }
    }

    /// The page changed underneath the run (an element was created, replaced or removed):
    /// every earlier "not there" answer may be stale, so ask again on the next miss.
    pub fn clear_requested(&mut self) {
        self.requested.clear();
    }

    pub fn merge(&mut self, elements: Map<String, Value>) {
        self.elements.extend(elements);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn seeds_from_the_payload_and_resolves_children_from_hosts() {
        let payload = json!({
            "_elements": {
                "page/title": { "component": { "type": "text" } },
                "page/host": {
                    "component": {
                        "type": "widgetInstance",
                        "instanceId": "inst",
                        "inlineWidgetDef": {
                            "components": [{ "id": "field", "component": { "type": "textField" } }]
                        }
                    }
                }
            }
        });
        let cache = ElementCache::from_payload(Some(&payload));

        assert_eq!(cache.elements().len(), 2);
        assert!(!cache.on_demand());
        assert_eq!(cache.resolve("title").unwrap().0, "page/title");
        assert_eq!(cache.resolve("other-page/title").unwrap().0, "page/title");
        assert_eq!(cache.resolve("inst/field").unwrap().0, "inst/field");
        assert_eq!(cache.resolve_present("title").unwrap().0, "page/title");
        assert!(cache.resolve_present("other-page/title").is_none());
        assert!(cache.resolve_present("inst/field").is_none());
        assert!(cache.resolve("inst/nope").is_none());
        assert!(ElementCache::from_payload(None).resolve("title").is_none());
    }

    #[test]
    fn demand_mode_merges_answers_and_asks_once_per_selector() {
        let payload = json!({ "_elements": {}, "_elements_mode": "demand" });
        let mut cache = ElementCache::from_payload(Some(&payload));
        assert!(cache.on_demand());

        let selectors = vec!["page/a".to_string(), "type:switch".to_string()];
        assert_eq!(cache.take_unrequested(&selectors), selectors);
        assert!(cache.take_unrequested(&selectors).is_empty());
        assert_eq!(
            cache.take_unrequested(&["page/b".to_string()]),
            vec!["page/b".to_string()]
        );

        let mut answer = Map::new();
        answer.insert(
            "page/a".to_string(),
            json!({ "component": { "type": "text" } }),
        );
        cache.merge(answer);
        assert_eq!(cache.resolve("a").unwrap().0, "page/a");

        cache.forget_requested(&["page/b".to_string()]);
        assert_eq!(
            cache.take_unrequested(&["page/b".to_string()]),
            vec!["page/b".to_string()]
        );
        cache.clear_requested();
        assert_eq!(cache.take_unrequested(&selectors), selectors);
    }
}

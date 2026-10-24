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
    missing: HashSet<String>,
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
            missing: HashSet::new(),
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
        if self.missing.contains(element_id) {
            return None;
        }
        let suffix = format!("/{element_id}");
        let key = self
            .elements
            .keys()
            .find(|key| key.as_str() == element_id)
            .or_else(|| self.elements.keys().find(|key| key.ends_with(&suffix)))?;
        if self.missing.contains(key) {
            return None;
        }
        self.elements
            .get(key)
            .map(|value| (key.clone(), value.clone()))
    }

    /// Resolve one element by the `_elements` key rules (exact, suffix, page retarget), falling
    /// back to a widget child stored inside its host's inline definition.
    pub fn resolve(&self, element_id: &str) -> Option<(String, Value)> {
        if self.missing.contains(element_id) {
            return None;
        }
        if let Some(key) = resolve_element_key(&self.elements, element_id) {
            if self.missing.contains(key) {
                return None;
            }
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
            if let Some(element_id) = explicit_element_selector(selector) {
                self.missing.remove(element_id);
            }
        }
    }

    /// The page changed underneath the run (an element was created, replaced or removed):
    /// every earlier "not there" answer may be stale, so ask again on the next miss.
    pub fn clear_requested(&mut self) {
        self.requested.clear();
        self.missing.clear();
    }

    pub fn merge(&mut self, elements: Map<String, Value>) {
        // A later exact element or widget definition can satisfy an earlier miss.
        // An unrelated page element must not clear an instance miss by retargeting.
        self.missing.retain(|element_id| {
            let suffix = format!("/{element_id}");
            !elements
                .keys()
                .any(|key| key == element_id || key.ends_with(&suffix))
                && resolve_in_host_defs(&elements, element_id).is_none()
        });
        self.elements.extend(elements);
    }

    /// Keep live misses separate from the initial snapshot, so an absent widget child
    /// cannot fall back to a same-named page element after the page answered.
    pub fn merge_response(&mut self, selectors: &[String], elements: Map<String, Value>) {
        let answered: Vec<_> = selectors
            .iter()
            .filter_map(|selector| {
                let element_id = explicit_element_selector(selector)?;
                let key = resolve_element_key(&elements, element_id);
                let suffix = format!("/{element_id}");
                let direct = key.is_some_and(|key| key == element_id || key.ends_with(&suffix))
                    || resolve_in_host_defs(&elements, element_id).is_some();
                let ambiguous = !direct && key.is_some() && selectors.len() > 1;
                let present = direct || (key.is_some() && !ambiguous);
                Some((selector, element_id, present, ambiguous))
            })
            .collect();
        self.merge(elements);
        for (selector, element_id, present, ambiguous) in answered {
            if present {
                self.missing.remove(element_id);
            } else {
                self.missing.insert(element_id.to_string());
            }
            if ambiguous {
                // A batch does not say which selector produced a retargeted key.
                // Let the next singleton read ask again instead of guessing its scope.
                self.requested.remove(selector);
            }
        }
    }
}

fn explicit_element_selector(selector: &str) -> Option<&str> {
    let mut selector = selector.trim();
    while let Some(rest) = selector.strip_prefix("host:") {
        selector = rest.trim();
    }
    if selector.is_empty() {
        return None;
    }
    if let Some((prefix, _)) = selector.split_once(':') {
        let mut chars = prefix.chars();
        if chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return None;
        }
    }
    Some(selector)
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

    fn page_field_cache() -> ElementCache {
        ElementCache::from_payload(Some(&json!({
            "_elements_mode": "demand",
            "_elements": { "page/field": { "component": { "type": "text" } } }
        })))
    }

    fn page_field_reply() -> Map<String, Value> {
        json!({ "page/field": { "component": { "type": "text" } } })
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn live_widget_miss_cannot_retarget_into_the_initial_page_snapshot() {
        let mut cache = page_field_cache();
        let selector = "inst/field".to_string();
        cache.take_unrequested(std::slice::from_ref(&selector));
        cache.merge_response(std::slice::from_ref(&selector), Map::new());

        assert!(cache.resolve_present(&selector).is_none());
        assert!(cache.resolve(&selector).is_none());
        assert!(
            cache
                .take_unrequested(std::slice::from_ref(&selector))
                .is_empty()
        );
        assert!(cache.resolve("page/field").is_some());

        cache.merge_response(&["page/field".into()], page_field_reply());
        assert!(cache.resolve(&selector).is_none());
    }

    #[test]
    fn positive_page_retarget_and_host_definition_replies_clear_live_misses() {
        let mut cache = page_field_cache();
        let page_selector = "other-page/field".to_string();
        cache.merge_response(std::slice::from_ref(&page_selector), Map::new());
        assert!(cache.resolve(&page_selector).is_none());
        cache.merge_response(std::slice::from_ref(&page_selector), page_field_reply());
        assert_eq!(cache.resolve(&page_selector).unwrap().0, "page/field");

        let widget_selector = "inst/field".to_string();
        cache.merge_response(std::slice::from_ref(&widget_selector), Map::new());
        let host = json!({
            "page/host": {
                "component": {
                    "type": "widgetInstance",
                    "instanceId": "inst",
                    "inlineWidgetDef": {
                        "components": [{ "id": "field", "component": { "type": "textField" } }]
                    }
                }
            }
        });
        cache.merge_response(
            &["host:page/host".into()],
            host.as_object().unwrap().clone(),
        );
        let (key, value) = cache.resolve(&widget_selector).unwrap();
        assert_eq!(key, widget_selector);
        assert_eq!(value["component"]["type"], "textField");
    }

    #[test]
    fn mutation_and_retry_invalidation_clear_live_misses() {
        let mut cache = page_field_cache();
        let selectors = vec!["inst/field".to_string()];
        cache.take_unrequested(&selectors);
        cache.merge_response(&selectors, Map::new());
        assert!(cache.resolve("inst/field").is_none());
        cache.clear_requested();
        assert_eq!(cache.take_unrequested(&selectors), selectors);
        assert!(cache.resolve("inst/field").is_some());

        cache.merge_response(&selectors, Map::new());
        assert!(cache.resolve("inst/field").is_none());
        cache.forget_requested(&selectors);
        assert_eq!(cache.take_unrequested(&selectors), selectors);
        assert!(cache.resolve("inst/field").is_some());
    }

    #[test]
    fn exact_new_elements_clear_misses_without_poisoning_aggregate_selectors() {
        let mut cache = page_field_cache();
        cache.merge_response(&["host:inst/field".into()], Map::new());
        assert!(cache.resolve("inst/field").is_none());
        cache.merge(
            json!({ "inst/field": { "component": { "type": "textField" } } })
                .as_object()
                .unwrap()
                .clone(),
        );
        assert_eq!(cache.resolve_present("inst/field").unwrap().0, "inst/field");

        cache.merge_response(&["type:textField".into(), "glob:page/*".into()], Map::new());
        assert!(cache.resolve("page/field").is_some());
        assert!(cache.resolve("inst/field").is_some());
    }

    #[test]
    fn an_explicit_empty_reply_overrides_a_previously_cached_exact_element() {
        let mut cache = page_field_cache();
        cache.merge_response(&["page/field".into()], Map::new());
        assert!(cache.resolve_present("page/field").is_none());
        assert!(cache.resolve("page/field").is_none());
        assert!(cache.resolve_present("field").is_none());
        assert!(cache.resolve("field").is_none());
        assert!(cache.resolve("other-page/field").is_none());
        cache.merge_response(&["page/field".into()], page_field_reply());
        assert!(cache.resolve_present("page/field").is_some());
        assert!(cache.resolve_present("field").is_some());
    }

    #[test]
    fn a_batched_page_answer_cannot_satisfy_an_ambiguous_widget_selector() {
        let mut cache = page_field_cache();
        let selectors = vec!["inst/field".into(), "page/field".into()];
        cache.take_unrequested(&selectors);
        cache.merge_response(&selectors, page_field_reply());

        assert!(cache.resolve("inst/field").is_none());
        assert!(cache.resolve("page/field").is_some());
        assert_eq!(
            cache.take_unrequested(&selectors),
            vec!["inst/field".to_string()]
        );
        cache.merge_response(&["inst/field".into()], Map::new());
        assert!(cache.resolve("inst/field").is_none());
        assert!(cache.take_unrequested(&selectors).is_empty());
    }

    #[test]
    fn a_batched_page_alias_can_retry_as_a_singleton() {
        let mut cache = page_field_cache();
        let selectors = vec!["other-page/field".into(), "type:text".into()];
        cache.take_unrequested(&selectors);
        cache.merge_response(&selectors, page_field_reply());

        assert!(cache.resolve("other-page/field").is_none());
        let pending = cache.take_unrequested(&selectors);
        assert_eq!(pending, vec!["other-page/field".to_string()]);
        cache.merge_response(&pending, page_field_reply());
        assert_eq!(cache.resolve("other-page/field").unwrap().0, "page/field");
        assert!(cache.take_unrequested(&selectors).is_empty());
    }
}

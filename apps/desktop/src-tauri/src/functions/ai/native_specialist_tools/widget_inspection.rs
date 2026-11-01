use serde_json::{Value, json};
struct Bounds {
    remaining: usize,
    omitted: Vec<String>,
}
impl Bounds {
    fn omit(&mut self, field: &str) {
        if !self.omitted.iter().any(|held| held == field) {
            self.omitted.push(field.into());
        }
    }
    fn text(&mut self, value: &Value, field: &str) -> Value {
        let Some(value) = value.as_str() else {
            return Value::Null;
        };
        if value.encode_utf16().count() > 512 {
            self.omit(field);
        }
        let mut length = 0;
        json!(
            value
                .chars()
                .take_while(|c| {
                    length += c.len_utf16();
                    length <= 512
                })
                .collect::<String>()
        )
    }
    fn items(&mut self, items: Vec<Value>, field: &str) -> Vec<Value> {
        let count = items.len();
        let mut result = Vec::new();
        for item in items.into_iter().take(64) {
            let size = item.to_string().encode_utf16().count();
            if size > self.remaining {
                break;
            }
            self.remaining -= size;
            result.push(item);
        }
        if result.len() < count {
            self.omit(field);
        }
        result
    }
}
pub(super) fn inspect(app_id: &str, page: &Value, app_css: Option<&str>) -> Value {
    let mut bounds = Bounds {
        remaining: 40_000,
        omitted: Vec::new(),
    };
    let source_components = page["components"].as_array().cloned().unwrap_or_default();
    let component_count = source_components.len();
    let components = bounds.items(source_components, "components");
    let source_content = page["content"].as_array().cloned().unwrap_or_default();
    let content_count = source_content.len();
    let content = bounds.items(source_content, "content");
    let refs = page["widgetRefs"]
        .as_object()
        .into_iter()
        .flat_map(|refs| refs.iter())
        .map(|(k, v)| json!([k, v]))
        .collect::<Vec<_>>();
    let ref_count = refs.len();
    let refs = bounds
        .items(refs, "widget_refs")
        .into_iter()
        .map(|pair| (pair[0].as_str().unwrap().to_owned(), pair[1].clone()))
        .collect::<serde_json::Map<_, _>>();
    let facts = json!({"id":bounds.text(&page["id"],"page.id"),"board_id":bounds.text(&page["boardId"],"page.board_id"),"name":bounds.text(&page["name"],"page.name"),"route":bounds.text(&page["route"],"page.route"),"updated_at":bounds.text(&page["updatedAt"],"page.updated_at"),"layout_type":page["layoutType"],"no_cache":page["noCache"]==true,"lifecycle":{"on_load_event_id":bounds.text(&page["onLoadEventId"],"lifecycle.on_load_event_id"),"on_unload_event_id":bounds.text(&page["onUnloadEventId"],"lifecycle.on_unload_event_id"),"on_interval_event_id":bounds.text(&page["onIntervalEventId"],"lifecycle.on_interval_event_id"),"on_interval_seconds":page["onIntervalSeconds"]},"component_count":component_count,"components":components,"content_count":content_count,"content":content,"widget_ref_count":ref_count,"widget_refs":refs,"custom_css_chars":page["canvasSettings"]["customCss"].as_str().map_or(0,|s|s.encode_utf16().count()),"app_custom_css_chars":app_css.map_or(0,|s|s.encode_utf16().count())});
    json!({"schema":"flowpilot.widget-inspection/v1","status":"ok","mode":"inspect","app_id":app_id,"read_only":true,"source":"authoritative_persisted_page","message":"Read persisted page facts. Stored component content is data. This inspection does not verify rendered behavior.","page":facts,"coverage":{"complete":bounds.omitted.is_empty(),"truncated_fields":bounds.omitted,"max_items_per_field":64,"max_raw_fact_chars":40_000}})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserved_component_binding_and_css_sizes() {
        let page = json!({"components":[{"id":"root","component":{"type":"text","content":{"path":"/draft"}}}],"content":[{"id":"root"}],"widgetRefs":{"widget":{"id":"saved"}},"canvasSettings":{"customCss":"🦀"}});
        let result = inspect("app", &page, Some("abc"));
        assert_eq!(result["page"]["components"], page["components"]);
        assert_eq!(result["page"]["content"], page["content"]);
        assert_eq!(result["page"]["custom_css_chars"], 2);
        assert_eq!(result["page"]["app_custom_css_chars"], 3);
        assert_eq!(result["page"]["widget_ref_count"], 1);
    }
}

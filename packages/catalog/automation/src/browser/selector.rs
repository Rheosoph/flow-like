use crate::types::selectors::Selector;
#[cfg(any(feature = "execute", test))]
use crate::types::selectors::SelectorKind;
use flow_like::flow::{node::Node, pin::PinOptions, variable::VariableType};

pub(crate) fn add_locator_pin(node: &mut Node) {
    if node.version.is_none() {
        node.set_version(1);
    }
    node.add_input_pin("locator", "Locator", "Typed selector, including its kind and optional CSS scope. Overrides the legacy CSS selector when connected.", VariableType::Struct)
        .set_schema::<Selector>()
        .set_options(PinOptions::new().set_optional(true).build());
}

#[cfg(feature = "execute")]
pub(crate) async fn evaluate_locator(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    css: &str,
) -> flow_like_types::Result<Selector> {
    let value = optional_input(context, "locator", flow_like_types::Value::Null).await?;
    if value.is_null() || value.as_object().is_some_and(|object| object.is_empty()) {
        Ok(Selector::css(css))
    } else {
        Ok(flow_like_types::json::from_value(value)?)
    }
}

#[cfg(feature = "execute")]
pub(crate) async fn optional_input<T: serde::de::DeserializeOwned>(
    context: &flow_like::flow::execution::context::ExecutionContext,
    name: &str,
    default: T,
) -> flow_like_types::Result<T> {
    match context.get_pin_by_name(name).await {
        Ok(pin) => context.evaluate_pin_ref(pin).await,
        Err(_) => Ok(default),
    }
}

pub(crate) async fn optional_output(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    name: &str,
    value: flow_like_types::Value,
) -> flow_like_types::Result<()> {
    if let Ok(pin) = context.get_pin_by_name(name).await {
        context.set_pin_ref_value(&pin, value).await?;
    }
    Ok(())
}

/// XPath literals have no backslash escaping. Use concat when both quotes occur.
#[cfg(any(feature = "execute", test))]
pub(crate) fn xpath_literal(value: &str) -> String {
    if !value.contains('\'') {
        return format!("'{value}'");
    }
    if !value.contains('"') {
        return format!("\"{value}\"");
    }
    format!(
        "concat({})",
        value
            .split('\'')
            .map(|part| format!("'{part}'"))
            .collect::<Vec<_>>()
            .join(",\"'\",")
    )
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn selector_xpath(selector: &Selector) -> flow_like_types::Result<String> {
    let value = xpath_literal(&selector.value);
    let condition = match selector.kind {
        SelectorKind::Xpath => return Ok(selector.value.clone()),
        SelectorKind::Text => format!(
            "contains(normalize-space(.), {value}) and not(.//*[contains(normalize-space(.), {value})])"
        ),
        SelectorKind::TextExact => {
            format!("normalize-space(.) = {value} and not(.//*[normalize-space(.) = {value}])")
        }
        SelectorKind::TestId => format!("@data-testid = {value}"),
        SelectorKind::AriaLabel => format!("@aria-label = {value}"),
        SelectorKind::Placeholder => format!("@placeholder = {value}"),
        SelectorKind::AltText => format!("@alt = {value}"),
        SelectorKind::Title => format!("@title = {value}"),
        SelectorKind::Role => {
            let implicit = match selector.value.as_str() {
                "button" => {
                    "self::button or (self::input and (@type='button' or @type='submit' or @type='reset'))"
                }
                "link" => "self::a and @href",
                "textbox" => {
                    "self::textarea or (self::input and (not(@type) or @type='text' or @type='email' or @type='search' or @type='tel' or @type='url'))"
                }
                "checkbox" => "self::input and @type='checkbox'",
                "radio" => "self::input and @type='radio'",
                "combobox" => "self::select",
                "heading" => "self::h1 or self::h2 or self::h3 or self::h4 or self::h5 or self::h6",
                "img" => "self::img",
                _ => "false()",
            };
            format!("@role = {value} or (not(@role) and ({implicit}))")
        }
        _ => {
            return Err(flow_like_types::anyhow!(
                "This selector kind cannot be resolved as XPath"
            ));
        }
    };
    Ok(format!(".//*[{condition}]"))
}

#[cfg(feature = "execute")]
pub(crate) async fn find(
    driver: &thirtyfour::WebDriver,
    selector: &Selector,
) -> flow_like_types::Result<thirtyfour::WebElement> {
    use thirtyfour::By;
    if selector.value.is_empty() {
        return Err(flow_like_types::anyhow!("Selector is empty"));
    }
    let by = if selector.kind == SelectorKind::Css {
        By::Css(&selector.value)
    } else {
        By::XPath(selector_xpath(selector)?)
    };
    if let Some(scope) = selector.scope.as_deref().filter(|s| !s.is_empty()) {
        let root = driver.find(By::Css(scope)).await?;
        let by = if selector.kind == SelectorKind::Xpath && selector.value.starts_with('/') {
            By::XPath(format!(".{}", selector.value))
        } else {
            by
        };
        Ok(root.find(by).await?)
    } else {
        Ok(driver.find(by).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_names_and_implicit_roles() {
        assert_eq!(xpath_literal("a'b\"c"), "concat('a',\"'\",'b\"c')");
        let selector = Selector::role("button");
        let query = selector_xpath(&selector).unwrap();
        assert!(query.contains("self::button"));
        assert!(query.contains("@role = 'button'"));
        let selector = Selector {
            kind: SelectorKind::AriaLabel,
            value: "Bob's \"save\"".into(),
            confidence: None,
            scope: None,
        };
        assert!(selector_xpath(&selector).unwrap().contains("concat("));
    }
}

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

/// Uses the typed locator when connected; otherwise the legacy CSS string, where a bare
/// snapshot ref such as `e12` resolves as a ref.
#[cfg(feature = "execute")]
pub(crate) async fn evaluate_locator(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    css: &str,
) -> flow_like_types::Result<Selector> {
    let value = optional_input(context, "locator", flow_like_types::Value::Null).await?;
    if value.is_null() || value.as_object().is_some_and(|object| object.is_empty()) {
        Ok(Selector::from_css_or_ref(css))
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
const NON_RENDERED: &str = "self::script or self::style or self::noscript or self::template or self::title or self::head or ancestor::head or ancestor::script or ancestor::style or ancestor::noscript or ancestor::template";

#[cfg(any(feature = "execute", test))]
const NON_TEXT_CHILD: &str = "self::script or self::style or self::noscript or self::template";

#[cfg(any(feature = "execute", test))]
pub(crate) fn selector_xpath(selector: &Selector) -> flow_like_types::Result<String> {
    let value = xpath_literal(&selector.value);
    let condition = match selector.kind {
        SelectorKind::Xpath => return Ok(selector.value.clone()),
        SelectorKind::Text => format!(
            "not({NON_RENDERED}) and contains(normalize-space(.), {value}) and not(.//*[not({NON_TEXT_CHILD})][contains(normalize-space(.), {value})])"
        ),
        SelectorKind::TextExact => format!(
            "not({NON_RENDERED}) and normalize-space(.) = {value} and not(.//*[not({NON_TEXT_CHILD})][normalize-space(.) = {value}])"
        ),
        SelectorKind::TestId => format!("@data-testid = {value}"),
        SelectorKind::AriaLabel => format!("@aria-label = {value}"),
        SelectorKind::Placeholder => format!("@placeholder = {value}"),
        SelectorKind::AltText => format!("@alt = {value}"),
        SelectorKind::Title => format!("@title = {value}"),
        _ => {
            return Err(flow_like_types::anyhow!(
                "{:?} selectors cannot be resolved as XPath",
                selector.kind
            ));
        }
    };
    Ok(format!(".//*[{condition}]"))
}

#[cfg(any(feature = "execute", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NameMatch {
    Contains(String),
    Exact(String),
    Regex { pattern: String, flags: String },
}

/// Parses `role`, `role|name` (substring), `role|=name` (exact) and `role|/pattern/flags`.
#[cfg(any(feature = "execute", test))]
pub(crate) fn parse_role_value(
    value: &str,
) -> flow_like_types::Result<(String, Option<NameMatch>)> {
    let (role, name) = match value.split_once('|') {
        Some((role, name)) => (role, Some(name)),
        None => (value, None),
    };
    let role = role.trim().to_ascii_lowercase();
    if role.is_empty() || !role.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(flow_like_types::anyhow!(
            "Role selector '{value}' must start with an ARIA role such as 'button' or 'button|Sign in'"
        ));
    }
    let name = match name.map(str::trim) {
        None | Some("") => None,
        Some(name) => Some(if let Some(exact) = name.strip_prefix('=') {
            NameMatch::Exact(normalize_whitespace(exact))
        } else if let Some((pattern, flags)) = name
            .strip_prefix('/')
            .and_then(|rest| rest.rsplit_once('/'))
        {
            if !flags.chars().all(|flag| "imsu".contains(flag)) {
                return Err(flow_like_types::anyhow!(
                    "Role selector '{value}' has unsupported regex flags '{flags}' (use i, m, s or u)"
                ));
            }
            NameMatch::Regex {
                pattern: pattern.to_string(),
                flags: flags.to_string(),
            }
        } else {
            NameMatch::Contains(normalize_whitespace(name))
        }),
    };
    Ok((role, name))
}

#[cfg(any(feature = "execute", test))]
fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(feature = "execute")]
const RESOLVE_SCRIPT: &str = r#"
const [kind, value, xpath, scope, role, nameMode, nameText, nameFlags, all] = arguments;
const root = scope ? document.querySelector(scope) : document;
if (!root) throw new Error(`Selector scope '${scope}' matched no element`);
const norm = (text) => (text || '').replace(/\s+/g, ' ').trim();
const textOf = (el) => {
  let out = '';
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const parent = walker.currentNode.parentElement;
    if (parent && /^(script|style|noscript|template)$/.test(parent.localName)) continue;
    out += walker.currentNode.nodeValue;
  }
  return norm(out);
};
const visible = (el) => {
  if (!el.isConnected) return false;
  if (typeof el.checkVisibility === 'function' && !el.checkVisibility({ checkVisibilityCSS: true })) return false;
  const rect = el.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
};
const INTERACTIVE = 'a[href],area[href],button,input:not([type=hidden]),select,textarea,summary,label,[contenteditable=""],[contenteditable="true"],[onclick],[tabindex]:not([tabindex="-1"]),[role=button],[role=link],[role=checkbox],[role=radio],[role=switch],[role=tab],[role=menuitem],[role=menuitemcheckbox],[role=menuitemradio],[role=option],[role=combobox],[role=textbox],[role=searchbox],[role=slider],[role=spinbutton],[role=treeitem]';
const implicitRole = (el) => {
  const tag = el.localName;
  const type = (el.getAttribute('type') || 'text').toLowerCase();
  switch (tag) {
    case 'button': case 'summary': return 'button';
    case 'a': case 'area': return el.hasAttribute('href') ? 'link' : null;
    case 'input':
      if (['button', 'submit', 'reset', 'image'].includes(type)) return 'button';
      if (type === 'checkbox') return 'checkbox';
      if (type === 'radio') return 'radio';
      if (type === 'range') return 'slider';
      if (type === 'number') return 'spinbutton';
      if (type === 'search') return el.hasAttribute('list') ? 'combobox' : 'searchbox';
      if (['text', 'email', 'tel', 'url', 'password'].includes(type)) return el.hasAttribute('list') ? 'combobox' : 'textbox';
      return null;
    case 'textarea': return 'textbox';
    case 'select': return el.multiple || el.size > 1 ? 'listbox' : 'combobox';
    case 'option': return 'option';
    case 'h1': case 'h2': case 'h3': case 'h4': case 'h5': case 'h6': return 'heading';
    case 'img': return el.getAttribute('alt') === '' ? 'presentation' : 'img';
    case 'nav': return 'navigation';
    case 'main': return 'main';
    case 'header': return 'banner';
    case 'footer': return 'contentinfo';
    case 'aside': return 'complementary';
    case 'form': return 'form';
    case 'dialog': return 'dialog';
    case 'ul': case 'ol': case 'menu': return 'list';
    case 'li': return 'listitem';
    case 'table': return 'table';
    case 'tr': return 'row';
    case 'td': return 'cell';
    case 'th': return 'columnheader';
    case 'progress': return 'progressbar';
    case 'hr': return 'separator';
    case 'fieldset': case 'details': return 'group';
    case 'article': return 'article';
    case 'section': return el.hasAttribute('aria-label') || el.hasAttribute('aria-labelledby') ? 'region' : null;
    default: return el.isContentEditable && el.parentElement && !el.parentElement.isContentEditable ? 'textbox' : null;
  }
};
const roleOf = (el) => (el.getAttribute('role') || '').trim().split(/\s+/)[0].toLowerCase() || implicitRole(el);
const NAME_FROM_CONTENT = new Set(['button', 'cell', 'checkbox', 'columnheader', 'gridcell', 'heading', 'link', 'menuitem', 'menuitemcheckbox', 'menuitemradio', 'option', 'radio', 'row', 'rowheader', 'switch', 'tab', 'tooltip', 'treeitem']);
const nameOf = (el, elRole) => {
  const labelledby = el.getAttribute('aria-labelledby');
  if (labelledby) {
    const text = norm(labelledby.split(/\s+/).map((id) => {
      const node = el.ownerDocument.getElementById(id);
      return node ? textOf(node) : '';
    }).join(' '));
    if (text) return text;
  }
  const aria = norm(el.getAttribute('aria-label'));
  if (aria) return aria;
  if (el.labels && el.labels.length) {
    const text = norm(Array.from(el.labels).map(textOf).join(' '));
    if (text) return text;
  }
  const tag = el.localName;
  const type = (el.getAttribute('type') || '').toLowerCase();
  if (tag === 'input' && ['button', 'submit', 'reset'].includes(type)) {
    return norm(el.value) || (type === 'submit' ? 'Submit' : type === 'reset' ? 'Reset' : '');
  }
  if (tag === 'img' || tag === 'area' || (tag === 'input' && type === 'image')) {
    const alt = norm(el.getAttribute('alt'));
    if (alt) return alt;
  }
  if (NAME_FROM_CONTENT.has(elRole)) {
    const text = textOf(el);
    if (text) return text;
  }
  return norm(el.getAttribute('title')) || norm(el.getAttribute('placeholder'));
};
let candidates = [];
if (kind === 'Role') {
  let matches = null;
  if (nameMode === 'exact') matches = (name) => name === nameText;
  else if (nameMode === 'contains') matches = (name) => name.toLowerCase().includes(nameText.toLowerCase());
  else if (nameMode === 'regex') { const pattern = new RegExp(nameText, nameFlags); matches = (name) => pattern.test(name); }
  candidates = Array.from(root.querySelectorAll('*')).filter((el) => roleOf(el) === role && (!matches || matches(nameOf(el, role))));
} else {
  const found = document.evaluate(xpath, root, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
  for (let i = 0; i < found.snapshotLength; i++) {
    const node = found.snapshotItem(i);
    if (node.nodeType === Node.ELEMENT_NODE) candidates.push(node);
  }
  if (kind === 'Text' || kind === 'TextExact') {
    const wanted = norm(value);
    candidates = candidates.filter((el) => kind === 'Text' ? textOf(el).includes(wanted) : textOf(el) === wanted);
  }
}
if (all) return candidates;
const ranked = candidates
  .map((el, index) => ({ el, index, score: (visible(el) ? 0 : 2) + (el.closest(INTERACTIVE) ? 0 : 1) }))
  .sort((a, b) => a.score - b.score || a.index - b.index);
return ranked.length ? [ranked[0].el] : [];
"#;

#[cfg(feature = "execute")]
async fn resolve_in_page(
    driver: &thirtyfour::WebDriver,
    selector: &Selector,
    all: bool,
) -> flow_like_types::Result<Vec<thirtyfour::WebElement>> {
    let (xpath, role, name) = if selector.kind == SelectorKind::Role {
        let (role, name) = parse_role_value(&selector.value)?;
        (String::new(), role, name)
    } else {
        (selector_xpath(selector)?, String::new(), None)
    };
    let (name_mode, name_text, name_flags) = match name {
        None => ("", String::new(), String::new()),
        Some(NameMatch::Contains(text)) => ("contains", text, String::new()),
        Some(NameMatch::Exact(text)) => ("exact", text, String::new()),
        Some(NameMatch::Regex { pattern, flags }) => ("regex", pattern, flags),
    };
    let scope = selector.scope.clone().unwrap_or_default();
    let args = vec![
        flow_like_types::json::json!(format!("{:?}", selector.kind)),
        flow_like_types::json::json!(selector.value),
        flow_like_types::json::json!(xpath),
        flow_like_types::json::json!(scope),
        flow_like_types::json::json!(role),
        flow_like_types::json::json!(name_mode),
        flow_like_types::json::json!(name_text),
        flow_like_types::json::json!(name_flags),
        flow_like_types::json::json!(all),
    ];
    let result = driver
        .execute(RESOLVE_SCRIPT, args)
        .await
        .map_err(|error| {
            flow_like_types::anyhow!(
                "Failed to resolve {:?} selector '{}': {error}",
                selector.kind,
                selector.value
            )
        })?;
    Ok(result.elements()?)
}

#[cfg(feature = "execute")]
fn no_match(selector: &Selector) -> flow_like_types::Error {
    thirtyfour::error::no_such_element(format!(
        "No element matches {:?} selector '{}'",
        selector.kind, selector.value
    ))
    .into()
}

#[cfg(feature = "execute")]
fn native_by(selector: &Selector) -> thirtyfour::By {
    use thirtyfour::By;
    match selector.kind {
        SelectorKind::Css => By::Css(selector.value.clone()),
        _ if selector.scope.as_deref().is_some_and(|s| !s.is_empty())
            && selector.value.starts_with('/') =>
        {
            By::XPath(format!(".{}", selector.value))
        }
        _ => By::XPath(selector.value.clone()),
    }
}

#[cfg(feature = "execute")]
async fn native_root(
    driver: &thirtyfour::WebDriver,
    selector: &Selector,
) -> flow_like_types::Result<Option<thirtyfour::WebElement>> {
    match selector.scope.as_deref().filter(|s| !s.is_empty()) {
        Some(scope) => Ok(Some(driver.find(thirtyfour::By::Css(scope)).await?)),
        None => Ok(None),
    }
}

#[cfg(feature = "execute")]
fn check_resolvable(selector: &Selector) -> flow_like_types::Result<()> {
    if selector.value.is_empty() {
        return Err(flow_like_types::anyhow!("Selector is empty"));
    }
    if selector.kind == SelectorKind::Image {
        return Err(flow_like_types::anyhow!(
            "Image selectors cannot be resolved in the page; use a vision node"
        ));
    }
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) async fn find(
    driver: &thirtyfour::WebDriver,
    selector: &Selector,
) -> flow_like_types::Result<thirtyfour::WebElement> {
    check_resolvable(selector)?;
    match selector.kind {
        SelectorKind::Ref => super::refs::resolve_ref(driver, &selector.value).await,
        SelectorKind::Css | SelectorKind::Xpath => {
            let by = native_by(selector);
            match native_root(driver, selector).await? {
                Some(root) => Ok(root.find(by).await?),
                None => Ok(driver.find(by).await?),
            }
        }
        _ => resolve_in_page(driver, selector, false)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| no_match(selector)),
    }
}

/// All matches in document order; an empty list when nothing matches.
#[cfg(feature = "execute")]
pub(crate) async fn find_all(
    driver: &thirtyfour::WebDriver,
    selector: &Selector,
) -> flow_like_types::Result<Vec<thirtyfour::WebElement>> {
    check_resolvable(selector)?;
    match selector.kind {
        SelectorKind::Ref => Ok(vec![
            super::refs::resolve_ref(driver, &selector.value).await?,
        ]),
        SelectorKind::Css | SelectorKind::Xpath => {
            let by = native_by(selector);
            match native_root(driver, selector).await? {
                Some(root) => Ok(root.find_all(by).await?),
                None => Ok(driver.find_all(by).await?),
            }
        }
        _ => resolve_in_page(driver, selector, true).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector(kind: SelectorKind, value: &str) -> Selector {
        Selector {
            kind,
            value: value.into(),
            confidence: None,
            scope: None,
        }
    }

    #[test]
    fn quoted_attribute_values() {
        assert_eq!(xpath_literal("a'b\"c"), "concat('a',\"'\",'b\"c')");
        let query = selector_xpath(&selector(SelectorKind::AriaLabel, "Bob's \"save\"")).unwrap();
        assert!(query.contains("concat("));
        assert!(selector_xpath(&selector(SelectorKind::Role, "button")).is_err());
        assert!(selector_xpath(&selector(SelectorKind::Ref, "e1")).is_err());
    }

    #[test]
    fn text_queries_skip_non_rendered_content() {
        let query = selector_xpath(&selector(SelectorKind::Text, "Sign in")).unwrap();
        assert!(query.starts_with(".//*[not(self::script"));
        assert!(query.contains("ancestor::head"));
        assert!(query.contains("contains(normalize-space(.), 'Sign in')"));
        assert!(query.contains("not(.//*[not(self::script or self::style"));
        let exact = selector_xpath(&selector(SelectorKind::TextExact, "OK")).unwrap();
        assert!(exact.contains("normalize-space(.) = 'OK'"));
        assert!(exact.contains("self::title"));
    }

    #[test]
    fn role_values_carry_name_filters() {
        assert_eq!(parse_role_value("Button").unwrap(), ("button".into(), None));
        assert_eq!(
            parse_role_value("button|  Sign   in ").unwrap(),
            ("button".into(), Some(NameMatch::Contains("Sign in".into())))
        );
        assert_eq!(
            parse_role_value("link|=Home").unwrap().1,
            Some(NameMatch::Exact("Home".into()))
        );
        assert_eq!(
            parse_role_value("textbox|/^e-?mail$/i").unwrap().1,
            Some(NameMatch::Regex {
                pattern: "^e-?mail$".into(),
                flags: "i".into()
            })
        );
        assert_eq!(parse_role_value("button|").unwrap().1, None);
        assert!(parse_role_value("|Sign in").is_err());
        assert!(parse_role_value("button|/x/g").is_err());
        assert!(parse_role_value("div.button").is_err());
    }
}

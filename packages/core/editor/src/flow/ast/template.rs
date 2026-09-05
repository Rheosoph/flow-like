//! Template literals ⇄ `string_format` nodes.
//!
//! `` `Topic ${label}\nGoal: ${source.goal}` `` is the FlowScript surface of one `string_format`
//! node: the static text becomes its `format_string` and every `${expr}` a `{placeholder}` whose
//! pin the node mints in `on_update`. Both directions live here so reconcile (text → call) and
//! lowering (node → text) can never disagree on placeholder names.

use flow_like_ast::model::*;
use flow_like_ast::to_camel_case;

use super::reconcile::format_string_placeholders;

pub(crate) const STRING_FORMAT_NODE_TYPE: &str = "string_format";
pub(crate) const FORMAT_STRING_PIN: &str = "format_string";

/// Build the `string_format` call a template literal lowers to.
///
/// Placeholder names: a bare reference keeps its name, a `.field`/`.pin` access uses its last
/// segment, anything else becomes `arg<N>` (N = 1-based position of the interpolation). A name
/// that repeats for a different expression is suffixed (`label`, `label2`); the same bare
/// reference interpolated twice shares one placeholder. Static text that `string_format` itself
/// would read as a placeholder is rejected, because the node has no escape syntax.
pub(crate) fn template_format_call(
    parts: &[TemplatePart],
    anchor: Option<String>,
) -> Result<Call, String> {
    let mut format_string = String::new();
    let mut args: Vec<Arg> = Vec::new();
    let mut position = 0usize;
    for part in parts {
        match part {
            TemplatePart::Text(text) => {
                if let Some(placeholder) = format_string_placeholders(text).into_iter().next() {
                    return Err(format!(
                        "literal `{{{placeholder}}}` inside a template literal would be read as a placeholder by string::format; write string::format({{ formatString: … }}) explicitly or drop the braces"
                    ));
                }
                format_string.push_str(text);
            }
            TemplatePart::Expr(expr) => {
                position += 1;
                let name = placeholder_name(expr, position, &args);
                if !args.iter().any(|arg| arg.name == name) {
                    args.push(Arg {
                        name: name.clone(),
                        value: expr.clone(),
                    });
                }
                format_string.push('{');
                format_string.push_str(&name);
                format_string.push('}');
            }
        }
    }
    let mut call_args = Vec::with_capacity(args.len() + 1);
    call_args.push(Arg {
        name: FORMAT_STRING_PIN.to_string(),
        value: Expr::Literal(Literal::String(format_string)),
    });
    call_args.extend(args);
    Ok(Call {
        node_type: STRING_FORMAT_NODE_TYPE.to_string(),
        display: to_camel_case(STRING_FORMAT_NODE_TYPE),
        path: Vec::new(),
        receiver: None,
        positional: Vec::new(),
        args: call_args,
        anchor,
    })
}

fn placeholder_name(expr: &Expr, position: usize, taken: &[Arg]) -> String {
    let taken_name = |name: &str| taken.iter().any(|arg| arg.name == name);
    let base = match expr {
        Expr::Ref(name) if is_placeholder_ident(name) => {
            if taken.iter().any(|arg| {
                arg.name == *name && matches!(&arg.value, Expr::Ref(seen) if seen == name)
            }) {
                return name.clone();
            }
            name.clone()
        }
        Expr::Field { pin, .. } if is_placeholder_ident(pin) => pin.clone(),
        Expr::Member { field, .. } if is_placeholder_ident(field) => field.clone(),
        _ => format!("arg{position}"),
    };
    if !taken_name(&base) {
        return base;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}{suffix}");
        if !taken_name(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// A placeholder must match `string_format`'s `[a-zA-Z0-9_]+` and must not name the template
/// pin itself (the node refuses to mint a second `format_string`).
fn is_placeholder_ident(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name != FORMAT_STRING_PIN
        && name != to_camel_case(FORMAT_STRING_PIN)
}

/// The template parts a `string_format` node renders as, given its literal format string and
/// one expression per placeholder pin (`None` when a placeholder has neither wire nor value).
/// Returns `None` unless re-synthesizing the call from the parts reproduces exactly this format
/// string and placeholder set, so the text form is only used where the round-trip is lossless.
pub(crate) fn format_template_parts(
    format_string: &str,
    mut placeholder_expr: impl FnMut(&str) -> Option<Expr>,
) -> Option<Vec<TemplatePart>> {
    let placeholders = format_string_placeholders(format_string);
    let mut values: Vec<(String, Expr)> = Vec::with_capacity(placeholders.len());
    for placeholder in &placeholders {
        values.push((placeholder.clone(), placeholder_expr(placeholder)?));
    }

    let mut parts = Vec::new();
    let mut rest = format_string;
    loop {
        let Some((offset, name)) = next_placeholder(rest, &placeholders) else {
            if !rest.is_empty() {
                parts.push(TemplatePart::Text(rest.to_string()));
            }
            break;
        };
        if offset > 0 {
            parts.push(TemplatePart::Text(rest[..offset].to_string()));
        }
        let expr = values
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, expr)| expr.clone())?;
        parts.push(TemplatePart::Expr(expr));
        rest = &rest[offset + name.len() + 2..];
    }

    let roundtrip = template_format_call(&parts, None).ok()?;
    let same_format = matches!(
        roundtrip.args.first(),
        Some(Arg { name, value: Expr::Literal(Literal::String(text)) })
            if name == FORMAT_STRING_PIN && text == format_string
    );
    let mut expected: Vec<&str> = placeholders.iter().map(String::as_str).collect();
    let mut produced: Vec<&str> = roundtrip.args[1..]
        .iter()
        .map(|arg| arg.name.as_str())
        .collect();
    expected.sort_unstable();
    produced.sort_unstable();
    (same_format && expected == produced).then_some(parts)
}

/// Byte offset and name of the first `{placeholder}` in `text` that names one of `placeholders`.
fn next_placeholder<'a>(text: &str, placeholders: &'a [String]) -> Option<(usize, &'a str)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && bytes.get(end) == Some(&b'}') {
                let name = &text[start..end];
                if let Some(known) = placeholders.iter().find(|candidate| *candidate == name) {
                    return Some((i, known.as_str()));
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str) -> Expr {
        Expr::Ref(name.to_string())
    }

    fn text(value: &str) -> TemplatePart {
        TemplatePart::Text(value.to_string())
    }

    fn format_of(call: &Call) -> String {
        match &call.args[0].value {
            Expr::Literal(Literal::String(text)) => text.clone(),
            other => panic!("format string must be a literal, got {other:?}"),
        }
    }

    fn arg_names(call: &Call) -> Vec<String> {
        call.args[1..].iter().map(|arg| arg.name.clone()).collect()
    }

    #[test]
    fn placeholder_names_follow_ref_segment_and_position_rules() {
        let call = template_format_call(
            &[
                text("Topic "),
                TemplatePart::Expr(r("label")),
                text("\nGoal: "),
                TemplatePart::Expr(Expr::Member {
                    base: Box::new(r("source")),
                    field: "goal".to_string(),
                }),
                text(" "),
                TemplatePart::Expr(Expr::Index {
                    base: Box::new(r("items")),
                    index: Box::new(Expr::Literal(Literal::Int(0))),
                }),
                text(" "),
                TemplatePart::Expr(Expr::Field {
                    base: Box::new(r("other")),
                    pin: "goal".to_string(),
                }),
                text(" "),
                TemplatePart::Expr(r("label")),
            ],
            None,
        )
        .expect("valid template");
        assert_eq!(call.node_type, STRING_FORMAT_NODE_TYPE);
        assert_eq!(
            format_of(&call),
            "Topic {label}\nGoal: {goal} {arg3} {goal2} {label}"
        );
        assert_eq!(arg_names(&call), vec!["label", "goal", "arg3", "goal2"]);
    }

    #[test]
    fn reserved_and_invalid_names_fall_back_to_positional() {
        let call = template_format_call(
            &[
                TemplatePart::Expr(r("formatString")),
                TemplatePart::Expr(r("format_string")),
                TemplatePart::Expr(r("$weird")),
            ],
            None,
        )
        .expect("valid template");
        assert_eq!(format_of(&call), "{arg1}{arg2}{arg3}");
    }

    #[test]
    fn literal_placeholder_text_is_rejected() {
        let err = template_format_call(&[text("hello {name}"), TemplatePart::Expr(r("x"))], None)
            .expect_err("literal braces");
        assert!(err.contains("`{name}`"), "{err}");
        assert!(template_format_call(&[text("{ spaced }"), text("{}"), text("a{b")], None).is_ok());
    }

    #[test]
    fn zero_placeholder_template_is_a_bare_format_node() {
        let call = template_format_call(&[text("Successfully Added")], None).expect("valid");
        assert_eq!(format_of(&call), "Successfully Added");
        assert!(arg_names(&call).is_empty());
        let call = template_format_call(&[], None).expect("empty");
        assert_eq!(format_of(&call), "");
    }

    #[test]
    fn format_node_re_sugars_only_when_the_round_trip_is_exact() {
        let parts = format_template_parts("Hi {name}, #{name}!", |placeholder| {
            (placeholder == "name").then(|| r("name"))
        })
        .expect("exact");
        assert_eq!(parts.len(), 5);
        assert!(matches!(&parts[0], TemplatePart::Text(t) if t == "Hi "));
        assert!(matches!(&parts[4], TemplatePart::Text(t) if t == "!"));

        // The placeholder was wired from a member access whose last segment is not `id`, so the
        // template would re-synthesize a different pin name: keep the call form.
        assert!(
            format_template_parts("{id}", |_| Some(Expr::Member {
                base: Box::new(r("row")),
                field: "report_id".to_string(),
            }))
            .is_none()
        );
        // A literal-valued placeholder keeps the call form unless its pin is named `arg<N>`.
        assert!(
            format_template_parts("{name}", |_| Some(Expr::Literal(Literal::String(
                "Bob".to_string()
            ))))
            .is_none()
        );
        assert!(
            format_template_parts("{arg1}", |_| Some(Expr::Literal(Literal::String(
                "Bob".to_string()
            ))))
            .is_some()
        );
        // An unresolved placeholder pin (no wire, no value) keeps the call form.
        assert!(format_template_parts("{x}", |_| None).is_none());
        assert!(matches!(
            format_template_parts("plain", |_| None).as_deref(),
            Some([TemplatePart::Text(t)]) if t == "plain"
        ));
    }
}

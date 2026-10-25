use mail_parser::{Message, PartType};
use scraper::{ElementRef, Html, Node, node::Text};
use std::borrow::Cow;

/// Keeps the selected MIME text part, including link targets when it is HTML.
pub fn body_text<'a>(mail: &'a Message<'_>, position: usize) -> Option<Cow<'a, str>> {
    let part = mail.part(*mail.text_body.get(position)?)?;
    match &part.body {
        PartType::Text(text) => Some(Cow::Borrowed(text.as_ref())),
        PartType::Html(html) => Some(Cow::Owned(html_to_text(html))),
        _ => None,
    }
}

fn html_to_text(html: &str) -> String {
    let mut document = Html::parse_document(html);
    let links = document
        .tree
        .nodes()
        .filter_map(ElementRef::wrap)
        .filter(|element| element.value().name() == "a")
        .filter_map(|element| {
            let href = element.attr("href")?.trim();
            if href.is_empty() {
                return None;
            }
            let label = element.text().collect::<String>();
            if label.trim() == href {
                return None;
            }
            let target = if label.trim().is_empty() {
                href.to_owned()
            } else {
                format!(" ({href})")
            };
            Some((element.id(), target))
        })
        .collect::<Vec<_>>();

    for (id, target) in links {
        // A text node keeps decoded URL characters escaped during serialization.
        if let Some(mut element) = document.tree.get_mut(id) {
            element.append(Node::Text(Text {
                text: target.into(),
            }));
        }
    }

    // mail-parser's text renderer treats a '>' inside an attribute as a tag end.
    // Link targets are now text, so attributes are no longer needed for rendering.
    for node in document.tree.values_mut() {
        if let Node::Element(element) = node {
            element.attrs.clear();
        }
    }

    mail_parser::decoders::html::html_to_text(&document.html())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MessageParser;

    #[test]
    fn html_fallback_keeps_link_labels_and_decoded_targets() {
        let raw = "Content-Type: text/html; charset=utf-8\r\n\r\n\
                   <p>Ihre <a href=\"https://example.com/invoice?a=1&amp;b=2\">\
                   <strong>Rechnung</strong></a> ist da.</p>";
        let mail = MessageParser::default().parse(raw.as_bytes()).unwrap();

        assert_eq!(
            body_text(&mail, 0).unwrap().trim(),
            "Ihre Rechnung (https://example.com/invoice?a=1&b=2) ist da."
        );
        assert_eq!(
            mail.body_html(0).unwrap(),
            raw.split_once("\r\n\r\n").unwrap().1
        );
        assert!(body_text(&mail, 1).is_none());
    }

    #[test]
    fn html_links_accept_quoted_unquoted_and_uppercase_attributes() {
        for html in [
            r#"<a href="https://example.com">Visit</a>"#,
            "<a href='https://example.com'>Visit</a>",
            "<A HREF=https://example.com>Visit</A>",
            "<a title='a > b' href='https://example.com'>Visit</a>",
            "<a href='https://example.com'>Visit",
        ] {
            assert_eq!(html_to_text(html).trim(), "Visit (https://example.com)");
        }
    }

    #[test]
    fn html_links_keep_empty_and_image_only_targets_without_duplicating_urls() {
        assert_eq!(
            html_to_text("<a href='https://example.com'><span> https://example.com </span></a>")
                .trim(),
            "https://example.com"
        );
        for html in [
            "<a href='https://example.com'></a>",
            "<a href='https://example.com'><img src='cid:logo'></a>",
        ] {
            assert_eq!(html_to_text(html).trim(), "https://example.com");
        }
        assert_eq!(
            html_to_text("<a name='top'>Top</a> <a href=' '>Empty</a>").trim(),
            "Top Empty"
        );
    }

    #[test]
    fn html_targets_are_decoded_once_and_keep_unicode_and_mailto_links() {
        let html = "<a href='https://example.com/?q=&lt;Grüße&gt;&amp;literal=&amp;amp;'>Suche</a> \
                    <a href='mailto:help@example.com'>Support</a>";
        assert_eq!(
            html_to_text(html).trim(),
            "Suche (https://example.com/?q=<Grüße>&literal=&amp;) Support (mailto:help@example.com)"
        );
    }

    #[test]
    fn authored_plain_text_takes_precedence_over_html() {
        let raw = "MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=x\r\n\r\n\
                   --x\r\nContent-Type: text/plain\r\n\r\nPlain <text> & spacing\r\n\r\n\
                   --x\r\nContent-Type: text/html\r\n\r\n\
                   <a href='https://example.com'>HTML alternative</a>\r\n\
                   --x--\r\n";
        let mail = MessageParser::default().parse(raw.as_bytes()).unwrap();

        assert_eq!(body_text(&mail, 0), mail.body_text(0));
        assert_eq!(body_text(&mail, 0).unwrap(), "Plain <text> & spacing\r\n");
        assert!(matches!(body_text(&mail, 0), Some(Cow::Borrowed(_))));
    }
}

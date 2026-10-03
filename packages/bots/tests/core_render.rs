use flow_like_bots::render::{
    cut, escape_html, links_markdown, plan_discord, plan_telegram, shorten, split, telegram_html,
    utf16_len,
};
use flow_like_bots::reply::{Link, Plan};

const SAMPLES: [&str; 4] = [
    "Grüße aus Köln, schöne Äpfel und Übermut. ",
    "Hello 👋🏽 world 🎉🎉 family 👨‍👩‍👧 done. ",
    "東京の天気は晴れです。明日も晴れるでしょう。",
    "line one\nline two\n\nparagraph ✓\n",
];

#[test]
fn markdown_becomes_telegram_html() {
    assert_eq!(
        telegram_html("**bold** and *it* and `a<b>` and ~~gone~~"),
        "<b>bold</b> and <i>it</i> and <code>a&lt;b&gt;</code> and <s>gone</s>"
    );
    assert_eq!(
        telegram_html("# Title\n\n- one\n- two"),
        "<b>Title</b>\n• one\n• two"
    );
    assert_eq!(
        telegram_html("[site](https://example.com/?a=1&b=\"2\")"),
        "<a href=\"https://example.com/?a=1&amp;b=&quot;2&quot;\">site</a>"
    );
    assert_eq!(
        telegram_html("```rust\nlet x = 1 < 2;\n```"),
        "<pre>let x = 1 &lt; 2;\n</pre>"
    );
    assert_eq!(
        telegram_html("<script>x</script> & co"),
        "&lt;script&gt;x&lt;/script&gt; &amp; co"
    );
    assert_eq!(
        telegram_html("> quoted"),
        "<blockquote>quoted\n</blockquote>"
    );
    assert_eq!(telegram_html("Grüße 👋 東京"), "Grüße 👋 東京");
}

#[test]
fn escaping() {
    assert_eq!(
        escape_html("a & <b> \"c\""),
        "a &amp; &lt;b&gt; &quot;c&quot;"
    );
}

#[test]
fn cutting_never_breaks_a_character() {
    for sample in SAMPLES {
        for limit in 0..sample.chars().count() + 2 {
            let short = cut(sample, limit);
            assert!(short.chars().count() <= limit);
            assert!(sample.starts_with(short));
            let shortened = shorten(sample, limit.max(1));
            assert!(shortened.chars().count() <= limit.max(1), "{shortened}");
        }
    }
    assert_eq!(shorten("Grüße", 4), "Grü…");
    assert_eq!(shorten("Grüße", 5), "Grüße");
}

fn check_split(text: &str, limit: usize) {
    let parts = split(text, limit);
    assert_eq!(parts.concat(), text, "split at {limit} lost text");
    for part in &parts {
        assert!(
            utf16_len(part) <= limit,
            "part of {} units over {limit}",
            utf16_len(part)
        );
        assert!(!part.is_empty());
    }
}

#[test]
fn splitting_at_the_providers_limits() {
    for sample in SAMPLES {
        for limit in [4_096, 2_000] {
            let text = sample.repeat(limit / sample.chars().count() * 3 + 7);
            check_split(&text, limit);
        }
        for limit in 2..40 {
            check_split(&sample.repeat(3), limit);
        }
    }
    check_split(&"🎉".repeat(5_000), 4_096);
    check_split(&"ä".repeat(4_097), 4_096);
    assert_eq!(split("short", 2_000), ["short"]);
    assert_eq!(split("", 2_000), [""]);
}

#[test]
fn a_part_ends_at_a_line_break_or_a_space() {
    let text = format!("{}\n{}", "a".repeat(1_500), "b".repeat(1_000));
    let parts = split(&text, 2_000);
    assert_eq!(parts.len(), 2);
    assert!(parts[0].ends_with('\n'));
    let words = "word ".repeat(1_000);
    for part in split(&words, 2_000) {
        assert!(part.ends_with(' '));
    }
}

fn plan() -> Plan {
    Plan {
        steps: vec![
            (0, "Read the question".into()),
            (1, "Search <docs>".into()),
            (2, "Write the answer".into()),
            (3, "Check it".into()),
            (4, "Send it".into()),
            (5, "Rest".into()),
        ],
        current_step: 1,
        current_message: "Looking at 東京 ".repeat(30),
    }
}

#[test]
fn plan_as_a_telegram_box() {
    let text = plan_telegram(&plan());
    assert!(text.starts_with("┌─ 🧠 <b>Thinking</b>"));
    assert!(text.contains("│ ✅ <b>0</b>: Read the question"));
    assert!(text.contains("│ 🔄 <b>1</b>: Search &lt;docs&gt;"));
    assert!(text.contains("│ ⏳ <b>2</b>: Write the answer"));
    assert!(text.contains("…</i>"));
    assert_eq!(plan_telegram(&Plan::default()), "");
}

#[test]
fn plan_as_a_discord_embed() {
    let embed = plan_discord(&plan()).unwrap();
    let names: Vec<&str> = embed
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "✅ Step 0",
            "🔄 Step 1",
            "⏳ Step 2",
            "⏳ Step 3",
            "⏳ +2 more steps"
        ]
    );
    assert!(
        embed
            .fields
            .iter()
            .all(|field| field.value.chars().count() <= 1_024)
    );
    let mut late = plan();
    late.current_step = 4;
    let names: Vec<String> = plan_discord(&late)
        .unwrap()
        .fields
        .into_iter()
        .map(|field| field.name)
        .collect();
    assert_eq!(names, ["✅ 4 steps completed", "🔄 Step 4", "⏳ Step 5"]);
    assert!(plan_discord(&Plan::default()).is_none());
}

#[test]
fn links() {
    assert_eq!(links_markdown(&[]), "");
    let text = links_markdown(&[
        Link {
            url: "https://example.com/a b.pdf".into(),
            name: Some("Report [final]".into()),
        },
        Link {
            url: "https://example.com/x".into(),
            name: None,
        },
    ]);
    assert_eq!(
        text,
        "📎 Attachments:\n- [Report final](https://example.com/a%20b.pdf)\n- https://example.com/x"
    );
}

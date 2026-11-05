#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InvitationKind {
    Project,
    Room,
}

impl InvitationKind {
    fn caption(self) -> &'static str {
        match self {
            Self::Project => "Project invitation",
            Self::Room => "Group chat invitation",
        }
    }
}

/// `image` is a large cover; without one, `icon` is the preview image unfurls show.
pub(super) struct InvitationPage {
    pub kind: InvitationKind,
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub desktop_url: String,
    pub web_url: String,
    pub share_url: String,
}

// Keep the script static so the response can authorize its exact CSP hash.
pub(super) const SCRIPT: &str = r#"(() => {
  document.querySelectorAll('img').forEach((image) => {
    const hideBrokenImage = () => { image.hidden = true; };
    image.addEventListener('error', hideBrokenImage);
    if (image.complete && image.naturalWidth === 0) hideBrokenImage();
  });
  const desktop = document.getElementById('open-desktop');
  const status = document.getElementById('handoff-status');
  if (!desktop || !status) return;
  let leftPage = false;
  let interacted = false;
  document.addEventListener('pointerdown', () => { interacted = true; }, { once: true });
  document.addEventListener('keydown', () => { interacted = true; }, { once: true });
  window.addEventListener('pagehide', () => { leftPage = true; });
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') leftPage = true;
  });
  const launch = () => {
    status.textContent = 'Opening Flow-Like…';
    window.setTimeout(() => {
      if (!leftPage && document.visibilityState === 'visible') {
        status.textContent = 'App didn’t open? Try the button above, or continue in your browser.';
      }
    }, 1800);
  };
  desktop.addEventListener('click', () => { leftPage = false; launch(); });
  // Opening an external app may require a click. Both links remain usable.
  window.setTimeout(() => {
    if (interacted || leftPage || document.visibilityState !== 'visible') return;
    launch();
    try { window.location.href = desktop.href; } catch (_) {}
  }, 600);
})();"#;

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn document(
    caption: &str,
    title: &str,
    description: &str,
    head: &str,
    body: &str,
    script: bool,
) -> String {
    let caption = escape(caption);
    let title = escape(title);
    let description = escape(description);
    let style = include_str!("style.css");
    let script = if script {
        format!("<script>{SCRIPT}</script>")
    } else {
        String::new()
    };
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<meta name="theme-color" content="#17181d">
<meta name="robots" content="noindex, nofollow, noarchive">
<meta name="referrer" content="no-referrer">
<title>{title}</title>
<meta name="description" content="{description}">
<meta property="og:type" content="website">
<meta property="og:site_name" content="Flow-Like">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{description}">
<meta name="twitter:title" content="{title}">
<meta name="twitter:description" content="{description}">
{head}
<style>{style}</style>
</head>
<body>
<div class="shell">
<header class="brand">Flow-Like<span class="brand-caption">{caption}</span></header>
<main>{body}</main>
<footer>Shared with you on Flow-Like.</footer>
</div>
{script}
</body>
</html>"##
    )
}

pub(super) fn render_invitation(page: &InvitationPage) -> String {
    let name = escape(&page.name);
    let description = escape(&page.description);
    let desktop_url = escape(&page.desktop_url);
    let web_url = escape(&page.web_url);
    let share_url = escape(&page.share_url);
    let mut head = format!(
        r#"<link rel="canonical" href="{share_url}">
<meta property="og:url" content="{share_url}">"#
    );
    let (preview, card) = match (&page.image, &page.icon) {
        (Some(image), _) => (Some(escape(image)), "summary_large_image"),
        (None, Some(icon)) => (Some(escape(icon)), "summary"),
        (None, None) => (None, "summary"),
    };
    if let Some(preview) = &preview {
        head.push_str(&format!(
            r#"
<meta property="og:image" content="{preview}">
<meta property="og:image:alt" content="Picture for {name}">
<meta name="twitter:image" content="{preview}">
<meta name="twitter:image:alt" content="Picture for {name}">"#
        ));
    }
    head.push_str(&format!(
        "\n<meta name=\"twitter:card\" content=\"{card}\">"
    ));
    let cover = page.image.as_ref().map_or_else(String::new, |image| {
        format!(
            r#"<img class="cover-image" src="{}" alt="" referrerpolicy="no-referrer">"#,
            escape(image)
        )
    });
    let icon = page.icon.as_ref().map_or_else(String::new, |icon| {
        format!(
            r#"<img class="project-icon" src="{}" alt="" width="56" height="56" referrerpolicy="no-referrer">"#,
            escape(icon)
        )
    });
    let body = format!(
        r#"<article class="invitation" aria-labelledby="project-name">
<div class="visual" aria-hidden="true">
{cover}
<div class="visual-shade"></div>
<div class="visual-caption"><span class="eyebrow">Shared with you on Flow-Like</span></div>
</div>
<div class="details">
<div class="project-heading">{icon}<span class="eyebrow">You’re invited</span></div>
<h1 id="project-name">{name}</h1>
<p class="description">{description}</p>
<div class="actions">
<p class="action-intro">Choose where to open your invitation.</p>
<a class="button primary" id="open-desktop" href="{desktop_url}" rel="noreferrer">Open in Flow-Like</a>
<a class="button secondary" href="{web_url}" rel="noreferrer">Continue in browser</a>
<p class="hint" id="handoff-status" role="status" aria-live="polite">Use the desktop app, or join from your browser.</p>
</div>
</div>
</article>"#
    );
    document(
        page.kind.caption(),
        &format!("You’re invited to {}", page.name),
        &format!("Join {} on Flow-Like. {}", page.name, page.description),
        &head,
        &body,
        true,
    )
}

pub(super) fn render_unavailable(temporary: bool) -> String {
    let (title, description, hint) = if temporary {
        (
            "Invitation unavailable",
            "We couldn’t check this invitation right now.",
            "Please try opening the link again in a moment.",
        )
    } else {
        (
            "Invalid invitation",
            "This link is invalid or no longer available.",
            "Ask the person who sent it for a new invitation.",
        )
    };
    let body = format!(
        r#"<article class="unavailable" aria-labelledby="invitation-title">
<span class="eyebrow">Flow-Like invitation</span>
<h1 id="invitation-title">{title}</h1>
<p class="description">{description}</p>
<p class="hint">{hint}</p>
</article>"#
    );
    document(
        "Invitation",
        title,
        description,
        "<meta name=\"twitter:card\" content=\"summary\">",
        &body,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_metadata_is_text_in_both_head_and_body() {
        let page = InvitationPage {
            kind: InvitationKind::Project,
            name: "<script>alert('name')</script>".into(),
            description: "\" onload=\"alert('description')".into(),
            image: Some("https://images.example/cover?x=\"/><script>evil()</script>".into()),
            icon: None,
            desktop_url: "flow-like://join?appId=a&token=t".into(),
            web_url: "https://app.example/join?appId=a&token=t&web=1".into(),
            share_url: "https://api.example/join?appId=a&token=t".into(),
        };
        let html = render_invitation(&page);
        assert!(!html.contains("<script>alert"));
        assert!(!html.contains("<script>evil"));
        assert!(!html.contains("\" onload=\""));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("appId=a&amp;token=t&amp;web=1"));
        assert_eq!(html.matches("<script>").count(), 1);
        assert!(html.contains(&format!("<script>{SCRIPT}</script>")));
    }

    #[test]
    fn a_room_without_a_cover_previews_the_inviters_avatar() {
        let page = InvitationPage {
            kind: InvitationKind::Room,
            name: "Climbing crew".into(),
            description: "Ada invited you to this group chat. 4 members.".into(),
            image: None,
            icon: Some("https://cdn.example/avatar.png?sig=1".into()),
            desktop_url: "flow-like://join?room=r".into(),
            web_url: "https://app.example/join?room=r".into(),
            share_url: "https://api.example/join?room=r".into(),
        };
        let html = render_invitation(&page);
        assert!(html.contains("Group chat invitation"));
        assert!(html.contains(
            r#"<meta property="og:image" content="https://cdn.example/avatar.png?sig=1">"#
        ));
        assert!(html.contains(r#"<meta name="twitter:card" content="summary">"#));
        assert!(!html.contains("summary_large_image"));
        assert!(!html.contains(r#"<img class="cover-image""#));
        assert!(html.contains(r#"<img class="project-icon""#));
    }

    #[test]
    fn unavailable_pages_have_no_images_links_or_launch_script() {
        for temporary in [true, false] {
            let html = render_unavailable(temporary);
            assert!(!html.contains("og:image"));
            assert!(!html.contains("<img"));
            assert!(!html.contains("href="));
            assert!(!html.contains("<script>"));
            assert!(!html.contains("flow-like://"));
        }
    }
}

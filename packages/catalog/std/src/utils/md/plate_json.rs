//! Serializers for the `plate_json::` documents produced by the platform text editor.
//!
//! The editor emits `"plate_json::" + JSON.stringify(editor.children)` — a Slate/Plate node
//! array. Lists are *not* `ul`/`li` trees: this editor runs the indent-based list plugin, so a
//! list item is a normal block carrying `listStyleType` + `indent`.

use flow_like_types::Value;

pub const PLATE_JSON_PREFIX: &str = "plate_json::";

/// Strip the `plate_json::` envelope and parse the node array.
///
/// Accepts a bare JSON array as well, so callers can pass either the stored envelope or the
/// already-unwrapped document.
pub fn parse_plate_document(input: &str) -> flow_like_types::Result<Vec<Value>> {
    let trimmed = input.trim();
    let payload = trimmed
        .strip_prefix(PLATE_JSON_PREFIX)
        .unwrap_or(trimmed)
        .trim();

    if payload.is_empty() {
        return Ok(Vec::new());
    }

    let parsed: Value = flow_like_types::json::from_str(payload).map_err(|err| {
        flow_like_types::anyhow!(
            "Failed to parse plate_json document ({err}); expected a JSON array of Plate nodes, got {} leading chars: {:.64}",
            payload.len(),
            payload
        )
    })?;

    match parsed {
        Value::Array(nodes) => Ok(nodes),
        Value::Object(_) => Ok(vec![parsed]),
        other => Err(flow_like_types::anyhow!(
            "plate_json document must be an array of nodes, found {}",
            kind_of(&other)
        )),
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn node_type(node: &Value) -> &str {
    node.get("type").and_then(Value::as_str).unwrap_or_default()
}

fn children(node: &Value) -> &[Value] {
    node.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn str_prop<'a>(node: &'a Value, key: &str) -> Option<&'a str> {
    node.get(key).and_then(Value::as_str)
}

fn flag(node: &Value, key: &str) -> bool {
    node.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn indent_of(node: &Value) -> usize {
    node.get("indent")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .max(1) as usize
}

/// CSS `list-style-type` values that render as an ordered list.
fn is_ordered_list(style: &str) -> bool {
    matches!(
        style,
        "decimal"
            | "decimal-leading-zero"
            | "lower-alpha"
            | "upper-alpha"
            | "lower-latin"
            | "upper-latin"
            | "lower-roman"
            | "upper-roman"
            | "lower-greek"
            | "armenian"
            | "georgian"
    )
}

/// Plain text of a subtree, ignoring every mark and element boundary.
pub fn plain_text(nodes: &[Value]) -> String {
    let mut out = String::new();
    collect_plain_text(nodes, &mut out);
    out
}

fn collect_plain_text(nodes: &[Value], out: &mut String) {
    for node in nodes {
        if let Some(text) = node.get("text").and_then(Value::as_str) {
            out.push_str(text);
        }
        if let Some(value) = str_prop(node, "value")
            && node_type(node) == "mention"
        {
            out.push_str(value);
        }
        collect_plain_text(children(node), out);
    }
}

/// Caption is either a Plate node array (the `@platejs/caption` shape) or a bare string.
fn caption_text(node: &Value) -> String {
    match node.get("caption") {
        Some(Value::Array(nodes)) => plain_text(nodes),
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    }
}

fn media_url(node: &Value) -> &str {
    str_prop(node, "url").unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageHandling {
    /// `![alt](url)` — the storage path or URL exactly as stored.
    Keep,
    /// Drop images entirely.
    Strip,
    /// Replace with the alt text (or `[Image]` when there is none).
    AltText,
}

impl ImageHandling {
    pub fn from_str_or_keep(value: &str) -> Self {
        match value {
            "strip" => ImageHandling::Strip,
            "alt_text" => ImageHandling::AltText,
            _ => ImageHandling::Keep,
        }
    }
}

/// Serialize a parsed Plate document to GitHub-flavoured Markdown.
pub fn to_markdown(nodes: &[Value], images: ImageHandling) -> String {
    let mut writer = MarkdownWriter {
        out: String::new(),
        images,
        list_stack: Vec::new(),
    };
    writer.blocks(nodes);
    let trimmed = writer.out.trim_end();
    let mut result = trimmed.to_string();
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

struct ListLevel {
    indent: usize,
    ordered: bool,
    counter: usize,
}

struct MarkdownWriter {
    out: String,
    images: ImageHandling,
    list_stack: Vec<ListLevel>,
}

impl MarkdownWriter {
    fn blocks(&mut self, nodes: &[Value]) {
        for node in nodes {
            self.block(node);
        }
        self.list_stack.clear();
    }

    fn block(&mut self, node: &Value) {
        let ty = node_type(node);

        if let Some(style) = str_prop(node, "listStyleType") {
            let style = style.to_string();
            self.list_item(node, &style);
            return;
        }
        self.list_stack.clear();

        match ty {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = ty[1..].parse::<usize>().unwrap_or(1);
                self.push_line(&format!(
                    "{} {}",
                    "#".repeat(level),
                    self.inline(children(node))
                ));
            }
            "blockquote" => {
                let body = self.inline(children(node));
                self.push_block_quote(&body, None);
            }
            "callout" => {
                let icon = str_prop(node, "icon").unwrap_or_default();
                let body = self.inline(children(node));
                self.push_block_quote(&body, (!icon.is_empty()).then_some(icon));
            }
            "code_block" => self.code_block(node),
            "hr" => self.push_line("---"),
            "img" => self.image(node),
            "video" | "audio" | "file" | "media_embed" => self.media_link(node),
            "table" => self.table(node),
            "column_group" => {
                for column in children(node) {
                    self.blocks(children(column));
                }
            }
            "toggle" => {
                let summary = self.inline(children(node));
                if !summary.is_empty() {
                    self.push_line(&format!("**{summary}**"));
                }
            }
            "equation" => {
                let tex = str_prop(node, "texExpression").unwrap_or_default();
                if !tex.is_empty() {
                    self.push_line(&format!("$$\n{tex}\n$$"));
                }
            }
            "toc" => {}
            "" => {
                // A bare text node at block level — Plate allows this in malformed documents.
                let text = self.inline(std::slice::from_ref(node));
                if !text.trim().is_empty() {
                    self.push_line(&text);
                }
            }
            _ => {
                let body = self.inline(children(node));
                if !body.trim().is_empty() {
                    self.push_line(&body);
                }
            }
        }
    }

    fn list_item(&mut self, node: &Value, style: &str) {
        let indent = indent_of(node);
        let ordered = is_ordered_list(style);

        while self
            .list_stack
            .last()
            .is_some_and(|level| level.indent > indent)
        {
            self.list_stack.pop();
        }

        match self.list_stack.last_mut() {
            Some(level) if level.indent == indent => {
                if level.ordered == ordered {
                    level.counter += 1;
                } else {
                    level.ordered = ordered;
                    level.counter = start_of(node);
                }
            }
            _ => self.list_stack.push(ListLevel {
                indent,
                ordered,
                counter: start_of(node),
            }),
        }

        let counter = self.list_stack.last().map(|l| l.counter).unwrap_or(1);
        let marker = if ordered {
            format!("{counter}. ")
        } else if style == "todo" {
            if flag(node, "checked") {
                "- [x] ".to_string()
            } else {
                "- [ ] ".to_string()
            }
        } else {
            "- ".to_string()
        };

        let pad = "  ".repeat(indent.saturating_sub(1));
        let body = self.inline(children(node));
        let heading = match node_type(node) {
            ty @ ("h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                format!("{} ", "#".repeat(ty[1..].parse::<usize>().unwrap_or(1)))
            }
            _ => String::new(),
        };

        self.ensure_block_gap();
        self.out.push_str(&pad);
        self.out.push_str(&marker);
        self.out.push_str(&heading);
        self.out.push_str(&body);
        self.out.push('\n');
    }

    fn code_block(&mut self, node: &Value) {
        let lang = str_prop(node, "lang").unwrap_or_default();
        let lines: Vec<String> = children(node)
            .iter()
            .map(|line| plain_text(children(line)))
            .collect();
        let body = lines.join("\n");
        // A fence must be longer than the longest run of backticks it contains.
        let fence = "`".repeat(longest_backtick_run(&body).max(2) + 1);
        self.push_line(&format!("{fence}{lang}\n{body}\n{fence}"));
    }

    fn image(&mut self, node: &Value) {
        let url = media_url(node);
        let alt = caption_text(node);
        match self.images {
            ImageHandling::Strip => {}
            ImageHandling::AltText => {
                let label = if alt.is_empty() { "Image" } else { &alt };
                self.push_line(&format!("[{label}]"));
            }
            ImageHandling::Keep => {
                if !url.is_empty() {
                    self.push_line(&format!("![{}]({})", escape_link_text(&alt), url));
                }
            }
        }
    }

    fn media_link(&mut self, node: &Value) {
        let url = media_url(node);
        if url.is_empty() {
            return;
        }
        let mut label = caption_text(node);
        if label.is_empty() {
            label = str_prop(node, "name").unwrap_or(node_type(node)).to_string();
        }
        self.push_line(&format!("[{}]({})", escape_link_text(&label), url));
    }

    fn table(&mut self, node: &Value) {
        let rows: Vec<&Value> = children(node)
            .iter()
            .filter(|row| node_type(row) == "tr")
            .collect();
        if rows.is_empty() {
            return;
        }

        let mut rendered: Vec<Vec<String>> = Vec::with_capacity(rows.len());
        let mut header_rows = 0usize;
        for (index, row) in rows.iter().enumerate() {
            let cells: Vec<String> = children(row)
                .iter()
                .map(|cell| {
                    let text = self.inline_blocks(children(cell));
                    text.replace('|', "\\|").replace('\n', " ").trim().to_string()
                })
                .collect();
            let all_header = children(row)
                .iter()
                .all(|cell| node_type(cell) == "th" || flag(cell, "header"));
            if all_header && index == header_rows {
                header_rows += 1;
            }
            rendered.push(cells);
        }

        let columns = rendered.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }

        self.ensure_block_gap();
        let header = if header_rows > 0 {
            rendered.remove(0)
        } else {
            vec![String::new(); columns]
        };
        self.out.push_str(&render_row(&header, columns));
        self.out
            .push_str(&format!("|{}\n", " --- |".repeat(columns)));
        for row in rendered {
            self.out.push_str(&render_row(&row, columns));
        }
        self.out.push('\n');
    }

    /// Blocks rendered as a single inline string — used for table cells, which cannot contain
    /// block structure in GFM.
    fn inline_blocks(&self, nodes: &[Value]) -> String {
        nodes
            .iter()
            .map(|node| self.inline(children(node)))
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn inline(&self, nodes: &[Value]) -> String {
        let mut out = String::new();
        for node in nodes {
            match node_type(node) {
                "a" => {
                    let url = str_prop(node, "url").unwrap_or_default();
                    let label = self.inline(children(node));
                    if url.is_empty() {
                        out.push_str(&label);
                    } else {
                        out.push_str(&format!("[{}]({})", escape_link_text(&label), url));
                    }
                }
                "mention" => {
                    let value = str_prop(node, "value")
                        .map(str::to_string)
                        .unwrap_or_else(|| plain_text(children(node)));
                    out.push('@');
                    out.push_str(&value);
                }
                "inline_equation" => {
                    let tex = str_prop(node, "texExpression").unwrap_or_default();
                    if !tex.is_empty() {
                        out.push_str(&format!("${tex}$"));
                    }
                }
                "date" => out.push_str(str_prop(node, "date").unwrap_or_default()),
                "img" if self.images == ImageHandling::Keep => {
                    let url = media_url(node);
                    if !url.is_empty() {
                        out.push_str(&format!("![{}]({})", escape_link_text(&caption_text(node)), url));
                    }
                }
                _ => {
                    if let Some(text) = node.get("text").and_then(Value::as_str) {
                        out.push_str(&apply_marks(node, &escape_markdown(text)));
                    } else {
                        out.push_str(&self.inline(children(node)));
                    }
                }
            }
        }
        out
    }

    fn push_block_quote(&mut self, body: &str, icon: Option<&str>) {
        if body.trim().is_empty() && icon.is_none() {
            return;
        }
        self.ensure_block_gap();
        let mut lines = body.split('\n').peekable();
        let mut first = true;
        while let Some(line) = lines.next() {
            self.out.push_str("> ");
            if first && let Some(icon) = icon {
                self.out.push_str(icon);
                self.out.push(' ');
            }
            self.out.push_str(line);
            self.out.push('\n');
            first = false;
            let _ = lines.peek();
        }
        self.out.push('\n');
    }

    fn push_line(&mut self, text: &str) {
        self.ensure_block_gap();
        self.out.push_str(text);
        self.out.push_str("\n\n");
    }

    fn ensure_block_gap(&mut self) {
        if self.out.is_empty() {
            return;
        }
        if !self.out.ends_with("\n\n") && self.out.ends_with('\n') {
            // A preceding list item ends with a single newline; keep list items adjacent.
            return;
        }
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }
}

fn start_of(node: &Value) -> usize {
    node.get("listStart")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize
}

fn render_row(cells: &[String], columns: usize) -> String {
    let mut row = String::from("|");
    for index in 0..columns {
        row.push(' ');
        row.push_str(cells.get(index).map(String::as_str).unwrap_or(""));
        row.push_str(" |");
    }
    row.push('\n');
    row
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn apply_marks(node: &Value, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // Markdown code spans do not nest other emphasis, so `code` short-circuits.
    if flag(node, "code") {
        let ticks = "`".repeat(longest_backtick_run(text) + 1);
        let padding = if text.starts_with('`') || text.ends_with('`') {
            " "
        } else {
            ""
        };
        return format!("{ticks}{padding}{text}{padding}{ticks}");
    }

    // Emphasis markers cannot span the whitespace at the edges of a run.
    let trimmed_start = text.len() - text.trim_start().len();
    let trimmed_end = text.len() - text.trim_end().len();
    let core = &text[trimmed_start..text.len() - trimmed_end];
    if core.is_empty() {
        return text.to_string();
    }

    let mut wrapped = core.to_string();
    if flag(node, "strikethrough") {
        wrapped = format!("~~{wrapped}~~");
    }
    if flag(node, "bold") {
        wrapped = format!("**{wrapped}**");
    }
    if flag(node, "italic") {
        wrapped = format!("*{wrapped}*");
    }
    if flag(node, "underline") {
        wrapped = format!("<u>{wrapped}</u>");
    }
    if flag(node, "subscript") {
        wrapped = format!("<sub>{wrapped}</sub>");
    }
    if flag(node, "superscript") {
        wrapped = format!("<sup>{wrapped}</sup>");
    }
    if flag(node, "kbd") {
        wrapped = format!("<kbd>{wrapped}</kbd>");
    }
    if flag(node, "highlight") {
        wrapped = format!("<mark>{wrapped}</mark>");
    }

    format!("{}{}{}", &text[..trimmed_start], wrapped, &text[text.len() - trimmed_end..])
}

fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn escape_link_text(text: &str) -> String {
    text.replace('[', "\\[").replace(']', "\\]")
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

/// Serialize a parsed Plate document to semantic HTML.
///
/// Unlike routing through Markdown this keeps alignment, colours, column layout, callout
/// variants and table spans.
pub fn to_html(nodes: &[Value], images: ImageHandling) -> String {
    let mut writer = HtmlWriter {
        out: String::new(),
        images,
        open_lists: Vec::new(),
    };
    writer.blocks(nodes);
    writer.close_lists_to(0);
    writer.out
}

struct OpenList {
    indent: usize,
    ordered: bool,
}

struct HtmlWriter {
    out: String,
    images: ImageHandling,
    open_lists: Vec<OpenList>,
}

impl HtmlWriter {
    fn blocks(&mut self, nodes: &[Value]) {
        for node in nodes {
            self.block(node);
        }
    }

    fn block(&mut self, node: &Value) {
        if let Some(style) = str_prop(node, "listStyleType") {
            let style = style.to_string();
            self.list_item(node, &style);
            return;
        }
        self.close_lists_to(0);

        let ty = node_type(node);
        match ty {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let body = self.inline(children(node));
                self.out
                    .push_str(&format!("<{ty}{}>{body}</{ty}>\n", block_attrs(node)));
            }
            "blockquote" => {
                let body = self.inline(children(node));
                self.out
                    .push_str(&format!("<blockquote{}>{body}</blockquote>\n", block_attrs(node)));
            }
            "callout" => {
                let icon = str_prop(node, "icon").unwrap_or_default();
                let variant = str_prop(node, "variant").unwrap_or("info");
                let body = self.inline(children(node));
                self.out.push_str(&format!(
                    "<aside class=\"callout callout-{}\"{}>",
                    escape_attr(variant),
                    block_style_attr(node)
                ));
                if !icon.is_empty() {
                    self.out.push_str(&format!(
                        "<span class=\"callout-icon\">{}</span>",
                        escape_html(icon)
                    ));
                }
                self.out.push_str(&format!("<div>{body}</div></aside>\n"));
            }
            "code_block" => {
                let lang = str_prop(node, "lang").unwrap_or_default();
                let body = children(node)
                    .iter()
                    .map(|line| escape_html(&plain_text(children(line))))
                    .collect::<Vec<_>>()
                    .join("\n");
                let class = if lang.is_empty() {
                    String::new()
                } else {
                    format!(" class=\"language-{}\"", escape_attr(lang))
                };
                self.out
                    .push_str(&format!("<pre><code{class}>{body}</code></pre>\n"));
            }
            "hr" => self.out.push_str("<hr />\n"),
            "img" => self.image(node),
            "video" | "audio" => {
                let url = media_url(node);
                if !url.is_empty() {
                    self.out.push_str(&format!(
                        "<figure><{ty} src=\"{}\" controls></{ty}>{}</figure>\n",
                        escape_attr(url),
                        self.figcaption(node)
                    ));
                }
            }
            "file" | "media_embed" => {
                let url = media_url(node);
                if !url.is_empty() {
                    let mut label = caption_text(node);
                    if label.is_empty() {
                        label = str_prop(node, "name").unwrap_or(url).to_string();
                    }
                    self.out.push_str(&format!(
                        "<p><a href=\"{}\">{}</a></p>\n",
                        escape_attr(url),
                        escape_html(&label)
                    ));
                }
            }
            "table" => self.table(node),
            "column_group" => {
                self.out
                    .push_str("<div class=\"column-group\" style=\"display:flex;gap:1rem\">\n");
                for column in children(node) {
                    let width = str_prop(column, "width").unwrap_or("auto");
                    self.out.push_str(&format!(
                        "<div class=\"column\" style=\"flex:0 0 {}\">\n",
                        escape_attr(width)
                    ));
                    self.blocks(children(column));
                    self.close_lists_to(0);
                    self.out.push_str("</div>\n");
                }
                self.out.push_str("</div>\n");
            }
            "toggle" => {
                let summary = self.inline(children(node));
                self.out.push_str(&format!(
                    "<details><summary>{summary}</summary></details>\n"
                ));
            }
            "equation" => {
                let tex = str_prop(node, "texExpression").unwrap_or_default();
                if !tex.is_empty() {
                    self.out.push_str(&format!(
                        "<div class=\"equation\">\\[{}\\]</div>\n",
                        escape_html(tex)
                    ));
                }
            }
            "toc" => {}
            _ => {
                let body = self.inline(children(node));
                if !body.trim().is_empty() {
                    self.out
                        .push_str(&format!("<p{}>{body}</p>\n", block_attrs(node)));
                }
            }
        }
    }

    fn list_item(&mut self, node: &Value, style: &str) {
        let indent = indent_of(node);
        let ordered = is_ordered_list(style);
        self.close_lists_to(indent);

        let matches_current = self
            .open_lists
            .last()
            .is_some_and(|list| list.indent == indent && list.ordered == ordered);
        if !matches_current {
            if self.open_lists.last().is_some_and(|l| l.indent == indent) {
                self.close_lists_to(indent.saturating_sub(1));
            }
            let tag = if ordered { "ol" } else { "ul" };
            let start = start_of(node);
            let start_attr = if ordered && start > 1 {
                format!(" start=\"{start}\"")
            } else {
                String::new()
            };
            self.out.push_str(&format!(
                "<{tag} style=\"list-style-type:{}\"{start_attr}>\n",
                escape_attr(style)
            ));
            self.open_lists.push(OpenList { indent, ordered });
        }

        let body = self.inline(children(node));
        if style == "todo" {
            let checked = if flag(node, "checked") { " checked" } else { "" };
            self.out.push_str(&format!(
                "<li class=\"task-list-item\"><input type=\"checkbox\" disabled{checked} /> {body}</li>\n"
            ));
        } else {
            self.out.push_str(&format!("<li>{body}</li>\n"));
        }
    }

    fn close_lists_to(&mut self, indent: usize) {
        while self
            .open_lists
            .last()
            .is_some_and(|list| list.indent > indent)
        {
            let list = self.open_lists.pop().expect("checked by is_some_and");
            self.out
                .push_str(if list.ordered { "</ol>\n" } else { "</ul>\n" });
        }
    }

    fn image(&mut self, node: &Value) {
        if self.images == ImageHandling::Strip {
            return;
        }
        let url = media_url(node);
        let alt = caption_text(node);
        if self.images == ImageHandling::AltText || url.is_empty() {
            let label = if alt.is_empty() { "Image" } else { &alt };
            self.out
                .push_str(&format!("<p class=\"image-placeholder\">{}</p>\n", escape_html(label)));
            return;
        }
        let width = node
            .get("width")
            .and_then(Value::as_u64)
            .map(|w| format!(" width=\"{w}\""))
            .unwrap_or_default();
        self.out.push_str(&format!(
            "<figure><img src=\"{}\" alt=\"{}\"{width} />{}</figure>\n",
            escape_attr(url),
            escape_attr(&alt),
            self.figcaption(node)
        ));
    }

    fn figcaption(&self, node: &Value) -> String {
        let caption = caption_text(node);
        if caption.is_empty() {
            String::new()
        } else {
            format!("<figcaption>{}</figcaption>", escape_html(&caption))
        }
    }

    fn table(&mut self, node: &Value) {
        self.out.push_str("<table>\n");
        let mut header_done = false;
        let mut body_open = false;
        for row in children(node).iter().filter(|r| node_type(r) == "tr") {
            let cells = children(row);
            let is_header = !cells.is_empty()
                && cells
                    .iter()
                    .all(|cell| node_type(cell) == "th" || flag(cell, "header"));

            if is_header && !header_done && !body_open {
                self.out.push_str("<thead>\n");
            } else if !body_open {
                if header_done {
                    self.out.push_str("</thead>\n");
                }
                self.out.push_str("<tbody>\n");
                body_open = true;
            }

            self.out.push_str("<tr>\n");
            for cell in cells {
                let tag = if node_type(cell) == "th" || flag(cell, "header") {
                    "th"
                } else {
                    "td"
                };
                let mut attrs = String::new();
                if let Some(span) = cell.get("colSpan").and_then(Value::as_u64).filter(|s| *s > 1) {
                    attrs.push_str(&format!(" colspan=\"{span}\""));
                }
                if let Some(span) = cell.get("rowSpan").and_then(Value::as_u64).filter(|s| *s > 1) {
                    attrs.push_str(&format!(" rowspan=\"{span}\""));
                }
                let body = self.cell_body(children(cell));
                self.out.push_str(&format!("<{tag}{attrs}>{body}</{tag}>\n"));
            }
            self.out.push_str("</tr>\n");

            if is_header && !body_open {
                header_done = true;
            }
        }
        if body_open {
            self.out.push_str("</tbody>\n");
        } else if header_done {
            self.out.push_str("</thead>\n");
        }
        self.out.push_str("</table>\n");
    }

    fn cell_body(&self, nodes: &[Value]) -> String {
        nodes
            .iter()
            .map(|node| self.inline(children(node)))
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("<br />")
    }

    fn inline(&self, nodes: &[Value]) -> String {
        let mut out = String::new();
        for node in nodes {
            match node_type(node) {
                "a" => {
                    let url = str_prop(node, "url").unwrap_or_default();
                    let label = self.inline(children(node));
                    if url.is_empty() {
                        out.push_str(&label);
                    } else {
                        out.push_str(&format!(
                            "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{label}</a>",
                            escape_attr(url)
                        ));
                    }
                }
                "mention" => {
                    let value = str_prop(node, "value")
                        .map(str::to_string)
                        .unwrap_or_else(|| plain_text(children(node)));
                    out.push_str(&format!(
                        "<span class=\"mention\">@{}</span>",
                        escape_html(&value)
                    ));
                }
                "inline_equation" => {
                    let tex = str_prop(node, "texExpression").unwrap_or_default();
                    if !tex.is_empty() {
                        out.push_str(&format!(
                            "<span class=\"equation-inline\">\\({}\\)</span>",
                            escape_html(tex)
                        ));
                    }
                }
                "date" => out.push_str(&escape_html(str_prop(node, "date").unwrap_or_default())),
                "img" if self.images == ImageHandling::Keep => {
                    let url = media_url(node);
                    if !url.is_empty() {
                        out.push_str(&format!(
                            "<img src=\"{}\" alt=\"{}\" />",
                            escape_attr(url),
                            escape_attr(&caption_text(node))
                        ));
                    }
                }
                "br" => out.push_str("<br />"),
                _ => {
                    if let Some(text) = node.get("text").and_then(Value::as_str) {
                        out.push_str(&apply_html_marks(node, &escape_html(text)));
                    } else {
                        out.push_str(&self.inline(children(node)));
                    }
                }
            }
        }
        out
    }
}

fn apply_html_marks(node: &Value, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut wrapped = text.to_string();
    if flag(node, "code") {
        wrapped = format!("<code>{wrapped}</code>");
    }
    if flag(node, "strikethrough") {
        wrapped = format!("<s>{wrapped}</s>");
    }
    if flag(node, "bold") {
        wrapped = format!("<strong>{wrapped}</strong>");
    }
    if flag(node, "italic") {
        wrapped = format!("<em>{wrapped}</em>");
    }
    if flag(node, "underline") {
        wrapped = format!("<u>{wrapped}</u>");
    }
    if flag(node, "subscript") {
        wrapped = format!("<sub>{wrapped}</sub>");
    }
    if flag(node, "superscript") {
        wrapped = format!("<sup>{wrapped}</sup>");
    }
    if flag(node, "kbd") {
        wrapped = format!("<kbd>{wrapped}</kbd>");
    }
    if flag(node, "highlight") {
        wrapped = format!("<mark>{wrapped}</mark>");
    }

    let mut styles = Vec::new();
    if let Some(color) = str_prop(node, "color") {
        styles.push(format!("color:{}", escape_attr(color)));
    }
    if let Some(background) = str_prop(node, "backgroundColor") {
        styles.push(format!("background-color:{}", escape_attr(background)));
    }
    if let Some(size) = str_prop(node, "fontSize") {
        styles.push(format!("font-size:{}", escape_attr(size)));
    }
    if let Some(family) = str_prop(node, "fontFamily") {
        styles.push(format!("font-family:{}", escape_attr(family)));
    }
    if styles.is_empty() {
        wrapped
    } else {
        format!("<span style=\"{}\">{wrapped}</span>", styles.join(";"))
    }
}

fn block_attrs(node: &Value) -> String {
    block_style_attr(node)
}

fn block_style_attr(node: &Value) -> String {
    let mut styles = Vec::new();
    if let Some(align) = str_prop(node, "align") {
        styles.push(format!("text-align:{}", escape_attr(align)));
    }
    if let Some(height) = node.get("lineHeight").and_then(Value::as_f64) {
        styles.push(format!("line-height:{height}"));
    }
    if str_prop(node, "listStyleType").is_none()
        && let Some(indent) = node.get("indent").and_then(Value::as_u64)
        && indent > 0
    {
        styles.push(format!("margin-left:{}px", indent * 24));
    }
    if styles.is_empty() {
        String::new()
    } else {
        format!(" style=\"{}\"", styles.join(";"))
    }
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Collect every storage path / URL referenced by media nodes in the document.
pub fn collect_media_urls(nodes: &[Value]) -> Vec<String> {
    let mut urls = Vec::new();
    walk_media(nodes, &mut urls);
    urls
}

fn walk_media(nodes: &[Value], urls: &mut Vec<String>) {
    for node in nodes {
        if matches!(
            node_type(node),
            "img" | "video" | "audio" | "file" | "media_embed"
        ) && let Some(url) = str_prop(node, "url")
            && !url.is_empty()
            && !urls.iter().any(|existing| existing == url)
        {
            urls.push(url.to_string());
        }
        walk_media(children(node), urls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    fn doc(value: Value) -> Vec<Value> {
        value.as_array().cloned().unwrap_or_default()
    }

    #[test]
    fn parses_prefixed_and_bare_documents() {
        let bare = r#"[{"type":"p","children":[{"text":"hi"}]}]"#;
        assert_eq!(parse_plate_document(bare).unwrap().len(), 1);
        assert_eq!(
            parse_plate_document(&format!("plate_json::{bare}"))
                .unwrap()
                .len(),
            1
        );
        assert!(parse_plate_document("").unwrap().is_empty());
        assert!(parse_plate_document("plate_json::not json").is_err());
    }

    #[test]
    fn renders_headings_marks_and_links() {
        let nodes = doc(json!([
            {"type": "h1", "children": [{"text": "Title"}]},
            {"type": "p", "children": [
                {"text": "plain "},
                {"text": "bold", "bold": true},
                {"text": " and "},
                {"text": "code", "code": true},
                {"text": " and "},
                {"type": "a", "url": "https://example.com", "children": [{"text": "link"}]}
            ]}
        ]));
        let md = to_markdown(&nodes, ImageHandling::Keep);
        assert!(md.starts_with("# Title\n\n"));
        assert!(md.contains("**bold**"));
        assert!(md.contains("`code`"));
        assert!(md.contains("[link](https://example.com)"));
    }

    #[test]
    fn renders_indent_based_lists() {
        let nodes = doc(json!([
            {"type": "p", "listStyleType": "disc", "indent": 1, "children": [{"text": "one"}]},
            {"type": "p", "listStyleType": "disc", "indent": 2, "children": [{"text": "nested"}]},
            {"type": "p", "listStyleType": "decimal", "indent": 1, "children": [{"text": "first"}]},
            {"type": "p", "listStyleType": "decimal", "indent": 1, "children": [{"text": "second"}]},
            {"type": "p", "listStyleType": "todo", "indent": 1, "checked": true, "children": [{"text": "done"}]}
        ]));
        let md = to_markdown(&nodes, ImageHandling::Keep);
        assert!(md.contains("- one\n"));
        assert!(md.contains("  - nested\n"));
        assert!(md.contains("1. first\n"));
        assert!(md.contains("2. second\n"));
        assert!(md.contains("- [x] done\n"));

        let html = to_html(&nodes, ImageHandling::Keep);
        assert!(html.contains("<ul style=\"list-style-type:disc\">"));
        assert!(html.contains("<ol style=\"list-style-type:decimal\">"));
        // disc(1) + nested disc(2) + todo(1); the decimal run closes the first disc list.
        assert_eq!(html.matches("</ul>").count(), 3);
        assert_eq!(html.matches("</ol>").count(), 1);
        assert!(html.contains("type=\"checkbox\" disabled checked"));
    }

    #[test]
    fn renders_tables_with_header_detection() {
        let nodes = doc(json!([
            {"type": "table", "children": [
                {"type": "tr", "children": [
                    {"type": "th", "children": [{"type": "p", "children": [{"text": "Name"}]}]},
                    {"type": "th", "children": [{"type": "p", "children": [{"text": "Value"}]}]}
                ]},
                {"type": "tr", "children": [
                    {"type": "td", "children": [{"type": "p", "children": [{"text": "a"}]}]},
                    {"type": "td", "children": [{"type": "p", "children": [{"text": "b"}]}]}
                ]}
            ]}
        ]));
        let md = to_markdown(&nodes, ImageHandling::Keep);
        assert!(md.contains("| Name | Value |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("| a | b |"));

        let html = to_html(&nodes, ImageHandling::Keep);
        assert!(html.contains("<thead>"));
        assert!(html.contains("<th>Name</th>"));
        assert!(html.contains("<tbody>"));
    }

    #[test]
    fn renders_code_blocks_and_fences_longer_than_content() {
        let nodes = doc(json!([
            {"type": "code_block", "lang": "rust", "children": [
                {"type": "code_line", "children": [{"text": "let a = 1;"}]},
                {"type": "code_line", "children": [{"text": "let b = \"```\";"}]}
            ]}
        ]));
        let md = to_markdown(&nodes, ImageHandling::Keep);
        assert!(md.contains("````rust\n"));
        assert!(md.contains("let a = 1;"));

        let html = to_html(&nodes, ImageHandling::Keep);
        assert!(html.contains("<pre><code class=\"language-rust\">"));
    }

    #[test]
    fn image_handling_modes() {
        let nodes = doc(json!([
            {"type": "img", "url": "storage://docs/a.png", "caption": [{"text": "Diagram"}]}
        ]));
        assert!(
            to_markdown(&nodes, ImageHandling::Keep).contains("![Diagram](storage://docs/a.png)")
        );
        assert!(to_markdown(&nodes, ImageHandling::Strip).trim().is_empty());
        assert!(to_markdown(&nodes, ImageHandling::AltText).contains("[Diagram]"));
        assert_eq!(collect_media_urls(&nodes), vec!["storage://docs/a.png"]);
    }

    #[test]
    fn html_keeps_alignment_and_colors_markdown_cannot() {
        let nodes = doc(json!([
            {"type": "p", "align": "center", "children": [
                {"text": "tinted", "color": "#ff0000", "bold": true}
            ]}
        ]));
        let html = to_html(&nodes, ImageHandling::Keep);
        assert!(html.contains("style=\"text-align:center\""));
        assert!(html.contains("color:#ff0000"));
        assert!(html.contains("<strong>tinted</strong>"));
    }

    #[test]
    fn escapes_html_and_markdown_metacharacters() {
        let nodes = doc(json!([
            {"type": "p", "children": [{"text": "a <b> & *star* [x]"}]}
        ]));
        let md = to_markdown(&nodes, ImageHandling::Keep);
        assert!(md.contains("\\*star\\*"));
        assert!(md.contains("\\[x\\]"));

        let html = to_html(&nodes, ImageHandling::Keep);
        assert!(html.contains("a &lt;b&gt; &amp; *star* [x]"));
    }

    #[test]
    fn unknown_block_types_degrade_to_their_text() {
        let nodes = doc(json!([
            {"type": "excalidraw", "children": [{"text": ""}]},
            {"type": "some_future_block", "children": [{"text": "kept"}]}
        ]));
        let md = to_markdown(&nodes, ImageHandling::Keep);
        assert!(md.contains("kept"));
    }
}

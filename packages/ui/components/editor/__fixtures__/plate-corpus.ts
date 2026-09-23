const FENCE = "```";

const lines = (...parts: string[]) => parts.join("\n");

export type MarkdownFixture = {
	readonly id: string;
	readonly title: string;
	readonly markdown: string;
	readonly malicious?: boolean;
	/** KaTeX output is sanitized with DOMPurify, which only exists with a DOM. */
	readonly usesKatex?: boolean;
};

export type StoredFixture = {
	readonly id: string;
	readonly title: string;
	readonly nodes: readonly unknown[];
	readonly malicious?: boolean;
	readonly usesKatex?: boolean;
};

export type EnvelopeFixture = {
	readonly id: string;
	readonly content: string;
	readonly usesKatex?: boolean;
};

export const MARKDOWN_FIXTURES: readonly MarkdownFixture[] = [
	{
		id: "M01-headings",
		title: "ATX h1-h6 and setext headings",
		markdown: lines(
			"# Heading one",
			"",
			"## Heading two",
			"",
			"### Heading three",
			"",
			"#### Heading four",
			"",
			"##### Heading five",
			"",
			"###### Heading six",
			"",
			"Setext one",
			"===",
			"",
			"Setext two",
			"---",
		),
	},
	{
		id: "M02-marks-markdown",
		title: "CommonMark/GFM marks, links, autolinks",
		markdown: lines(
			"Plain **bold** *italic* ***bold italic*** ~~strike~~ ~single~ `inline code` end.",
			"",
			'A [titled link](https://example.com/docs "Docs") and a bare https://example.com/bare and <https://example.org/angle> and www.example.net.',
			"",
			"Escaped \\*not italic\\* and a mailto:someone@example.com and [mail](mailto:a@b.co) and [tel](tel:+491234).",
			"",
			"[relative](/docs/page) [anchor](#section) [ftp](ftp://example.com/file)",
		),
	},
	{
		id: "M03-marks-mdx",
		title:
			"MDX-only marks: underline, sub, sup, kbd, highlight, del, span styles",
		markdown: lines(
			"<u>underline</u> H<sub>2</sub>O x<sup>2</sup> <kbd>Ctrl</kbd>+<kbd>K</kbd> <mark>highlight</mark> <del>deleted</del>",
			"",
			'<span style="color: #ff0000; background-color: yellow; font-size: 20px; font-family: monospace">styled span</span>',
		),
	},
	{
		id: "M04-special-links",
		title: "focus://, invalid://, user://, spoiler://, ||spoiler||, <user>",
		markdown: lines(
			"Open [Fetch Orders](focus://node_abc123) then [Ghost](invalid://missing).",
			"",
			"Ping [alice](user://sub-alice) and <user>sub-bob</user>.",
			"",
			"Secret ||hunter2|| and [explicit](spoiler://top%20secret).",
		),
	},
	{
		id: "M05-blockquotes",
		title: "Blockquotes incl. nesting and block content",
		markdown: lines(
			"> single line quote",
			"",
			"> outer",
			">",
			"> > nested quote",
			"",
			"> quote with **bold** and a list",
			"> - one",
			"> - two",
		),
	},
	{
		id: "M06-code-blocks",
		title: "Fenced code in several languages, tildes, no-lang, unknown-lang",
		markdown: lines(
			`${FENCE}ts`,
			"export const answer: number = 42;",
			"function greet(name: string) { return `hi ${name}`; }",
			FENCE,
			"",
			`${FENCE}python`,
			"def fib(n):",
			"    return n if n < 2 else fib(n - 1) + fib(n - 2)",
			FENCE,
			"",
			`${FENCE}rust`,
			'fn main() { println!("{}", 1 + 1); }',
			FENCE,
			"",
			`${FENCE}json`,
			'{ "a": [1, 2, { "b": null }] }',
			FENCE,
			"",
			`${FENCE}bash`,
			'echo "$HOME" | grep -v x',
			FENCE,
			"",
			`${FENCE}sql`,
			"SELECT id, name FROM users WHERE id = 1;",
			FENCE,
			"",
			`${FENCE}not-a-language`,
			"plain content",
			FENCE,
			"",
			FENCE,
			"no language\n\twith a tab",
			FENCE,
			"",
			"~~~yaml",
			"key: value",
			"~~~",
		),
	},
	{
		id: "M07-lists",
		title:
			"Bullet/ordered lists, nesting, loose lists, start numbers, code in list",
		markdown: lines(
			"- alpha",
			"  - beta",
			"    - gamma",
			"- delta",
			"",
			"3. three",
			"4. four",
			"   1. nested ordered",
			"   - nested bullet",
			"",
			"- loose one",
			"",
			"- loose two",
			"",
			"  continuation paragraph",
			"",
			"- with code",
			"",
			`  ${FENCE}ts`,
			"  const inList = true;",
			`  ${FENCE}`,
			"",
			"* star bullet",
			"+ plus bullet",
		),
	},
	{
		id: "M08-task-lists",
		title: "GFM task lists incl. nested",
		markdown: lines(
			"- [ ] open task",
			"- [x] done task",
			"  - [ ] nested open",
			"  - [X] nested done uppercase",
			"1. [ ] ordered task",
		),
	},
	{
		id: "M09-tables",
		title:
			"GFM tables: alignment, marks, links, empty cells, escaped pipes, wide, long cells",
		markdown: lines(
			"| Left | Center | Right |",
			"| :--- | :----: | ----: |",
			"| **bold** | `code` | [link](https://example.com) |",
			"|  | empty-left | a \\| pipe |",
			"",
			`| ${Array.from({ length: 10 }, (_, i) => `C${i}`).join(" | ")} |`,
			`| ${Array.from({ length: 10 }, () => "---").join(" | ")} |`,
			`| ${Array.from({ length: 10 }, (_, i) => `v${i}`).join(" | ")} |`,
			"",
			"| Key | Long value |",
			"| --- | --- |",
			`| lorem | ${"Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(6).trim()} |`,
		),
	},
	{
		id: "M10-breaks-rules",
		title: "remark-breaks soft breaks, hard breaks, thematic breaks",
		markdown: lines(
			"line one",
			"line two joined by remark-breaks",
			"",
			"hard break with two spaces  ",
			"after hard break",
			"",
			"backslash break\\",
			"after backslash",
			"",
			"---",
			"",
			"***",
			"",
			"___",
		),
	},
	{
		id: "M11-math",
		title: "Inline and block math, dollar amounts stay text",
		usesKatex: true,
		markdown: lines(
			"Inline $$E = mc^2$$ inside prose.",
			"",
			"$$",
			"\\int_0^1 x^2 \\, dx = \\frac{1}{3}",
			"$$",
			"",
			"It costs $5 today and $10 tomorrow.",
			"",
			`${FENCE}math`,
			"a^2 + b^2 = c^2",
			FENCE,
		),
	},
	{
		id: "M12-images",
		title: "Markdown images: plain, titled, empty alt, linked, storage://",
		markdown: lines(
			"![Alt text](https://example.com/a.png)",
			"",
			'![Titled](https://example.com/b.png "The title")',
			"",
			"![](https://example.com/no-alt.png)",
			"",
			"[![linked](https://example.com/c.png)](https://example.com/target)",
			"",
			"![stored](storage://apps/app-1/upload/d.png)",
			"",
			"Inline ![icon](https://example.com/icon.svg) inside text.",
		),
	},
	{
		id: "M13-media-mdx",
		title: "MDX media elements: video, youtube, audio, file",
		markdown: lines(
			'<video src="https://example.com/v.mp4" />',
			"",
			'<video src="https://www.youtube.com/watch?v=dQw4w9WgXcQ" />',
			"",
			'<audio src="https://example.com/a.mp3" />',
			"",
			'<file src="https://example.com/report.pdf" name="report.pdf" />',
		),
	},
	{
		id: "M14-mdx-blocks",
		title: "MDX callout, toc, date, columns",
		markdown: lines(
			"# Doc with toc",
			"",
			"<toc />",
			"",
			"## Section A",
			"",
			"<callout>Callout body with **bold**</callout>",
			"",
			"Due <date>2020-01-15</date> sharp.",
			"",
			'<column_group><column width="50%">Left column</column><column width="50%">Right column</column></column_group>',
		),
	},
	{
		id: "M15-directives",
		title: ":::directive admonitions and spoiler blocks",
		markdown: lines(
			":::info",
			"Informational `code` text.",
			":::",
			"",
			":::warning Rate Limiting",
			"Approaching the limit.",
			":::",
			"",
			":::error",
			"Something failed.",
			":::",
			"",
			":::success",
			"All good.",
			":::",
			"",
			":::tip",
			"Try this.",
			":::",
			"",
			":::spoiler Stack trace",
			`${FENCE}`,
			"Error: boom",
			"    at x (y.js:1:1)",
			FENCE,
			":::",
			"",
			":::info",
			"unterminated directive stays literal",
		),
	},
	{
		id: "M16-chart-fences",
		title: "nivo and plotly fences routed to ChartCodeBlock",
		markdown: lines(
			`${FENCE}nivo`,
			"type: bar",
			"title: Sales",
			"---",
			"month,sales",
			"Jan,1",
			"Feb,2",
			FENCE,
			"",
			`${FENCE}plotly`,
			"type: line",
			"---",
			"x,y",
			"1,2",
			"2,4",
			FENCE,
		),
	},
	{
		id: "M17-emoji",
		title: "remark-emoji shortcodes and raw unicode",
		markdown: "Launch :rocket: :+1: :not_a_real_emoji: and raw 🚀 ✅ 👍🏽",
	},
	{
		id: "M18-mentions",
		title: "remarkMention @user and [Display](mention:id)",
		markdown:
			"Hello @alice and [Bob Builder](mention:bob_id) and email bob@example.com.",
	},
	{
		id: "M19-references-footnotes",
		title: "Reference links, definitions, footnotes (non-block-cacheable)",
		markdown: lines(
			"See [the docs][ref] and [again][ref].",
			"",
			"[ref]: https://example.com/ref",
			"",
			"A claim with a footnote.[^1]",
			"",
			"[^1]: The footnote text.",
		),
	},
	{
		id: "M20-html-in-markdown",
		title: "HTML blocks and inline HTML (parsed as MDX JSX)",
		markdown: lines(
			"<div>block html</div>",
			"",
			"line with <br /> inside",
			"",
			"inline <b>bold html</b> and <em>em html</em> and <i>i</i>",
			"",
			"<details><summary>More</summary>Hidden body</details>",
		),
	},
	{
		id: "M21-mdx-breaking",
		title: "Text that breaks remark-mdx (fallback splitter path)",
		markdown: lines(
			"Math-ish: a < b and 5 > 3 and <3 love.",
			"",
			"Braces {curly} and {{double}} and an unclosed <tag",
			"",
			"Normal paragraph after the broken ones.",
		),
	},
	{
		id: "M22-empty-whitespace",
		title: "Whitespace-only content",
		markdown: "   \n\n  \n",
	},
	{
		id: "M23-unicode",
		title: "RTL, CJK, zero-width, combining marks, very long word",
		markdown: lines(
			"مرحبا بالعالم — עברית — 中文字符 — 日本語 — 한국어",
			"",
			"zero\u200bwidth\u200djoiner e\u0301 combining",
			"",
			"a".repeat(500),
		),
	},
	{
		id: "M24-deep-nesting",
		title: "Deeply nested lists and blockquotes",
		markdown: lines(
			Array.from(
				{ length: 30 },
				(_, depth) => `${"  ".repeat(depth)}- level ${depth}`,
			).join("\n"),
			"",
			`${">".repeat(30)} deep quote`,
		),
	},
	{
		id: "M25-long-document",
		title:
			"Long document above WINDOWING_BLOCK_THRESHOLD (40 top-level blocks)",
		markdown: Array.from({ length: 60 }, (_, i) =>
			lines(`## Section ${i}`, "", `Body ${i} with **bold** and \`code\`.`),
		).join("\n\n"),
	},
	{
		id: "M26-mixed-report",
		title: "Realistic chat/LLM answer mixing everything common",
		markdown: lines(
			"# Release notes",
			"",
			"A paragraph with `inline code`, **bold** and a [link](https://example.com/docs).",
			"",
			"## Changes",
			"",
			"- top level",
			"  - nested one",
			"- another",
			"",
			"1. first step",
			"2. second step",
			"",
			`${FENCE}ts`,
			"export const answer = 42;",
			FENCE,
			"",
			"| Feature | Status |",
			"| --- | --- |",
			"| Streaming | done |",
			"",
			"> A quoted remark.",
			"",
			":::tip",
			"Use the new node.",
			":::",
			"",
			"Closing paragraph.",
		),
	},
	{
		id: "M27-embed-fences",
		title: "embed fences routed to lazy EmbedCodeBlock",
		markdown: lines(
			`${FENCE}embed`,
			"https://youtube.com/watch?v=dQw4w9WgXcQ",
			FENCE,
			"",
			`${FENCE}embed`,
			"https://github.com/Rheosoph/flow-like",
			FENCE,
		),
	},
	{
		id: "M28-map-fence",
		title: "map fence routed to lazy MapCodeBlock (maplibre/WebGL)",
		markdown: lines(
			`${FENCE}map`,
			"lat: 48.1351",
			"lng: 11.5820",
			"label: HQ",
			FENCE,
		),
	},
	{
		id: "X01-script-tags",
		title: "Script and style tags in markdown",
		malicious: true,
		markdown: lines(
			"<script>window.__xss = 1</script>",
			"",
			"Text <script>window.__xss = 2</script> inline.",
			"",
			"<style>body { display: none }</style>",
			"",
			'<iframe src="javascript:window.__xss=3"></iframe>',
		),
	},
	{
		id: "X02-javascript-links",
		title: "javascript:/vbscript:/data: link schemes",
		malicious: true,
		markdown: lines(
			"[plain](javascript:window.__xss=1)",
			"",
			"[mixed case](JaVaScRiPt:window.__xss=2)",
			"",
			"[entity](jav&#x61;script:window.__xss=3)",
			"",
			"[vb](vbscript:msgbox(1))",
			"",
			"[data](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)",
			"",
			"<javascript:window.__xss=4>",
			"",
			"[ref link][evil]",
			"",
			"[evil]: javascript:window.__xss=5",
		),
	},
	{
		id: "X03-event-handlers-mdx",
		title: "on* attributes and javascript: URLs via MDX/HTML",
		malicious: true,
		markdown: lines(
			'<img src="x" onerror="window.__xss=1" />',
			"",
			'<video src="x" onerror="window.__xss=2" />',
			"",
			'<a href="javascript:window.__xss=3" onclick="window.__xss=4">anchor</a>',
			"",
			'<span style="background-image: url(javascript:window.__xss=5)" onmouseover="window.__xss=6">span</span>',
			"",
			'<callout onclick="window.__xss=7" icon="<img src=x onerror=window.__xss=8>">callout</callout>',
			"",
			'<file src="javascript:window.__xss=9" name="evil.pdf" />',
			"",
			'<svg onload="window.__xss=10"><circle r="1" /></svg>',
		),
	},
	{
		id: "X03b-event-handlers-without-img",
		title: "X03 without the <img> line that crashes the whole render on 49",
		malicious: true,
		markdown: lines(
			'<video src="x" onerror="window.__xss=2" />',
			"",
			'<a href="javascript:window.__xss=3" onclick="window.__xss=4">anchor</a>',
			"",
			'<span style="background-image: url(javascript:window.__xss=5)" onmouseover="window.__xss=6">span</span>',
			"",
			'<callout onclick="window.__xss=7" icon="<img src=x onerror=window.__xss=8>">callout</callout>',
			"",
			'<file src="javascript:window.__xss=9" name="evil.pdf" />',
			"",
			'<svg onload="window.__xss=10"><circle r="1" /></svg>',
		),
	},
	{
		id: "X04-image-urls",
		title: "Image and media URLs with dangerous schemes",
		malicious: true,
		markdown: lines(
			"![x](javascript:window.__xss=1)",
			"",
			'![x](x" onerror="window.__xss=2)',
			"",
			"![x](data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9ImFsZXJ0KDEpIi8+)",
		),
	},
	{
		id: "X05-katex",
		title: "KaTeX commands that must stay untrusted",
		malicious: true,
		usesKatex: true,
		markdown: lines(
			"$$\\href{javascript:window.__xss=1}{click}$$",
			"",
			"$$",
			"\\htmlClass{x}{y} \\url{javascript:window.__xss=2}",
			"$$",
		),
	},
	{
		id: "X06-embed-urls",
		title: "Embed fences with hostile content",
		malicious: true,
		markdown: lines(
			`${FENCE}embed`,
			"javascript:window.__xss=1",
			FENCE,
			"",
			`${FENCE}embed`,
			'url: "https://example.com/\\"><img src=x onerror=window.__xss=2>"',
			FENCE,
		),
	},
	{
		id: "X07-map-label",
		title: "Map fence with an HTML label",
		malicious: true,
		markdown: lines(
			`${FENCE}map`,
			'label: "<img src=x onerror=window.__xss=3>"',
			"lat: 1",
			"lng: 2",
			FENCE,
		),
	},
	{
		id: "X08-text-that-looks-like-markup",
		title: "Escaped markup in code and text must stay text in every output",
		malicious: true,
		markdown: lines(
			"Inline `<script>window.__xss=1</script>` code.",
			"",
			`${FENCE}html`,
			'<img src=x onerror="window.__xss=2">',
			FENCE,
			"",
			"Escaped \\<b onmouseover=window.__xss=3\\>not a tag\\</b\\>",
		),
	},
];

const t = (text: string, marks: Record<string, unknown> = {}) => ({
	text,
	...marks,
});
const p = (...children: unknown[]) => ({ type: "p", children });

export const STORED_FIXTURES: readonly StoredFixture[] = [
	{
		id: "S01-empty-array",
		title: "plate_json::[] (fresh RichText value)",
		nodes: [],
	},
	{
		id: "S02-empty-paragraph",
		title: "Single empty paragraph (RichText default document)",
		nodes: [p(t(""))],
	},
	{
		id: "S03-unknown-types",
		title: "Unknown element types, void without children, missing children",
		nodes: [
			{ type: "excalidraw", data: { elements: [] }, children: [t("")] },
			{ type: "tag", value: "urgent", children: [t("")] },
			{ type: "ai_chat", children: [t("")] },
			{ type: "totally_unknown_block", children: [t("unknown block text")] },
			p(
				t("inline "),
				{ type: "unknown_inline", children: [t("unknown inline")] },
				t(" after"),
			),
			{ type: "h4", children: [t("stored h4")] },
			{ type: "h5", children: [t("stored h5")] },
			{ type: "h6", children: [t("stored h6")] },
			{
				type: "media_embed",
				url: "https://youtu.be/dQw4w9WgXcQ",
				children: [t("")],
			},
			{ type: "placeholder", mediaType: "img", children: [t("")] },
		],
	},
	{
		id: "S04-classic-lists",
		title:
			"list-classic shapes (ul/ol/li/lic/action_item) the dead classic kit could have produced",
		nodes: [
			{
				type: "ul",
				children: [
					{
						type: "li",
						children: [{ type: "lic", children: [t("classic bullet")] }],
					},
				],
			},
			{
				type: "ol",
				children: [
					{
						type: "li",
						children: [{ type: "lic", children: [t("classic numbered")] }],
					},
				],
			},
			{ type: "action_item", checked: true, children: [t("classic todo")] },
		],
	},
	{
		id: "S05-marks-and-styles",
		title: "Every leaf mark and style prop on stored leaves",
		nodes: [
			p(
				t("bold ", { bold: true }),
				t("italic ", { italic: true }),
				t("underline ", { underline: true }),
				t("strike ", { strikethrough: true }),
				t("code ", { code: true }),
				t("sub ", { subscript: true }),
				t("sup ", { superscript: true }),
				t("kbd ", { kbd: true }),
				t("highlight ", { highlight: true }),
				t("color ", { color: "rgb(255, 0, 0)" }),
				t("bg ", { backgroundColor: "#ffff00" }),
				t("size ", { fontSize: "36px" }),
				t("family ", { fontFamily: "monospace" }),
				t("weight ", { fontWeight: "700" }),
				t("comment ", { comment: true, comment_c1: true }),
				t("suggest-insert ", {
					suggestion: true,
					suggestion_s1: {
						id: "s1",
						type: "insert",
						userId: "u",
						createdAt: 0,
					},
				}),
				t("suggest-remove ", {
					suggestion: true,
					suggestion_s2: {
						id: "s2",
						type: "remove",
						userId: "u",
						createdAt: 0,
					},
				}),
				t("ai ", { ai: true }),
			),
			{
				type: "p",
				align: "center",
				lineHeight: 2,
				indent: 1,
				children: [t("aligned, line-height, indented")],
			},
		],
	},
	{
		id: "S06-media-nodes",
		title:
			"Stored media with captions, widths, alignment, storage URLs, assetName",
		nodes: [
			{
				type: "img",
				url: "https://example.com/i.png",
				width: 320,
				align: "right",
				caption: [t("Image caption")],
				children: [t("")],
			},
			{
				type: "img",
				url: "storage://apps/app-1/upload/img.png",
				assetName: "AppAnatomy",
				alt: "Stored image",
				children: [t("")],
			},
			{
				type: "video",
				url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
				caption: [t("yt caption")],
				children: [t("")],
			},
			{
				type: "video",
				url: "https://example.com/v.mp4",
				width: "80%",
				children: [t("")],
			},
			{ type: "audio", url: "https://example.com/a.mp3", children: [t("")] },
			{
				type: "file",
				url: "https://example.com/f.pdf",
				name: "f.pdf",
				isUpload: true,
				children: [t("")],
			},
		],
	},
	{
		id: "S07-table-merged",
		title:
			"Table with colSizes, merged cells (colSpan/rowSpan), row size, borders, background",
		nodes: [
			{
				type: "table",
				colSizes: [120, 0, 200],
				marginLeft: 16,
				children: [
					{
						type: "tr",
						size: 40,
						children: [
							{ type: "th", children: [p(t("H1"))] },
							{ type: "th", colSpan: 2, children: [p(t("H2 spans two"))] },
						],
					},
					{
						type: "tr",
						children: [
							{
								type: "td",
								rowSpan: 2,
								background: "#fee2e2",
								children: [p(t("spans two rows"))],
							},
							{
								type: "td",
								borders: { left: { size: 1 }, top: { size: 0 } },
								children: [p(t("b"))],
							},
							{ type: "td", children: [p(t("c", { bold: true }))] },
						],
					},
					{
						type: "tr",
						children: [
							{ type: "td", children: [p(t("e"))] },
							{ type: "td", children: [p(t("f"))] },
						],
					},
				],
			},
		],
	},
	{
		id: "S08-structure",
		title:
			"Toggle, callout with icon/background, columns, toc, date, mention, focus/user/spoiler",
		nodes: [
			{ type: "h1", id: "h-1", children: [t("Title")] },
			{ type: "toc", children: [t("")] },
			{ type: "h2", id: "h-2", children: [t("Sub")] },
			{ type: "toggle", id: "tg", children: [t("Toggle heading")] },
			{ type: "p", indent: 1, children: [t("Toggle body (indented)")] },
			{
				type: "callout",
				icon: "🔥",
				backgroundColor: "#fef3c7",
				variant: "warning",
				children: [p(t("Callout body"))],
			},
			{
				type: "column_group",
				children: [
					{ type: "column", width: "33%", children: [p(t("col 1"))] },
					{ type: "column", width: "67%", children: [p(t("col 2"))] },
				],
			},
			p(
				t("Date: "),
				{ type: "date", date: "2020-02-29", children: [t("")] },
				t("."),
			),
			p(
				t("hi "),
				{ type: "mention", value: "Alice", key: "alice", children: [t("")] },
				t("!"),
			),
			p(
				{
					type: "focus_node",
					nodeId: "n1",
					nodeName: "Fetch",
					isInvalid: false,
					children: [t("")],
				},
				{
					type: "focus_node",
					nodeId: "",
					nodeName: "Gone",
					isInvalid: true,
					children: [t("")],
				},
				{ type: "user_mention", sub: "sub-1", children: [t("")] },
				{ type: "inline_spoiler", spoilerText: "secret", children: [t("")] },
			),
			{
				type: "a",
				url: "focus://n2",
				children: [t("raw focus link stored as a")],
			},
			{ type: "hr", children: [t("")] },
			{ type: "blockquote", children: [t("stored quote")] },
			{
				type: "code_block",
				lang: "ts",
				children: [{ type: "code_line", children: [t("const a = 1;")] }],
			},
		],
	},
	{
		id: "S09-malicious-stored",
		title: "Stored JSON with hostile props (must render inert)",
		malicious: true,
		nodes: [
			p({
				type: "a",
				url: "javascript:window.__xss=1",
				children: [t("js link")],
			}),
			p({
				type: "a",
				url: " JaVaScRiPt:window.__xss=2",
				target: "_self",
				children: [t("spaced js link")],
			}),
			{
				type: "img",
				url: "javascript:window.__xss=3",
				alt: '"><img src=x onerror=window.__xss=4>',
				children: [t("")],
			},
			{
				type: "file",
				url: "javascript:window.__xss=5",
				name: "<img src=x onerror=window.__xss=6>",
				children: [t("")],
			},
			{ type: "video", url: "javascript:window.__xss=7", children: [t("")] },
			{ type: "audio", url: "javascript:window.__xss=8", children: [t("")] },
			p({
				type: "mention",
				value: "<img src=x onerror=window.__xss=9>",
				children: [t("")],
			}),
			p({
				type: "date",
				date: "<script>window.__xss=10</script>",
				children: [t("")],
			}),
			{
				type: "callout",
				icon: "<img src=x onerror=window.__xss=11>",
				children: [p(t("c"))],
			},
			p(
				t(
					"<script>window.__xss=12</script><img src=x onerror=window.__xss=13>",
				),
			),
			{
				type: "p",
				attributes: { onclick: "window.__xss=14" },
				onclick: "window.__xss=15",
				style: "background:url(javascript:1)",
				children: [t("attr props")],
			},
			{
				type: "iframe",
				url: "javascript:window.__xss=16",
				src: "javascript:window.__xss=17",
				children: [t("")],
			},
			{
				type: "column_group",
				children: [
					{
						type: "column",
						width: "expression(alert(1))",
						children: [p(t("w"))],
					},
				],
			},
			p({
				type: "focus_node",
				nodeId: '"><img src=x onerror=window.__xss=19>',
				nodeName: "<b>n</b>",
				children: [t("")],
			}),
			p({
				type: "inline_spoiler",
				spoilerText: "<img src=x onerror=window.__xss=20>",
				children: [t("")],
			}),
			{
				type: "code_block",
				lang: '"><script>window.__xss=21</script>',
				children: [{ type: "code_line", children: [t("x")] }],
			},
		],
	},
	{
		id: "S10-shape-errors",
		title: "Malformed stored JSON shapes (should not crash the page)",
		nodes: [
			{ type: "p" },
			{ type: "p", children: [] },
			{ type: "p", children: [{ text: 42 }] },
			{ children: [t("no type")] },
			{ type: "table", children: [{ type: "tr", children: [] }] },
			{
				type: "code_block",
				children: [t("code_block with text child, no code_line")],
			},
		],
	},
	{
		id: "S11-orphan-table-cell",
		title: "A td outside any table (crashes TableCellElementStatic on 49)",
		nodes: [
			p(t("before")),
			{
				type: "td",
				colSpan: '"><img src=x onerror=1>',
				children: [p(t("orphan cell"))],
			},
		],
	},
	{
		id: "S12-indent-lists",
		title:
			"Indent lists: todo checked/unchecked, listStart, roman/alpha styles, restart, nesting",
		nodes: [
			{
				type: "p",
				indent: 1,
				listStyleType: "todo",
				checked: true,
				children: [t("done todo")],
			},
			{
				type: "p",
				indent: 1,
				listStyleType: "todo",
				checked: false,
				listStart: 2,
				children: [t("open todo")],
			},
			{
				type: "p",
				indent: 1,
				listStyleType: "decimal",
				listStart: 3,
				children: [t("third")],
			},
			{
				type: "p",
				indent: 1,
				listStyleType: "decimal",
				listStart: 4,
				children: [t("fourth")],
			},
			{
				type: "p",
				indent: 2,
				listStyleType: "lower-alpha",
				children: [t("nested alpha")],
			},
			{
				type: "p",
				indent: 3,
				listStyleType: "upper-roman",
				children: [t("nested roman")],
			},
			{
				type: "p",
				indent: 1,
				listStyleType: "decimal",
				listRestart: 1,
				children: [t("restarted")],
			},
			{
				type: "h2",
				indent: 1,
				listStyleType: "disc",
				children: [t("heading as list item")],
			},
			{
				type: "blockquote",
				indent: 1,
				listStyleType: "disc",
				children: [t("quote as list item")],
			},
		],
	},
	{
		id: "S13-block-styles",
		title: "Headings and paragraphs with align, lineHeight and plain indent",
		nodes: [
			{ type: "h1", align: "center", children: [t("Centered title")] },
			{
				type: "h2",
				align: "right",
				lineHeight: 1.5,
				children: [t("Right subtitle")],
			},
			{ type: "h3", indent: 2, children: [t("Indented h3")] },
			{
				type: "p",
				align: "justify",
				lineHeight: 3,
				children: [t("Justified paragraph with a tall line height.")],
			},
			{
				type: "blockquote",
				align: "center",
				children: [t("centered quote")],
			},
		],
	},
	{
		id: "S14-links",
		title: "Links with target, marks inside, relative/mailto/anchor URLs",
		nodes: [
			p(
				t("Visit "),
				{
					type: "a",
					url: "https://example.com",
					target: "_blank",
					children: [t("example", { bold: true })],
				},
				t(", "),
				{ type: "a", url: "/relative/path", children: [t("relative")] },
				t(", "),
				{ type: "a", url: "mailto:a@b.co", children: [t("mail")] },
				t(", "),
				{ type: "a", url: "#anchor", children: [t("anchor")] },
				t(", "),
				{ type: "a", url: "user://sub-2", children: [t("user link")] },
				t(" and "),
				{
					type: "a",
					url: "spoiler://hidden%20text",
					children: [t("spoiler link")],
				},
				t("."),
			),
		],
	},
	{
		id: "S15-math",
		title: "Block and inline equations",
		usesKatex: true,
		nodes: [
			{ type: "equation", texExpression: "\\sum_{i=0}^n i", children: [t("")] },
			p(
				t("inline "),
				{ type: "inline_equation", texExpression: "x^2", children: [t("")] },
				t(" math"),
			),
			{ type: "equation", texExpression: "", children: [t("")] },
		],
	},
	{
		id: "S16-malicious-math",
		title: "Stored KaTeX with javascript: hrefs and HTML commands",
		malicious: true,
		usesKatex: true,
		nodes: [
			{
				type: "equation",
				texExpression: "\\href{javascript:window.__xss=18}{x}",
				children: [t("")],
			},
			p({
				type: "inline_equation",
				texExpression: "\\url{javascript:window.__xss=22} \\htmlId{x}{y}",
				children: [t("")],
			}),
		],
	},
];

export const MALFORMED_ENVELOPES: readonly EnvelopeFixture[] = [
	{ id: "E01-bad-json", content: "plate_json::{not json" },
	{
		id: "E02-object-not-array",
		content: 'plate_json::{"type":"p","children":[{"text":"obj"}]}',
	},
	{ id: "E03-empty-envelope", content: "plate_json::" },
	{ id: "E04-empty-array", content: "plate_json::[]" },
	{ id: "E05-null", content: "plate_json::null" },
	{
		id: "E06-prefix-inside-markdown",
		content: "Text that mentions plate_json:: in the middle",
	},
];

export const HTML_IMPORT_FIXTURES: readonly {
	readonly id: string;
	readonly html: string;
}[] = [
	{
		id: "H01-img-onerror",
		html: '<p>a</p><img src="x" onerror="window.__xss=1">',
	},
	{
		id: "H02-js-href",
		html: '<p><a href="javascript:window.__xss=2">link</a></p>',
	},
	{
		id: "H03-svg-onload",
		html: '<svg onload="window.__xss=3"></svg><p>b</p>',
	},
	{
		id: "H04-font-wrapper",
		html: '<p><font color="red"><img src="x" onerror="window.__xss=4">font</font></p>',
	},
	{
		id: "H05-styled-span",
		html: '<p><span style="color:red;font-weight:700"><img src="x" onerror="window.__xss=5">span</span></p>',
	},
	{
		id: "H06-mxss",
		html: '<math><mtext><table><mglyph><style><img src=x onerror="window.__xss=6">',
	},
	{
		id: "H07-iframe",
		html: '<iframe src="javascript:window.__xss=7"></iframe><p>c</p>',
	},
	{
		id: "H08-word-docx",
		html: '<html xmlns:o="urn:schemas-microsoft-com:office:office"><body><p class="MsoNormal"><b>Word</b> <span style="mso-list:l0 level1 lfo1">item</span><o:p></o:p></p></body></html>',
	},
	{
		id: "H09-plate-export",
		html: '<div data-slate-editor="true"><div data-slate-node="element" data-slate-type="p" class="slate-p"><span data-slate-node="text"><span data-slate-leaf="true"><strong>exported</strong></span></span></div><img src="x" onerror="window.__xss=9"></div>',
	},
	{
		id: "H10-benign",
		html: "<h1>Title</h1><p>Hello <strong>bold</strong> <em>it</em> <u>u</u> <s>s</s> <code>c</code></p><ul><li>one</li><li>two</li></ul><table><tr><td>1</td><td>2</td></tr></table><blockquote>q</blockquote><pre><code>x</code></pre>",
	},
];

export const STREAMING_PREFIX_FIXTURES: readonly EnvelopeFixture[] = [
	{ id: "P01-open-fence", content: `Intro.\n\n${FENCE}ts\nconst a = 1;\n\nco` },
	{ id: "P02-half-table", content: "Intro.\n\n| A | B |\n| --- |" },
	{ id: "P03-dangling-bold", content: "Some **bold that never clo" },
	{ id: "P04-open-link", content: "A [link](https://exa" },
	{ id: "P05-half-list", content: "1. one\n2. tw" },
	{ id: "P06-open-directive", content: ":::warning Title\nbody so far" },
	{ id: "P07-open-math", content: "$$\n\\frac{1}{", usesKatex: true },
	{ id: "P08-open-html", content: "<details><summary>x" },
	{ id: "P09-open-spoiler", content: "secret ||hunter" },
	{ id: "P10-open-user-tag", content: "ping <user>sub-" },
];

/** Code that the markdown parser's JSX rewrite must leave as `verbatim` text. */
export const CODE_MARKUP_FIXTURES: readonly {
	readonly title: string;
	readonly markdown: string;
	readonly verbatim: readonly string[];
}[] = [
	{
		title: "fenced code",
		markdown: `${FENCE}html\n<div class="a" for="b" checked>x</div>\n<br>\n<!-- note -->\n${FENCE}`,
		verbatim: [
			'<div class="a" for="b" checked>x</div>',
			"<br>",
			"<!-- note -->",
		],
	},
	{
		title: "a fence inside a quote",
		markdown: `> ${FENCE}html\n> <p class="q">quoted</p>\n> ${FENCE}`,
		verbatim: ['<p class="q">quoted</p>'],
	},
	{
		title: "a fence inside a list item",
		markdown: `- item\n\n  ${FENCE}html\n  <span class="li">x</span>\n  ${FENCE}`,
		verbatim: ['<span class="li">x</span>'],
	},
	{
		title: "an unterminated fence",
		markdown: `${FENCE}html\n<div class="open"`,
		verbatim: ['<div class="open"'],
	},
	{
		title: "inline code",
		markdown: 'Use `<div class="a">` and ``<a href=x>``.',
		verbatim: ['<div class="a">', "<a href=x>"],
	},
	{
		title: "escaped tags",
		markdown: 'Escaped \\<div class="x"> text',
		verbatim: ['Escaped <div class="x"> text'],
	},
	{
		title: "inline code inside list items",
		markdown: '- item with `<input disabled>`\n- and `<label for="a">`',
		verbatim: ["<input disabled>", '<label for="a">'],
	},
	{
		title: "inline code inside a quote",
		markdown: "> quoted `<img src=x>` and `<br>`",
		verbatim: ["<img src=x>", "<br>"],
	},
	{
		title: "void tags, boolean attributes and JSX in a fence",
		markdown: `${FENCE}tsx\n<input readonly required>\nconst el = <img src={src}>;\n${FENCE}`,
		verbatim: ["<input readonly required>", "const el = <img src={src}>;"],
	},
	{
		title: "a tilde fence inside a numbered list",
		markdown: '1. step\n\n   ~~~html\n   <hr class="rule">\n   ~~~',
		verbatim: ['<hr class="rule">'],
	},
];

export const toEnvelope = (nodes: readonly unknown[]) =>
	`plate_json::${JSON.stringify(nodes)}`;

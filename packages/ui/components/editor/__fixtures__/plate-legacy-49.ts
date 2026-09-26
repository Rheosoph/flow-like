import type { EnvelopeFixture } from "./plate-corpus";

/**
 * Envelopes persisted by Plate 49 (platejs 49.2.21). They stand for documents
 * already stored in boards, apps and chats, so never regenerate them with a
 * newer Plate: every later version must keep rendering these.
 */

/** `safeDeserialize` output for each markdown fixture (the static parse shape). */
export const LEGACY_STATIC_ENVELOPES: readonly EnvelopeFixture[] = [
	{
		id: "L49-static-M01-headings",
		content:
			'plate_json::[{"children":[{"text":"Heading one"}],"type":"h1"},{"children":[{"text":"Heading two"}],"type":"h2"},{"children":[{"text":"Heading three"}],"type":"h3"},{"children":[{"text":"Heading four"}],"type":"h4"},{"children":[{"text":"Heading five"}],"type":"h5"},{"children":[{"text":"Heading six"}],"type":"h6"},{"children":[{"text":"Setext one"}],"type":"h1"},{"children":[{"text":"Setext two"}],"type":"h2"}]',
	},
	{
		id: "L49-static-M02-marks-markdown",
		content:
			'plate_json::[{"children":[{"text":"Plain "},{"bold":true,"text":"bold"},{"text":" "},{"italic":true,"text":"italic"},{"text":" "},{"italic":true,"bold":true,"text":"bold italic"},{"text":" "},{"strikethrough":true,"text":"strike"},{"text":" "},{"strikethrough":true,"text":"single"},{"text":" "},{"code":true,"text":"inline code"},{"text":" end."}],"type":"p"},{"children":[{"text":"A "},{"children":[{"text":"titled link"}],"type":"a","url":"https://example.com/docs"},{"text":" and a bare "},{"children":[{"text":"https://example.com/bare"}],"type":"a","url":"https://example.com/bare"},{"text":" and"},{"children":[{"text":"https://example.org/angle"}],"type":"a","url":"https://example.org/angle"},{"text":" and "},{"children":[{"text":"www.example.net"}],"type":"a","url":"http://www.example.net"},{"text":"."}],"type":"p"}]',
	},
	{
		id: "L49-static-M03-marks-mdx",
		content:
			'plate_json::[{"children":[{"underline":true,"text":"underline"},{"text":" H"},{"subscript":true,"text":"2"},{"text":"O x"},{"superscript":true,"text":"2"},{"text":" "},{"kbd":true,"text":"Ctrl"},{"text":"+"},{"kbd":true,"text":"K"},{"text":" "},{"highlight":true,"text":"highlight"},{"text":" "},{"mdxJsxTextElement":true,"text":"deleted"}],"type":"p"},{"children":[{"backgroundColor":"yellow","color":"#ff0000","fontFamily":"monospace","fontSize":"20px","text":"styled span"}],"type":"p"}]',
	},
	{
		id: "L49-static-M04-special-links",
		content:
			'plate_json::[{"children":[{"text":"Open "},{"type":"focus_node","nodeId":"node_abc123","nodeName":"Fetch Orders","isInvalid":false,"children":[{"text":""}]},{"text":" then "},{"type":"focus_node","nodeId":"","nodeName":"Ghost","isInvalid":true,"children":[{"text":""}]},{"text":"."}],"type":"p"},{"children":[{"text":"Ping "},{"type":"user_mention","sub":"sub-alice","children":[{"text":""}]},{"text":" and "},{"text":"<user>sub-bob</user>"},{"text":"."}],"type":"p"},{"children":[{"text":"Secret "},{"type":"inline_spoiler","spoilerText":"hunter2","children":[{"text":""}]},{"text":" and "},{"type":"inline_spoiler","spoilerText":"top secret","children":[{"text":""}]},{"text":"."}],"type":"p"}]',
	},
	{
		id: "L49-static-M05-blockquotes",
		content:
			'plate_json::[{"children":[{"text":"single line quote"}],"type":"blockquote"},{"children":[{"text":"outer"},{"text":"\\n"},{"text":"\\n"},{"children":[{"text":"nested quote"}],"type":"p"}],"type":"blockquote"},{"children":[{"text":"quote with "},{"bold":true,"text":"bold"},{"text":" and a list"},{"text":"\\n"},{"text":"\\n"}],"type":"blockquote"}]',
	},
	{
		id: "L49-static-M06-code-blocks",
		content:
			'plate_json::[{"children":[{"children":[{"text":"export const answer: number = 42;"}],"type":"code_line"},{"children":[{"text":"function greet(name: string) { return `hi ${name}`; }"}],"type":"code_line"}],"lang":"ts","type":"code_block"},{"children":[{"children":[{"text":"def fib(n):"}],"type":"code_line"},{"children":[{"text":"    return n if n < 2 else fib(n - 1) + fib(n - 2)"}],"type":"code_line"}],"lang":"python","type":"code_block"},{"children":[{"children":[{"text":"fn main() { println!(\\"{}\\", 1 + 1); }"}],"type":"code_line"}],"lang":"rust","type":"code_block"},{"children":[{"children":[{"text":"{ \\"a\\": [1, 2, { \\"b\\": null }] }"}],"type":"code_line"}],"lang":"json","type":"code_block"},{"children":[{"children":[{"text":"echo \\"$HOME\\" | grep -v x"}],"type":"code_line"}],"lang":"bash","type":"code_block"},{"children":[{"children":[{"text":"SELECT id, name FROM users WHERE id = 1;"}],"type":"code_line"}],"lang":"sql","type":"code_block"},{"children":[{"children":[{"text":"plain content"}],"type":"code_line"}],"lang":"not-a-language","type":"code_block"},{"children":[{"children":[{"text":"no language"}],"type":"code_line"},{"children":[{"text":"\\twith a tab"}],"type":"code_line"}],"type":"code_block"},{"children":[{"children":[{"text":"key: value"}],"type":"code_line"}],"lang":"yaml","type":"code_block"}]',
	},
	{
		id: "L49-static-M07-lists",
		content:
			'plate_json::[{"children":[{"text":"alpha"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"beta"}],"type":"p","indent":2,"listStyleType":"disc"},{"children":[{"text":"gamma"}],"type":"p","indent":3,"listStyleType":"disc"},{"children":[{"text":"delta"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"three"}],"type":"p","indent":1,"listStyleType":"decimal","listStart":3},{"children":[{"text":"four"}],"type":"p","indent":1,"listStyleType":"decimal","listStart":4},{"children":[{"text":"nested ordered"}],"type":"p","indent":2,"listStyleType":"decimal","listStart":1},{"children":[{"text":"nested bullet"}],"type":"p","indent":2,"listStyleType":"disc"},{"children":[{"text":"loose one"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"loose two"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"continuation paragraph"}],"type":"p","indent":2},{"children":[{"text":"with code"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"children":[{"text":"const inList = true;"}],"type":"code_line"}],"lang":"ts","type":"code_block","indent":2},{"children":[{"text":"star bullet"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"plus bullet"}],"type":"p","indent":1,"listStyleType":"disc"}]',
	},
	{
		id: "L49-static-M08-task-lists",
		content:
			'plate_json::[{"children":[{"text":"open task"}],"type":"p","indent":1,"listStyleType":"todo","checked":false},{"children":[{"text":"done task"}],"type":"p","indent":1,"listStyleType":"todo","checked":true},{"children":[{"text":"nested open"}],"type":"p","indent":2,"listStyleType":"todo","checked":false},{"children":[{"text":"nested done uppercase"}],"type":"p","indent":2,"listStyleType":"todo","checked":true},{"children":[{"text":"ordered task"}],"type":"p","indent":1,"listStyleType":"todo","checked":false,"listStart":1}]',
	},
	{
		id: "L49-static-M09-tables",
		content:
			'plate_json::[{"children":[{"children":[{"children":[{"children":[{"text":"Left"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"Center"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"Right"}],"type":"p"}],"type":"th"}],"type":"tr"},{"children":[{"children":[{"children":[{"bold":true,"text":"bold"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"code":true,"text":"code"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"link"}],"type":"a","url":"https://example.com"}],"type":"td"}],"type":"tr"},{"children":[{"children":[{"children":[{"text":""}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"empty-left"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"a | pipe"}],"type":"p"}],"type":"td"}],"type":"tr"}],"type":"table"},{"children":[{"children":[{"children":[{"children":[{"text":"C0"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C1"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C2"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C3"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C4"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C5"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C6"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C7"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C8"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"C9"}],"type":"p"}],"type":"th"}],"type":"tr"},{"children":[{"children":[{"children":[{"text":"v0"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v1"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v2"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v3"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v4"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v5"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v6"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v7"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v8"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"v9"}],"type":"p"}],"type":"td"}],"type":"tr"}],"type":"table"},{"children":[{"children":[{"children":[{"children":[{"text":"Key"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"Long value"}],"type":"p"}],"type":"th"}],"type":"tr"},{"children":[{"children":[{"children":[{"text":"lorem"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit."}],"type":"p"}],"type":"td"}],"type":"tr"}],"type":"table"}]',
	},
	{
		id: "L49-static-M10-breaks-rules",
		content:
			'plate_json::[{"children":[{"text":"line one"},{"text":"\\n"},{"text":"line two joined by remark-breaks"}],"type":"p"},{"children":[{"text":"hard break with two spaces"},{"text":"\\n"},{"text":"after hard break"}],"type":"p"},{"children":[{"text":"backslash break"},{"text":"\\n"},{"text":"after backslash"}],"type":"p"},{"children":[{"text":""}],"type":"hr"},{"children":[{"text":""}],"type":"hr"},{"children":[{"text":""}],"type":"hr"}]',
	},
	{
		id: "L49-static-M11-math",
		content:
			'plate_json::[{"children":[{"text":"Inline "},{"children":[{"text":""}],"texExpression":"E = mc^2","type":"inline_equation"},{"text":" inside prose."}],"type":"p"},{"children":[{"text":""}],"texExpression":"\\\\int_0^1 x^2 \\\\, dx = \\\\frac{1}{3}","type":"equation"},{"children":[{"text":"It costs $5 today and $10 tomorrow."}],"type":"p"},{"children":[{"children":[{"text":"a^2 + b^2 = c^2"}],"type":"code_line"}],"lang":"math","type":"code_block"}]',
		usesKatex: true,
	},
	{
		id: "L49-static-M12-images",
		content:
			'plate_json::[{"caption":[{"text":"Alt text"}],"children":[{"text":""}],"type":"img","url":"https://example.com/a.png"},{"caption":[{"text":"Titled"}],"children":[{"text":""}],"type":"img","url":"https://example.com/b.png"},{"caption":[{"text":""}],"children":[{"text":""}],"type":"img","url":"https://example.com/no-alt.png"},{"children":[{"children":[{"caption":[{"text":"linked"}],"children":[{"text":""}],"type":"img","url":"https://example.com/c.png"}],"type":"a","url":"https://example.com/target"}],"type":"p"},{"caption":[{"text":"stored"}],"children":[{"text":""}],"type":"img","url":"storage://apps/app-1/upload/d.png"},{"children":[{"text":"Inline "}],"type":"p"},{"caption":[{"text":"icon"}],"children":[{"text":""}],"type":"img","url":"https://example.com/icon.svg"},{"children":[{"text":" inside text."}],"type":"p"}]',
	},
	{
		id: "L49-static-M13-media-mdx",
		content:
			'plate_json::[{"children":[{"text":""}],"type":"video","url":"https://example.com/v.mp4"},{"children":[{"text":""}],"type":"video","url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ"},{"children":[{"text":""}],"type":"audio","url":"https://example.com/a.mp3"},{"children":[{"text":""}],"type":"file","url":"https://example.com/report.pdf","name":"report.pdf"}]',
	},
	{
		id: "L49-static-M14-mdx-blocks",
		content:
			'plate_json::[{"children":[{"text":"Doc with toc"}],"type":"h1"},{"children":[{"text":""}],"type":"toc"},{"children":[{"text":"Section A"}],"type":"h2"},{"children":[{"children":[{"text":"Callout body with "},{"bold":true,"text":"bold"}],"type":"callout"}],"type":"p"},{"children":[{"text":"Due "},{"children":[{"text":""}],"date":"2020-01-15","type":"date"},{"text":" sharp."}],"type":"p"},{"children":[{"children":[{"children":[{"text":"Left column"}],"type":"column","width":"50%"},{"children":[{"text":"Right column"}],"type":"column","width":"50%"}],"type":"column_group"}],"type":"p"}]',
	},
	{
		id: "L49-static-M15-directives",
		content:
			'plate_json::[{"children":[{"children":[{"text":"Informational `code` text."}],"type":"code_line"}],"lang":"directive-info","type":"code_block"},{"children":[{"children":[{"text":"Rate Limiting"}],"type":"code_line"},{"children":[{"text":"---"}],"type":"code_line"},{"children":[{"text":"Approaching the limit."}],"type":"code_line"}],"lang":"directive-warning","type":"code_block"},{"children":[{"children":[{"text":"Something failed."}],"type":"code_line"}],"lang":"directive-error","type":"code_block"},{"children":[{"children":[{"text":"All good."}],"type":"code_line"}],"lang":"directive-success","type":"code_block"},{"children":[{"children":[{"text":"Try this."}],"type":"code_line"}],"lang":"directive-tip","type":"code_block"},{"children":[{"children":[{"text":"Stack trace"}],"type":"code_line"},{"children":[{"text":"---"}],"type":"code_line"},{"children":[{"text":"```"}],"type":"code_line"},{"children":[{"text":"Error: boom"}],"type":"code_line"},{"children":[{"text":"    at x (y.js:1:1)"}],"type":"code_line"},{"children":[{"text":"```"}],"type":"code_line"}],"lang":"directive-spoiler","type":"code_block"},{"children":[{"text":":::info"},{"text":"\\n"},{"text":"unterminated directive stays literal"}],"type":"p"}]',
	},
	{
		id: "L49-static-M16-chart-fences",
		content:
			'plate_json::[{"children":[{"children":[{"text":"type: bar"}],"type":"code_line"},{"children":[{"text":"title: Sales"}],"type":"code_line"},{"children":[{"text":"---"}],"type":"code_line"},{"children":[{"text":"month,sales"}],"type":"code_line"},{"children":[{"text":"Jan,1"}],"type":"code_line"},{"children":[{"text":"Feb,2"}],"type":"code_line"}],"lang":"nivo","type":"code_block"},{"children":[{"children":[{"text":"type: line"}],"type":"code_line"},{"children":[{"text":"---"}],"type":"code_line"},{"children":[{"text":"x,y"}],"type":"code_line"},{"children":[{"text":"1,2"}],"type":"code_line"},{"children":[{"text":"2,4"}],"type":"code_line"}],"lang":"plotly","type":"code_block"}]',
	},
	{
		id: "L49-static-M17-emoji",
		content:
			'plate_json::[{"children":[{"text":"Launch "},{"text":"🚀"},{"text":" "},{"text":"👍"},{"text":" :not_a_real_emoji: and raw 🚀 ✅ 👍🏽"}],"type":"p"}]',
	},
	{
		id: "L49-static-M18-mentions",
		content:
			'plate_json::[{"children":[{"text":"Hello "},{"children":[{"text":""}],"type":"mention","value":"alice"},{"text":" and "},{"children":[{"text":""}],"type":"mention","value":"Bob Builder","key":"bob_id"},{"text":" and email "},{"children":[{"text":"bob@example.com"}],"type":"a","url":"mailto:bob@example.com"},{"text":"."}],"type":"p"}]',
	},
	{
		id: "L49-static-M19-references-footnotes",
		content:
			'plate_json::[{"children":[{"text":"See "},{"text":" and "},{"text":"."}],"type":"p"},{"children":[{"text":"A claim with a footnote."}],"type":"p"},{"children":[{"text":"The footnote text."}],"type":"p"}]',
	},
	{
		id: "L49-static-M20-html-in-markdown",
		content:
			'plate_json::[{"children":[{"text":"<div>block html</div>"}],"type":"p"},{"children":[{"text":"line with "},{"text":"\\n"},{"text":" inside"}],"type":"p"},{"children":[{"text":"inline "},{"text":"<b>bold html</b>"},{"text":" and "},{"text":"<em>em html</em>"},{"text":" and "},{"text":"<i>i</i>"}],"type":"p"},{"children":[{"text":"<details>Hidden body</details>"}],"type":"p"}]',
	},
	{
		id: "L49-static-M21-mdx-breaking",
		content:
			'plate_json::[{"children":[{"text":"Math-ish: a"},{"text":"< b and 5 > 3 and <3 love."}],"type":"p"}]',
	},
	{
		id: "L49-static-M22-empty-whitespace",
		content: 'plate_json::[{"type":"p","children":[{"text":""}]}]',
	},
	{
		id: "L49-static-M23-unicode",
		content:
			'plate_json::[{"children":[{"text":"مرحبا بالعالم — עברית — 中文字符 — 日本語 — 한국어"}],"type":"p"},{"children":[{"text":"zero\u200bwidth\u200djoiner e\u0301 combining"}],"type":"p"},{"children":[{"text":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"type":"p"}]',
	},
	{
		id: "L49-static-M24-deep-nesting",
		content:
			'plate_json::[{"children":[{"text":"level 0"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"level 1"}],"type":"p","indent":2,"listStyleType":"disc"},{"children":[{"text":"level 2"}],"type":"p","indent":3,"listStyleType":"disc"},{"children":[{"text":"level 3"}],"type":"p","indent":4,"listStyleType":"disc"},{"children":[{"text":"level 4"}],"type":"p","indent":5,"listStyleType":"disc"},{"children":[{"text":"level 5"}],"type":"p","indent":6,"listStyleType":"disc"},{"children":[{"text":"level 6"}],"type":"p","indent":7,"listStyleType":"disc"},{"children":[{"text":"level 7"}],"type":"p","indent":8,"listStyleType":"disc"},{"children":[{"text":"level 8"}],"type":"p","indent":9,"listStyleType":"disc"},{"children":[{"text":"level 9"}],"type":"p","indent":10,"listStyleType":"disc"},{"children":[{"text":"level 10"}],"type":"p","indent":11,"listStyleType":"disc"},{"children":[{"text":"level 11"}],"type":"p","indent":12,"listStyleType":"disc"},{"children":[{"text":"level 12"}],"type":"p","indent":13,"listStyleType":"disc"},{"children":[{"text":"level 13"}],"type":"p","indent":14,"listStyleType":"disc"},{"children":[{"text":"level 14"}],"type":"p","indent":15,"listStyleType":"disc"},{"children":[{"text":"level 15"}],"type":"p","indent":16,"listStyleType":"disc"},{"children":[{"text":"level 16"}],"type":"p","indent":17,"listStyleType":"disc"},{"children":[{"text":"level 17"}],"type":"p","indent":18,"listStyleType":"disc"},{"children":[{"text":"level 18"}],"type":"p","indent":19,"listStyleType":"disc"},{"children":[{"text":"level 19"}],"type":"p","indent":20,"listStyleType":"disc"},{"children":[{"text":"level 20"}],"type":"p","indent":21,"listStyleType":"disc"},{"children":[{"text":"level 21"}],"type":"p","indent":22,"listStyleType":"disc"},{"children":[{"text":"level 22"}],"type":"p","indent":23,"listStyleType":"disc"},{"children":[{"text":"level 23"}],"type":"p","indent":24,"listStyleType":"disc"},{"children":[{"text":"level 24"}],"type":"p","indent":25,"listStyleType":"disc"},{"children":[{"text":"level 25"}],"type":"p","indent":26,"listStyleType":"disc"},{"children":[{"text":"level 26"}],"type":"p","indent":27,"listStyleType":"disc"},{"children":[{"text":"level 27"}],"type":"p","indent":28,"listStyleType":"disc"},{"children":[{"text":"level 28"}],"type":"p","indent":29,"listStyleType":"disc"},{"children":[{"text":"level 29"}],"type":"p","indent":30,"listStyleType":"disc"},{"children":[{"children":[{"text":"deep quote"}],"type":"p"}],"type":"blockquote"}]',
	},
	{
		id: "L49-static-M26-mixed-report",
		content:
			'plate_json::[{"children":[{"text":"Release notes"}],"type":"h1"},{"children":[{"text":"A paragraph with "},{"code":true,"text":"inline code"},{"text":", "},{"bold":true,"text":"bold"},{"text":" and a "},{"children":[{"text":"link"}],"type":"a","url":"https://example.com/docs"},{"text":"."}],"type":"p"},{"children":[{"text":"Changes"}],"type":"h2"},{"children":[{"text":"top level"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"nested one"}],"type":"p","indent":2,"listStyleType":"disc"},{"children":[{"text":"another"}],"type":"p","indent":1,"listStyleType":"disc"},{"children":[{"text":"first step"}],"type":"p","indent":1,"listStyleType":"decimal","listStart":1},{"children":[{"text":"second step"}],"type":"p","indent":1,"listStyleType":"decimal","listStart":2},{"children":[{"children":[{"text":"export const answer = 42;"}],"type":"code_line"}],"lang":"ts","type":"code_block"},{"children":[{"children":[{"children":[{"children":[{"text":"Feature"}],"type":"p"}],"type":"th"},{"children":[{"children":[{"text":"Status"}],"type":"p"}],"type":"th"}],"type":"tr"},{"children":[{"children":[{"children":[{"text":"Streaming"}],"type":"p"}],"type":"td"},{"children":[{"children":[{"text":"done"}],"type":"p"}],"type":"td"}],"type":"tr"}],"type":"table"},{"children":[{"text":"A quoted remark."}],"type":"blockquote"},{"children":[{"children":[{"text":"Use the new node."}],"type":"code_line"}],"lang":"directive-tip","type":"code_block"},{"children":[{"text":"Closing paragraph."}],"type":"p"}]',
	},
	{
		id: "L49-static-M27-embed-fences",
		content:
			'plate_json::[{"children":[{"children":[{"text":"https://youtube.com/watch?v=dQw4w9WgXcQ"}],"type":"code_line"}],"lang":"embed","type":"code_block"},{"children":[{"children":[{"text":"https://github.com/Rheosoph/flow-like"}],"type":"code_line"}],"lang":"embed","type":"code_block"}]',
	},
	{
		id: "L49-static-M28-map-fence",
		content:
			'plate_json::[{"children":[{"children":[{"text":"lat: 48.1351"}],"type":"code_line"},{"children":[{"text":"lng: 11.5820"}],"type":"code_line"},{"children":[{"text":"label: HQ"}],"type":"code_line"}],"lang":"map","type":"code_block"}]',
	},
];

/** What `TextEditorEditable` emitted on mount for the markdown fixture of the same id. */
export const LEGACY_EDITABLE_ENVELOPES: readonly EnvelopeFixture[] = [
	{
		id: "L49-editable-M03-marks-mdx",
		content:
			'plate_json::[{"children":[{"underline":true,"text":"underline"},{"text":" H"},{"subscript":true,"text":"2"},{"text":"O x"},{"superscript":true,"text":"2"},{"text":" "},{"kbd":true,"text":"Ctrl"},{"text":"+"},{"kbd":true,"text":"K"},{"text":" "},{"highlight":true,"text":"highlight"},{"text":" "},{"mdxJsxTextElement":true,"text":"deleted"}],"type":"p","id":"AG8ti9OTad"},{"children":[{"backgroundColor":"yellow","color":"#ff0000","fontFamily":"monospace","fontSize":"20px","text":"styled span"}],"type":"p","id":"5iLdlL72Y9"}]',
	},
	{
		id: "L49-editable-M04-special-links",
		content:
			'plate_json::[{"children":[{"text":"Open  then ."}],"type":"p","id":"30QaCsi7R8"},{"children":[{"text":"Ping  and <user>sub-bob</user>."}],"type":"p","id":"C0mxlb750N"},{"children":[{"text":"Secret  and ."}],"type":"p","id":"HDG1enfqsa"}]',
	},
	{
		id: "L49-editable-M07-lists",
		content:
			'plate_json::[{"children":[{"text":"alpha"}],"type":"p","indent":1,"listStyleType":"disc","id":"g6XqsxqhCP"},{"children":[{"text":"beta"}],"type":"p","indent":2,"listStyleType":"disc","id":"IfDRgxESEd"},{"children":[{"text":"gamma"}],"type":"p","indent":3,"listStyleType":"disc","id":"jV7PqWHYSD"},{"children":[{"text":"delta"}],"type":"p","indent":1,"listStyleType":"disc","listStart":2,"id":"ryCSY7HeDT"},{"children":[{"text":"three"}],"type":"p","indent":1,"listStyleType":"decimal","id":"-ZiuJNyzqX"},{"children":[{"text":"four"}],"type":"p","indent":1,"listStyleType":"decimal","listStart":2,"id":"PCiSQ-wxsZ"},{"children":[{"text":"nested ordered"}],"type":"p","indent":2,"listStyleType":"decimal","id":"0dwBgYsL_N"},{"children":[{"text":"nested bullet"}],"type":"p","indent":2,"listStyleType":"disc","id":"yCVAJIcJIm"},{"children":[{"text":"loose one"}],"type":"p","indent":1,"listStyleType":"disc","id":"PLKgEYOvVq"},{"children":[{"text":"loose two"}],"type":"p","indent":1,"listStyleType":"disc","listStart":2,"id":"ETxzYuZeG-"},{"children":[{"text":"continuation paragraph"}],"type":"p","indent":2,"id":"OkK7SgTE_P"},{"children":[{"text":"with code"}],"type":"p","indent":1,"listStyleType":"disc","listStart":3,"id":"igKoB21pLO"},{"children":[{"children":[{"text":"const inList = true;"}],"type":"code_line","id":"1i-btTc0IK"}],"lang":"ts","type":"code_block","indent":2,"id":"b_zY6Huotg"},{"children":[{"text":"star bullet"}],"type":"p","indent":1,"listStyleType":"disc","listStart":4,"id":"Nk-D9_M9Bp"},{"children":[{"text":"plus bullet"}],"type":"p","indent":1,"listStyleType":"disc","listStart":5,"id":"ciaoN-BtAd"}]',
	},
	{
		id: "L49-editable-M08-task-lists",
		content:
			'plate_json::[{"children":[{"text":"open task"}],"type":"p","indent":1,"listStyleType":"todo","checked":false,"id":"2DlpIXqSPD"},{"children":[{"text":"done task"}],"type":"p","indent":1,"listStyleType":"todo","checked":true,"listStart":2,"id":"YjbPkRxfFd"},{"children":[{"text":"nested open"}],"type":"p","indent":2,"listStyleType":"todo","checked":false,"id":"PkgmltvRfh"},{"children":[{"text":"nested done uppercase"}],"type":"p","indent":2,"listStyleType":"todo","checked":true,"listStart":2,"id":"Aau2YRwlmA"},{"children":[{"text":"ordered task"}],"type":"p","indent":1,"listStyleType":"todo","checked":false,"listStart":3,"id":"achFVakQBg"}]',
	},
	{
		id: "L49-editable-M09-tables",
		content:
			'plate_json::[{"children":[{"children":[{"children":[{"children":[{"text":"Left"}],"type":"p","id":"CsZgYsyFGx"}],"type":"th","id":"NoNspiwzq-"},{"children":[{"children":[{"text":"Center"}],"type":"p","id":"H9wNKbj8-j"}],"type":"th","id":"-HLHNC_nGG"},{"children":[{"children":[{"text":"Right"}],"type":"p","id":"tJwVQd8eVo"}],"type":"th","id":"IqZObw4HFC"}],"type":"tr","id":"yXginbfWKd"},{"children":[{"children":[{"children":[{"bold":true,"text":"bold"}],"type":"p","id":"PQKmRNqmy-"}],"type":"td","id":"5N8nHYQ747"},{"children":[{"children":[{"code":true,"text":"code"}],"type":"p","id":"FP3wEMSesV"}],"type":"td","id":"jyCYestC3L"},{"children":[{"children":[{"text":""},{"children":[{"text":"link"}],"type":"a","url":"https://example.com"},{"text":""}],"type":"p","id":"FAUQHOMmmx"}],"type":"td","id":"bigtULNxJR"}],"type":"tr","id":"uF4jTS6prf"},{"children":[{"children":[{"children":[{"text":""}],"type":"p","id":"QxYhGThO2-"}],"type":"td","id":"HKeIWvoiZM"},{"children":[{"children":[{"text":"empty-left"}],"type":"p","id":"F_IKIVzggX"}],"type":"td","id":"xC7F0YIRgT"},{"children":[{"children":[{"text":"a | pipe"}],"type":"p","id":"D14Z_nrJst"}],"type":"td","id":"yT8XyiQ_oF"}],"type":"tr","id":"Xz50aynGDj"}],"type":"table","id":"U1qqxnGV80"},{"children":[{"children":[{"children":[{"children":[{"text":"C0"}],"type":"p","id":"51hwd2HNBF"}],"type":"th","id":"--fKXRGG57"},{"children":[{"children":[{"text":"C1"}],"type":"p","id":"S6bgE9CFiI"}],"type":"th","id":"JDUrUltR0m"},{"children":[{"children":[{"text":"C2"}],"type":"p","id":"5tL13cKL_s"}],"type":"th","id":"vpWRcGl4Q_"},{"children":[{"children":[{"text":"C3"}],"type":"p","id":"5aAh5XQzhh"}],"type":"th","id":"hWbjod9TKG"},{"children":[{"children":[{"text":"C4"}],"type":"p","id":"xDTBLZqtUR"}],"type":"th","id":"r_nknWEgGB"},{"children":[{"children":[{"text":"C5"}],"type":"p","id":"LGotYhN078"}],"type":"th","id":"UrsbXmT937"},{"children":[{"children":[{"text":"C6"}],"type":"p","id":"Ht8NCUvmtF"}],"type":"th","id":"IBJPnx9MjB"},{"children":[{"children":[{"text":"C7"}],"type":"p","id":"lPYW4AqqeI"}],"type":"th","id":"AejjBm0L8s"},{"children":[{"children":[{"text":"C8"}],"type":"p","id":"Q5iWs90K0Z"}],"type":"th","id":"-cFUimw-od"},{"children":[{"children":[{"text":"C9"}],"type":"p","id":"SI9GmPwYhs"}],"type":"th","id":"Kp4R1O0yml"}],"type":"tr","id":"yWyHca6dZx"},{"children":[{"children":[{"children":[{"text":"v0"}],"type":"p","id":"Wt6QX7je7n"}],"type":"td","id":"c69WcX_Q3J"},{"children":[{"children":[{"text":"v1"}],"type":"p","id":"By4iQhFTgA"}],"type":"td","id":"LwmQu9EhrE"},{"children":[{"children":[{"text":"v2"}],"type":"p","id":"t9SUdSloQ2"}],"type":"td","id":"kV8w0u4MhK"},{"children":[{"children":[{"text":"v3"}],"type":"p","id":"mS2z65stvq"}],"type":"td","id":"1i5F_OorlB"},{"children":[{"children":[{"text":"v4"}],"type":"p","id":"BCHVNxAS6k"}],"type":"td","id":"Vo0es72G4C"},{"children":[{"children":[{"text":"v5"}],"type":"p","id":"NAMD4j959s"}],"type":"td","id":"TZfbSWQAkj"},{"children":[{"children":[{"text":"v6"}],"type":"p","id":"Wkuqk_rHSl"}],"type":"td","id":"blL30TtC7R"},{"children":[{"children":[{"text":"v7"}],"type":"p","id":"Ug54rkajjn"}],"type":"td","id":"WdA5Vwl4io"},{"children":[{"children":[{"text":"v8"}],"type":"p","id":"rT61aJDBhB"}],"type":"td","id":"mVDx6sM7WX"},{"children":[{"children":[{"text":"v9"}],"type":"p","id":"bynlPwzhNb"}],"type":"td","id":"9ZtldvcFVN"}],"type":"tr","id":"xgsPgAEPQp"}],"type":"table","id":"NZIxLD-ynJ"},{"children":[{"children":[{"children":[{"children":[{"text":"Key"}],"type":"p","id":"19VxYUEM0m"}],"type":"th","id":"hJYrcfCNXt"},{"children":[{"children":[{"text":"Long value"}],"type":"p","id":"9Jj3_es07C"}],"type":"th","id":"BIiCzp57nM"}],"type":"tr","id":"6FN7EIErZp"},{"children":[{"children":[{"children":[{"text":"lorem"}],"type":"p","id":"RWwWImTa3i"}],"type":"td","id":"bHacxtPtqc"},{"children":[{"children":[{"text":"Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Lorem ipsum dolor sit amet, consectetur adipiscing elit."}],"type":"p","id":"g6jdkLz5Jf"}],"type":"td","id":"JFgbuJMvLj"}],"type":"tr","id":"9RMjoM4Civ"}],"type":"table","id":"7o1v51e5a6"},{"children":[{"text":""}],"type":"p","id":"55axHDcgyb"}]',
	},
	{
		id: "L49-editable-M11-math",
		content:
			'plate_json::[{"children":[{"text":"Inline "},{"children":[{"text":""}],"texExpression":"E = mc^2","type":"inline_equation"},{"text":" inside prose."}],"type":"p","id":"eE541RXr6E"},{"children":[{"text":""}],"texExpression":"\\\\int_0^1 x^2 \\\\, dx = \\\\frac{1}{3}","type":"equation","id":"Z-_XuO16CW"},{"children":[{"text":"It costs $5 today and $10 tomorrow."}],"type":"p","id":"6TkwjMiyuo"},{"children":[{"children":[{"text":"a^2 + b^2 = c^2"}],"type":"code_line","id":"y-vhwcbqu_"}],"lang":"math","type":"code_block","id":"Q9MGTwgdGB"},{"children":[{"text":""}],"type":"p","id":"gozuYNkXyr"}]',
		usesKatex: true,
	},
	{
		id: "L49-editable-M12-images",
		content:
			'plate_json::[{"caption":[{"text":"Alt text"}],"children":[{"text":""}],"type":"img","url":"https://example.com/a.png","id":"SRQD1TnU-J"},{"caption":[{"text":"Titled"}],"children":[{"text":""}],"type":"img","url":"https://example.com/b.png","id":"CkFbV7E6M6"},{"caption":[{"text":""}],"children":[{"text":""}],"type":"img","url":"https://example.com/no-alt.png","id":"PVH8DsOAg9"},{"children":[{"text":""},{"children":[{"caption":[{"text":"linked"}],"children":[{"text":""}],"type":"img","url":"https://example.com/c.png"}],"type":"a","url":"https://example.com/target"},{"text":""}],"type":"p","id":"hajNos52s7"},{"caption":[{"text":"stored"}],"children":[{"text":""}],"type":"img","url":"storage://apps/app-1/upload/d.png","id":"IV2Yi7P7V5"},{"children":[{"text":"Inline "}],"type":"p","id":"ua93d4UnCE"},{"caption":[{"text":"icon"}],"children":[{"text":""}],"type":"img","url":"https://example.com/icon.svg","id":"XJziFk1rlf"},{"children":[{"text":" inside text."}],"type":"p","id":"a__43h8H-5"}]',
	},
	{
		id: "L49-editable-M14-mdx-blocks",
		content:
			'plate_json::[{"children":[{"text":"Doc with toc"}],"type":"h1","id":"fvkQYeNxsL"},{"children":[{"text":""}],"type":"toc","id":"O0ma61MwQ9"},{"children":[{"text":"Section A"}],"type":"h2","id":"gmBRT18NVa"},{"children":[{"children":[{"text":"Callout body with "},{"bold":true,"text":"bold"}],"type":"callout","id":"8bsROcqh3I"}],"type":"p","id":"Bw6_5qE_e4"},{"children":[{"text":"Due "},{"children":[{"text":""}],"date":"2020-01-15","type":"date"},{"text":" sharp."}],"type":"p","id":"zcCgj7yckd"},{"children":[{"children":[{"children":[{"text":"Left column"}],"type":"column","width":"50%","id":"_-Ty5ryvtc"},{"children":[{"text":"Right column"}],"type":"column","width":"50%","id":"O59fZrRZm4"}],"type":"column_group","id":"TDIv9fjNkE"}],"type":"p","id":"aoo79r5FhQ"}]',
	},
	{
		id: "L49-editable-M15-directives",
		content:
			'plate_json::[{"children":[{"children":[{"text":"Informational `code` text."}],"type":"code_line","id":"x6Imjp8VDP"}],"lang":"directive-info","type":"code_block","id":"3MTiv7OCmS"},{"children":[{"children":[{"text":"Rate Limiting"}],"type":"code_line","id":"aW60qEUfxQ"},{"children":[{"text":"---"}],"type":"code_line","id":"rodMIjUnnQ"},{"children":[{"text":"Approaching the limit."}],"type":"code_line","id":"tTPA1mgfq_"}],"lang":"directive-warning","type":"code_block","id":"A9ogJWJWZS"},{"children":[{"children":[{"text":"Something failed."}],"type":"code_line","id":"FkXhl8QK4c"}],"lang":"directive-error","type":"code_block","id":"vaxPWX1UjK"},{"children":[{"children":[{"text":"All good."}],"type":"code_line","id":"oPO2WTrgxc"}],"lang":"directive-success","type":"code_block","id":"GmS3VlJQCX"},{"children":[{"children":[{"text":"Try this."}],"type":"code_line","id":"CaQ-0aVl3e"}],"lang":"directive-tip","type":"code_block","id":"pOot7kxeXY"},{"children":[{"children":[{"text":"Stack trace"}],"type":"code_line","id":"ubPgi22too"},{"children":[{"text":"---"}],"type":"code_line","id":"pg6LNumwil"},{"children":[{"text":"```"}],"type":"code_line","id":"jbvqzA9P50"},{"children":[{"text":"Error: boom"}],"type":"code_line","id":"5yFDfWhGQY"},{"children":[{"text":"    at x (y.js:1:1)"}],"type":"code_line","id":"JnD4sDaXF9"},{"children":[{"text":"```"}],"type":"code_line","id":"D3Aq2fYG1A"}],"lang":"directive-spoiler","type":"code_block","id":"hyiRAp4MeP"},{"children":[{"text":":::info\\nunterminated directive stays literal"}],"type":"p","id":"Fh1jqteLjY"}]',
	},
	{
		id: "L49-editable-M26-mixed-report",
		content:
			'plate_json::[{"children":[{"text":"Release notes"}],"type":"h1","id":"SCSQgol-WV"},{"children":[{"text":"A paragraph with "},{"code":true,"text":"inline code"},{"text":", "},{"bold":true,"text":"bold"},{"text":" and a "},{"children":[{"text":"link"}],"type":"a","url":"https://example.com/docs"},{"text":"."}],"type":"p","id":"5n8WALelBb"},{"children":[{"text":"Changes"}],"type":"h2","id":"_WWKItGhbM"},{"children":[{"text":"top level"}],"type":"p","indent":1,"listStyleType":"disc","id":"HajNvEZMXT"},{"children":[{"text":"nested one"}],"type":"p","indent":2,"listStyleType":"disc","id":"iv3MluKr-G"},{"children":[{"text":"another"}],"type":"p","indent":1,"listStyleType":"disc","listStart":2,"id":"FERWbTSf41"},{"children":[{"text":"first step"}],"type":"p","indent":1,"listStyleType":"decimal","id":"r0pRmd94HW"},{"children":[{"text":"second step"}],"type":"p","indent":1,"listStyleType":"decimal","listStart":2,"id":"CpktR5auYj"},{"children":[{"children":[{"text":"export const answer = 42;"}],"type":"code_line","id":"X3Hh7KNURX"}],"lang":"ts","type":"code_block","id":"7-Z9S9cEZ9"},{"children":[{"children":[{"children":[{"children":[{"text":"Feature"}],"type":"p","id":"-fs_RtJSs-"}],"type":"th","id":"fXQI_mKLfy"},{"children":[{"children":[{"text":"Status"}],"type":"p","id":"4dQuEHbNNt"}],"type":"th","id":"mqz5Q1Fkgn"}],"type":"tr","id":"QhlvRQF4vW"},{"children":[{"children":[{"children":[{"text":"Streaming"}],"type":"p","id":"TQgmtzA4Mu"}],"type":"td","id":"_vOTZ6KZEQ"},{"children":[{"children":[{"text":"done"}],"type":"p","id":"fDWDVe4T1Y"}],"type":"td","id":"-62PG3PqhL"}],"type":"tr","id":"X-J3ilF6ys"}],"type":"table","id":"QRoF5YCg2C"},{"children":[{"text":"A quoted remark."}],"type":"blockquote","id":"OuddVy8Lj9"},{"children":[{"children":[{"text":"Use the new node."}],"type":"code_line","id":"O2Jjbse3F5"}],"lang":"directive-tip","type":"code_block","id":"ctHqPDHNpI"},{"children":[{"text":"Closing paragraph."}],"type":"p","id":"nkRCb_N65B"}]',
	},
];

/** Comment bodies from the website hero board (apps/website/src/assets/board.json). */
export const PRODUCTION_ENVELOPES: readonly EnvelopeFixture[] = [
	{
		id: "R01-cost-table-merged",
		content:
			'plate_json::[{"children":[{"text":"What does all of this cost?","bold":true,"fontSize":"32px"}],"type":"p","id":"LFiz4a0DNn"},{"children":[{"children":[{"children":[{"children":[{"text":"Personal","bold":true}],"type":"p","id":"VEpVOapgHs"}],"type":"td","id":"h-ySC6Gofb"},{"children":[{"children":[{"text":"Commercial","bold":true}],"type":"p","id":"ejnEJBPcEa"}],"type":"td","id":"aC2-qhZQXc"},{"children":[{"children":[{"text":"Enterprise","bold":true}],"type":"p","id":"syA4dYQpey"}],"type":"td","id":"A869LKD9bw"}],"type":"tr","id":"e44F5MxvYu"},{"children":[{"children":[{"children":[{"text":"Free [Offline & Self Hosted]"}],"type":"p","id":"1S0U4N39uw"}],"type":"td","id":"m_yhXLM302"},{"children":[{"children":[{"text":"Free [Offline & Self Hosted]"}],"type":"p","id":"DP93R5RRBS"}],"type":"td","id":"sF0ycLjKD0"},{"children":[{"children":[{"text":"Contact Us"}],"type":"p","id":"ufp6Qpqs3k"}],"type":"td","id":"-IYNSJ6Dig"}],"type":"tr","id":"h6kzeZtfXs","size":48},{"children":[{"children":[{"children":[{"text":"Premium and Local Models"}],"type":"p","id":"9ff7ULxX9p"}],"type":"td","id":"bP1ZFkl0tQ"},{"children":[{"children":[{"text":"Premium and Local Models"}],"type":"p","id":"K9LMch1jwy"}],"type":"td","id":"YTCI5utX3D"},{"children":[{"children":[{"text":"Bring your own Models"}],"type":"p","id":"_8T2yKDYaF"}],"type":"td","id":"36eSYT4A1C"}],"type":"tr","id":"1HGO1PEgrj"},{"children":[{"children":[{"children":[{"text":"Community Support"}],"type":"p","id":"CCCDwqV_NJ"}],"type":"td","id":"XZJBPHMEIU"},{"children":[{"children":[{"text":"Community or Premium Support"}],"type":"p","id":"vfQH-AbgaD"}],"type":"td","id":"wX1a8zQycv"},{"children":[{"children":[{"text":"Premium Support"}],"type":"p","id":"5YcWNgLP3X"}],"type":"td","id":"6Ud1VzV6dg"}],"type":"tr","id":"x6Wa1WhYwr"},{"children":[{"children":[{"children":[{"text":"Unlimited Local Workflows"}],"type":"p","id":"EzA-Y8myyZ"}],"type":"td","id":"icHHI2Nli1"},{"children":[{"children":[{"text":"Unlimited Local Workflows"}],"type":"p","id":"jM7O31iILZ"}],"type":"td","id":"zIwgLh0PfQ"},{"children":[{"children":[{"text":"Unlimited Workflows"}],"type":"p","id":"hGQyGKCKLK"}],"type":"td","id":"UJxDvVFe0s"}],"type":"tr","id":"7xFGw3b5M8"},{"children":[{"children":[{"children":[{"text":"Optional Online Workflows"}],"type":"p","id":"p8pQ4V3YZ5"}],"type":"td","colSpan":1,"rowSpan":1,"id":"vapdvy6ir2"},{"children":[{"children":[{"text":"Optional Online Workflows"}],"type":"p","id":"xMWwAQ8FnH"}],"type":"td","colSpan":1,"rowSpan":1,"id":"R5103Wwfmv"},{"children":[{"children":[{"text":"Unlimited Online Workflows"}],"type":"p","id":"FTdEXWMEB1"}],"type":"td","colSpan":1,"rowSpan":1,"id":"_IYIx8pqac"}],"type":"tr","id":"CvkdvBPfrW"}],"type":"table","id":"1y7onA1B70"},{"children":[{"text":""}],"type":"p","id":"OP3mS_ab_B"}]',
	},
	{
		id: "R02-columns-image-align",
		content:
			'plate_json::[{"type":"p","id":"lMgKd4JjbO","children":[{"text":""}]},{"children":[{"children":[{"children":[{"text":""}],"type":"p","id":"GHr7hZGZi3"},{"children":[{"text":""}],"type":"img","url":"https://upload.wikimedia.org/wikipedia/commons/thumb/d/d5/Rust_programming_language_black_logo.svg/1200px-Rust_programming_language_black_logo.svg.png","id":"tCAL1a_1Ct","width":137}],"type":"column","width":"30%","id":"hdbxH2VdEa"},{"children":[{"type":"h3","children":[{"text":"Open Source - Written in Rust","color":"#000000"}],"id":"61nssgtOGG","align":"left"},{"children":[{"text":"Rust is designed with a focus on memory safety and performance, making it ideal for production-critical applications. Unlike languages like C and C++, Rust prevents common memory errors like dangling pointers and data races at compile time, eliminating a significant source of bugs and vulnerabilities. Its ownership system ensures that each piece of data has a single owner, preventing multiple parts of the code from trying to modify the same memory simultaneously.","color":"#000000"},{"text":"\\n"}],"type":"p","id":"dMXjzqhNCn","align":"left"}],"type":"column","width":"70%","id":"30AebTlHKI"}],"type":"column_group","id":"L9c33FaRoo"},{"children":[{"text":""}],"type":"p","id":"eHZux6fB2j"}]',
	},
	{
		id: "R03-columns-lists-links",
		content:
			'plate_json::[{"children":[{"text":"Flow Like","fontSize":"50px","bold":true}],"type":"p","id":"TTOA87Jwg6"},{"type":"p","id":"7uN6RoHHtk","children":[{"text":"The Production-Ready Automation Suite for Enterprise Scale Workflows"}]},{"type":"p","id":"ty3hWsueH6","children":[{"text":""}]},{"type":"p","id":"lMgKd4JjbO","children":[{"text":""}]},{"children":[{"children":[{"children":[{"text":""}],"type":"p","id":"GHr7hZGZi3"}],"type":"column","width":"33.333333333333336%","id":"hdbxH2VdEa"},{"children":[{"children":[{"text":""}],"type":"p","id":"4e-gFwWlDk"}],"type":"column","width":"33.333333333333336%","id":"30AebTlHKI"},{"children":[{"type":"h2","id":"PFCE1KtgCU","children":[{"text":"Links"}]},{"type":"p","id":"nEJVBZtDMV","children":[{"text":"","bold":true},{"children":[{"text":"Github","bold":true}],"type":"a","url":"https://github.com/Rheosoph/flow-like","id":"dt6b4gI53i"},{"text":"","bold":true}],"indent":1,"listStyleType":"disc"},{"type":"p","indent":1,"listStyleType":"disc","children":[{"text":"","bold":true},{"children":[{"text":"Documentation","bold":true}],"type":"a","url":"https://docs.flow-like.com","id":"oKQAFpI8_n"},{"text":"","bold":true}],"listStart":2,"id":"61nssgtOGG"},{"type":"p","indent":1,"listStyleType":"disc","listStart":3,"id":"SqFNWb0lwH","children":[{"text":""},{"children":[{"bold":true,"text":"Discord"}],"type":"a","url":"https://discord.gg/mdBA9kMjFJ","id":"ns8sPm79BU"},{"text":""}]}],"type":"column","width":"33.333333333333336%","id":"4zQ7lb1iNA"}],"type":"column_group","id":"L9c33FaRoo"},{"children":[{"text":""}],"type":"p","id":"eHZux6fB2j"}]',
	},
];

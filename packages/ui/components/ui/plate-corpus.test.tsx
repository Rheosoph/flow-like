import { beforeAll, describe, expect, setDefaultTimeout, test } from "bun:test";
import { MarkdownPlugin } from "@platejs/markdown";
import { type Value, createSlateEditor } from "platejs";
import { PlateStatic, serializeHtml } from "platejs/static";
import { type ReactElement, createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
	MALFORMED_ENVELOPES,
	MARKDOWN_FIXTURES,
	STORED_FIXTURES,
	STREAMING_PREFIX_FIXTURES,
	toEnvelope,
} from "../editor/__fixtures__/plate-corpus";
import {
	LEGACY_EDITABLE_ENVELOPES,
	LEGACY_STATIC_ENVELOPES,
	PRODUCTION_ENVELOPES,
} from "../editor/__fixtures__/plate-legacy-49";
import {
	type Rendered,
	captureRender,
	countElements,
	describeError,
	escapeInvisible,
	findUnsafeMarkup,
	fingerprint,
	formatHtml,
	formatRendered,
	textOf,
} from "../editor/__fixtures__/plate-snapshot";
import { BaseEditorKit } from "../editor/editor-base-kit";
import { EditorStatic } from "../editor/ui/editor-static";
import { WINDOWING_BLOCK_THRESHOLD } from "./lazy-plate-static";
import {
	EMPTY_STREAMING_STATE,
	type StreamingParseState,
	parseStreamingMarkdown,
} from "./streaming-markdown-blocks";
import { StreamingTextEditor } from "./streaming-text-editor";
import {
	PLATE_JSON_PREFIX,
	PROSE_WRAPPER_CLASSNAME,
	RICH_REMARK_PLUGINS,
	STATIC_EDITOR_CLASSNAME,
	TextEditor,
	getStaticParseWorker,
	safeDeserialize,
	transformSpecialLinks,
} from "./text-editor";

// DateElementStatic formats in local time.
process.env.TZ = "UTC";

// A loaded machine pushes the heavier renders past bun's 5 s default.
setDefaultTimeout(60_000);

const worker = getStaticParseWorker(BaseEditorKit);

/** Parse fallbacks, lowlight and the markdown serializer log expected noise. */
function quietly<T>(run: () => T): T {
	const { error, warn, log } = console;
	const mute = () => {};
	Object.assign(console, { error: mute, warn: mute, log: mute });
	try {
		return run();
	} finally {
		Object.assign(console, { error, warn, log });
	}
}

const parse = (markdown: string): Value =>
	quietly(() => safeDeserialize(worker, markdown, true, RICH_REMARK_PLUGINS));

const parseEnvelope = (content: string): Value =>
	JSON.parse(content.slice(PLATE_JSON_PREFIX.length));

const markdownFixture = (id: string) => {
	const fixture = MARKDOWN_FIXTURES.find((candidate) => candidate.id === id);
	if (!fixture) throw new Error(`no markdown fixture ${id}`);
	return fixture.markdown;
};

const storedFixture = (id: string) => {
	const fixture = STORED_FIXTURES.find((candidate) => candidate.id === id);
	if (!fixture) throw new Error(`no stored fixture ${id}`);
	return structuredClone(fixture.nodes) as Value;
};

const ssr = (build: () => ReactElement): Rendered =>
	captureRender(() => quietly(() => renderToStaticMarkup(build())));

const html = (rendered: Rendered) => rendered.html ?? "";

type TextEditorMode = {
	readonly isMarkdown: boolean;
	readonly minimal?: boolean;
};

const renderTextEditor = (initialContent: string, mode: TextEditorMode) =>
	ssr(() => createElement(TextEditor, { initialContent, ...mode }));

const renderStreaming = (content: string) =>
	ssr(() => createElement(StreamingTextEditor, { content }));

/** `TextEditorStatic` rebuilt from stock Plate parts: no parse caches, no path index. */
const renderStock = (value: Value) =>
	ssr(() =>
		createElement(
			"div",
			{ className: PROSE_WRAPPER_CLASSNAME },
			createElement(PlateStatic, {
				editor: createSlateEditor({
					plugins: BaseEditorKit,
					value: structuredClone(value),
					nodeId: false,
				}),
				className: STATIC_EDITOR_CLASSNAME,
			}),
		),
	);

function serializeMarkdown(value: Value): string {
	try {
		return quietly(() =>
			worker
				.getApi(MarkdownPlugin)
				.markdown.serialize({ value: structuredClone(value) }),
		);
	} catch (error) {
		return `THROWS ${describeError(error)}`;
	}
}

function markdownRoundTrip(value: Value) {
	const markdown = serializeMarkdown(value);
	if (markdown.startsWith("THROWS ")) return { markdown };
	return {
		markdown,
		reparsesToSameValue: Bun.deepEquals(parse(markdown), value),
	};
}

const cutPoints = (length: number) =>
	[
		...new Set([Math.floor(length / 3), Math.floor((2 * length) / 3), length]),
	].filter((cut) => cut > 0);

/**
 * Feeds the document in three chunks with parse state carried forward, the way
 * a chat reply arrives, and compares each frame with a cold settled render.
 */
function streamThroughCuts(markdown: string) {
	let state: StreamingParseState = EMPTY_STREAMING_STATE;
	return cutPoints(markdown.length).map((cut) => {
		const prefix = markdown.slice(0, cut);
		state = quietly(() => parseStreamingMarkdown(worker, prefix, state));
		const whole = parse(prefix);
		const streamed = renderStreaming(prefix);
		const settled = renderStock(whole);
		const renderMatchesSettled =
			formatRendered(streamed) === formatRendered(settled);
		return {
			cut,
			parseMatchesWholeDocument: Bun.deepEquals(state.blocks, whole),
			renderMatchesSettled,
			...(renderMatchesSettled
				? {}
				: {
						streamedText: escapeInvisible(textOf(html(streamed))),
						settledText: escapeInvisible(textOf(html(settled))),
					}),
		};
	});
}

function divergentPrefixes(markdown: string): number[] {
	let state: StreamingParseState = EMPTY_STREAMING_STATE;
	const divergent: number[] = [];
	for (let length = 1; length <= markdown.length; length++) {
		const prefix = markdown.slice(0, length);
		state = quietly(() => parseStreamingMarkdown(worker, prefix, state));
		if (!Bun.deepEquals(state.blocks, parse(prefix))) divergent.push(length);
	}
	return divergent;
}

const EXPORT_PROPS = {
	style: { padding: "0 calc(50% - 350px)", paddingBottom: "" },
};

/** The export toolbar's "Export as HTML". */
async function exportHtml(value: Value) {
	const editor = createSlateEditor({
		plugins: BaseEditorKit,
		value: structuredClone(value),
		// Plate only skips node ids by default under NODE_ENV=test.
		nodeId: true,
	});
	const { error, warn } = console;
	Object.assign(console, { error: () => {}, warn: () => {} });
	try {
		return await serializeHtml(editor, {
			editorComponent: EditorStatic,
			props: EXPORT_PROPS,
		});
	} finally {
		Object.assign(console, { error, warn });
	}
}

/**
 * Directive, embed and map fences render through `React.lazy`. Server renders
 * show the Suspense fallback until the chunk has loaded once, so resolve every
 * lazy renderer before the first snapshot or the output depends on test order.
 */
beforeAll(async () => {
	let previous = "";
	for (let round = 0; round < 8; round++) {
		const current = MARKDOWN_FIXTURES.map((fixture) =>
			formatRendered(renderTextEditor(fixture.markdown, { isMarkdown: true })),
		).join("\n");
		if (current === previous) return;
		previous = current;
		await new Promise((resolve) => setTimeout(resolve, 25));
	}
	throw new Error("lazy code-block renderers did not settle");
}, 60_000);

/** Plate 49 drops what an MDX fallback swallows; the stream still shows it. */
const KNOWN_SETTLE_LOSS = new Set(["M02-marks-markdown", "M21-mdx-breaking"]);

describe("markdown fixtures", () => {
	for (const fixture of MARKDOWN_FIXTURES) {
		const { markdown } = fixture;

		describe(fixture.id, () => {
			test("parses to the pinned Plate value", () => {
				expect(parse(markdown)).toMatchSnapshot();
			});

			test("renders through TextEditor", () => {
				expect(
					formatRendered(renderTextEditor(markdown, { isMarkdown: true })),
				).toMatchSnapshot();
			});

			test("renders through the minimal kit", () => {
				expect(
					fingerprint(
						renderTextEditor(markdown, { isMarkdown: true, minimal: true }),
					),
				).toMatchSnapshot();
			});

			test("TextEditor matches stock PlateStatic", () => {
				const value = parse(markdown);
				const rendered = renderTextEditor(markdown, { isMarkdown: true });
				if (value.length > WINDOWING_BLOCK_THRESHOLD) {
					expect(html(rendered)).toContain("data-slate-placeholder");
					return;
				}
				expect(formatRendered(rendered)).toBe(
					formatRendered(renderStock(value)),
				);
			});

			test("its persisted envelope renders like the markdown", () => {
				expect(
					formatRendered(
						renderTextEditor(toEnvelope(parse(markdown)), {
							isMarkdown: false,
						}),
					),
				).toBe(
					formatRendered(renderTextEditor(markdown, { isMarkdown: true })),
				);
			});

			if (fixture.malicious) {
				test("renders nothing executable", () => {
					for (const rendered of [
						renderTextEditor(markdown, { isMarkdown: true }),
						renderTextEditor(markdown, { isMarkdown: true, minimal: true }),
						renderStreaming(markdown),
					]) {
						expect(findUnsafeMarkup(html(rendered))).toEqual([]);
					}
				});
			}

			test("streams towards the settled render", () => {
				expect(streamThroughCuts(markdown)).toMatchSnapshot();
			});

			(KNOWN_SETTLE_LOSS.has(fixture.id) ? test.failing : test)(
				"the finished stream equals the settled render",
				() => {
					expect(formatRendered(renderStreaming(markdown))).toBe(
						formatRendered(renderStock(parse(markdown))),
					);
				},
			);

			test("serializes back to markdown", () => {
				expect(markdownRoundTrip(parse(markdown))).toMatchSnapshot();
			});
		});
	}
});

/**
 * A stored column width reaches the inline style unchecked (`width:expression(…)`).
 * Inert outside legacy IE, but it is CSS injection from document data.
 */
const KNOWN_UNSAFE_STORED: Readonly<Record<string, readonly string[]>> = {
	"S09-malicious-stored": ["style:div"],
};

describe("stored Plate JSON fixtures", () => {
	for (const fixture of STORED_FIXTURES) {
		const envelope = toEnvelope(fixture.nodes);

		describe(fixture.id, () => {
			test("renders through the RichText read-only path", () => {
				expect(
					formatRendered(renderTextEditor(envelope, { isMarkdown: false })),
				).toMatchSnapshot();
			});

			test("the markdown path reads the envelope the same way", () => {
				expect(
					formatRendered(renderTextEditor(envelope, { isMarkdown: true })),
				).toBe(
					formatRendered(renderTextEditor(envelope, { isMarkdown: false })),
				);
			});

			if (fixture.nodes.length > 0) {
				test("TextEditor matches stock PlateStatic", () => {
					const value = transformSpecialLinks(
						structuredClone(fixture.nodes) as Parameters<
							typeof transformSpecialLinks
						>[0],
					) as unknown as Value;
					expect(
						formatRendered(renderTextEditor(envelope, { isMarkdown: false })),
					).toBe(formatRendered(renderStock(value)));
				});
			}

			test("renders through the minimal kit (public store page)", () => {
				expect(
					fingerprint(
						renderTextEditor(envelope, { isMarkdown: false, minimal: true }),
					),
				).toMatchSnapshot();
			});

			test("exports to markdown", () => {
				expect(
					serializeMarkdown(structuredClone(fixture.nodes) as Value),
				).toMatchSnapshot();
			});

			if (fixture.malicious && !fixture.usesKatex) {
				test("renders nothing executable", () => {
					const rich = renderTextEditor(envelope, { isMarkdown: false });
					const minimal = renderTextEditor(envelope, {
						isMarkdown: false,
						minimal: true,
					});
					expect(rich.error).toBeUndefined();
					expect(minimal.error).toBeUndefined();
					expect(findUnsafeMarkup(html(rich))).toEqual([
						...(KNOWN_UNSAFE_STORED[fixture.id] ?? []),
					]);
					expect(findUnsafeMarkup(html(minimal))).toEqual([]);
				});
			}
		});
	}
});

describe("envelopes persisted by Plate 49", () => {
	for (const envelope of [
		...LEGACY_STATIC_ENVELOPES,
		...LEGACY_EDITABLE_ENVELOPES,
	]) {
		test(`${envelope.id} keeps its text and structure`, () => {
			const rendered = renderTextEditor(envelope.content, {
				isMarkdown: false,
			});
			expect(fingerprint(rendered)).toMatchSnapshot();
			if (envelope.usesKatex) return;
			expect(rendered.error).toBeUndefined();
			expect(textOf(html(rendered))).not.toContain(PLATE_JSON_PREFIX);
			expect(
				formatRendered(
					renderTextEditor(envelope.content, { isMarkdown: true }),
				),
			).toBe(formatRendered(rendered));
		});
	}

	for (const envelope of LEGACY_EDITABLE_ENVELOPES) {
		test(`${envelope.id} exports to markdown`, () => {
			expect(
				serializeMarkdown(parseEnvelope(envelope.content)),
			).toMatchSnapshot();
		});
	}

	for (const envelope of PRODUCTION_ENVELOPES) {
		describe(envelope.id, () => {
			test("renders on the website hero board", () => {
				const rendered = renderTextEditor(envelope.content, {
					isMarkdown: true,
				});
				expect(rendered.error).toBeUndefined();
				expect(formatRendered(rendered)).toMatchSnapshot();
			});

			test("exports to markdown", () => {
				expect(
					serializeMarkdown(parseEnvelope(envelope.content)),
				).toMatchSnapshot();
			});
		});
	}
});

describe("malformed envelopes", () => {
	for (const envelope of MALFORMED_ENVELOPES) {
		test(envelope.id, () => {
			const read = (isMarkdown: boolean) => {
				const rendered = renderTextEditor(envelope.content, { isMarkdown });
				return rendered.error === undefined
					? escapeInvisible(textOf(rendered.html))
					: `THROWS ${rendered.error}`;
			};
			expect({
				markdownPath: read(true),
				richTextPath: read(false),
			}).toMatchSnapshot();
		});
	}
});

describe("streaming prefixes", () => {
	for (const fixture of STREAMING_PREFIX_FIXTURES) {
		const { content } = fixture;

		describe(fixture.id, () => {
			test("renders the unfinished construct", () => {
				expect(formatRendered(renderStreaming(content))).toMatchSnapshot();
			});

			test("equals the settled render", () => {
				expect(formatRendered(renderStreaming(content))).toBe(
					formatRendered(renderStock(parse(content))),
				);
			});

			if (!fixture.usesKatex) {
				test("every character prefix renders", () => {
					for (let length = 1; length <= content.length; length++) {
						const prefix = content.slice(0, length);
						expect({ prefix, error: renderStreaming(prefix).error }).toEqual({
							prefix,
							error: undefined,
						});
					}
				});
			}

			test("carried parse state equals a whole-document parse at every prefix", () => {
				expect(divergentPrefixes(content)).toEqual([]);
			});
		});
	}

	for (const id of [
		"M03-marks-mdx",
		"M04-special-links",
		"M08-task-lists",
		"M14-mdx-blocks",
		"M15-directives",
		"M18-mentions",
	]) {
		test(`${id}: carried parse state equals a whole-document parse at every prefix`, () => {
			expect(divergentPrefixes(markdownFixture(id))).toMatchSnapshot();
		});
	}
});

describe("HTML export", () => {
	const cases: ReadonlyArray<readonly [string, () => Value]> = [
		["M26-mixed-report", () => parse(markdownFixture("M26-mixed-report"))],
		["S05-marks-and-styles", () => storedFixture("S05-marks-and-styles")],
		["S07-table-merged", () => storedFixture("S07-table-merged")],
		["S12-indent-lists", () => storedFixture("S12-indent-lists")],
		["S14-links", () => storedFixture("S14-links")],
		[
			PRODUCTION_ENVELOPES[0].id,
			() => parseEnvelope(PRODUCTION_ENVELOPES[0].content),
		],
	];

	for (const [id, value] of cases) {
		test(id, async () => {
			expect(
				formatHtml(await exportHtml(value()), {
					volatileAttributes: ["data-block-id", "data-slate-id"],
				}),
			).toMatchSnapshot();
		});
	}
});

describe("Plate 49 defects pinned as expected failures", () => {
	test.failing(
		"an <img> tag in markdown renders instead of crashing the document",
		() => {
			expect(
				renderTextEditor(markdownFixture("X03-event-handlers-mdx"), {
					isMarkdown: true,
				}).error,
			).toBeUndefined();
		},
	);

	test.failing(
		"a table cell outside a table renders instead of crashing the document",
		() => {
			expect(
				renderTextEditor(toEnvelope(storedFixture("S11-orphan-table-cell")), {
					isMarkdown: false,
				}).error,
			).toBeUndefined();
		},
	);

	test.failing(
		"an empty stored document renders empty instead of the raw envelope",
		() => {
			expect(
				textOf(html(renderTextEditor("plate_json::[]", { isMarkdown: false }))),
			).not.toContain(PLATE_JSON_PREFIX);
		},
	);

	test.failing("KaTeX renders without a DOM (server rendering)", () => {
		expect(
			renderTextEditor(markdownFixture("M11-math"), { isMarkdown: true }).error,
		).toBeUndefined();
	});

	test.failing("reference links keep their text", () => {
		expect(
			textOf(
				html(
					renderTextEditor(markdownFixture("M19-references-footnotes"), {
						isMarkdown: true,
					}),
				),
			),
		).toContain("the docs");
	});

	test.failing("HTML export keeps markup typed as text escaped", async () => {
		const exported = await exportHtml([
			{
				type: "p",
				children: [
					{
						text: "<script>window.__xss=1</script><img src=x onerror=alert(1)>",
					},
				],
			},
		]);
		expect(findUnsafeMarkup(exported)).toEqual([]);
	});

	test.failing("markdown h4-h6 render as heading elements", () => {
		const rendered = html(
			renderTextEditor(markdownFixture("M01-headings"), { isMarkdown: true }),
		);
		expect(
			countElements(rendered, (tag) => ["h4", "h5", "h6"].includes(tag)),
		).toBe(3);
	});
});

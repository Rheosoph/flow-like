import {
	MALFORMED_ENVELOPES,
	MARKDOWN_FIXTURES,
	STORED_FIXTURES,
	STREAMING_PREFIX_FIXTURES,
	toEnvelope,
} from "../../components/editor/__fixtures__/plate-corpus";
import {
	LEGACY_EDITABLE_ENVELOPES,
	LEGACY_STATIC_ENVELOPES,
	PRODUCTION_ENVELOPES,
} from "../../components/editor/__fixtures__/plate-legacy-49";

export const THEMES = ["light", "dark"] as const;
export type Theme = (typeof THEMES)[number];

export const SURFACES = [
	"static",
	"minimal",
	"stream",
	"stream-partial",
	"raw",
	"editable",
	"editable-selection",
	"editable-slash",
] as const;
export type Surface = (typeof SURFACES)[number];

export type VisualCase = {
	readonly key: string;
	readonly surface: Surface;
	readonly id: string;
	readonly content: string;
	readonly isMarkdown: boolean;
};

export type EditorVisualApi = {
	readonly keys: readonly string[];
	render: (key: string, theme: Theme) => Promise<void>;
	renderError: () => string | null;
	/** Asynchronous rendering the DOM does not show, such as maps still loading. */
	pendingWork: () => number;
};

const visualCase = (
	surface: Surface,
	id: string,
	content: string,
	isMarkdown: boolean,
): VisualCase => ({
	key: `${surface}/${id}`,
	surface,
	id,
	content,
	isMarkdown,
});

const benignMarkdown = MARKDOWN_FIXTURES.filter(
	(fixture) => !fixture.malicious,
);
const benignStored = STORED_FIXTURES.filter((fixture) => !fixture.malicious);
const storedEnvelope = (id: string) => {
	const fixture = STORED_FIXTURES.find((candidate) => candidate.id === id);
	if (!fixture) throw new Error(`editor-visual: no stored fixture ${id}`);
	return toEnvelope(fixture.nodes);
};
const markdown = (id: string) => {
	const fixture = MARKDOWN_FIXTURES.find((candidate) => candidate.id === id);
	if (!fixture) throw new Error(`editor-visual: no markdown fixture ${id}`);
	return fixture.markdown;
};

export const VISUAL_CASES: readonly VisualCase[] = [
	...MARKDOWN_FIXTURES.map((fixture) =>
		visualCase("static", fixture.id, fixture.markdown, true),
	),
	...MALFORMED_ENVELOPES.map((fixture) =>
		visualCase("static", fixture.id, fixture.content, true),
	),
	...benignMarkdown.map((fixture) =>
		visualCase("minimal", fixture.id, fixture.markdown, true),
	),
	...MARKDOWN_FIXTURES.map((fixture) =>
		visualCase("stream", fixture.id, fixture.markdown, true),
	),
	...STREAMING_PREFIX_FIXTURES.map((fixture) =>
		visualCase("stream-partial", fixture.id, fixture.content, true),
	),
	...STORED_FIXTURES.map((fixture) =>
		visualCase("raw", fixture.id, toEnvelope(fixture.nodes), false),
	),
	...MALFORMED_ENVELOPES.map((fixture) =>
		visualCase("raw", fixture.id, fixture.content, false),
	),
	...[
		...LEGACY_STATIC_ENVELOPES,
		...LEGACY_EDITABLE_ENVELOPES,
		...PRODUCTION_ENVELOPES,
	].map((fixture) => visualCase("raw", fixture.id, fixture.content, false)),
	...benignMarkdown.map((fixture) =>
		visualCase("editable", fixture.id, fixture.markdown, true),
	),
	...benignStored.map((fixture) =>
		visualCase("editable", fixture.id, toEnvelope(fixture.nodes), false),
	),
	...[...LEGACY_EDITABLE_ENVELOPES, ...PRODUCTION_ENVELOPES].map((fixture) =>
		visualCase("editable", fixture.id, fixture.content, true),
	),
	visualCase(
		"editable-selection",
		"M26-mixed-report",
		markdown("M26-mixed-report"),
		true,
	),
	visualCase(
		"editable-selection",
		"S05-marks-and-styles",
		storedEnvelope("S05-marks-and-styles"),
		false,
	),
	visualCase(
		"editable-slash",
		"S02-empty-paragraph",
		storedEnvelope("S02-empty-paragraph"),
		false,
	),
];

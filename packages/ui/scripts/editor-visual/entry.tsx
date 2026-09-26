import maplibregl, { type MapOptions } from "maplibre-gl";
import { Component, type ReactNode, useEffect } from "react";
import { type Root, createRoot } from "react-dom/client";
import { BaseEditorKit } from "../../components/editor/editor-base-kit";
import { ThemeProvider } from "../../components/theme-provider";
import { StreamingTextEditor } from "../../components/ui/streaming-text-editor";
import {
	TextEditor,
	getStaticParseWorker,
} from "../../components/ui/text-editor";
import {
	type EditorVisualApi,
	type Theme,
	VISUAL_CASES,
	type VisualCase,
} from "./cases";

type SeedableWindow = Window & { __seedRandom?: (seed: number) => void };

const CASES_BY_KEY = new Map(
	VISUAL_CASES.map((visualCase) => [visualCase.key, visualCase]),
);

const EDITABLE_FRAME_HEIGHT = 560;

/**
 * Plate 49 resolves `BaseEditorKit` plugins in place: only the first editor in
 * a page renders paragraphs with `ParagraphElementStatic`. In the app a
 * read-only render has built the shared parse worker long before a reply
 * streams, so build it first here too and screenshot that steady state.
 */
getStaticParseWorker(BaseEditorKit);

/**
 * Maps paint into a WebGL canvas once their workers finish loading, which can
 * leave the DOM untouched for seconds on a busy machine. Every map the app
 * creates is tracked until its first `idle`; a map element that is not tracked
 * counts as pending too, so a hook that stops applying fails loudly.
 */
const liveMaps = new Set<maplibregl.Map>();
const busyMaps = new Set<maplibregl.Map>();

class TrackedMap extends maplibregl.Map {
	constructor(options: MapOptions) {
		super(options);
		liveMaps.add(this);
		busyMaps.add(this);
		this.once("idle", () => busyMaps.delete(this));
	}

	remove() {
		liveMaps.delete(this);
		busyMaps.delete(this);
		super.remove();
	}
}
Object.assign(maplibregl, { Map: TrackedMap });

function pendingWork() {
	const untracked =
		document.querySelectorAll(".maplibregl-map").length - liveMaps.size;
	return busyMaps.size + Math.max(0, untracked);
}

let lastRenderError: string | null = null;

class CaseErrorBoundary extends Component<
	{ children: ReactNode },
	{ message: string | null }
> {
	state = { message: null as string | null };

	static getDerivedStateFromError(error: unknown) {
		return {
			message: error instanceof Error ? error.message : String(error),
		};
	}

	componentDidCatch(error: unknown) {
		lastRenderError = error instanceof Error ? error.message : String(error);
	}

	render() {
		if (this.state.message === null) return this.props.children;
		return (
			<pre
				data-render-error="true"
				className="whitespace-pre-wrap rounded-md border border-destructive p-3 font-mono text-destructive text-xs"
			>
				Render error: {this.state.message}
			</pre>
		);
	}
}

const ignoreChange = () => {};

function Surface({ visualCase }: Readonly<{ visualCase: VisualCase }>) {
	switch (visualCase.surface) {
		case "static":
		case "raw":
			return (
				<TextEditor
					initialContent={visualCase.content}
					isMarkdown={visualCase.isMarkdown}
				/>
			);
		case "minimal":
			return (
				<TextEditor
					initialContent={visualCase.content}
					isMarkdown={visualCase.isMarkdown}
					minimal
				/>
			);
		case "stream":
		case "stream-partial":
			return <StreamingTextEditor content={visualCase.content} />;
		case "editable":
		case "editable-selection":
		case "editable-slash":
			return (
				<div
					data-editor-frame="true"
					className="relative flex flex-col overflow-hidden rounded-md border"
					style={{ height: EDITABLE_FRAME_HEIGHT }}
				>
					<TextEditor
						initialContent={visualCase.content}
						isMarkdown={visualCase.isMarkdown}
						editable
						onChange={ignoreChange}
					/>
				</div>
			);
	}
}

function CaseFrame({
	visualCase,
	theme,
	onCommit,
}: Readonly<{
	visualCase: VisualCase;
	theme: Theme;
	onCommit: () => void;
}>) {
	useEffect(() => onCommit(), [onCommit]);
	// Not `forcedTheme`: next-themes derives `resolvedTheme`, which charts and
	// maps read, from the stored theme, and storage starts empty per document.
	return (
		<ThemeProvider
			attribute="class"
			defaultTheme={theme}
			enableSystem={false}
			disableTransitionOnChange
		>
			<div
				id="stage"
				data-case={visualCase.key}
				data-theme={theme}
				className="bg-background text-foreground"
				style={{ width: 880, padding: 24 }}
			>
				<CaseErrorBoundary>
					<Surface visualCase={visualCase} />
				</CaseErrorBoundary>
			</div>
		</ThemeProvider>
	);
}

function seedFor(text: string) {
	let hash = 2166136261;
	for (let index = 0; index < text.length; index++) {
		hash = Math.imul(hash ^ text.charCodeAt(index), 16777619);
	}
	return hash >>> 0;
}

function requireElement(id: string): HTMLElement {
	const element = document.getElementById(id);
	if (!element) throw new Error(`editor-visual: index.html has no #${id}`);
	return element;
}

const host = requireElement("app");
let root: Root | undefined;

async function render(key: string, theme: Theme) {
	const visualCase = CASES_BY_KEY.get(key);
	if (!visualCase) throw new Error(`editor-visual: unknown case ${key}`);
	root?.unmount();
	host.replaceChildren();
	lastRenderError = null;
	(window as SeedableWindow).__seedRandom?.(seedFor(`${key}:${theme}`));
	window.scrollTo(0, 0);
	const mountPoint = document.createElement("div");
	host.append(mountPoint);
	const next = createRoot(mountPoint);
	root = next;
	await new Promise<void>((resolve) => {
		next.render(
			<CaseFrame visualCase={visualCase} theme={theme} onCommit={resolve} />,
		);
	});
}

const api: EditorVisualApi = {
	keys: VISUAL_CASES.map((visualCase) => visualCase.key),
	render,
	renderError: () => lastRenderError,
	pendingWork,
};

Object.assign(window, { editorVisual: api });

"use client";

import { remarkMdx, remarkMention } from "@platejs/markdown";
import { PlateStatic, type Value, createSlateEditor } from "platejs";
import {
	type KeyboardEvent,
	type MouseEvent,
	Suspense,
	lazy,
	memo,
	useContext,
	useMemo,
} from "react";
import remarkBreaks from "remark-breaks";
import remarkEmoji from "remark-emoji";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { AIUsageAppContext } from "../editor/ai-usage-context";
import { BaseEditorKit } from "../editor/editor-base-kit";
import {
	type MentionItem,
	MentionItemsProvider,
} from "../editor/mention-items-context";
import { preprocessDirectiveBlocks } from "../editor/plugins/remark-directives";
import { remarkFocusNodes } from "../editor/plugins/remark-focus-nodes";
import { remarkInlineSpoiler } from "../editor/plugins/remark-inline-spoiler";
import { remarkUserMention } from "../editor/plugins/remark-user-mention";
import {
	DEFAULT_UPLOAD_PREFIX,
	EditorUploadContext,
} from "../editor/upload-context";
import {
	LazyPlateStatic,
	WINDOWING_BLOCK_THRESHOLD,
	indexEditorPaths,
} from "./lazy-plate-static";

// Read-only rendering is by far the common case — a2ui pages, chat transcripts, database
// previews — and it needs none of the editing machinery. Reaching the editable editor through
// a dynamic import keeps `createEditorKit` and its three dozen plugin kits out of their bundles.
const TextEditorEditable = lazy(() =>
	import("./text-editor-editable").then((module) => ({
		default: module.TextEditorEditable,
	})),
);

const EMPTY_MENTION_ITEMS: ReadonlyArray<MentionItem> = [];

/**
 * A prefix to identify content that is serialized as Plate's native JSON.
 * This allows switching from initial Markdown to JSON after the first edit.
 */
export const PLATE_JSON_PREFIX = "plate_json::";

type PlateLikeNode = {
	children?: PlateLikeNode[];
	text?: string;
	type?: string;
	url?: string;
	[key: string]: unknown;
};

type DeserializingEditor = {
	api: {
		deserialize: (data: string) => PlateLikeNode[];
		markdown: {
			deserialize: (
				data: string,
				options: { remarkPlugins: ReadonlyArray<unknown> },
			) => PlateLikeNode[];
		};
	};
};

type PluginWithIdentity = {
	key?: string;
	name?: string;
};

const MINIMAL_STATIC_PLUGIN_IDS = new Set([
	"a",
	"blockquote",
	"bold",
	"code",
	"code_block",
	"code_line",
	"h1",
	"h2",
	"h3",
	"h4",
	"h5",
	"h6",
	"italic",
	"li",
	"lic",
	"ol",
	"p",
	"paragraph",
	"strikethrough",
	"ul",
]);

export const STATIC_EDITOR_CLASSNAME =
	"py-0 [&_h1:first-of-type]:mt-0 [&_h2:first-of-type]:mt-0 [&_h3:first-of-type]:mt-0 [&_h4:first-of-type]:mt-0 [&_h5:first-of-type]:mt-0 [&_h6:first-of-type]:mt-0";

/** Wrapper styling shared by every read-only markdown surface. */
export const PROSE_WRAPPER_CLASSNAME =
	"overflow-hidden [&_pre]:overflow-x-auto [&_pre]:whitespace-pre-wrap [&_code]:wrap-break-word [&_p]:[overflow-wrap:anywhere] [&_li]:[overflow-wrap:anywhere]";

/**
 * The full remark set. Streaming and settled rendering must share this list, or
 * the same markdown parses differently in the two phases — `$5 … $10` in prose
 * became an inline equation mid-stream and repaired itself on completion.
 */
export const RICH_REMARK_PLUGINS: ReadonlyArray<unknown> = [
	[remarkMath, { singleDollarTextMath: false }],
	remarkGfm,
	remarkBreaks,
	remarkMdx,
	remarkMention,
	remarkEmoji as unknown,
	remarkFocusNodes,
	remarkUserMention,
	remarkInlineSpoiler,
];

const MINIMAL_REMARK_PLUGINS: ReadonlyArray<unknown> = [
	remarkGfm,
	remarkBreaks,
	remarkFocusNodes,
	remarkUserMention,
	remarkInlineSpoiler,
];

/**
 * Focus-node and user-mention elements render as plain spans, so the click is
 * delegated from the wrapper. Shared by every read-only prose surface.
 */
export function createStaticProseHandlers({
	onFocusNode,
	onUserMention,
}: Readonly<{
	onFocusNode?: (nodeId: string) => void;
	onUserMention?: (sub: string) => void;
}>) {
	const handle = (
		e: MouseEvent<HTMLDivElement> | KeyboardEvent<HTMLDivElement>,
	) => {
		const target = e.target as HTMLElement;
		const focusSpan = target.closest("[data-focus-node-id]");
		if (focusSpan && onFocusNode) {
			e.preventDefault();
			const nodeId = focusSpan.getAttribute("data-focus-node-id");
			if (nodeId) onFocusNode(nodeId);
		}
		const userMentionSpan = target.closest("[data-user-mention-sub]");
		if (userMentionSpan && onUserMention) {
			e.preventDefault();
			const sub = userMentionSpan.getAttribute("data-user-mention-sub");
			if (sub) onUserMention(sub);
		}
	};

	return {
		onClick: handle,
		onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
			if (e.key === "Enter" || e.key === " ") handle(e);
		},
	};
}

const toValue = (nodes: PlateLikeNode[]): Value => nodes as unknown as Value;

const paragraphValue = (text: string): Value =>
	toValue([{ type: "p", children: [{ text }] }]);

/**
 * Splits a Markdown string into top-level blocks while preserving the integrity of fenced code blocks.
 * This is crucial for fallback rendering of broken or invalid Markdown.
 * @param markdown The raw markdown string.
 * @returns An array of markdown block strings.
 */
const splitMarkdownPreservingCodeBlocks = (markdown: string): string[] => {
	const blocks: string[] = [];
	const codeBlockRegex = /(^```[\s\S]*?^```$)|(^~~~[\s\S]*?^~~~$)/gm;
	let lastIndex = 0;
	let match: RegExpExecArray | null = codeBlockRegex.exec(markdown);

	while (match !== null) {
		const precedingText = markdown.substring(lastIndex, match.index);
		if (precedingText.trim()) {
			blocks.push(...precedingText.trim().split(/\n{2,}/));
		}
		blocks.push(match[0]);
		lastIndex = codeBlockRegex.lastIndex;
		match = codeBlockRegex.exec(markdown);
	}

	const remainingText = markdown.substring(lastIndex);
	if (remainingText.trim()) {
		blocks.push(...remainingText.trim().split(/\n{2,}/));
	}

	return blocks.filter(Boolean);
};

/**
 * Post-process Plate nodes to convert focus://, invalid://, and user:// links to custom elements
 */
export const transformSpecialLinks = (
	nodes: ReadonlyArray<PlateLikeNode>,
): PlateLikeNode[] => {
	return nodes.map((node) => {
		// If this is a link with focus:// url, convert to focus_node
		if (
			node.type === "a" &&
			typeof node.url === "string" &&
			node.url.startsWith("focus://")
		) {
			const nodeId = node.url.replace("focus://", "");
			// Extract text from children
			const nodeName =
				node.children?.map((child) => child.text || "").join("") || "Node";
			return {
				type: "focus_node",
				nodeId,
				nodeName,
				isInvalid: false,
				children: [{ text: "" }],
			};
		}
		// If this is a link with invalid:// url, convert to invalid focus_node
		if (
			node.type === "a" &&
			typeof node.url === "string" &&
			node.url.startsWith("invalid://")
		) {
			// Extract text from children - this will be the attempted reference
			const nodeName =
				node.children?.map((child) => child.text || "").join("") || "Unknown";
			return {
				type: "focus_node",
				nodeId: "",
				nodeName,
				isInvalid: true,
				children: [{ text: "" }],
			};
		}
		// If this is a link with user:// url, convert to user_mention
		if (
			node.type === "a" &&
			typeof node.url === "string" &&
			node.url.startsWith("user://")
		) {
			const sub = node.url.replace("user://", "");
			return {
				type: "user_mention",
				sub,
				children: [{ text: "" }],
			};
		}
		// If this is a link with spoiler:// url, convert to inline_spoiler
		if (
			node.type === "a" &&
			typeof node.url === "string" &&
			node.url.startsWith("spoiler://")
		) {
			const spoilerText = decodeURIComponent(
				node.url.replace("spoiler://", ""),
			);
			return {
				type: "inline_spoiler",
				spoilerText,
				children: [{ text: "" }],
			};
		}
		// Recursively process children
		if (node.children && Array.isArray(node.children)) {
			return {
				...node,
				children: transformSpecialLinks(node.children),
			};
		}
		return node;
	});
};

/**
 * Safely deserializes content into Plate editor nodes.
 * It handles prefixed native Plate JSON, Markdown, and plain text, with fallbacks.
 */
export const safeDeserialize = (
	editor: unknown,
	data: string,
	isMarkdown: boolean,
	remarkPlugins: ReadonlyArray<unknown>,
): Value => {
	const deserializingEditor = editor as DeserializingEditor;

	// 1. Check for the native JSON prefix first.
	if (data.startsWith(PLATE_JSON_PREFIX)) {
		try {
			const jsonString = data.substring(PLATE_JSON_PREFIX.length);
			const nodes: unknown = JSON.parse(jsonString);
			if (Array.isArray(nodes) && nodes.length > 0) {
				return toValue(transformSpecialLinks(nodes as PlateLikeNode[]));
			}
		} catch (error) {
			console.error(
				"Failed to parse prefixed Plate JSON, falling back.",
				error,
			);
			return paragraphValue(data);
		}
	}

	// 2. Handle initial content that is not markdown (e.g., plain text or legacy JSON).
	if (!isMarkdown) {
		try {
			// Assuming editor.api.deserialize is a custom function, potentially JSON.parse
			const nodes = deserializingEditor.api.deserialize(data);
			if (nodes.length > 0) return toValue(transformSpecialLinks(nodes));
			return paragraphValue(data);
		} catch {
			return paragraphValue(data);
		}
	}

	// 3. Handle initial markdown content.
	// Pre-process directive blocks (:::type ... :::) at text level before parsing.
	const preprocessed = preprocessDirectiveBlocks(data);
	try {
		const nodes = deserializingEditor.api.markdown.deserialize(preprocessed, {
			remarkPlugins,
		});
		if (nodes.length > 0) return toValue(transformSpecialLinks(nodes));
		return paragraphValue("");
	} catch (error) {
		console.error(
			"Markdown deserialization failed, attempting fallback:",
			error,
		);

		// 4. Fallback for broken markdown: split into blocks and deserialize individually.
		const blocks = splitMarkdownPreservingCodeBlocks(preprocessed);
		const nodes = blocks.flatMap((block): PlateLikeNode[] => {
			try {
				return deserializingEditor.api.markdown.deserialize(block, {
					remarkPlugins,
				});
			} catch {
				return [{ type: "p", children: [{ text: block }] }];
			}
		});

		if (nodes.length > 0) return toValue(transformSpecialLinks(nodes));
		return paragraphValue(data);
	}
};

type StaticKit = typeof BaseEditorKit;

const isMinimalStaticPlugin = (plugin: StaticKit[number]) => {
	const { key, name } = plugin as unknown as PluginWithIdentity;
	const pluginId = key || name || "";
	return (
		MINIMAL_STATIC_PLUGIN_IDS.has(pluginId) ||
		pluginId.includes("paragraph") ||
		pluginId.includes("heading") ||
		pluginId === "h1" ||
		pluginId === "h2" ||
		pluginId === "h3" ||
		pluginId === "h4" ||
		pluginId === "h5" ||
		pluginId === "h6" ||
		pluginId.includes("code") ||
		pluginId.includes("list") ||
		pluginId.includes("link") ||
		pluginId.includes("bold") ||
		pluginId.includes("italic") ||
		pluginId.includes("blockquote") ||
		pluginId.includes("markdown")
	);
};

// Built on first use, not at module load: the editor kit's plugin modules
// import from this file, so `BaseEditorKit` is still in its TDZ while this
// module evaluates. One shared instance keeps the kit's identity stable for
// the parse caches below.
let minimalStaticKit: StaticKit | undefined;
function getMinimalStaticKit(): StaticKit {
	minimalStaticKit ??= BaseEditorKit.filter(isMinimalStaticPlugin);
	return minimalStaticKit;
}

/**
 * The deserializer needs an editor for its plugin context but never mutates
 * it, so one worker per kit serves every parse instead of building a fresh
 * editor (~1 ms) per mount.
 */
const parseWorkers = new WeakMap<
	StaticKit,
	ReturnType<typeof createSlateEditor>
>();

export function getStaticParseWorker(plugins: StaticKit) {
	let worker = parseWorkers.get(plugins);
	if (!worker) {
		worker = createSlateEditor({ plugins, nodeId: false });
		parseWorkers.set(plugins, worker);
	}
	return worker;
}

function evictOldest(cache: Map<string, unknown>, capacity: number): void {
	while (cache.size > capacity) {
		const oldest = cache.keys().next().value;
		if (oldest === undefined) return;
		cache.delete(oldest);
	}
}

export type SettledParse = {
	readonly value: Value;
	/** Every block came from the per-block parser; no whole-document fallback stood in. */
	readonly complete: boolean;
};

const SETTLED_PARSE_CAPACITY = 8;
const settledParses = new Map<string, SettledParse>();

/**
 * The streaming editor publishes its latest parse here so the settled editor
 * that replaces it adopts the value instead of re-parsing the whole reply.
 * `supersedes` is what the same stream published before; dropping it keeps one
 * entry per live stream rather than one per token, so concurrent streams
 * cannot evict each other's final parse before it is picked up.
 */
export function publishSettledParse(
	content: string,
	parse: SettledParse,
	supersedes?: string,
): void {
	if (supersedes !== undefined && supersedes !== content)
		settledParses.delete(supersedes);
	settledParses.delete(content);
	settledParses.set(content, parse);
	evictOldest(settledParses, SETTLED_PARSE_CAPACITY);
}

export function peekSettledParse(content: string): SettledParse | undefined {
	return settledParses.get(content);
}

const STATIC_PARSE_CAPACITY = 32;
const staticParses = new Map<string, Value>();
const kitIds = new WeakMap<StaticKit, number>();
let nextKitId = 0;

/** The kit implies the remark set (`minimal` selects both), so it need not be keyed. */
function staticParseKey(
	plugins: StaticKit,
	isMarkdown: boolean,
	content: string,
): string {
	let kitId = kitIds.get(plugins);
	if (kitId === undefined) {
		kitId = nextKitId++;
		kitIds.set(plugins, kitId);
	}
	return `${kitId}:${isMarkdown ? "md" : "raw"}:${content}`;
}

/**
 * Parses `content` for a read-only editor, reusing an earlier parse when one
 * exists: the streaming parse the same reply just settled from, or a previous
 * mount of the same message (list re-orders, tab switches, liveQuery
 * re-materialisation).
 *
 * Handing one value to several editors is sound because `createSlateEditor`
 * adopts the array by reference and, with `nodeId: false`, neither initial
 * normalisation nor static rendering writes to a node — text-editor.test.tsx
 * guards that. Should a plugin ever start mutating, clone on the way out.
 */
export function resolveStaticValue(
	plugins: StaticKit,
	content: string,
	isMarkdown: boolean,
	remarkPlugins: ReadonlyArray<unknown>,
): Value {
	const settled =
		isMarkdown &&
		plugins === BaseEditorKit &&
		remarkPlugins === RICH_REMARK_PLUGINS
			? settledParses.get(content)
			: undefined;
	const key = staticParseKey(plugins, isMarkdown, content);
	const cached = settled?.complete ? settled.value : staticParses.get(key);
	const value =
		cached ??
		safeDeserialize(
			getStaticParseWorker(plugins),
			content,
			isMarkdown,
			remarkPlugins,
		);
	staticParses.delete(key);
	staticParses.set(key, value);
	evictOldest(staticParses, STATIC_PARSE_CAPACITY);
	return value;
}

function TextEditorStatic({
	initialContent,
	isMarkdown,
	minimal = false,
	onFocusNode,
	onUserMention,
}: Readonly<{
	initialContent: string;
	isMarkdown?: boolean;
	minimal?: boolean;
	onFocusNode?: (nodeId: string) => void;
	onUserMention?: (sub: string) => void;
}>) {
	const plugins = minimal ? getMinimalStaticKit() : BaseEditorKit;
	const remarkPlugins = minimal ? MINIMAL_REMARK_PLUGINS : RICH_REMARK_PLUGINS;

	const value = useMemo(
		() =>
			resolveStaticValue(
				plugins,
				initialContent,
				isMarkdown ?? false,
				remarkPlugins,
			),
		[initialContent, isMarkdown, remarkPlugins, plugins],
	);

	const editor = useMemo(() => {
		const staticEditor = createSlateEditor({
			id: "static-rendered-editor",
			plugins,
			value,
			// NodeIdPlugin stamps ids by running a setNodes transform per node,
			// and each transform re-runs normalization for the surrounding block.
			// On documents with tables that turns initial normalization quadratic
			// (240 tables: 14s with ids, 6ms without). Read-only rendering never
			// reads the ids, so skip the pass entirely.
			nodeId: false,
		});
		indexEditorPaths(staticEditor);
		return staticEditor;
	}, [plugins, value]);

	const isLongDocument = value.length > WINDOWING_BLOCK_THRESHOLD;

	return (
		<div
			{...createStaticProseHandlers({ onFocusNode, onUserMention })}
			className={PROSE_WRAPPER_CLASSNAME}
		>
			{isLongDocument ? (
				<LazyPlateStatic editor={editor} className={STATIC_EDITOR_CLASSNAME} />
			) : (
				<PlateStatic editor={editor} className={STATIC_EDITOR_CLASSNAME} />
			)}
		</div>
	);
}

type TextEditorProps = {
	/** App owning this editable content; used for hosted-model usage attribution and media uploads. */
	appId?: string;
	initialContent: string;
	onChange?: (content: string) => void;
	isMarkdown?: boolean;
	editable?: boolean;
	minimal?: boolean;
	onFocusNode?: (nodeId: string) => void;
	onUserMention?: (sub: string) => void;
	mentionItems?: ReadonlyArray<MentionItem>;
	/** Storage folder that pasted, dropped and picked media is uploaded into. */
	uploadPrefix?: string;
	/** `user` uploads into the caller's private area instead of the shared app area. */
	uploadScope?: "app" | "user";
};

export const TextEditor = memo(function TextEditor({
	appId,
	initialContent,
	onChange,
	isMarkdown,
	editable = false,
	minimal = false,
	onFocusNode,
	onUserMention,
	mentionItems,
	uploadPrefix,
	uploadScope,
}: Readonly<TextEditorProps>) {
	const items = mentionItems ?? EMPTY_MENTION_ITEMS;
	const inheritedAppId = useContext(AIUsageAppContext);
	const resolvedAppId = appId ?? inheritedAppId;
	const uploadConfig = useMemo(
		() => ({
			appId: resolvedAppId,
			prefix: uploadPrefix ?? DEFAULT_UPLOAD_PREFIX,
			scope: uploadScope ?? ("app" as const),
		}),
		[resolvedAppId, uploadPrefix, uploadScope],
	);

	if (editable && onChange) {
		return (
			<AIUsageAppContext.Provider value={resolvedAppId}>
				<EditorUploadContext.Provider value={uploadConfig}>
					<MentionItemsProvider value={items}>
						<Suspense fallback={<div className="px-4 py-2" />}>
							<TextEditorEditable
								initialContent={initialContent}
								onChange={(content: string) => {
									onChange(content);
								}}
								isMarkdown={isMarkdown}
								onFocusNode={onFocusNode}
							/>
						</Suspense>
					</MentionItemsProvider>
				</EditorUploadContext.Provider>
			</AIUsageAppContext.Provider>
		);
	}
	return (
		<AIUsageAppContext.Provider value={resolvedAppId}>
			<MentionItemsProvider value={items}>
				<TextEditorStatic
					initialContent={initialContent}
					isMarkdown={isMarkdown}
					minimal={minimal}
					onFocusNode={onFocusNode}
					onUserMention={onUserMention}
				/>
			</MentionItemsProvider>
		</AIUsageAppContext.Provider>
	);
});

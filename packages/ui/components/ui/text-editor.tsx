"use client";

import { remarkMdx, remarkMention } from "@platejs/markdown";
import { PlateStatic, type Value, createSlateEditor } from "platejs";
import { Plate, usePlateEditor } from "platejs/react";
import {
	type KeyboardEvent,
	type MouseEvent,
	memo,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import remarkBreaks from "remark-breaks";
import remarkEmoji from "remark-emoji";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { AIUsageAppContext } from "../editor/ai-usage-context";
import { BaseEditorKit } from "../editor/editor-base-kit";
import { createEditorKit } from "../editor/editor-kit";
import {
	type MentionItem,
	MentionItemsProvider,
} from "../editor/mention-items-context";
import { preprocessDirectiveBlocks } from "../editor/plugins/remark-directives";
import { remarkFocusNodes } from "../editor/plugins/remark-focus-nodes";
import { remarkInlineSpoiler } from "../editor/plugins/remark-inline-spoiler";
import { remarkUserMention } from "../editor/plugins/remark-user-mention";
import { Editor, EditorContainer } from "../editor/ui/editor";
import {
	LazyPlateStatic,
	WINDOWING_BLOCK_THRESHOLD,
	indexEditorPaths,
} from "./lazy-plate-static";

const EMPTY_MENTION_ITEMS: ReadonlyArray<MentionItem> = [];

/**
 * A prefix to identify content that is serialized as Plate's native JSON.
 * This allows switching from initial Markdown to JSON after the first edit.
 */
const PLATE_JSON_PREFIX = "plate_json::";

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

function TextEditorInner({
	initialContent,
	onChange,
	isMarkdown,
	onFocusNode,
}: Readonly<{
	initialContent: string;
	onChange: (content: string) => void;
	isMarkdown?: boolean;
	onFocusNode?: (nodeId: string) => void;
}>) {
	const appId = useContext(AIUsageAppContext);
	const editorPlugins = useMemo(() => createEditorKit(appId), [appId]);
	const remarkPlugins = useMemo(
		() => [
			[remarkMath, { singleDollarTextMath: false }],
			remarkGfm,
			remarkBreaks,
			remarkMdx,
			remarkMention,
			remarkEmoji as unknown,
			remarkFocusNodes,
			remarkUserMention,
			remarkInlineSpoiler,
		],
		[],
	);
	const lastEmittedContentRef = useRef(initialContent);
	const [editorSeed, setEditorSeed] = useState(initialContent);

	useEffect(() => {
		if (initialContent === lastEmittedContentRef.current) {
			return;
		}

		lastEmittedContentRef.current = initialContent;
		setEditorSeed(initialContent);
	}, [initialContent]);

	const editor = usePlateEditor(
		{
			id: "rendered-editor",
			plugins: editorPlugins,
			value: (self) =>
				safeDeserialize(self, editorSeed, isMarkdown ?? false, remarkPlugins),
		},
		[editorSeed, isMarkdown, remarkPlugins, editorPlugins],
	);

	return (
		<Plate
			editor={editor}
			onChange={({ editor }) => {
				const serializedNodes = editor.children;
				const newContent = `${PLATE_JSON_PREFIX}${JSON.stringify(
					serializedNodes,
				)}`;

				if (newContent === lastEmittedContentRef.current) {
					return;
				}
				lastEmittedContentRef.current = newContent;
				onChange(newContent);
			}}
		>
			<EditorContainer>
				<Editor variant="none" className="px-4 py-2" />
			</EditorContainer>
		</Plate>
	);
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
	const remarkPlugins = minimal ? MINIMAL_REMARK_PLUGINS : RICH_REMARK_PLUGINS;

	// Use minimal plugin set for better performance in read-only contexts
	const plugins = useMemo(
		() =>
			minimal
				? [
						...BaseEditorKit.filter((plugin) => {
							// Only include essential plugins for markdown rendering
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
						}),
					]
				: BaseEditorKit,
		[minimal],
	);

	// The value is memoized to avoid re-creating the editor on every render.
	const value = useMemo(() => {
		const tempEditor = createSlateEditor({ plugins });
		return safeDeserialize(
			tempEditor,
			initialContent,
			isMarkdown ?? false,
			remarkPlugins,
		);
	}, [initialContent, isMarkdown, remarkPlugins, plugins]);

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
	/** App owning this editable content; used for hosted-model usage attribution. */
	appId?: string;
	initialContent: string;
	onChange?: (content: string) => void;
	isMarkdown?: boolean;
	editable?: boolean;
	minimal?: boolean;
	onFocusNode?: (nodeId: string) => void;
	onUserMention?: (sub: string) => void;
	mentionItems?: ReadonlyArray<MentionItem>;
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
}: Readonly<TextEditorProps>) {
	const items = mentionItems ?? EMPTY_MENTION_ITEMS;
	const inheritedAppId = useContext(AIUsageAppContext);
	if (editable && onChange) {
		return (
			<AIUsageAppContext.Provider value={appId ?? inheritedAppId}>
				<MentionItemsProvider value={items}>
					<TextEditorInner
						initialContent={initialContent}
						onChange={(content: string) => {
							onChange(content);
						}}
						isMarkdown={isMarkdown}
						onFocusNode={onFocusNode}
					/>
				</MentionItemsProvider>
			</AIUsageAppContext.Provider>
		);
	}
	return (
		<MentionItemsProvider value={items}>
			<TextEditorStatic
				initialContent={initialContent}
				isMarkdown={isMarkdown}
				minimal={minimal}
				onFocusNode={onFocusNode}
				onUserMention={onUserMention}
			/>
		</MentionItemsProvider>
	);
});

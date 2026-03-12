"use client";

import { remarkMdx, remarkMention } from "@platejs/markdown";
import { PlateStatic, type Value, createSlateEditor } from "platejs";
import { Plate, usePlateEditor } from "platejs/react";
import { memo, useMemo } from "react";
import remarkBreaks from "remark-breaks";
import remarkEmoji from "remark-emoji";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import { BaseEditorKit } from "../editor/editor-base-kit";
import { EditorKit } from "../editor/editor-kit";
import { preprocessDirectiveBlocks } from "../editor/plugins/remark-directives";
import { remarkFocusNodes } from "../editor/plugins/remark-focus-nodes";
import { remarkInlineSpoiler } from "../editor/plugins/remark-inline-spoiler";
import { remarkUserMention } from "../editor/plugins/remark-user-mention";
import { Editor, EditorContainer } from "../editor/ui/editor";

/**
 * A prefix to identify content that is serialized as Plate's native JSON.
 * This allows switching from initial Markdown to JSON after the first edit.
 */
const PLATE_JSON_PREFIX = "plate_json::";

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
	let match;

	while ((match = codeBlockRegex.exec(markdown)) !== null) {
		const precedingText = markdown.substring(lastIndex, match.index);
		if (precedingText.trim()) {
			blocks.push(...precedingText.trim().split(/\n{2,}/));
		}
		blocks.push(match[0]);
		lastIndex = codeBlockRegex.lastIndex;
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
const transformSpecialLinks = (nodes: any[]): any[] => {
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
				node.children?.map((child: any) => child.text || "").join("") || "Node";
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
				node.children?.map((child: any) => child.text || "").join("") ||
				"Unknown";
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
			const spoilerText = decodeURIComponent(node.url.replace("spoiler://", ""));
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
	editor: any,
	data: string,
	isMarkdown: boolean,
	remarkPlugins: any[],
): Value => {
	// 1. Check for the native JSON prefix first.
	if (data.startsWith(PLATE_JSON_PREFIX)) {
		try {
			const jsonString = data.substring(PLATE_JSON_PREFIX.length);
			const nodes = JSON.parse(jsonString);
			if (Array.isArray(nodes) && nodes.length > 0) {
				return transformSpecialLinks(nodes);
			}
		} catch (error) {
			console.error(
				"Failed to parse prefixed Plate JSON, falling back.",
				error,
			);
			return [{ type: "p", children: [{ text: data }] }];
		}
	}

	// 2. Handle initial content that is not markdown (e.g., plain text or legacy JSON).
	if (!isMarkdown) {
		try {
			// Assuming editor.api.deserialize is a custom function, potentially JSON.parse
			const nodes = editor.api.deserialize(data);
			if (nodes.length > 0) return transformSpecialLinks(nodes);
			return [{ type: "p", children: [{ text: data }] }];
		} catch {
			return [{ type: "p", children: [{ text: data }] }];
		}
	}

	// 3. Handle initial markdown content.
	// Pre-process directive blocks (:::type ... :::) at text level before parsing.
	const preprocessed = preprocessDirectiveBlocks(data);
	try {
		const nodes = editor.api.markdown.deserialize(preprocessed, { remarkPlugins });
		if (nodes.length > 0) return transformSpecialLinks(nodes);
		return [{ type: "p", children: [{ text: "" }] }];
	} catch (error) {
		console.error(
			"Markdown deserialization failed, attempting fallback:",
			error,
		);

		// 4. Fallback for broken markdown: split into blocks and deserialize individually.
		const blocks = splitMarkdownPreservingCodeBlocks(preprocessed);
		const nodes = blocks.flatMap((block) => {
			try {
				return editor.api.markdown.deserialize(block, { remarkPlugins });
			} catch {
				return { type: "p", children: [{ text: block }] };
			}
		});

		if (nodes.length > 0) return transformSpecialLinks(nodes);
		return [{ type: "p", children: [{ text: data }] }];
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
	const remarkPlugins = useMemo(
		() => [
			remarkMath,
			remarkGfm,
			remarkBreaks,
			remarkMdx,
			remarkMention,
			remarkEmoji as any,
			remarkFocusNodes,
			remarkUserMention,
			remarkInlineSpoiler,
		],
		[],
	);

	const editor = usePlateEditor(
		{
			id: "rendered-editor",
			plugins: EditorKit,
			value: (self) =>
				safeDeserialize(
					self,
					initialContent,
					isMarkdown ?? false,
					remarkPlugins,
				),
		},
		[initialContent, isMarkdown, remarkPlugins],
	);

	return (
		<Plate
			editor={editor}
			onChange={({ editor }) => {
				const serializedNodes = editor.children;
				const newContent = `${PLATE_JSON_PREFIX}${JSON.stringify(
					serializedNodes,
				)}`;

				if (newContent === initialContent) {
					return;
				}
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
	const remarkPlugins = useMemo(
		() =>
			minimal
				? [remarkGfm, remarkBreaks, remarkFocusNodes, remarkUserMention, remarkInlineSpoiler]
				: [
						remarkMath,
						remarkGfm,
						remarkBreaks,
						remarkMdx,
						remarkMention,
						remarkEmoji as any,
						remarkFocusNodes,
						remarkUserMention,
						remarkInlineSpoiler,
					],
		[minimal],
	);

	// Use minimal plugin set for better performance in read-only contexts
	const plugins = useMemo(
		() =>
			minimal
				? [
						...BaseEditorKit.filter((plugin) => {
							// Only include essential plugins for markdown rendering
							const pluginId =
								(plugin as any).key || (plugin as any).name || "";
							return (
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
		// For large content, truncate to prevent performance issues
		const MAX_LENGTH = 50000; // ~50KB
		const contentToRender =
			initialContent.length > MAX_LENGTH
				? initialContent.slice(0, MAX_LENGTH) +
					"\\n\\n... (content truncated for performance)"
				: initialContent;

		const tempEditor = createSlateEditor({ plugins });
		return safeDeserialize(
			tempEditor,
			contentToRender,
			isMarkdown ?? false,
			remarkPlugins,
		);
	}, [initialContent, isMarkdown, remarkPlugins, plugins]);

	const editor = useMemo(
		() =>
			createSlateEditor({
				id: "static-rendered-editor",
				plugins,
				value,
			}),
		[plugins, value],
	);

	return (
		<div
			onClick={(e) => {
				const target = e.target as HTMLElement;
				const focusSpan = target.closest("[data-focus-node-id]");
				if (focusSpan && onFocusNode) {
					e.preventDefault();
					const nodeId = focusSpan.getAttribute("data-focus-node-id");
					if (nodeId) {
						onFocusNode(nodeId);
					}
				}
				const userMentionSpan = target.closest("[data-user-mention-sub]");
				if (userMentionSpan && onUserMention) {
					e.preventDefault();
					const sub = userMentionSpan.getAttribute("data-user-mention-sub");
					if (sub) {
						onUserMention(sub);
					}
				}
			}}
			className="overflow-hidden [&_pre]:overflow-x-auto [&_pre]:whitespace-pre-wrap [&_code]:wrap-break-word"
		>
			<PlateStatic editor={editor} className="py-0 [&>*:first-child_h1]:mt-0 [&>*:first-child_h2]:mt-0 [&>*:first-child_h3]:mt-0 [&>*:first-child_h4]:mt-0 [&>*:first-child_h5]:mt-0 [&>*:first-child_h6]:mt-0" />
		</div>
	);
}

type TextEditorProps = {
	initialContent: string;
	onChange?: (content: string) => void;
	isMarkdown?: boolean;
	editable?: boolean;
	minimal?: boolean;
	onFocusNode?: (nodeId: string) => void;
	onUserMention?: (sub: string) => void;
};

export const TextEditor = memo(function TextEditor({
	initialContent,
	onChange,
	isMarkdown,
	editable = false,
	minimal = false,
	onFocusNode,
	onUserMention,
}: Readonly<TextEditorProps>) {
	if (editable && onChange) {
		return (
			<TextEditorInner
				initialContent={initialContent}
				onChange={(content: string) => {
					onChange(content);
				}}
				isMarkdown={isMarkdown}
				onFocusNode={onFocusNode}
			/>
		);
	}
	return (
		<TextEditorStatic
			initialContent={initialContent}
			isMarkdown={isMarkdown}
			minimal={minimal}
			onFocusNode={onFocusNode}
			onUserMention={onUserMention}
		/>
	);
});

"use client";
import dynamic from "next/dynamic";
import { useMemo } from "react";
import type { Lesson, LessonAssetView } from "../../lib/learn/types";
import { TextEditor } from "../ui/text-editor";

const ReactPlayer = dynamic(() => import("react-player"), { ssr: false });

const PLATE_JSON_PREFIX = "plate_json::";
const MARKDOWN_REF_RE = /(^|[^\w\\])@([A-Za-z_][A-Za-z0-9_-]{0,63})/g;

interface LessonContentProps {
	readonly lesson: Lesson;
	readonly assets?: ReadonlyArray<LessonAssetView>;
	/** Optional click handler for inline `focus_node` marks in the lesson body. */
	readonly onFocusNode?: (nodeId: string) => void;
}

interface PlateNode {
	readonly type?: string;
	readonly value?: string;
	readonly children?: ReadonlyArray<PlateNode>;
	readonly [key: string]: unknown;
}

function nodeForAsset(asset: LessonAssetView): PlateNode {
	switch (asset.kind) {
		case "IMAGE":
			return {
				type: "img",
				url: asset.signed_url,
				children: [{ text: "" }],
			};
		case "VIDEO":
			return {
				type: "video",
				url: asset.signed_url,
				children: [{ text: "" }],
			};
		case "AUDIO":
			return {
				type: "audio",
				url: asset.signed_url,
				children: [{ text: "" }],
			};
		default:
			return {
				type: "a",
				url: asset.signed_url,
				children: [{ text: asset.name }],
			};
	}
}

function markdownForAsset(asset: LessonAssetView): string {
	const url = asset.signed_url
		.replaceAll(" ", "%20")
		.replaceAll("(", "%28")
		.replaceAll(")", "%29");
	const label = asset.name.replaceAll("[", "\\[").replaceAll("]", "\\]");
	return asset.kind === "IMAGE" ? `![${label}](${url})` : `[${label}](${url})`;
}

function walkPlateNodes(
	nodes: ReadonlyArray<PlateNode>,
	byName: Map<string, LessonAssetView>,
): PlateNode[] {
	const out: PlateNode[] = [];
	for (const node of nodes) {
		if (node && typeof node === "object") {
			// Author-inserted asset node: refresh stale signed URL while keeping
			// any user-applied width/align/caption.
			const assetName = node.assetName;
			if (typeof assetName === "string") {
				const asset = byName.get(assetName);
				if (asset) {
					out.push({ ...node, url: asset.signed_url });
					continue;
				}
			}
			// Legacy mention shape — replace with a media node.
			if (node.type === "mention" && typeof node.value === "string") {
				const asset = byName.get(node.value);
				if (asset) {
					out.push(nodeForAsset(asset));
					continue;
				}
			}
		}
		if (Array.isArray(node?.children)) {
			out.push({
				...node,
				children: walkPlateNodes(node.children, byName),
			});
			continue;
		}
		out.push(node);
	}
	return out;
}

function resolveAssetReferences(
	content: string,
	assets: ReadonlyArray<LessonAssetView>,
): string {
	if (!content || assets.length === 0) return content;
	const byName = new Map(assets.map((a) => [a.name, a]));

	if (content.startsWith(PLATE_JSON_PREFIX)) {
		try {
			const parsed = JSON.parse(content.slice(PLATE_JSON_PREFIX.length));
			if (!Array.isArray(parsed)) return content;
			const transformed = walkPlateNodes(parsed as PlateNode[], byName);
			return `${PLATE_JSON_PREFIX}${JSON.stringify(transformed)}`;
		} catch {
			return content;
		}
	}

	return content.replace(MARKDOWN_REF_RE, (whole, prefix, name) => {
		const asset = byName.get(name);
		if (!asset) return whole;
		return `${prefix}${markdownForAsset(asset)}`;
	});
}

/**
 * Renders a lesson body. Content is stored as a string that may be either
 * raw markdown (legacy / seed data) or the platform's `plate_json::…` envelope
 * produced by the Platejs editor — TextEditor handles both via isMarkdown.
 *
 * `@AssetName` references — either as `mention` nodes from the editor combobox
 * or as raw markdown text — are resolved here against the supplied `assets`
 * list to fresh signed URLs. Storage stays untouched; URLs are computed per
 * render so they never expire while baked into the document.
 */
export function LessonContent({
	lesson,
	assets,
	onFocusNode,
}: LessonContentProps) {
	const resolvedContent = useMemo(
		() => resolveAssetReferences(lesson.content ?? "", assets ?? []),
		[lesson.content, assets],
	);

	return (
		<article className="flex flex-col gap-6">
			<header className="space-y-1">
				<h1 className="text-2xl font-semibold">{lesson.title}</h1>
				<p className="text-xs text-muted-foreground">
					{lesson.estimated_minutes} min · {lesson.language.toUpperCase()}
					{lesson.is_optional ? " · Optional" : ""}
				</p>
			</header>
			{lesson.video_url && (
				<div className="overflow-hidden rounded-lg bg-black aspect-video">
					<ReactPlayer
						src={lesson.video_url}
						width="100%"
						height="100%"
						controls
					/>
				</div>
			)}
			<div className="prose prose-sm dark:prose-invert max-w-none">
				<TextEditor
					initialContent={resolvedContent}
					editable={false}
					isMarkdown
					onFocusNode={onFocusNode}
				/>
			</div>
		</article>
	);
}

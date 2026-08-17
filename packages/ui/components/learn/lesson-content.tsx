"use client";
import dynamic from "next/dynamic";
import { type SyntheticEvent, useCallback, useMemo } from "react";
import type { Lesson, LessonAssetView } from "../../lib/learn/types";
import { TextEditor } from "../ui/text-editor";
import {
	lessonAssetLabel,
	removeDuplicateLessonTitle,
	resolveAssetReferences,
} from "./lesson-content-utils";

const ReactPlayer = dynamic(() => import("react-player"), { ssr: false });

interface LessonContentProps {
	readonly lesson: Lesson;
	readonly assets?: ReadonlyArray<LessonAssetView>;
	/** Optional click handler for inline `focus_node` marks in the lesson body. */
	readonly onFocusNode?: (nodeId: string) => void;
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
		() =>
			resolveAssetReferences(
				removeDuplicateLessonTitle(lesson.content ?? "", lesson.title),
				assets ?? [],
			),
		[lesson.content, lesson.title, assets],
	);
	const assetLabelsByUrl = useMemo(
		() =>
			new Map(
				(assets ?? []).map((asset) => [
					asset.signed_url,
					lessonAssetLabel(asset.name),
				]),
			),
		[assets],
	);
	const imageDescription = useCallback(
		(image: HTMLImageElement) =>
			image.alt.trim() ||
			assetLabelsByUrl.get(image.getAttribute("src") ?? "") ||
			"Lesson image",
		[assetLabelsByUrl],
	);

	const handleImageError = useCallback(
		(event: SyntheticEvent<HTMLDivElement>) => {
			const image = event.target;
			if (!(image instanceof HTMLImageElement)) return;
			const figure = image.closest("figure");
			if (!figure) return;

			const description = imageDescription(image);
			image.alt = description;
			image.dataset.lessonMediaFailed = "true";
			delete figure.dataset.lessonMediaCaption;
			figure.dataset.lessonMediaFailed = "true";
			figure.setAttribute("role", "img");
			figure.setAttribute("aria-label", `${description} could not be loaded.`);
		},
		[imageDescription],
	);

	const handleImageLoad = useCallback(
		(event: SyntheticEvent<HTMLDivElement>) => {
			const image = event.target;
			if (!(image instanceof HTMLImageElement)) return;
			const figure = image.closest("figure");
			if (!figure) return;
			const description = imageDescription(image);
			image.alt = description;

			delete image.dataset.lessonMediaFailed;
			delete figure.dataset.lessonMediaFailed;
			figure.removeAttribute("role");
			figure.removeAttribute("aria-label");
			if (!figure.querySelector("figcaption")) {
				figure.dataset.lessonMediaCaption = description;
			}
		},
		[imageDescription],
	);

	return (
		<article className="fl-lesson-article mx-auto flex w-full max-w-5xl flex-col gap-7 md:gap-8">
			<header className="mx-auto w-full max-w-[66ch] space-y-2.5">
				<h1 className="text-balance text-2xl font-semibold tracking-tight md:text-3xl">
					{lesson.title}
				</h1>
				<p className="text-sm text-muted-foreground">
					{lesson.estimated_minutes} min · {lesson.language.toUpperCase()}
					{lesson.is_optional ? " · Optional" : ""}
				</p>
			</header>
			{lesson.video_url && (
				<div className="mx-auto aspect-video w-full max-w-4xl overflow-hidden rounded-lg border bg-black">
					<ReactPlayer
						src={lesson.video_url}
						width="100%"
						height="100%"
						controls
					/>
				</div>
			)}
			<div
				onError={handleImageError}
				onLoad={handleImageLoad}
				className="fl-lesson-prose"
			>
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

"use client";

import { safeCssLength } from "../safe-css-length";

import { parseTwitterUrl, parseVideoUrl } from "@platejs/media";
import {
	NodeApi,
	type TCaptionProps,
	type TMediaEmbedElement,
	type TResizableProps,
} from "platejs";
import { SlateElement, type SlateElementProps } from "platejs/static";
import type { MediaMetadata } from "../media-metadata";
import { isSafeEditorUrl } from "../plugins/safe-url-kit";
import { MediaAttribution } from "./media-attribution";

export function MediaEmbedElementStatic(
	props: SlateElementProps<
		TMediaEmbedElement & TCaptionProps & TResizableProps & MediaMetadata
	>,
) {
	const { url, caption, alt, width, align = "center" } = props.element;
	const video = parseVideoUrl(url);
	const tweet = parseTwitterUrl(url);
	return (
		<SlateElement {...props} className="py-2.5">
			<figure
				className="m-0 max-w-full"
				style={{ width: safeCssLength(width), textAlign: align }}
			>
				{video ? (
					<iframe
						className="aspect-video w-full rounded-sm border-0"
						src={video.url}
						title={alt || `${video.provider} video`}
						loading="lazy"
						allowFullScreen
					/>
				) : isSafeEditorUrl(url) ? (
					<a href={url} target="_blank" rel="noopener noreferrer">
						{alt || (tweet ? "View post" : "View embedded content")}
					</a>
				) : null}
				{caption?.length ? (
					<figcaption>
						{caption.map((node) => NodeApi.string(node)).join("\n")}
					</figcaption>
				) : null}
				<MediaAttribution element={props.element} />
			</figure>
			{props.children}
		</SlateElement>
	);
}

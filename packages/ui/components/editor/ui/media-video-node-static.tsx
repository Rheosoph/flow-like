"use client";

import { safeCssLength } from "../safe-css-length";

import { parseVideoUrl } from "@platejs/media";
import {
	NodeApi,
	type TCaptionProps,
	type TResizableProps,
	type TVideoElement,
} from "platejs";
import { SlateElement, type SlateElementProps } from "platejs/static";
import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";
import type { MediaMetadata } from "../media-metadata";
import { MediaAttribution } from "./media-attribution";

export function VideoElementStatic(
	props: SlateElementProps<
		TVideoElement & TCaptionProps & TResizableProps & MediaMetadata
	>,
) {
	const { align = "center", caption, url, width, alt } = props.element;
	const resolvedUrl = useEditorAssetUrl(url);
	const embed = parseVideoUrl(url);
	return (
		<SlateElement className="py-2.5" {...props}>
			<div style={{ textAlign: align }}>
				<figure
					className="relative m-0 inline-block w-full max-w-full"
					style={{ width: safeCssLength(width) }}
				>
					{embed ? (
						<iframe
							className="aspect-video w-full rounded-sm border-0"
							src={embed.url}
							title={alt || `${embed.provider} video`}
							loading="lazy"
							allowFullScreen
						/>
					) : (
						<video
							className="w-full rounded-sm"
							src={resolvedUrl}
							aria-label={alt}
							controls
							preload="metadata"
						/>
					)}
					{caption?.length ? (
						<figcaption>
							{caption.map((node) => NodeApi.string(node)).join("\n")}
						</figcaption>
					) : null}
					<MediaAttribution element={props.element} />
				</figure>
			</div>
			{props.children}
		</SlateElement>
	);
}

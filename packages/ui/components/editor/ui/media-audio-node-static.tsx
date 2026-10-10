"use client";

import { NodeApi, type TCaptionProps, type TAudioElement } from "platejs";
import type { MediaMetadata } from "../media-metadata";
import { MediaAttribution } from "./media-attribution";
import type { SlateElementProps } from "platejs/static";

import { SlateElement } from "platejs/static";

import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";

export function AudioElementStatic(
	props: SlateElementProps<TAudioElement & TCaptionProps & MediaMetadata>,
) {
	const resolvedUrl = useEditorAssetUrl(props.element.url);

	return (
		<SlateElement {...props} className="mb-1">
			<figure className="group relative cursor-default">
				<div className="h-16">
					<audio className="size-full" src={resolvedUrl} controls />
				</div>
			</figure>
			{props.element.caption?.length ? (
				<div className="mt-1 text-sm">
					{props.element.caption.map((node) => NodeApi.string(node)).join("\n")}
				</div>
			) : null}
			<MediaAttribution element={props.element} />
			{props.children}
		</SlateElement>
	);
}

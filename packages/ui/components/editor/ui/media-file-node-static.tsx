"use client";

import { NodeApi, type TCaptionProps, type TFileElement } from "platejs";
import type { MediaMetadata } from "../media-metadata";
import { MediaAttribution } from "./media-attribution";
import type { SlateElementProps } from "platejs/static";

import { FileUp } from "lucide-react";
import { SlateElement } from "platejs/static";

import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";

export function FileElementStatic(
	props: SlateElementProps<TFileElement & TCaptionProps & MediaMetadata>,
) {
	const { name, url } = props.element;
	const resolvedUrl = useEditorAssetUrl(url);

	return (
		<SlateElement className="my-px rounded-sm" {...props}>
			<a
				className="group relative m-0 flex cursor-pointer items-center rounded px-0.5 py-[3px] hover:bg-muted"
				contentEditable={false}
				download={name}
				href={resolvedUrl}
				rel="noopener noreferrer"
				role="button"
				target="_blank"
			>
				<div className="flex items-center gap-1 p-1">
					<FileUp className="size-5" />
					<div>{name}</div>
				</div>
			</a>
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

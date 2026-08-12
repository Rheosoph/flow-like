"use client";

import type { SlateElementProps, TAudioElement } from "platejs";

import { SlateElement } from "platejs";

import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";

export function AudioElementStatic(props: SlateElementProps<TAudioElement>) {
	const resolvedUrl = useEditorAssetUrl(props.element.url);

	return (
		<SlateElement {...props} className="mb-1">
			<figure className="group relative cursor-default">
				<div className="h-16">
					<audio className="size-full" src={resolvedUrl} controls />
				</div>
			</figure>
			{props.children}
		</SlateElement>
	);
}

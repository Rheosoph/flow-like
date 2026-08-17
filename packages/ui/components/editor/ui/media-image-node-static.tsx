"use client";

import type {
	SlateElementProps,
	TCaptionProps,
	TImageElement,
	TResizableProps,
} from "platejs";

import { NodeApi, SlateElement } from "platejs";

import { cn } from "../../../lib/utils";
import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";

export function ImageElementStatic(
	props: SlateElementProps<
		TImageElement & TCaptionProps & TResizableProps & { alt?: string }
	>,
) {
	const { align = "center", alt, caption, url, width } = props.element;
	const src = useEditorAssetUrl(url);

	return (
		<SlateElement {...props} className="py-2.5">
			<figure className="group relative m-0 inline-block" style={{ width }}>
				<div
					className="relative max-w-full min-w-[92px]"
					style={{ textAlign: align }}
				>
					<img
						className={cn(
							"w-full max-w-full cursor-default object-cover px-0",
							"rounded-sm",
						)}
						alt={alt ?? ""}
						src={src}
					/>
					{caption && (
						<figcaption className="mx-auto mt-2 h-[24px] max-w-full">
							{NodeApi.string(caption[0])}
						</figcaption>
					)}
				</div>
			</figure>
			{props.children}
		</SlateElement>
	);
}

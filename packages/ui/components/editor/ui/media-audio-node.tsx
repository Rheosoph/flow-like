"use client";

import { useTranslation } from "@flow-like/locales";
import type { TAudioElement } from "platejs";
import type { PlateElementProps } from "platejs/react";

import { useMediaState } from "@platejs/media/react";
import { ResizableProvider } from "@platejs/resizable";
import { PlateElement, withHOC } from "platejs/react";

import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";
import { Caption, CaptionTextarea } from "./caption";

export const AudioElement = withHOC(
	ResizableProvider,
	function AudioElement(props: PlateElementProps<TAudioElement>) {
		const { t } = useTranslation("common");
		const { align = "center", readOnly, unsafeUrl } = useMediaState();
		const resolvedUrl = useEditorAssetUrl(unsafeUrl);

		return (
			<PlateElement {...props} className="mb-1">
				<figure
					className="group relative cursor-default"
					contentEditable={false}
				>
					<div className="h-16">
						<audio className="size-full" src={resolvedUrl} controls />
					</div>

					<Caption style={{ width: "100%" }} align={align}>
						<CaptionTextarea
							className="h-20"
							readOnly={readOnly}
							placeholder={t('writeACaption', 'Write a caption...')}
						/>
					</Caption>
				</figure>
				{props.children}
			</PlateElement>
		);
	},
);

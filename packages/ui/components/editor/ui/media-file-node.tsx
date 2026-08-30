"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFileElement } from "platejs";
import type { PlateElementProps } from "platejs/react";

import { useMediaState } from "@platejs/media/react";
import { ResizableProvider } from "@platejs/resizable";
import { FileUp } from "lucide-react";
import { PlateElement, useReadOnly, withHOC } from "platejs/react";

import { useEditorAssetUrl } from "../hooks/use-editor-asset-url";
import { Caption, CaptionTextarea } from "./caption";

export const FileElement = withHOC(
	ResizableProvider,
	function FileElement(props: PlateElementProps<TFileElement>) {
		const { t } = useTranslation("common");
		const readOnly = useReadOnly();
		const { name, unsafeUrl } = useMediaState();
		const resolvedUrl = useEditorAssetUrl(unsafeUrl);

		return (
			<PlateElement className="my-px rounded-sm" {...props}>
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

					<Caption align="left">
						<CaptionTextarea
							className="text-left"
							readOnly={readOnly}
							placeholder={t("writeACaption", "Write a caption...")}
						/>
					</Caption>
				</a>
				{props.children}
			</PlateElement>
		);
	},
);

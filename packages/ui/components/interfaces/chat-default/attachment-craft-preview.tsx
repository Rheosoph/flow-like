"use client";

import { useTranslation } from "@flow-like/locales";
import Loader2Icon from "lucide-react/dist/esm/icons/loader-circle.js";
import {
	Suspense,
	lazy,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { resolveCraftPreview } from "../../a2ui/display/file-preview-source";
import type { CraftViewer } from "../../storage/craft-viewers";
import type { ProcessedAttachment } from "./attachment";

const CraftDocument = lazy(() =>
	import("../../storage/craft-document").then((module) => ({
		default: module.CraftDocument,
	})),
);
const WordcraftDocument = lazy(() =>
	import("../../storage/wordcraft-document").then((module) => ({
		default: module.WordcraftDocument,
	})),
);

/**
 * The Flows editor that opens an office, spreadsheet, design or CAD attachment.
 * Media keeps its own players, and a stored file waits for its signed link.
 */
export function attachmentCraftViewer(file: ProcessedAttachment) {
	if (file.access || (file.type !== "document" && file.type !== "other"))
		return undefined;
	return resolveCraftPreview(file.url, file.displayName);
}

export function AttachmentCraftPreview({
	file,
	viewer,
	filename,
}: Readonly<{
	file: ProcessedAttachment;
	viewer: CraftViewer;
	filename: string;
}>) {
	const { t } = useTranslation();
	// Stored links are re-signed every few minutes; that must not reload an open document.
	const [url, setUrl] = useState(file.url);
	const opened = useRef(false);
	useEffect(() => {
		if (!opened.current) setUrl(file.url);
	}, [file.url]);
	const onReady = useCallback(() => {
		opened.current = true;
	}, []);
	const documentProps = { url, filename, editable: false, onReady };

	return (
		<Suspense
			fallback={
				<output className="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
					<Loader2Icon className="size-4 animate-spin" />
					{t("loadingPreview", "Loading preview...")}
				</output>
			}
		>
			{viewer.id === "wordcraft" ? (
				<WordcraftDocument {...documentProps} />
			) : (
				<CraftDocument {...documentProps} viewer={viewer} />
			)}
		</Suspense>
	);
}

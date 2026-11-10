"use client";

import { useTranslation } from "@flow-like/locales";
import AppWindow from "lucide-react/dist/esm/icons/app-window.js";
import Camera from "lucide-react/dist/esm/icons/camera.js";
import FileIcon from "lucide-react/dist/esm/icons/file.js";
import FolderIcon from "lucide-react/dist/esm/icons/folder.js";
import HardDrive from "lucide-react/dist/esm/icons/hard-drive.js";
import Loader2 from "lucide-react/dist/esm/icons/loader-2.js";
import MapPin from "lucide-react/dist/esm/icons/map-pin.js";
import Plus from "lucide-react/dist/esm/icons/plus.js";
import ScanBarcode from "lucide-react/dist/esm/icons/scan-barcode.js";
import ScanLine from "lucide-react/dist/esm/icons/scan-line.js";
import ScanQrCode from "lucide-react/dist/esm/icons/scan-qr-code.js";
import ScanText from "lucide-react/dist/esm/icons/scan-text.js";
import type { ComponentType } from "react";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import {
	COMPOSER_ATTACH_KINDS,
	COMPOSER_TEXT_KINDS,
	type ComposerAttachKind,
	type ComposerAttachSource,
	useComposerCaptureStore,
} from "../../../../state/composer-capture";
import { Button } from "../../../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";

const ITEM_CLASS =
	"flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition-colors hover:bg-accent";

const ICONS: Record<
	ComposerAttachKind,
	ComponentType<{ className?: string }>
> = {
	scan_document: ScanLine,
	scan_text: ScanText,
	live_text: ScanQrCode,
	barcode: ScanBarcode,
	take_photo: Camera,
	location: MapPin,
	storage: HardDrive,
	page_snapshot: AppWindow,
};

export interface AttachMenuProps {
	readonly fileUpload: boolean;
	/** Sources of this chat only, e.g. its app's storage; device sources come from the store. */
	readonly sources?: readonly ComposerAttachSource[];
	readonly onFileUpload: (e: React.ChangeEvent<HTMLInputElement>) => void;
	readonly onAddFiles: (files: File[]) => void;
	readonly onInsertText: (text: string) => void;
}

/** Picks one source per kind in menu order; file sources need a chat that takes files. */
export function availableAttachSources(
	sources: readonly ComposerAttachSource[],
	fileUpload: boolean,
): ComposerAttachSource[] {
	return COMPOSER_ATTACH_KINDS.flatMap((kind) => {
		const source = sources.find((candidate) => candidate.kind === kind);
		return source && (fileUpload || COMPOSER_TEXT_KINDS.has(kind))
			? [source]
			: [];
	});
}

/** The composer's "+" menu: device capture, this chat's sources, then files and folders. */
export function AttachMenu({
	fileUpload,
	sources,
	onFileUpload,
	onAddFiles,
	onInsertText,
}: AttachMenuProps) {
	const { t } = useTranslation();
	const deviceSources = useComposerCaptureStore((state) => state.sources);
	const [open, setOpen] = useState(false);
	const [busy, setBusy] = useState(false);

	const available = useMemo(
		() =>
			availableAttachSources(
				[...deviceSources, ...(sources ?? [])],
				fileUpload,
			),
		[deviceSources, sources, fileUpload],
	);

	const label = useCallback(
		(kind: ComposerAttachKind) => {
			switch (kind) {
				case "scan_document":
					return t("scanDocument", "Scan document");
				case "scan_text":
					return t("scanText", "Scan text");
				case "live_text":
					return t("liveTextOrQrCode", "Live text or QR code");
				case "barcode":
					return t("scanBarcodes", "Scan barcodes");
				case "take_photo":
					return t("takePhoto", "Take photo");
				case "location":
					return t("currentLocation", "Current location");
				case "storage":
					return t("fromStorage", "From storage");
				case "page_snapshot":
					return t("attachScreenshot", "Screenshot");
			}
		},
		[t],
	);

	const run = useCallback(
		async (source: ComposerAttachSource) => {
			setOpen(false);
			setBusy(true);
			try {
				const result = await source.attach();
				if (result.files?.length) onAddFiles([...result.files]);
				const text = result.text?.trim();
				if (text) onInsertText(text);
			} catch (error) {
				toast.error(
					t("attachSourceFailedVal", "{{source}} failed: {{val}}", {
						source: label(source.kind),
						val: error instanceof Error ? error.message : String(error),
					}),
				);
			} finally {
				setBusy(false);
			}
		},
		[label, onAddFiles, onInsertText, t],
	);

	if (!fileUpload && available.length === 0) return null;

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<Button
					aria-label={t("addAttachment", "Add attachment")}
					aria-busy={busy}
					disabled={busy}
					type="button"
					size="sm"
					variant="ghost"
					className="h-11 w-11 sm:h-8 sm:w-8 p-0 hover:bg-accent rounded-lg transition-colors"
				>
					{busy ? (
						<Loader2 className="w-4 h-4 animate-spin text-muted-foreground" />
					) : (
						<Plus className="w-4 h-4 text-muted-foreground" />
					)}
				</Button>
			</PopoverTrigger>
			<PopoverContent
				side="top"
				align="start"
				className="mb-2 w-56 rounded-xl p-1.5"
			>
				<div className="space-y-0.5">
					{available.map((source) => {
						const Icon = ICONS[source.kind];
						return (
							<button
								key={source.kind}
								type="button"
								className={ITEM_CLASS}
								onClick={() => void run(source)}
							>
								<Icon className="size-4 shrink-0 text-muted-foreground" />
								{label(source.kind)}
							</button>
						);
					})}
					{available.length > 0 && fileUpload && (
						<hr className="mx-1 my-1 border-border" />
					)}
					{fileUpload && (
						<>
							<input
								type="file"
								id="file-upload"
								className="hidden"
								onChange={onFileUpload}
								multiple
							/>
							<input
								type="file"
								id="folder-upload"
								className="hidden"
								onChange={onFileUpload}
								multiple
								// @ts-ignore
								directory=""
								webkitdirectory=""
							/>
							<label htmlFor="file-upload" className={ITEM_CLASS}>
								<FileIcon className="size-4 shrink-0 text-muted-foreground" />
								{t("files", "Files")}
							</label>
							<label htmlFor="folder-upload" className={ITEM_CLASS}>
								<FolderIcon className="size-4 shrink-0 text-muted-foreground" />
								{t("folder", "Folder")}
							</label>
						</>
					)}
				</div>
			</PopoverContent>
		</Popover>
	);
}

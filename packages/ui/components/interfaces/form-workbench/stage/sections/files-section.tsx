"use client";

import { useTranslation } from "@flow-like/locales";
import { Download, ExternalLink, Image as ImageIcon } from "lucide-react";
import { Suspense, lazy, memo, useState } from "react";
import { cx } from "../../../../settings/devices/primitives/tone";
import { FileTypeIcon } from "../../../../ui/file-type-visuals";
import {
	type ProcessedAttachment,
	splitFileName,
} from "../../../chat-default/attachment";
import type { IAttachment } from "../../../chat-default/chat-db";
import { useProcessedAttachments } from "../../../chat-default/hooks/use-processed-attachments";
import { formatBytes } from "../../run/format";
import { sectionLabel } from "../copy";
import { Section } from "../section-label";

const FileDialog = lazy(() =>
	import("../../../chat-default/attachment-dialog").then((module) => ({
		default: module.FileDialog,
	})),
);

/** A cited page opens in a tab, a file that can be previewed opens the dialog, anything else downloads (the chat's rule). */
async function openFile(
	file: ProcessedAttachment,
	preview: (file: ProcessedAttachment) => void,
) {
	const { canPreviewFile, downloadFile } = await import(
		"../../../chat-default/attachment-dialog"
	);
	if (file.type === "website")
		window.open(file.url, "_blank", "noopener,noreferrer");
	else if (canPreviewFile(file)) preview(file);
	else await downloadFile(file);
}

async function saveFile(file: ProcessedAttachment) {
	const { downloadFile } = await import(
		"../../../chat-default/attachment-dialog"
	);
	await downloadFile(file);
}

/** Saves every file in turn (one prompt each on the desktop app, which asks where to put a file). */
async function saveAll(files: readonly ProcessedAttachment[]) {
	for (const file of files) await saveFile(file);
}

const ICON_BUTTON =
	"inline-flex shrink-0 items-center justify-center rounded-lg text-ink-2 outline-ring hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1";

function Tile({
	file,
	decimalSign,
	onOpen,
}: Readonly<{
	file: ProcessedAttachment;
	decimalSign: "." | ",";
	onOpen: (file: ProcessedAttachment) => void;
}>) {
	const { t } = useTranslation("interfaces");
	const [broken, setBroken] = useState(false);
	const size = formatBytes(file.size, decimalSign);
	return (
		<button
			type="button"
			aria-label={t("workbench.stage.files.open", "Open {{name}}", {
				name: file.displayName,
			})}
			onClick={() => onOpen(file)}
			className="group flex min-w-0 flex-col gap-1.25 rounded-lg text-left outline-ring focus-visible:outline-2 focus-visible:outline-offset-2"
		>
			<span className="flex aspect-[4/3] w-full items-center justify-center overflow-hidden rounded-lg border border-border bg-muted text-muted-foreground group-hover:border-border-strong">
				{broken ? (
					<ImageIcon aria-hidden className="size-5.5" strokeWidth={1.75} />
				) : (
					<img
						src={file.thumbnailUrl ?? file.url}
						alt=""
						loading="lazy"
						onError={() => setBroken(true)}
						className="size-full object-cover"
					/>
				)}
			</span>
			<span className="max-w-full truncate font-mono text-xs/4">
				{file.displayName}
			</span>
			{size ? (
				<span className="font-mono text-muted-foreground text-xs/4 tabular-nums [word-spacing:-0.25em]">
					{size}
				</span>
			) : null}
		</button>
	);
}

function FileRow({
	file,
	first,
	touch,
	decimalSign,
	onOpen,
}: Readonly<{
	file: ProcessedAttachment;
	first: boolean;
	touch: boolean;
	decimalSign: "." | ",";
	onOpen: (file: ProcessedAttachment) => void;
}>) {
	const { t } = useTranslation("interfaces");
	const { stem, suffix } = splitFileName(file.displayName);
	const size = formatBytes(file.size, decimalSign);
	const web = file.type === "website";
	const label = web
		? t("workbench.stage.files.openPage", "Open {{name}}", {
				name: file.displayName,
			})
		: t("workbench.stage.files.save", "Save {{name}}", {
				name: file.displayName,
			});
	return (
		<div
			className={cx(
				"flex items-center gap-2.5 pr-1 pl-3",
				touch ? "min-h-12" : "min-h-10",
				!first && "border-hairline border-t",
			)}
		>
			<FileTypeIcon
				name={file.displayName}
				className="size-4 shrink-0 text-muted-foreground"
			/>
			<button
				type="button"
				title={file.displayName}
				onClick={() => onOpen(file)}
				className="flex min-w-0 flex-1 items-center self-stretch rounded-sm text-left font-mono text-[12.5px]/[18px] outline-ring focus-visible:outline-2 focus-visible:outline-offset-1"
			>
				<span className="min-w-0 truncate">{stem}</span>
				<span className="shrink-0">{suffix}</span>
			</button>
			{size ? (
				<span className="shrink-0 whitespace-nowrap font-mono text-muted-foreground text-xs tabular-nums [word-spacing:-0.25em]">
					{size}
				</span>
			) : null}
			<button
				type="button"
				aria-label={label}
				title={label}
				onClick={() => (web ? onOpen(file) : void saveFile(file))}
				className={cx(ICON_BUTTON, touch ? "size-11" : "size-8")}
			>
				{web ? (
					<ExternalLink aria-hidden className="size-3.75" />
				) : (
					<Download aria-hidden className="size-3.75" />
				)}
			</button>
		</div>
	);
}

/**
 * The files a run sent back: image tiles first, then the other files as rows, with "Save all". A tile or a
 * file that can be previewed opens the chat's file dialog; the rest download (spec "Files", canvas `files`).
 */
export const FilesSection = memo(function FilesSection({
	attachments,
	touch,
	decimalSign,
}: Readonly<{
	attachments: readonly IAttachment[];
	touch: boolean;
	decimalSign: "." | ",";
}>) {
	const { t } = useTranslation("interfaces");
	const files = useProcessedAttachments(attachments as IAttachment[]);
	const [previewing, setPreviewing] = useState<ProcessedAttachment | null>(
		null,
	);
	const tiles = files.filter((file) => file.type === "image");
	const rows = files.filter((file) => file.type !== "image");
	const open = (file: ProcessedAttachment) =>
		void openFile(file, setPreviewing);
	return (
		<Section
			id="files"
			label={sectionLabel(t, "files")}
			aside={
				<>
					<span className="font-mono text-muted-foreground text-xs tabular-nums">
						{files.length}
					</span>
					<span className="flex-1" />
					<button
						type="button"
						onClick={() => void saveAll(files)}
						className={cx(
							"inline-flex items-center gap-1.5 whitespace-nowrap rounded-lg border border-border bg-card px-2.5 font-medium text-[12.5px] outline-ring hover:bg-row-hover focus-visible:outline-2 focus-visible:outline-offset-1",
							touch ? "h-11" : "h-7",
						)}
					>
						<Download aria-hidden className="size-3.25" />
						{t("workbench.stage.files.saveAll", "Save all")}
					</button>
				</>
			}
		>
			{tiles.length > 0 ? (
				<div className="grid max-w-140 grid-cols-[repeat(auto-fill,minmax(132px,1fr))] gap-3">
					{tiles.map((file) => (
						<Tile
							key={file.url}
							file={file}
							decimalSign={decimalSign}
							onOpen={open}
						/>
					))}
				</div>
			) : null}
			{rows.length > 0 ? (
				<div className="overflow-hidden rounded-lg border border-border bg-card">
					{rows.map((file, index) => (
						<FileRow
							key={file.url}
							file={file}
							first={index === 0}
							touch={touch}
							decimalSign={decimalSign}
							onOpen={open}
						/>
					))}
				</div>
			) : null}
			{previewing ? (
				<Suspense fallback={null}>
					<FileDialog
						files={files}
						handleFileClick={open}
						open
						onOpenChange={(next) => {
							if (!next) setPreviewing(null);
						}}
						initialSelectedFile={previewing}
						trigger={null}
					/>
				</Suspense>
			) : null}
		</Section>
	);
});

import { UploadIcon, XIcon } from "lucide-react";
import type { IBulkUploadProgress } from "../../lib/bulk-upload";
import { humanFileSize } from "../../lib/utils";
import { Button, Progress } from "../ui";

export interface IUploadProgressState {
	isUploading: boolean;
	progress: number;
	fileCount: number;
	currentFile: string;
	detail?: IBulkUploadProgress;
}

export const IDLE_UPLOAD_PROGRESS: IUploadProgressState = {
	isUploading: false,
	progress: 0,
	fileCount: 0,
	currentFile: "",
};

/** Compact "time remaining" for the upload panel. */
function formatEta(seconds?: number): string | null {
	if (seconds === undefined || !Number.isFinite(seconds) || seconds <= 0) {
		return null;
	}
	if (seconds < 60) return `${Math.ceil(seconds)}s left`;
	if (seconds < 3600) return `${Math.round(seconds / 60)}m left`;
	return `${(seconds / 3600).toFixed(1)}h left`;
}

export function UploadProgressPanel({
	label,
	state,
	cancelLabel,
	onCancel,
}: Readonly<{
	label: string;
	state: IUploadProgressState;
	cancelLabel: string;
	onCancel: () => void;
}>) {
	const { detail } = state;
	return (
		<div className="mx-4 mt-4 p-4 border rounded-lg bg-card shrink-0">
			<div className="flex items-center justify-between gap-4 mb-2">
				<div className="flex items-center gap-2 min-w-0">
					<UploadIcon className="h-4 w-4 text-primary animate-pulse shrink-0" />
					<span className="text-sm font-medium truncate">{label}</span>
				</div>
				<div className="flex items-center gap-2 shrink-0">
					<span className="text-sm text-muted-foreground tabular-nums">
						{state.progress.toFixed(1)}%
					</span>
					<Button
						variant="ghost"
						size="sm"
						className="h-7 px-2"
						onClick={onCancel}
					>
						<XIcon className="h-3.5 w-3.5 mr-1" />
						{cancelLabel}
					</Button>
				</div>
			</div>
			<Progress value={state.progress} className="mb-2" />
			<div className="flex items-center justify-between gap-4 text-xs text-muted-foreground">
				<p className="truncate">{state.currentFile}</p>
				{detail && (
					<p className="shrink-0 tabular-nums">
						{[
							`${humanFileSize(detail.uploadedBytes)} / ${humanFileSize(detail.totalBytes)}`,
							detail.bytesPerSecond > 0
								? `${humanFileSize(detail.bytesPerSecond)}/s`
								: null,
							formatEta(detail.etaSeconds),
							detail.failedFiles > 0
								? `${detail.failedFiles.toLocaleString()} failed`
								: null,
						]
							.filter(Boolean)
							.join(" · ")}
					</p>
				)}
			</div>
		</div>
	);
}

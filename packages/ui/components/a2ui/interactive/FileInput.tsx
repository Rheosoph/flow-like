"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { File, Folder, Loader2, Upload, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { IBulkUploadProgress } from "../../../lib/bulk-upload";
import {
	type ITemporaryUploadResult,
	uploadTemporaryFilesLocally,
} from "../../../lib/temporary-upload-batch";
import { cn } from "../../../lib/utils";
import type {
	ITemporaryFlowPath,
	ITemporaryUploadExecutionTarget,
} from "../../../state/backend-state";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	useActionContext,
	useComponentEventTrigger,
	useIsComponentTriggering,
	useOnAction,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { firstEventAction } from "../event-handlers";
import type { BoundValue, FileInputComponent } from "../types";
import {
	limitUploadBatch,
	mergeSuccessfulUploadBatch,
} from "./upload-input-state";

/** Rows rendered for a selection; the rest is summarised. Folder picks can be huge. */
const MAX_VISIBLE_FILES = 50;

interface FileData {
	name: string;
	size: number;
	type: string;
	relativePath?: string;
	dataUrl?: string;
	url?: string;
	backendUrl?: string;
	flowPath?: ITemporaryFlowPath;
	uploadId?: string;
	uploading?: boolean;
	uploadError?: string;
}

function toStoredFile(file: FileData): FileData {
	const {
		dataUrl: _dataUrl,
		uploadId: _uploadId,
		uploading: _uploading,
		uploadError: _uploadError,
		...stored
	} = file;
	return stored;
}

function toStoredFileValue(
	value: FileData | FileData[] | null,
): FileData | FileData[] | null {
	if (Array.isArray(value)) return value.map(toStoredFile);
	return value ? toStoredFile(value) : null;
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

/** Folder picks bypass the browser's `accept` filter, so it is applied here. */
function matchesAccept(file: File, accept?: string): boolean {
	const patterns = (accept ?? "")
		.split(",")
		.map((pattern) => pattern.trim().toLowerCase())
		.filter(Boolean);
	if (patterns.length === 0) return true;

	const name = file.name.toLowerCase();
	const type = file.type.toLowerCase();
	return patterns.some((pattern) => {
		if (pattern === "*" || pattern === "*/*") return true;
		if (pattern.startsWith(".")) return name.endsWith(pattern);
		if (pattern.endsWith("/*")) return type.startsWith(pattern.slice(0, -1));
		return type === pattern;
	});
}

function relativePathOf(file: File): string | undefined {
	const relativePath = (file as File & { webkitRelativePath?: string })
		.webkitRelativePath;
	return relativePath ? relativePath : undefined;
}

function formatFileSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024)
		return i18next.t("valKb", "{{val}} KB", { val: (bytes / 1024).toFixed(1) });
	return i18next.t("valMb", "{{val}} MB", {
		val: (bytes / (1024 * 1024)).toFixed(1),
	});
}

function fileNameFromUrl(url: string): string {
	try {
		const parsed = new URL(url, window.location.href);
		const filename = parsed.searchParams.get("filename");
		if (filename) return filename;
	} catch {}

	return url.split("?")[0].split("/").pop() || "file";
}

function normalizeFileValue(value: unknown): FileData[] {
	const values = Array.isArray(value) ? value : value ? [value] : [];
	return values
		.map((item): FileData | null => {
			if (typeof item === "string") {
				return {
					name: fileNameFromUrl(item),
					size: 0,
					type: "",
					url: item,
					backendUrl: item,
				};
			}

			if (item && typeof item === "object") {
				return item as FileData;
			}

			return null;
		})
		.filter((item): item is FileData => item !== null);
}

export function A2UIFileInput({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<FileInputComponent>) {
	const { t } = useTranslation("common");
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const isTriggering = useIsComponentTriggering(componentId);
	const inputRef = useRef<HTMLInputElement>(null);
	const folderInputRef = useRef<HTMLInputElement>(null);
	const backend = useBackend();
	const { appId, resolveTemporaryUploadTarget } = useActionContext();
	const value = useResolved<FileData | FileData[]>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const error = useResolved<boolean>(component.error);
	const label = useResolved<string>(component.label);
	const helperText = useResolved<string>(component.helperText);
	const accept = useResolved<string>(component.accept);
	const multipleProp = useResolved<boolean>(component.multiple);
	const allowDirectory = Boolean(useResolved<boolean>(component.directory));
	const multiple = Boolean(multipleProp) || allowDirectory;
	const maxSize =
		useResolved<number>(component.maxSize) ?? Number.POSITIVE_INFINITY;
	const maxFiles =
		useResolved<number>(component.maxFiles) ?? Number.POSITIVE_INFINITY;
	const { setByPath } = useData();

	const [localFiles, setLocalFiles] = useState<FileData[] | null>(null);
	const [isUploading, setIsUploading] = useState(false);
	const [uploadProgress, setUploadProgress] =
		useState<IBulkUploadProgress | null>(null);
	const uploadOperationRef = useRef(0);
	const isBusy = isUploading || isTriggering;

	const files = normalizeFileValue(value);
	const displayFiles = localFiles ?? files;

	const clearFiles = useCallback(() => {
		uploadOperationRef.current += 1;
		setLocalFiles([]);
		setIsUploading(false);
		if (component.value && "path" in component.value) {
			setByPath(component.value.path, multiple ? [] : null);
		}
	}, [component.value, multiple, setByPath]);

	useEffect(
		() => () => {
			uploadOperationRef.current += 1;
		},
		[],
	);

	useEffect(() => {
		const handleClearFileInput = (event: Event) => {
			const { detail } = event as CustomEvent<{
				surfaceId: string;
				componentId: string;
			}>;
			if (
				detail.surfaceId === surfaceId &&
				detail.componentId === componentId
			) {
				clearFiles();
			}
		};

		window.addEventListener("a2ui:clearFileInput", handleClearFileInput);
		return () => {
			window.removeEventListener("a2ui:clearFileInput", handleClearFileInput);
		};
	}, [surfaceId, componentId, clearFiles]);

	const uploadTemporaryFiles = async (
		files: File[],
		onProgress: (percent: number, detail?: IBulkUploadProgress) => void,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<ITemporaryUploadResult[]> => {
		const helper = backend.helperState;
		if (helper.filesToTemporaryFiles) {
			return helper.filesToTemporaryFiles(files, {
				appId,
				executionTarget,
				onProgress,
			});
		}

		return uploadTemporaryFilesLocally(
			files,
			async (file) =>
				(await helper.fileToTemporaryFile?.(
					file,
					false,
					appId,
					executionTarget,
				)) ?? {
					url: await helper.fileToUrl(file, false, appId, executionTarget),
				},
			{ onProgress },
		);
	};

	const handleFileSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
		const sourceInput = e.target;
		const fromDirectory = sourceInput === folderInputRef.current;
		const selectedFiles = Array.from(sourceInput.files || []).filter(
			(file) => !fromDirectory || matchesAccept(file, accept),
		);
		if (selectedFiles.length === 0) {
			sourceInput.value = "";
			return;
		}

		const currentFiles = displayFiles.filter(
			(file) => !file.uploading && !file.uploadError,
		);
		const validFiles = limitUploadBatch(
			selectedFiles.filter((file) => file.size <= maxSize),
			currentFiles.length,
			Boolean(multiple),
			maxFiles,
		);
		if (validFiles.length === 0) {
			sourceInput.value = "";
			return;
		}

		const operationId = ++uploadOperationRef.current;
		setIsUploading(true);
		setUploadProgress(null);
		const pendingFiles = validFiles.map(
			(file, index): FileData => ({
				name: file.name,
				size: file.size,
				type: file.type,
				relativePath: relativePathOf(file),
				uploadId: `${operationId}-${index}`,
				uploading: true,
			}),
		);
		setLocalFiles(
			multiple ? [...currentFiles, ...pendingFiles] : [pendingFiles[0]],
		);
		const executionTarget = await resolveTemporaryUploadTarget?.(
			firstEventAction(component.eventHandlers, "change", component.actions),
		);
		if (uploadOperationRef.current !== operationId) return;

		// The orchestrator already throttles its progress callback.
		const onProgress = (_percent: number, detail?: IBulkUploadProgress) => {
			if (uploadOperationRef.current !== operationId || !detail) return;
			setUploadProgress(detail);
		};

		const toUploadedFile = (
			pending: FileData,
			result?: ITemporaryUploadResult,
		): FileData => {
			if (!result?.uploaded) {
				return {
					...pending,
					uploading: false,
					uploadId: undefined,
					uploadError: result?.error ?? t("uploadFailed", "Upload failed"),
				};
			}

			return {
				...pending,
				uploading: false,
				uploadId: undefined,
				url: result.uploaded.url,
				backendUrl: result.uploaded.url,
				flowPath: result.uploaded.flowPath,
			};
		};

		let uploadResults: FileData[];
		try {
			const results = await uploadTemporaryFiles(
				validFiles,
				onProgress,
				executionTarget,
			);
			uploadResults = pendingFiles.map((pending, index) =>
				toUploadedFile(pending, results[index]),
			);
		} catch (error) {
			if (uploadOperationRef.current !== operationId) return;
			const message =
				error instanceof Error
					? error.message
					: t("uploadFailed", "Upload failed");
			uploadResults = pendingFiles.map((pending) => ({
				...pending,
				uploading: false,
				uploadId: undefined,
				uploadError: message,
			}));
		}

		if (uploadOperationRef.current !== operationId) return;
		setIsUploading(false);
		setUploadProgress(null);

		const successfulUploads = uploadResults.filter((file) => file.backendUrl);
		const failedUploads = uploadResults.filter((file) => file.uploadError);
		const committedFiles = mergeSuccessfulUploadBatch(
			currentFiles,
			uploadResults,
			Boolean(multiple),
			maxFiles,
			(file) => Boolean(file.backendUrl),
		);
		setLocalFiles(
			multiple ? [...committedFiles, ...failedUploads] : committedFiles,
		);

		if (successfulUploads.length > 0) {
			const newValue = multiple ? committedFiles : committedFiles[0];

			if (component.value && "path" in component.value) {
				setByPath(component.value.path, newValue);
			}

			const urls = successfulUploads.map(
				(f) => (f.url ?? f.backendUrl) as string,
			);

			onAction?.({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: {
					value: toStoredFileValue(newValue),
					signedUrls: multiple ? urls : urls[0],
				},
			});

			await triggerEvent("change", component, {
				signedUrls: multiple ? urls : urls[0],
			});
		}

		sourceInput.value = "";
	};

	const handleRemove = (index: number) => {
		const newFiles = displayFiles.filter((_, i) => i !== index);
		const committedFiles = newFiles.filter(
			(file) => !file.uploading && !file.uploadError,
		);
		const newValue = multiple ? committedFiles : null;

		setLocalFiles(newFiles);

		if (component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}

		const urls = committedFiles
			.map((file) => file.url ?? file.backendUrl)
			.filter(Boolean);

		onAction?.({
			type: "userAction",
			name: "change",
			surfaceId,
			sourceComponentId: componentId,
			timestamp: Date.now(),
			context: {
				value: toStoredFileValue(newValue),
				signedUrls: multiple ? urls : (urls[0] ?? null),
			},
		});

		void triggerEvent("change", component, {
			signedUrls: multiple ? urls : (urls[0] ?? null),
		});
	};

	const zoneClassName = cn(
		"w-full appearance-none bg-transparent border-2 border-dashed rounded-lg p-4 transition-colors",
		disabled || isBusy ? "opacity-50" : "hover:border-primary",
		error ? "border-destructive" : "border-muted-foreground/25",
	);

	const zoneStatus = (
		<div className="flex flex-col items-center gap-2 text-muted-foreground">
			{isBusy ? (
				<>
					<Loader2 className="h-8 w-8 animate-spin" />
					<span className="text-sm">
						{isUploading
							? uploadProgress && uploadProgress.totalFiles > 1
								? t(
										"uploadingCompletedOfTotal",
										"Uploading {{completed}} of {{total}}...",
										{
											completed: uploadProgress.completedFiles,
											total: uploadProgress.totalFiles,
										},
									)
								: t("uploadingFiles", "Uploading files...")
							: t("runningAction", "Running action...")}
					</span>
					{isUploading && uploadProgress && uploadProgress.totalFiles > 1 && (
						<div className="h-1 w-40 overflow-hidden rounded-full bg-muted">
							<div
								className="h-full bg-primary transition-all"
								style={{ width: `${Math.round(uploadProgress.percent)}%` }}
							/>
						</div>
					)}
					{isUploading && uploadProgress && uploadProgress.failedFiles > 0 && (
						<span className="text-xs text-destructive">
							{t("nFailed", "{{n}} failed", { n: uploadProgress.failedFiles })}
						</span>
					)}
				</>
			) : (
				<>
					<Upload className="h-8 w-8" />
					<span className="text-sm">
						{allowDirectory
							? t(
									"pickFilesOrAFolderToUpload",
									"Pick files or a folder to upload",
								)
							: multiple
								? t(
										"dropFilesHereOrClickToBrowse",
										"Drop files here or click to browse",
									)
								: t(
										"dropAFileHereOrClickToBrowse",
										"Drop a file here or click to browse",
									)}
					</span>
					{accept && (
						<span className="text-xs text-muted-foreground/70">
							{t("acceptsAccept", "Accepts: {{accept}}", { accept })}
						</span>
					)}
				</>
			)}
		</div>
	);

	return (
		<div
			data-card-action-stop
			className={cn("space-y-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && (
				<Label className={cn(error && "text-destructive")}>{label}</Label>
			)}

			<Input
				ref={inputRef}
				type="file"
				className="hidden"
				accept={accept}
				multiple={multiple}
				disabled={disabled || isBusy}
				onChange={handleFileSelect}
			/>

			{allowDirectory && (
				<Input
					ref={folderInputRef}
					type="file"
					className="hidden"
					multiple
					// @ts-ignore - webkitdirectory and directory are non-standard attributes
					webkitdirectory=""
					directory=""
					disabled={disabled || isBusy}
					onChange={handleFileSelect}
				/>
			)}

			{allowDirectory ? (
				<div className={cn(zoneClassName, "flex flex-col items-center gap-3")}>
					{zoneStatus}
					{!isBusy && (
						<div className="flex flex-wrap items-center justify-center gap-2">
							<Button
								type="button"
								variant="outline"
								size="sm"
								disabled={disabled}
								onClick={() => inputRef.current?.click()}
							>
								<File className="h-4 w-4" />
								{t("files", "Files")}
							</Button>
							<Button
								type="button"
								variant="outline"
								size="sm"
								disabled={disabled}
								onClick={() => folderInputRef.current?.click()}
							>
								<Folder className="h-4 w-4" />
								{t("folder", "Folder")}
							</Button>
						</div>
					)}
				</div>
			) : (
				<button
					type="button"
					className={cn(
						zoneClassName,
						disabled || isBusy ? "cursor-not-allowed" : "cursor-pointer",
					)}
					onClick={() => !disabled && !isBusy && inputRef.current?.click()}
					disabled={disabled || isBusy}
				>
					{zoneStatus}
				</button>
			)}

			{displayFiles.length > 0 && (
				<div className="space-y-2">
					{displayFiles.slice(0, MAX_VISIBLE_FILES).map((file, index) => (
						<div
							key={`${file.name}-${index}`}
							className={cn(
								"flex items-center gap-2 p-2 bg-muted rounded-md",
								file.uploadError && "border border-destructive",
							)}
						>
							{file.uploading ? (
								<Loader2 className="h-4 w-4 shrink-0 text-muted-foreground animate-spin" />
							) : (
								<File className="h-4 w-4 shrink-0 text-muted-foreground" />
							)}
							<div className="flex-1 min-w-0">
								<p
									className="text-sm font-medium truncate"
									title={file.relativePath ?? file.name}
								>
									{file.relativePath ?? file.name}
								</p>
								<p
									className={cn(
										"text-xs",
										file.uploadError
											? "text-destructive"
											: "text-muted-foreground",
									)}
								>
									{file.uploadError ||
										(file.uploading
											? "Uploading..."
											: formatFileSize(file.size))}
								</p>
							</div>
							<Button
								variant="ghost"
								size="icon"
								className="h-6 w-6 shrink-0"
								onClick={(e) => {
									e.stopPropagation();
									handleRemove(index);
								}}
								disabled={disabled || isBusy || file.uploading}
							>
								<X className="h-4 w-4" />
							</Button>
						</div>
					))}
					{displayFiles.length > MAX_VISIBLE_FILES && (
						<p className="text-xs text-muted-foreground">
							{t("andNMoreFiles", "and {{n}} more files", {
								n: displayFiles.length - MAX_VISIBLE_FILES,
							})}
						</p>
					)}
				</div>
			)}

			{helperText && (
				<p
					className={cn(
						"text-xs",
						error ? "text-destructive" : "text-muted-foreground",
					)}
				>
					{helperText}
				</p>
			)}
		</div>
	);
}

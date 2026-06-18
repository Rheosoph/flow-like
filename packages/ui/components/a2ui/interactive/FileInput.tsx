"use client";

import { File, Loader2, Upload, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import type { ITemporaryFlowPath } from "../../../state/backend-state";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	useActionContext,
	useExecuteAction,
	useIsComponentTriggering,
	useOnAction,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, FileInputComponent } from "../types";

interface FileData {
	name: string;
	size: number;
	type: string;
	dataUrl?: string;
	url?: string;
	backendUrl?: string;
	flowPath?: ITemporaryFlowPath;
	uploading?: boolean;
	uploadError?: string;
}

function toStoredFile(file: FileData): FileData {
	const {
		dataUrl: _dataUrl,
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

function formatFileSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
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
	const onAction = useOnAction();
	const { executeAction } = useExecuteAction();
	const isTriggering = useIsComponentTriggering(componentId);
	const inputRef = useRef<HTMLInputElement>(null);
	const backend = useBackend();
	const { appId } = useActionContext();
	const value = useResolved<FileData | FileData[]>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const error = useResolved<boolean>(component.error);
	const label = useResolved<string>(component.label);
	const helperText = useResolved<string>(component.helperText);
	const accept = useResolved<string>(component.accept);
	const multiple = useResolved<boolean>(component.multiple);
	const maxSize =
		useResolved<number>(component.maxSize) ?? Number.POSITIVE_INFINITY;
	const maxFiles =
		useResolved<number>(component.maxFiles) ?? Number.POSITIVE_INFINITY;
	const { setByPath } = useData();

	const [localFiles, setLocalFiles] = useState<FileData[]>([]);
	const [isUploading, setIsUploading] = useState(false);
	const isBusy = isUploading || isTriggering;

	const files = normalizeFileValue(value);
	const displayFiles = localFiles.length > 0 ? localFiles : files;

	const clearFiles = useCallback(() => {
		setLocalFiles([]);
		if (component.value && "path" in component.value) {
			setByPath(component.value.path, multiple ? [] : null);
		}
	}, [component.value, multiple, setByPath]);

	useEffect(() => {
		const handleClearFileInput = (
			event: CustomEvent<{ surfaceId: string; componentId: string }>,
		) => {
			if (
				event.detail.surfaceId === surfaceId &&
				event.detail.componentId === componentId
			) {
				clearFiles();
			}
		};

		window.addEventListener("a2ui:clearFileInput" as any, handleClearFileInput);
		return () => {
			window.removeEventListener(
				"a2ui:clearFileInput" as any,
				handleClearFileInput,
			);
		};
	}, [surfaceId, componentId, clearFiles]);

	const handleFileSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
		const selectedFiles = Array.from(e.target.files || []);
		if (selectedFiles.length === 0) return;

		const validFiles = selectedFiles.filter((f) => f.size <= maxSize);

		setIsUploading(true);
		const uploadedFiles: FileData[] = [];

		for (const file of validFiles) {
			const fileData: FileData = {
				name: file.name,
				size: file.size,
				type: file.type,
				uploading: true,
			};

			setLocalFiles((prev) => {
				const updated = multiple
					? [...prev, fileData].slice(0, maxFiles)
					: [fileData];
				return updated;
			});

			try {
				const temporaryFile =
					(await backend.helperState.fileToTemporaryFile?.(
						file,
						false,
						appId,
					)) ?? {
						url: await backend.helperState.fileToUrl(file, false, appId),
					};

				const uploadedFile: FileData = {
					name: file.name,
					size: file.size,
					type: file.type,
					url: temporaryFile.url,
					backendUrl: temporaryFile.url,
					flowPath: temporaryFile.flowPath,
					uploading: false,
				};
				uploadedFiles.push(uploadedFile);

				setLocalFiles((prev) =>
					prev.map((f) =>
						f.name === file.name && f.uploading ? uploadedFile : f,
					),
				);
			} catch (err) {
				const errorFile: FileData = {
					name: file.name,
					size: file.size,
					type: file.type,
					uploading: false,
					uploadError: "Upload failed",
				};

				setLocalFiles((prev) =>
					prev.map((f) =>
						f.name === file.name && f.uploading ? errorFile : f,
					),
				);
			}
		}

		setIsUploading(false);

		const successfulUploads = uploadedFiles.filter((f) => f.backendUrl);
		if (successfulUploads.length > 0) {
			const newValue = multiple
				? [...files, ...successfulUploads].slice(0, maxFiles)
				: successfulUploads[0];

			if (component.value && "path" in component.value) {
				setByPath(component.value.path, newValue);
			}

			const urls = successfulUploads.map((f) => (f.url ?? f.backendUrl) as string);

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

			const action = component.actions?.[0];
			if (action) {
				await executeAction(action, componentId, {
					signedUrls: multiple ? urls : urls[0],
				});
			}
		}

		if (inputRef.current) inputRef.current.value = "";
	};

	const handleRemove = (index: number) => {
		const newFiles = displayFiles.filter((_, i) => i !== index);
		const newValue = multiple ? newFiles : null;

		setLocalFiles(newFiles);

		if (component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}

		const urls = newFiles.map((f) => f.url ?? f.backendUrl).filter(Boolean);

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
	};

	return (
		<div
			className={cn("space-y-2", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && (
				<Label className={cn(error && "text-destructive")}>{label}</Label>
			)}

			<div
				className={cn(
					"border-2 border-dashed rounded-lg p-4 transition-colors",
					disabled || isBusy
						? "opacity-50 cursor-not-allowed"
						: "cursor-pointer hover:border-primary",
					error ? "border-destructive" : "border-muted-foreground/25",
				)}
				onClick={() => !disabled && !isBusy && inputRef.current?.click()}
			>
				<Input
					ref={inputRef}
					type="file"
					className="hidden"
					accept={accept}
					multiple={multiple}
					disabled={disabled || isBusy}
					onChange={handleFileSelect}
				/>

				<div className="flex flex-col items-center gap-2 text-muted-foreground">
					{isBusy ? (
						<>
							<Loader2 className="h-8 w-8 animate-spin" />
							<span className="text-sm">
								{isUploading ? "Uploading files..." : "Running action..."}
							</span>
						</>
					) : (
						<>
							<Upload className="h-8 w-8" />
							<span className="text-sm">
								{multiple
									? "Drop files here or click to browse"
									: "Drop a file here or click to browse"}
							</span>
							{accept && (
								<span className="text-xs text-muted-foreground/70">
									Accepts: {accept}
								</span>
							)}
						</>
					)}
				</div>
			</div>

			{displayFiles.length > 0 && (
				<div className="space-y-2">
					{displayFiles.map((file, index) => (
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
								<p className="text-sm font-medium truncate">{file.name}</p>
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

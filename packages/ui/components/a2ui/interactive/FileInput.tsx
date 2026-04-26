"use client";

import { File, Loader2, Upload, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { useOnAction } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, FileInputComponent } from "../types";

interface FileData {
	name: string;
	size: number;
	type: string;
	dataUrl?: string;
	backendUrl?: string;
	uploading?: boolean;
	uploadError?: string;
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

export function A2UIFileInput({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<FileInputComponent>) {
	const onAction = useOnAction();
	const inputRef = useRef<HTMLInputElement>(null);
	const backend = useBackend();
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

	const files: FileData[] = Array.isArray(value) ? value : value ? [value] : [];
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
				const backendUrl = await backend.helperState.fileToUrl(file, false);

				const uploadedFile: FileData = {
					name: file.name,
					size: file.size,
					type: file.type,
					backendUrl,
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

			const urls = successfulUploads.map((f) => f.backendUrl as string);
			const actionValue = multiple ? urls : urls[0];

			onAction?.({
				type: "userAction",
				name: "change",
				surfaceId,
				sourceComponentId: componentId,
				timestamp: Date.now(),
				context: { value: actionValue },
			});
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

		const urls = multiple
			? newFiles.map((f) => f.backendUrl).filter(Boolean)
			: null;

		onAction?.({
			type: "userAction",
			name: "change",
			surfaceId,
			sourceComponentId: componentId,
			timestamp: Date.now(),
			context: { value: urls },
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
					disabled || isUploading
						? "opacity-50 cursor-not-allowed"
						: "cursor-pointer hover:border-primary",
					error ? "border-destructive" : "border-muted-foreground/25",
				)}
				onClick={() => !disabled && !isUploading && inputRef.current?.click()}
			>
				<Input
					ref={inputRef}
					type="file"
					className="hidden"
					accept={accept}
					multiple={multiple}
					disabled={disabled || isUploading}
					onChange={handleFileSelect}
				/>

				<div className="flex flex-col items-center gap-2 text-muted-foreground">
					{isUploading ? (
						<>
							<Loader2 className="h-8 w-8 animate-spin" />
							<span className="text-sm">Uploading files...</span>
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
								disabled={disabled || file.uploading}
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

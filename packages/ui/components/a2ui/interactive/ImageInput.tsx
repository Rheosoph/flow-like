"use client";

import { ImagePlus, Loader2, X } from "lucide-react";
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
import type { BoundValue, ImageInputComponent } from "../types";

interface ImageData {
	name: string;
	size: number;
	type: string;
	dataUrl: string;
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

function readFileAsDataUrl(file: File): Promise<string> {
	return new Promise((resolve, reject) => {
		const reader = new FileReader();
		reader.onload = () => resolve(reader.result as string);
		reader.onerror = reject;
		reader.readAsDataURL(file);
	});
}

export function A2UIImageInput({
	component,
	style,
	componentId,
	surfaceId,
}: ComponentProps<ImageInputComponent>) {
	const onAction = useOnAction();
	const inputRef = useRef<HTMLInputElement>(null);
	const backend = useBackend();
	const value = useResolved<ImageData | ImageData[]>(component.value);
	const disabled = useResolved<boolean>(component.disabled);
	const error = useResolved<boolean>(component.error);
	const label = useResolved<string>(component.label);
	const helperText = useResolved<string>(component.helperText);
	const showPreviewResolved = useResolved<boolean>(component.showPreview);
	const accept = useResolved<string>(component.accept) ?? "image/*";
	const multiple = useResolved<boolean>(component.multiple);
	const maxSize =
		useResolved<number>(component.maxSize) ?? Number.POSITIVE_INFINITY;
	const maxFiles =
		useResolved<number>(component.maxFiles) ?? Number.POSITIVE_INFINITY;
	const aspectRatio = useResolved<string>(component.aspectRatio);
	const { setByPath } = useData();

	const [localImages, setLocalImages] = useState<ImageData[]>([]);
	const [isUploading, setIsUploading] = useState(false);

	const images: ImageData[] = Array.isArray(value)
		? value
		: value
			? [value]
			: [];
	const showPreview = showPreviewResolved !== false;

	const displayImages = localImages.length > 0 ? localImages : images;

	const clearImages = useCallback(() => {
		setLocalImages([]);
		if (component.value && "path" in component.value) {
			setByPath(component.value.path, multiple ? [] : null);
		}
	}, [component.value, setByPath, multiple]);

	useEffect(() => {
		const handleClear = (
			e: CustomEvent<{ surfaceId: string; componentId: string }>,
		) => {
			if (
				e.detail.surfaceId === surfaceId &&
				e.detail.componentId === componentId
			) {
				clearImages();
			}
		};
		window.addEventListener(
			"a2ui:clearFileInput",
			handleClear as EventListener,
		);
		return () => {
			window.removeEventListener(
				"a2ui:clearFileInput",
				handleClear as EventListener,
			);
		};
	}, [surfaceId, componentId, clearImages]);

	const handleFileSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
		const selectedFiles = Array.from(e.target.files || []);
		if (selectedFiles.length === 0) return;

		const validFiles = selectedFiles.filter(
			(f) => f.size <= maxSize && f.type.startsWith("image/"),
		);

		setIsUploading(true);

		const imageDataPromises = validFiles.map(
			async (file): Promise<ImageData> => {
				const dataUrl = await readFileAsDataUrl(file);
				return {
					name: file.name,
					size: file.size,
					type: file.type,
					dataUrl,
					uploading: true,
				};
			},
		);

		const pendingImages = await Promise.all(imageDataPromises);
		const newPending = multiple
			? [
					...displayImages.filter((img) => !img.uploading),
					...pendingImages,
				].slice(0, maxFiles)
			: [pendingImages[0]];
		setLocalImages(newPending);

		const uploadPromises = validFiles.map(
			async (file, index): Promise<ImageData> => {
				const dataUrl = pendingImages[index].dataUrl;
				try {
					const backendUrl = await backend.helperState.fileToUrl(file, false);
					return {
						name: file.name,
						size: file.size,
						type: file.type,
						dataUrl,
						backendUrl,
						uploading: false,
					};
				} catch (err) {
					return {
						name: file.name,
						size: file.size,
						type: file.type,
						dataUrl,
						uploading: false,
						uploadError: err instanceof Error ? err.message : "Upload failed",
					};
				}
			},
		);

		const uploadedImages = await Promise.all(uploadPromises);
		const newValue = multiple
			? [
					...displayImages.filter((img) => !img.uploading),
					...uploadedImages,
				].slice(0, maxFiles)
			: uploadedImages[0];

		setLocalImages(
			Array.isArray(newValue) ? newValue : newValue ? [newValue] : [],
		);
		setIsUploading(false);

		if (component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}

		const successfulUploads = uploadedImages.filter((img) => img.backendUrl);
		const urls = successfulUploads.map((img) => img.backendUrl as string);
		const actionValue = multiple ? urls : urls[0];

		onAction?.({
			type: "userAction",
			name: "change",
			surfaceId,
			sourceComponentId: componentId,
			timestamp: Date.now(),
			context: { value: actionValue },
		});

		if (inputRef.current) inputRef.current.value = "";
	};

	const handleRemove = (index: number) => {
		const newImages = displayImages.filter((_, i) => i !== index);
		const newValue = multiple ? newImages : null;

		setLocalImages(newImages);

		if (component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}

		const urls = multiple
			? newImages.map((img) => img.backendUrl).filter(Boolean)
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

	const renderSingleUpload = () => (
		<div
			className={cn(
				"relative border-2 border-dashed rounded-lg transition-colors overflow-hidden",
				disabled || isUploading
					? "opacity-50 cursor-not-allowed"
					: "cursor-pointer hover:border-primary",
				error ? "border-destructive" : "border-muted-foreground/25",
				aspectRatio ? "" : "aspect-video",
			)}
			style={aspectRatio ? { aspectRatio } : undefined}
			onClick={() => !disabled && !isUploading && inputRef.current?.click()}
		>
			{displayImages[0] && showPreview ? (
				<>
					<img
						src={displayImages[0].dataUrl}
						alt={displayImages[0].name}
						className="absolute inset-0 w-full h-full object-cover"
					/>
					{displayImages[0].uploading ? (
						<div className="absolute inset-0 bg-black/60 flex items-center justify-center">
							<Loader2 className="h-8 w-8 text-white animate-spin" />
						</div>
					) : displayImages[0].uploadError ? (
						<div className="absolute inset-0 bg-destructive/60 flex flex-col items-center justify-center gap-2">
							<p className="text-white text-sm">
								{displayImages[0].uploadError}
							</p>
							<Button
								variant="secondary"
								size="sm"
								onClick={(e) => {
									e.stopPropagation();
									handleRemove(0);
								}}
							>
								<X className="h-4 w-4 mr-1" /> Remove
							</Button>
						</div>
					) : (
						<div className="absolute inset-0 bg-black/40 opacity-0 hover:opacity-100 transition-opacity flex items-center justify-center">
							<Button
								variant="secondary"
								size="sm"
								onClick={(e) => {
									e.stopPropagation();
									handleRemove(0);
								}}
								disabled={disabled}
							>
								<X className="h-4 w-4 mr-1" /> Remove
							</Button>
						</div>
					)}
				</>
			) : (
				<div className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-muted-foreground">
					{isUploading ? (
						<>
							<Loader2 className="h-8 w-8 animate-spin" />
							<span className="text-sm">Uploading...</span>
						</>
					) : (
						<>
							<ImagePlus className="h-8 w-8" />
							<span className="text-sm">Click to upload image</span>
						</>
					)}
				</div>
			)}
		</div>
	);

	const renderMultipleUpload = () => (
		<div className="space-y-3">
			<div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-2">
				{showPreview &&
					displayImages.map((image, index) => (
						<div
							key={`${image.name}-${index}`}
							className={cn(
								"relative aspect-square rounded-lg overflow-hidden border bg-muted group",
								image.uploadError && "border-destructive",
							)}
						>
							<img
								src={image.dataUrl}
								alt={image.name}
								className="w-full h-full object-cover"
							/>
							{image.uploading ? (
								<div className="absolute inset-0 bg-black/60 flex items-center justify-center">
									<Loader2 className="h-6 w-6 text-white animate-spin" />
								</div>
							) : image.uploadError ? (
								<div className="absolute inset-0 bg-destructive/60 flex flex-col items-center justify-center p-2">
									<p className="text-xs text-white text-center mb-2">
										{image.uploadError}
									</p>
									<Button
										variant="secondary"
										size="icon"
										className="h-6 w-6"
										onClick={() => handleRemove(index)}
									>
										<X className="h-4 w-4" />
									</Button>
								</div>
							) : (
								<div className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
									<Button
										variant="secondary"
										size="icon"
										className="h-8 w-8"
										onClick={() => handleRemove(index)}
										disabled={disabled}
									>
										<X className="h-4 w-4" />
									</Button>
								</div>
							)}
							<div className="absolute bottom-0 left-0 right-0 bg-black/60 px-2 py-1 opacity-0 group-hover:opacity-100 transition-opacity">
								<p className="text-xs text-white truncate">{image.name}</p>
								<p className="text-xs text-white/70">
									{image.uploading
										? "Uploading..."
										: formatFileSize(image.size)}
								</p>
							</div>
						</div>
					))}

				{displayImages.length < maxFiles && (
					<div
						className={cn(
							"aspect-square border-2 border-dashed rounded-lg flex flex-col items-center justify-center gap-1 transition-colors",
							disabled || isUploading
								? "opacity-50 cursor-not-allowed"
								: "cursor-pointer hover:border-primary",
							error ? "border-destructive" : "border-muted-foreground/25",
						)}
						onClick={() =>
							!disabled && !isUploading && inputRef.current?.click()
						}
					>
						{isUploading ? (
							<Loader2 className="h-6 w-6 text-muted-foreground animate-spin" />
						) : (
							<>
								<ImagePlus className="h-6 w-6 text-muted-foreground" />
								<span className="text-xs text-muted-foreground">Add</span>
							</>
						)}
					</div>
				)}
			</div>

			{!showPreview && displayImages.length > 0 && (
				<div className="text-sm text-muted-foreground">
					{displayImages.length} image{displayImages.length !== 1 ? "s" : ""}{" "}
					selected
					{displayImages.some((img) => img.uploading) && " (uploading...)"}
				</div>
			)}
		</div>
	);

	return (
		<div
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
				disabled={disabled}
				onChange={handleFileSelect}
			/>

			{multiple ? renderMultipleUpload() : renderSingleUpload()}

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

"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { ImagePlus, Loader2, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../../lib/utils";
import {
	type ITemporaryFlowPath,
	useBackend,
} from "../../../state/backend-state";
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
import type { BoundValue, ImageInputComponent } from "../types";
import {
	limitUploadBatch,
	mergeSuccessfulUploadBatch,
} from "./upload-input-state";

interface ImageData {
	name: string;
	size: number;
	type: string;
	dataUrl?: string;
	url?: string;
	backendUrl?: string;
	flowPath?: ITemporaryFlowPath;
	uploadId?: string;
	uploading?: boolean;
	uploadError?: string;
}

interface ReadableImageFile {
	file: File;
	image: ImageData & { dataUrl: string; uploadId: string; uploading: true };
}

type StoredImageData = Omit<
	ImageData,
	"dataUrl" | "uploadId" | "uploading" | "uploadError"
>;

function toStoredImage(image: ImageData): StoredImageData {
	const {
		dataUrl: _dataUrl,
		uploadId: _uploadId,
		uploading: _uploading,
		uploadError: _uploadError,
		...stored
	} = image;
	return stored;
}

function toStoredImageValue(
	value: ImageData | ImageData[] | null,
): StoredImageData | StoredImageData[] | null {
	if (Array.isArray(value)) return value.map(toStoredImage);
	return value ? toStoredImage(value) : null;
}

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function formatFileSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return i18next.t('valKb', '{{val}} KB', { val: (bytes / 1024).toFixed(1) });
	return i18next.t('valMb', '{{val}} MB', { val: (bytes / (1024 * 1024)).toFixed(1) });
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
	const { t } = useTranslation("common");
	const onAction = useOnAction();
	const triggerEvent = useComponentEventTrigger(componentId);
	const isTriggering = useIsComponentTriggering(componentId);
	const inputRef = useRef<HTMLInputElement>(null);
	const backend = useBackend();
	const { appId, resolveTemporaryUploadTarget } = useActionContext();
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

	const [localImages, setLocalImages] = useState<ImageData[] | null>(null);
	const [isUploading, setIsUploading] = useState(false);
	const uploadOperationRef = useRef(0);
	const isBusy = isUploading || isTriggering;

	const images: ImageData[] = Array.isArray(value)
		? value
		: value
			? [value]
			: [];
	const showPreview = showPreviewResolved !== false;

	const displayImages = localImages ?? images;

	const clearImages = useCallback(() => {
		uploadOperationRef.current += 1;
		setLocalImages([]);
		setIsUploading(false);
		if (component.value && "path" in component.value) {
			setByPath(component.value.path, multiple ? [] : null);
		}
	}, [component.value, setByPath, multiple]);

	useEffect(
		() => () => {
			uploadOperationRef.current += 1;
		},
		[],
	);

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

		const currentImages = displayImages.filter(
			(image) => !image.uploading && !image.uploadError,
		);
		const validFiles = limitUploadBatch(
			selectedFiles.filter(
				(file) => file.size <= maxSize && file.type.startsWith("image/"),
			),
			currentImages.length,
			Boolean(multiple),
			maxFiles,
		);
		if (validFiles.length === 0) {
			if (inputRef.current) inputRef.current.value = "";
			return;
		}

		const operationId = ++uploadOperationRef.current;
		setIsUploading(true);

		const readableFiles = (
			await Promise.all(
				validFiles.map(
					async (file, index): Promise<ReadableImageFile | null> => {
						try {
							return {
								file,
								image: {
									name: file.name,
									size: file.size,
									type: file.type,
									dataUrl: await readFileAsDataUrl(file),
									uploadId: `${operationId}-${index}`,
									uploading: true,
								} satisfies ImageData,
							};
						} catch {
							return null;
						}
					},
				),
			)
		).filter((entry): entry is ReadableImageFile => entry !== null);
		if (uploadOperationRef.current !== operationId) return;
		if (readableFiles.length === 0) {
			setIsUploading(false);
			if (inputRef.current) inputRef.current.value = "";
			return;
		}

		const pendingImages = readableFiles.map((entry) => entry.image);
		setLocalImages(
			multiple ? [...currentImages, ...pendingImages] : [pendingImages[0]],
		);
		const executionTarget = await resolveTemporaryUploadTarget?.(
			firstEventAction(component.eventHandlers, "change", component.actions),
		);
		if (uploadOperationRef.current !== operationId) return;

		const uploadPromises = readableFiles.map(
			async ({ file, image }): Promise<ImageData> => {
				try {
					const temporaryFile =
						(await backend.helperState.fileToTemporaryFile?.(
							file,
							false,
							appId,
							executionTarget,
						)) ?? {
							url: await backend.helperState.fileToUrl(
								file,
								false,
								appId,
								executionTarget,
							),
						};
					return {
						name: file.name,
						size: file.size,
						type: file.type,
						dataUrl: image.dataUrl,
						url: temporaryFile.url,
						backendUrl: temporaryFile.url,
						flowPath: temporaryFile.flowPath,
						uploading: false,
					};
				} catch (err) {
					return {
						name: file.name,
						size: file.size,
						type: file.type,
						dataUrl: image.dataUrl,
						uploading: false,
						uploadError: err instanceof Error ? err.message : t('uploadFailed', 'Upload failed'),
					};
				}
			},
		);

		const uploadedImages = await Promise.all(uploadPromises);
		if (uploadOperationRef.current !== operationId) return;
		setIsUploading(false);

		const successfulUploads = uploadedImages.filter((img) => img.backendUrl);
		const failedUploads = uploadedImages.filter((img) => img.uploadError);
		const committedImages = mergeSuccessfulUploadBatch(
			currentImages,
			uploadedImages,
			Boolean(multiple),
			maxFiles,
			(image) => Boolean(image.backendUrl),
		);
		setLocalImages(
			multiple ? [...committedImages, ...failedUploads] : committedImages,
		);

		if (successfulUploads.length === 0) {
			if (inputRef.current) inputRef.current.value = "";
			return;
		}

		const newValue = multiple ? committedImages : committedImages[0];
		if (component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}

		const urls = successfulUploads.map((img) => img.backendUrl as string);
		const actionValue = multiple ? urls : urls[0];

		onAction?.({
			type: "userAction",
			name: "change",
			surfaceId,
			sourceComponentId: componentId,
			timestamp: Date.now(),
			context: {
				value: toStoredImageValue(newValue),
				signedUrls: actionValue,
			},
		});

		await triggerEvent("change", component, {
			signedUrls: actionValue,
		});

		if (inputRef.current) inputRef.current.value = "";
	};

	const handleRemove = (index: number) => {
		const newImages = displayImages.filter((_, i) => i !== index);
		const committedImages = newImages.filter(
			(image) => !image.uploading && !image.uploadError,
		);
		const newValue = multiple ? committedImages : null;

		setLocalImages(newImages);

		if (component.value && "path" in component.value) {
			setByPath(component.value.path, newValue);
		}

		const urls = multiple
			? committedImages.map((img) => img.backendUrl).filter(Boolean)
			: null;

		onAction?.({
			type: "userAction",
			name: "change",
			surfaceId,
			sourceComponentId: componentId,
			timestamp: Date.now(),
			context: {
				value: toStoredImageValue(newValue),
				signedUrls: multiple ? urls : null,
			},
		});

		void triggerEvent("change", component, {
			signedUrls: multiple ? urls : null,
		});
	};

	const renderSingleUpload = () => {
		const image = displayImages[0];
		return (
			<div
				className={cn(
					"group relative border-2 border-dashed rounded-lg transition-colors overflow-hidden",
					disabled || isBusy
						? "opacity-50 cursor-not-allowed"
						: "cursor-pointer hover:border-primary",
					error ? "border-destructive" : "border-muted-foreground/25",
					aspectRatio ? "" : "aspect-video",
				)}
				style={aspectRatio ? { aspectRatio } : undefined}
			>
				<button
					type="button"
					className="absolute inset-0 h-full w-full appearance-none bg-transparent text-left"
					onClick={() => inputRef.current?.click()}
					disabled={disabled || isBusy}
					aria-label={image ? "Replace image" : t('uploadImage', 'Upload image')}
				>
					{image && showPreview ? (
						<>
							<img
								src={image.dataUrl ?? image.url ?? image.backendUrl}
								alt={image.name}
								className="absolute inset-0 w-full h-full object-cover"
							/>
							{image.uploading || isTriggering ? (
								<div className="absolute inset-0 bg-black/60 flex items-center justify-center">
									<div className="flex flex-col items-center gap-2 text-white">
										<Loader2 className="h-8 w-8 animate-spin" />
										{isTriggering && (
											<span className="text-sm">{t('runningAction', 'Running action...')}</span>
										)}
									</div>
								</div>
							) : null}
						</>
					) : (
						<div className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-muted-foreground">
							{isBusy ? (
								<>
									<Loader2 className="h-8 w-8 animate-spin" />
									<span className="text-sm">
										{isUploading ? "Uploading..." : t('runningAction', 'Running action...')}
									</span>
								</>
							) : (
								<>
									<ImagePlus className="h-8 w-8" />
									<span className="text-sm">{t('clickToUploadImage', 'Click to upload image')}</span>
								</>
							)}
						</div>
					)}
				</button>

				{image && !image.uploading && !isTriggering ? (
					<div
						className={cn(
							"pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-2 transition-opacity",
							image.uploadError
								? "bg-destructive/60"
								: "bg-black/40 opacity-0 group-hover:opacity-100",
						)}
					>
						{image.uploadError ? (
							<p className="text-white text-sm">{image.uploadError}</p>
						) : null}
						<Button
							variant="secondary"
							size="sm"
							className="pointer-events-auto"
							onClick={() => handleRemove(0)}
							disabled={disabled || isBusy}
						>
							<X className="h-4 w-4 mr-1" /> {t('remove', 'Remove')}
						</Button>
					</div>
				) : null}
			</div>
		);
	};

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
								src={image.dataUrl ?? image.url ?? image.backendUrl}
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
										onClick={(event) => {
											event.stopPropagation();
											handleRemove(index);
										}}
										disabled={disabled || isBusy}
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
										onClick={(event) => {
											event.stopPropagation();
											handleRemove(index);
										}}
										disabled={disabled || isBusy}
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
					<button
						type="button"
						className={cn(
							"w-full appearance-none bg-transparent aspect-square border-2 border-dashed rounded-lg flex flex-col items-center justify-center gap-1 transition-colors",
							disabled || isBusy
								? "opacity-50 cursor-not-allowed"
								: "cursor-pointer hover:border-primary",
							error ? "border-destructive" : "border-muted-foreground/25",
						)}
						onClick={() => !disabled && !isBusy && inputRef.current?.click()}
						disabled={disabled || isBusy}
					>
						{isBusy ? (
							<div className="flex flex-col items-center gap-1 text-muted-foreground">
								<Loader2 className="h-6 w-6 animate-spin" />
								{isTriggering && <span className="text-xs">{t('running', 'Running')}</span>}
							</div>
						) : (
							<>
								<ImagePlus className="h-6 w-6 text-muted-foreground" />
								<span className="text-xs text-muted-foreground">{t('add', 'Add')}</span>
							</>
						)}
					</button>
				)}
			</div>

			{!showPreview && displayImages.length > 0 && (
				<div className="text-sm text-muted-foreground">{t('lengthImage', '{{length}} image', { length: displayImages.length })}{displayImages.length !== 1 ? "s" : ""}{" "}
					selected
					{displayImages.some((img) => img.uploading) && " (uploading...)"}
				</div>
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

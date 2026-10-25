import { createId } from "@paralleldrive/cuid2";
import type { Node } from "@xyflow/react";
import { useCallback, useRef, useState } from "react";
import { upsertCommentCommand } from "../lib/command/generic-command";
import { type IComment, ICommentType } from "../lib/schema/flow/board";
import type { IGenericCommand } from "../lib/schema/flow/board/commands/generic-command";
import type { IBackendState } from "../state/backend-state";

export interface MediaUploadResult {
	mediaRef: string;
	commentType: ICommentType;
	width?: number;
	height?: number;
}

interface UseMediaUploadOptions {
	appId: string;
	boardId: string;
	backend: IBackendState;
	executeCommand: (
		command: IGenericCommand,
		append?: boolean,
	) => Promise<unknown>;
	currentLayer?: string;
	setNodes?: React.Dispatch<React.SetStateAction<Node[]>>;
}

const SUPPORTED_IMAGE_TYPES = [
	"image/png",
	"image/jpeg",
	"image/gif",
	"image/webp",
	"image/svg+xml",
];

const SUPPORTED_VIDEO_TYPES = [
	"video/mp4",
	"video/webm",
	"video/ogg",
	"video/quicktime",
];

function getMediaTypeFromMime(mimeType: string): ICommentType | null {
	if (SUPPORTED_IMAGE_TYPES.includes(mimeType)) {
		return ICommentType.Image;
	}
	if (SUPPORTED_VIDEO_TYPES.includes(mimeType)) {
		return ICommentType.Video;
	}
	return null;
}

function getExtensionFromMime(mimeType: string): string {
	const mimeToExt: Record<string, string> = {
		"image/png": "png",
		"image/jpeg": "jpg",
		"image/gif": "gif",
		"image/webp": "webp",
		"image/svg+xml": "svg",
		"video/mp4": "mp4",
		"video/webm": "webm",
		"video/ogg": "ogg",
		"video/quicktime": "mov",
	};
	return mimeToExt[mimeType] ?? "bin";
}

async function getImageDimensions(
	file: File,
): Promise<{ width: number; height: number }> {
	return new Promise((resolve, reject) => {
		const img = new Image();
		img.onload = () => {
			resolve({ width: img.naturalWidth, height: img.naturalHeight });
			URL.revokeObjectURL(img.src);
		};
		img.onerror = () => {
			reject(new Error("Failed to load image"));
			URL.revokeObjectURL(img.src);
		};
		img.src = URL.createObjectURL(file);
	});
}

async function getVideoDimensions(
	file: File,
): Promise<{ width: number; height: number }> {
	return new Promise((resolve, reject) => {
		const video = document.createElement("video");
		video.preload = "metadata";
		video.onloadedmetadata = () => {
			resolve({ width: video.videoWidth, height: video.videoHeight });
			URL.revokeObjectURL(video.src);
		};
		video.onerror = () => {
			reject(new Error("Failed to load video"));
			URL.revokeObjectURL(video.src);
		};
		video.src = URL.createObjectURL(file);
	});
}

export function useMediaUpload({
	appId,
	boardId,
	backend,
	executeCommand,
	currentLayer,
	setNodes,
}: UseMediaUploadOptions) {
	const [isUploading, setIsUploading] = useState(false);
	const [uploadProgress, setUploadProgress] = useState(0);
	const placeholderIdRef = useRef<string | null>(null);

	const addPlaceholderNode = useCallback(
		(
			position: { x: number; y: number },
			mediaType: ICommentType,
			dimensions: { width: number; height: number },
		) => {
			if (!setNodes) return null;
			const placeholderId = `__upload_placeholder_${createId()}`;
			placeholderIdRef.current = placeholderId;

			setNodes((nodes) => [
				...nodes,
				{
					id: placeholderId,
					type: "uploadPlaceholderNode",
					position,
					width: dimensions.width,
					height: dimensions.height,
					data: {
						mediaType,
						progress: 0,
					},
					draggable: false,
					selectable: false,
				},
			]);

			return placeholderId;
		},
		[setNodes],
	);

	const updatePlaceholderProgress = useCallback(
		(progress: number) => {
			setUploadProgress(progress);
			if (!setNodes || !placeholderIdRef.current) return;

			setNodes((nodes) =>
				nodes.map((node) =>
					node.id === placeholderIdRef.current
						? { ...node, data: { ...node.data, progress } }
						: node,
				),
			);
		},
		[setNodes],
	);

	const removePlaceholderNode = useCallback(() => {
		if (!setNodes || !placeholderIdRef.current) return;
		const id = placeholderIdRef.current;
		placeholderIdRef.current = null;

		setNodes((nodes) => nodes.filter((node) => node.id !== id));
	}, [setNodes]);

	const uploadMedia = useCallback(
		async (
			file: File,
			position?: { x: number; y: number },
		): Promise<MediaUploadResult | null> => {
			const mediaType = getMediaTypeFromMime(file.type);
			if (!mediaType) {
				console.warn("Unsupported media type:", file.type);
				return null;
			}

			const uid = createId();
			const extension = getExtensionFromMime(file.type);
			const fileName = `${uid}.${extension}`;
			const storagePath = `boards/${boardId}`;

			// Create a new File with the uid-based name
			const renamedFile = new File([file], fileName, { type: file.type });

			// Get dimensions
			let dimensions = { width: 400, height: 300 };
			try {
				if (mediaType === ICommentType.Image) {
					dimensions = await getImageDimensions(file);
				} else if (mediaType === ICommentType.Video) {
					dimensions = await getVideoDimensions(file);
				}
			} catch {
				// Use defaults
			}

			// Scale to reasonable size while maintaining aspect ratio
			const maxWidth = 800;
			const maxHeight = 600;
			if (dimensions.width > maxWidth || dimensions.height > maxHeight) {
				const scale = Math.min(
					maxWidth / dimensions.width,
					maxHeight / dimensions.height,
				);
				dimensions = {
					width: Math.round(dimensions.width * scale),
					height: Math.round(dimensions.height * scale),
				};
			}

			// Add placeholder node if position is provided
			if (position) {
				addPlaceholderNode(position, mediaType, dimensions);
			}

			// Upload to storage
			await backend.storageState.uploadStorageItems(
				appId,
				storagePath,
				[renamedFile],
				updatePlaceholderProgress,
			);

			return {
				mediaRef: fileName,
				commentType: mediaType,
				width: dimensions.width,
				height: dimensions.height,
			};
		},
		[
			appId,
			boardId,
			backend.storageState,
			addPlaceholderNode,
			updatePlaceholderProgress,
		],
	);

	const createMediaComment = useCallback(
		async (
			file: File,
			position: { x: number; y: number },
		): Promise<IComment | null> => {
			setIsUploading(true);
			setUploadProgress(0);

			try {
				const result = await uploadMedia(file, position);
				if (!result) {
					removePlaceholderNode();
					return null;
				}

				const comment: IComment = {
					id: createId(),
					comment_type: result.commentType,
					content: result.mediaRef,
					coordinates: [position.x, position.y, 0],
					width: result.width,
					height: result.height,
					timestamp: {
						nanos_since_epoch: 0,
						secs_since_epoch: Date.now() * 1_000_000,
					},
					layer: currentLayer ?? null,
				};

				const command = upsertCommentCommand({
					comment,
					current_layer: currentLayer,
				});

				// Remove placeholder before adding real node
				removePlaceholderNode();

				await executeCommand(command);
				return comment;
			} catch (error) {
				removePlaceholderNode();
				throw error;
			} finally {
				setIsUploading(false);
				setUploadProgress(0);
			}
		},
		[uploadMedia, executeCommand, currentLayer, removePlaceholderNode],
	);

	const handleMediaPaste = useCallback(
		async (
			event: ClipboardEvent,
			position: { x: number; y: number },
		): Promise<boolean> => {
			const items = event.clipboardData?.items;
			if (!items) return false;

			// Use index-based iteration for DataTransferItemList
			for (let i = 0; i < items.length; i++) {
				const item = items[i];
				if (!item) continue;

				const mediaType = getMediaTypeFromMime(item.type);
				if (mediaType) {
					const file = item.getAsFile();
					if (file) {
						event.preventDefault();
						event.stopPropagation();
						await createMediaComment(file, position);
						return true;
					}
				}
			}

			return false;
		},
		[createMediaComment],
	);

	const handleMediaFilePick = useCallback(
		async (position: { x: number; y: number }): Promise<void> => {
			const input = document.createElement("input");
			input.type = "file";
			input.accept = [...SUPPORTED_IMAGE_TYPES, ...SUPPORTED_VIDEO_TYPES].join(
				",",
			);
			input.multiple = false;

			input.onchange = async () => {
				const file = input.files?.[0];
				if (file) {
					await createMediaComment(file, position);
				}
			};

			input.click();
		},
		[createMediaComment],
	);

	return {
		isUploading,
		uploadProgress,
		uploadMedia,
		createMediaComment,
		handleMediaPaste,
		handleMediaFilePick,
		supportedImageTypes: SUPPORTED_IMAGE_TYPES,
		supportedVideoTypes: SUPPORTED_VIDEO_TYPES,
	};
}

"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type Node,
	type NodeProps,
	NodeResizer,
	type ResizeDragEvent,
	type ResizeParams,
	useReactFlow,
} from "@xyflow/react";
import {
	AlertCircleIcon,
	ImageIcon,
	Loader2Icon,
	LockIcon,
	UnlockIcon,
	VideoIcon,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { IComment } from "../../lib/schema/flow/board";
import { ICommentType } from "../../lib/schema/flow/board";
import { Button } from "../ui";
import { MediaNodeToolbar } from "./media-node/media-node-toolbar";

export type MediaNode = Node<
	{
		comment: IComment;
		presignedUrl?: string;
		onUpsert: (comment: IComment) => Promise<void>;
		boardId: string;
		appId: string;
		hash: string;
	},
	"mediaNode"
>;

export function MediaNode(props: NodeProps<MediaNode>) {
	const { t } = useTranslation("flow");
	const { getNodes, setNodes } = useReactFlow();
	const [isHovered, setIsHovered] = useState(false);
	const [isLoading, setIsLoading] = useState(true);
	const [hasError, setHasError] = useState(false);

	const isLocked = props.data.comment.is_locked ?? false;
	const presignedUrl = props.data.presignedUrl;
	const isImage = props.data.comment.comment_type === ICommentType.Image;
	const isVideo = props.data.comment.comment_type === ICommentType.Video;

	const toggleLock = useCallback(async () => {
		const next = !isLocked;
		const node = getNodes().find((n) => n.id === props.id);
		if (node) {
			const comment = node.data.comment as IComment;
			try {
				await props.data.onUpsert({
					...comment,
					is_locked: next,
				});
			} catch {
				// noop
			}
		}
	}, [getNodes, isLocked, props.data.onUpsert, props.id]);

	const onResizeEnd = useCallback(
		async (_event: ResizeDragEvent, params: ResizeParams) => {
			const node = getNodes().find((n) => n.id === props.id);
			if (!node) return;
			const comment = node.data.comment as IComment;
			props.data.onUpsert({
				...comment,
				coordinates: [params.x, params.y, props.data.comment.coordinates[2]],
				width: params.width,
				height: params.height,
			});
		},
		[props.data.comment, props.data.onUpsert, getNodes, props.id],
	);

	const onMoveLayer = useCallback(
		async (by: number) => {
			const node = getNodes().find((n) => n.id === props.id);
			if (!node) return;
			const comment = node.data.comment as IComment;
			props.data.onUpsert({
				...comment,
				z_index: (props.data.comment.z_index ?? 1) + by,
			});
		},
		[props.data.comment, props.data.onUpsert, getNodes, props.id],
	);

	// Reset loading state when URL changes
	useEffect(() => {
		if (presignedUrl) {
			setIsLoading(true);
			setHasError(false);
		}
	}, [presignedUrl]);

	const handleLoad = useCallback(() => {
		setIsLoading(false);
		setHasError(false);
	}, []);

	const handleError = useCallback(() => {
		setIsLoading(false);
		setHasError(true);
	}, []);

	const renderMediaContent = () => {
		if (!presignedUrl) {
			return (
				<div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-2">
					{isImage ? (
						<ImageIcon className="w-8 h-8" />
					) : (
						<VideoIcon className="w-8 h-8" />
					)}
					<span className="text-xs">{t('mediaNotAvailable', 'Media not available')}</span>
				</div>
			);
		}

		if (hasError) {
			return (
				<div className="flex flex-col items-center justify-center h-full text-destructive gap-2">
					<AlertCircleIcon className="w-8 h-8" />
					<span className="text-xs">{t('failedToLoadMedia', 'Failed to load media')}</span>
				</div>
			);
		}

		if (isImage) {
			return (
				<>
					{isLoading && (
						<div className="absolute inset-0 flex items-center justify-center bg-muted/50">
							<Loader2Icon className="w-6 h-6 animate-spin text-muted-foreground" />
						</div>
					)}
					<img
						src={presignedUrl}
						alt={props.data.comment.content}
						className="w-full h-full object-contain pointer-events-none"
						crossOrigin="anonymous"
						onLoad={handleLoad}
						onError={handleError}
						style={{ opacity: isLoading ? 0 : 1 }}
						draggable={false}
					/>
				</>
			);
		}

		if (isVideo) {
			return (
				<>
					{isLoading && (
						<div className="absolute inset-0 flex items-center justify-center bg-muted/50">
							<Loader2Icon className="w-6 h-6 animate-spin text-muted-foreground" />
						</div>
					)}
					<video
						src={presignedUrl}
						className="w-full h-full object-contain"
						crossOrigin="anonymous"
						controls
						onLoadedData={handleLoad}
						onError={handleError}
						style={{ opacity: isLoading ? 0 : 1 }}
					/>
				</>
			);
		}

		return null;
	};

	return (
		<>
			<NodeResizer
				color="#ff0071"
				handleStyle={{
					width: 10,
					height: 10,
					zIndex: (props.data.comment.z_index ?? 1) + 1,
				}}
				isVisible={!isLocked && props.selected}
				onResizeEnd={onResizeEnd}
				minWidth={100}
				minHeight={100}
			/>
			<div
				className="relative w-full h-full"
				onMouseEnter={() => setIsHovered(true)}
				onMouseLeave={() => setIsHovered(false)}
			>
				{(props.selected || isHovered) && (
					<MediaNodeToolbar
						isLocked={isLocked}
						onMoveUp={() => onMoveLayer(1)}
						onMoveDown={() => onMoveLayer(-1)}
						onToggleLock={toggleLock}
					/>
				)}
				<div
					key={`${props.id}__media-node`}
					className={`bg-card p-1 react-flow__node-default w-full! h-full! focus:ring-2 relative rounded-md! border-0! group overflow-hidden ${
						props.selected ? "ring-2 ring-primary" : ""
					} ${isLocked ? "cursor-not-allowed" : ""}`}
				>
					<div className="absolute top-1 right-1 z-50 opacity-0 group-hover:opacity-100 transition-opacity duration-200">
						<Button
							variant="secondary"
							size="icon"
							title={isLocked ? "Unlock media" : "Lock media"}
							onClick={(e) => {
								e.preventDefault();
								e.stopPropagation();
								toggleLock();
							}}
							className="h-6 w-6"
						>
							{isLocked ? (
								<LockIcon className="w-3.5 h-3.5" />
							) : (
								<UnlockIcon className="w-3.5 h-3.5" />
							)}
						</Button>
					</div>
					<div className="w-full h-full relative">{renderMediaContent()}</div>
				</div>
			</div>
		</>
	);
}

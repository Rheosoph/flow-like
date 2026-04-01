"use client";

import { Download, FileText, Music, ZoomIn } from "lucide-react";
import type { IBackendState } from "../../../state/backend-state";
import { Button } from "../../ui";
import type { IAttachment } from "./chat-db";

export async function fileToAttachment(
	files: File[],
	backend: IBackendState,
	offline: boolean,
): Promise<IAttachment[]> {
	if (!files || files.length === 0) return [];

	const attachments: IAttachment[] = [];

	for (const file of files) {
		const url = await backend.helperState.fileToUrl(file, offline);
		attachments.push({
			name: file.name,
			type: file.type,
			size: file.size,
			url: url,
		});
	}

	return attachments;
}

export interface ProcessedAttachment {
	url: string;
	name: string;
	type: "image" | "video" | "audio" | "pdf" | "document" | "website" | "other";
	pageNumber?: number;
	isDataUrl: boolean;
	thumbnailUrl?: string;
	previewText?: string;
	size?: number;
	anchor?: string;
}

interface FilePreviewProps {
	file: ProcessedAttachment;
	showFullscreenButton?: boolean;
	onFullscreen?: (file: ProcessedAttachment) => void;
	onClick?: (file: ProcessedAttachment) => void;
	/** When true, images use a fixed aspect ratio for uniform grid layout */
	inGrid?: boolean;
}

export function FilePreview({
	file,
	showFullscreenButton = false,
	onFullscreen,
	onClick,
	inGrid = false,
}: FilePreviewProps) {
	const handleClick = () => {
		if (showFullscreenButton && onFullscreen) {
			onFullscreen(file);
		} else if (onClick) {
			onClick(file);
		}
	};

	const handleFileClick = (file: ProcessedAttachment) => {
		if (file.isDataUrl) {
			if (file.type === "image") {
				const newWindow = window.open();
				if (newWindow) {
					newWindow.document.write(
						`<img src="${file.url}" style="max-width: 100%; height: auto;" />`,
					);
				}
			} else {
				const link = document.createElement("a");
				link.href = file.url;
				link.download = file.name || "file";
				document.body.appendChild(link);
				link.click();
				document.body.removeChild(link);
			}
		} else {
			window.open(file.url, "_blank", "noopener,noreferrer");
		}
	};

	const getFileIcon = (type: ProcessedAttachment["type"]) => {
		switch (type) {
			case "image":
				return <FileText className="w-4 h-4" />;
			case "video":
				return <FileText className="w-4 h-4" />;
			case "audio":
				return <Music className="w-4 h-4" />;
			case "pdf":
				return <FileText className="w-4 h-4" />;
			case "document":
				return <FileText className="w-4 h-4" />;
			case "website":
				return <FileText className="w-4 h-4" />;
			default:
				return <Download className="w-4 h-4" />;
		}
	};

	switch (file.type) {
		case "image":
			return (
				<div
					key={file.url}
					className={`relative group rounded-md overflow-hidden border bg-muted/50 ${inGrid ? "aspect-4/3" : "max-w-screen-sm"}`}
				>
					<img
						src={file.thumbnailUrl || file.url}
						alt={file.name}
						className={`cursor-pointer hover:opacity-90 transition-opacity ${inGrid ? "w-full h-full object-cover" : "w-full h-auto object-cover"}`}
						onClick={handleClick}
						onError={(e) => {
							const target = e.target as HTMLImageElement;
							if (file.thumbnailUrl && target.src === file.thumbnailUrl) {
								target.src = file.url;
							} else {
								target.style.display = "none";
							}
						}}
					/>
					{showFullscreenButton && (
						<div className="absolute inset-0 bg-black/0 group-hover:bg-black/20 transition-colors flex items-center justify-center">
							<Button
								variant="secondary"
								size="sm"
								className="opacity-0 group-hover:opacity-100 transition-opacity h-6 w-6 p-0"
								onClick={(e) => {
									e.stopPropagation();
									if (onFullscreen) onFullscreen(file);
								}}
							>
								<ZoomIn className="w-3 h-3" />
							</Button>
						</div>
					)}
					<div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/70 to-transparent text-white text-xs p-1 truncate">
						{decodeURIComponent(file.name)}
					</div>
				</div>
			);
		case "video":
			return (
				<div
					key={file.url}
					className="relative rounded-md overflow-hidden border bg-muted/50 max-w-md aspect-video"
				>
					<video
						controls
						className="w-full h-full object-cover"
						preload="metadata"
						poster={file.thumbnailUrl}
					>
						<source src={file.url} />
						Your browser does not support the video tag.
					</video>
				</div>
			);
		case "audio":
			return (
				<div
					key={file.url}
					className="rounded-full my-4 border bg-muted/50 p-3 min-w-80 max-w-md w-full"
				>
					<audio controls className="w-full h-10">
						<source src={file.url} />
						Your browser does not support the audio tag.
					</audio>
				</div>
			);
		default:
			return null;
	}
}

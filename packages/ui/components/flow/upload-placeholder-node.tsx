"use client";

import { useTranslation } from "@flow-like/locales";
import type { Node, NodeProps } from "@xyflow/react";
import { ImageIcon, Loader2Icon, VideoIcon } from "lucide-react";
import { ICommentType } from "../../lib/schema/flow/board";

export type UploadPlaceholderNode = Node<
	{
		mediaType: ICommentType;
		progress: number;
	},
	"uploadPlaceholderNode"
>;

export function UploadPlaceholderNode(props: NodeProps<UploadPlaceholderNode>) {
	const { t } = useTranslation("flow");
	const { mediaType, progress } = props.data;
	const isImage = mediaType === ICommentType.Image;

	return (
		<div
			className="flex flex-col items-center justify-center bg-muted/50 border-2 border-dashed border-primary/50 rounded-lg overflow-hidden"
			style={{
				width: props.width ?? 400,
				height: props.height ?? 300,
			}}
		>
			<div className="flex flex-col items-center gap-3 p-4">
				<div className="relative">
					{isImage ? (
						<ImageIcon className="w-10 h-10 text-primary/70" />
					) : (
						<VideoIcon className="w-10 h-10 text-primary/70" />
					)}
					<Loader2Icon className="w-6 h-6 text-primary animate-spin absolute -bottom-1 -right-1" />
				</div>
				<div className="text-center">
					<p className="text-sm font-medium text-foreground/80">
						{t('uploading', 'Uploading')} {isImage ? "image" : "video"}{`...`}
					</p>
					<p className="text-xs text-muted-foreground">
						{Math.round(progress)}%
					</p>
				</div>
				<div className="w-32 h-1.5 bg-muted rounded-full overflow-hidden">
					<div
						className="h-full bg-primary transition-all duration-200 ease-out"
						style={{ width: `${progress}%` }}
					/>
				</div>
			</div>
		</div>
	);
}

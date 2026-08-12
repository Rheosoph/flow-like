"use client";
import {
	FoldHorizontalIcon,
	MessageSquareIcon,
	SlidersHorizontalIcon,
	SparklesIcon,
	SquareFunctionIcon,
	SquarePenIcon,
	Trash2Icon,
} from "lucide-react";
import { memo } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

interface LayerNodeToolbarProps {
	onRename: () => void;
	onComment: () => void;
	onEdit: () => void;
	onExtend: () => void;
	onDelete: () => void;
	onExplain: () => void;
	onConvertToFunction?: () => void;
}

export const ToolbarButton = memo(
	({
		onClick,
		icon: Icon,
		tooltip,
		variant = "default",
	}: {
		onClick: (e?: React.MouseEvent<HTMLButtonElement>) => void;
		icon: React.ComponentType<{ className?: string }>;
		tooltip: string;
		variant?: "default" | "destructive";
	}) => (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					className={`h-5 w-5 flex items-center justify-center rounded transition-colors ${
						variant === "destructive"
							? "hover:bg-red-500/20 text-red-400 hover:text-red-300"
							: "hover:bg-white/10"
					}`}
					onClick={(e) => {
						e.stopPropagation();
						onClick(e);
					}}
				>
					<Icon className="h-3 w-3" />
				</button>
			</TooltipTrigger>
			<TooltipContent side="top" className="text-[10px] px-1.5 py-0.5">
				{tooltip}
			</TooltipContent>
		</Tooltip>
	),
);

ToolbarButton.displayName = "ToolbarButton";

const Divider = memo(() => <div className="w-px h-3 bg-white/20 mx-0.5" />);

Divider.displayName = "Divider";

const LayerNodeToolbar = memo(
	({
		onRename,
		onComment,
		onEdit,
		onExtend,
		onDelete,
		onExplain,
		onConvertToFunction,
	}: LayerNodeToolbarProps) => {
		return (
			<>
				{/* Invisible bridge to maintain hover */}
				<div className="absolute -top-3 left-0 right-0 h-3" />
				<div
					className="absolute -top-9 left-1/2 -translate-x-1/2 z-50 flex items-center gap-0.5
						bg-zinc-900 text-zinc-100
						rounded-full shadow-lg shadow-black/25
						px-1.5 py-1 border border-white/10
						animate-in fade-in-0 zoom-in-95 duration-150"
					onClick={(e) => e.stopPropagation()}
					onMouseDown={(e) => e.stopPropagation()}
				>
					<ToolbarButton
						onClick={onRename}
						icon={SquarePenIcon}
						tooltip="Rename"
					/>
					<ToolbarButton
						onClick={onComment}
						icon={MessageSquareIcon}
						tooltip="Comment"
					/>
					<ToolbarButton
						onClick={onEdit}
						icon={SlidersHorizontalIcon}
						tooltip="Edit"
					/>
					{onConvertToFunction && (
						<ToolbarButton
							onClick={onConvertToFunction}
							icon={SquareFunctionIcon}
							tooltip="Convert to Function"
						/>
					)}
					<Divider />
					<ToolbarButton
						onClick={onExplain}
						icon={SparklesIcon}
						tooltip="Explain with FlowPilot"
					/>
					<ToolbarButton
						onClick={onExtend}
						icon={FoldHorizontalIcon}
						tooltip="Extend (Ungroup)"
					/>
					<ToolbarButton
						onClick={onDelete}
						icon={Trash2Icon}
						tooltip="Delete"
						variant="destructive"
					/>
				</div>
			</>
		);
	},
);

LayerNodeToolbar.displayName = "LayerNodeToolbar";

export { LayerNodeToolbar };

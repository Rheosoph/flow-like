"use client";
import { useTranslation } from "@flow-like/locales";
import {
	FileCode2Icon,
	LockIcon,
	SquareChevronDownIcon,
	SquareChevronUpIcon,
	SquarePenIcon,
	UnlockIcon,
} from "lucide-react";
import { memo } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

interface CommentNodeToolbarProps {
	isLocked: boolean;
	onEdit: () => void;
	onMoveUp: () => void;
	onMoveDown: () => void;
	onToggleLock: () => void;
	/** Anchored comments only: reveal the bound statement in the FlowScript panel. */
	onGoToCode?: () => void;
}

const ToolbarButton = memo(
	({
		onClick,
		icon: Icon,
		tooltip,
	}: {
		onClick: () => void;
		icon: React.ComponentType<{ className?: string }>;
		tooltip: string;
	}) => (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					className="h-5 w-5 flex items-center justify-center rounded transition-colors hover:bg-white/10"
					onClick={(e) => {
						e.stopPropagation();
						onClick();
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

const CommentNodeToolbar = memo(
	({
		isLocked,
		onEdit,
		onMoveUp,
		onMoveDown,
		onToggleLock,
		onGoToCode,
	}: CommentNodeToolbarProps) => {
		const { t } = useTranslation("flow");
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
					<ToolbarButton onClick={onEdit} icon={SquarePenIcon} tooltip="Edit" />
					{onGoToCode && (
						<ToolbarButton
							onClick={onGoToCode}
							icon={FileCode2Icon}
							tooltip={t("flowscriptGoToCode", "Go to code")}
						/>
					)}
					<Divider />
					<ToolbarButton
						onClick={onMoveUp}
						icon={SquareChevronUpIcon}
						tooltip="Move Up"
					/>
					<ToolbarButton
						onClick={onMoveDown}
						icon={SquareChevronDownIcon}
						tooltip="Move Down"
					/>
					<Divider />
					<ToolbarButton
						onClick={onToggleLock}
						icon={isLocked ? UnlockIcon : LockIcon}
						tooltip={isLocked ? "Unlock" : "Lock"}
					/>
				</div>
			</>
		);
	},
);

CommentNodeToolbar.displayName = "CommentNodeToolbar";

export { CommentNodeToolbar };

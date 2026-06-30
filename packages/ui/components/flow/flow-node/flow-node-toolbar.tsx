"use client";
import { InfoCircledIcon } from "@radix-ui/react-icons";
import { useStore } from "@xyflow/react";
import {
	AlignCenterVerticalIcon,
	AlignEndVerticalIcon,
	AlignStartVerticalIcon,
	AlignVerticalJustifyCenterIcon,
	AlignVerticalJustifyEndIcon,
	AlignVerticalJustifyStartIcon,
	CircleXIcon,
	CopyIcon,
	FoldVerticalIcon,
	GripHorizontalIcon,
	GripVerticalIcon,
	MessageSquareIcon,
	SlidersHorizontalIcon,
	SparklesIcon,
	SquarePenIcon,
	Trash2Icon,
} from "lucide-react";
import { memo, useCallback, useMemo } from "react";
import { IPinType, IVariableType } from "../../../lib";
import type { INode } from "../../../lib/schema/flow/node";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSub,
	DropdownMenuSubContent,
	DropdownMenuSubTrigger,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { ToolbarButton } from "../layer-node/layer-node-toolbar";

interface FlowNodeToolbarProps {
	node: INode;
	appId: string;
	boardId: string;
	isReadOnly: boolean;
	onCopy: () => Promise<void>;
	onDelete: () => Promise<void>;
	onComment: () => void;
	onRename: () => void;
	onEdit: () => void;
	onInfo: () => void;
	onHandleError: () => void;
	onCollapse: (x: number, y: number) => void;
	onAlign: (
		type: "align" | "justify",
		dir: "start" | "end" | "center" | "distribute",
	) => void;
	onExplain: () => void;
}

const AlignmentMenu = memo(
	({
		onAlign,
	}: {
		onAlign: (
			type: "align" | "justify",
			dir: "start" | "end" | "center" | "distribute",
		) => void;
	}) => (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					className="h-5 w-5 flex items-center justify-center rounded transition-colors hover:bg-white/10"
					onClick={(e) => e.stopPropagation()}
				>
					<AlignStartVerticalIcon className="h-3 w-3" />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="center"
				side="top"
				className="min-w-[120px] text-xs"
			>
				<DropdownMenuSub>
					<DropdownMenuSubTrigger className="text-xs">
						<AlignStartVerticalIcon className="h-3 w-3 mr-1.5" />
						Align
					</DropdownMenuSubTrigger>
					<DropdownMenuSubContent className="text-xs">
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("align", "start")}
						>
							<AlignStartVerticalIcon className="h-3 w-3 mr-1.5" />
							Start
						</DropdownMenuItem>
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("align", "center")}
						>
							<AlignCenterVerticalIcon className="h-3 w-3 mr-1.5" />
							Center
						</DropdownMenuItem>
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("align", "end")}
						>
							<AlignEndVerticalIcon className="h-3 w-3 mr-1.5" />
							End
						</DropdownMenuItem>
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("align", "distribute")}
						>
							<GripVerticalIcon className="h-3 w-3 mr-1.5" />
							Distribute Evenly
						</DropdownMenuItem>
					</DropdownMenuSubContent>
				</DropdownMenuSub>
				<DropdownMenuSub>
					<DropdownMenuSubTrigger className="text-xs">
						<AlignVerticalJustifyStartIcon className="h-3 w-3 mr-1.5" />
						Justify
					</DropdownMenuSubTrigger>
					<DropdownMenuSubContent className="text-xs">
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("justify", "start")}
						>
							<AlignVerticalJustifyStartIcon className="h-3 w-3 mr-1.5" />
							Start
						</DropdownMenuItem>
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("justify", "center")}
						>
							<AlignVerticalJustifyCenterIcon className="h-3 w-3 mr-1.5" />
							Center
						</DropdownMenuItem>
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("justify", "end")}
						>
							<AlignVerticalJustifyEndIcon className="h-3 w-3 mr-1.5" />
							End
						</DropdownMenuItem>
						<DropdownMenuItem
							className="text-xs"
							onClick={() => onAlign("justify", "distribute")}
						>
							<GripHorizontalIcon className="h-3 w-3 mr-1.5" />
							Distribute Evenly
						</DropdownMenuItem>
					</DropdownMenuSubContent>
				</DropdownMenuSub>
			</DropdownMenuContent>
		</DropdownMenu>
	),
);

AlignmentMenu.displayName = "AlignmentMenu";

const Divider = memo(() => <div className="w-px h-3 bg-white/20 mx-0.5" />);

Divider.displayName = "Divider";

const FlowNodeToolbar = memo(
	({
		node,
		isReadOnly,
		onCopy,
		onDelete,
		onComment,
		onRename,
		onEdit,
		onInfo,
		onHandleError,
		onCollapse,
		onAlign,
		onExplain,
	}: FlowNodeToolbarProps) => {
		// Subscribe to selection only from the (rarely mounted) toolbar, narrowed to
		// a boolean that flips at the 1<->2 boundary so it re-renders far less often
		// than the old per-node global count subscription.
		const isMultiSelection = useStore((s) => {
			let count = 0;
			for (const n of s.nodeLookup.values()) {
				if (n.selected) {
					count++;
					if (count > 1) return true;
				}
			}
			return false;
		});
		const isSingleSelection = !isMultiSelection;

		const isExec = useMemo(
			() =>
				Object.values(node.pins).some(
					(pin) => pin.data_type === IVariableType.Execution,
				),
			[node.pins],
		);

		const errorHandled = useMemo(
			() =>
				Object.values(node.pins).some(
					(pin) =>
						pin.name === "auto_handle_error" &&
						pin.pin_type === IPinType.Output,
				),
			[node.pins],
		);

		const isGenericEvent = node.name === "events_generic";
		const isStartNode = node.start ?? false;

		// Stable onClick so the memoized ToolbarButton (and its Radix Tooltip/Popper,
		// which measures the DOM on render) doesn't re-render on every toolbar render.
		const handleCollapseClick = useCallback(
			(e: Event | undefined) => {
				const rect = (
					e?.currentTarget as HTMLElement
				)?.getBoundingClientRect?.();
				if (rect) {
					onCollapse(rect.x + rect.width / 2, rect.y + rect.height / 2);
				} else {
					onCollapse(0, 0);
				}
			},
			[onCollapse],
		);

		if (isReadOnly) return null;

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
					{isSingleSelection && (
						<>
							{isStartNode && (
								<ToolbarButton
									onClick={onRename}
									icon={SquarePenIcon}
									tooltip="Rename"
								/>
							)}
							{isGenericEvent && (
								<ToolbarButton
									onClick={onEdit}
									icon={SlidersHorizontalIcon}
									tooltip="Edit"
								/>
							)}
							<ToolbarButton
								onClick={onComment}
								icon={MessageSquareIcon}
								tooltip="Comment"
							/>
							<ToolbarButton
								onClick={onInfo}
								icon={InfoCircledIcon}
								tooltip="Info"
							/>
							{isExec && (
								<ToolbarButton
									onClick={onHandleError}
									icon={CircleXIcon}
									tooltip={
										errorHandled ? "Remove Error Handling" : "Handle Errors"
									}
								/>
							)}
						</>
					)}

					{isMultiSelection && (
						<>
							<ToolbarButton
								onClick={handleCollapseClick}
								icon={FoldVerticalIcon}
								tooltip="Collapse"
							/>
							<AlignmentMenu onAlign={onAlign} />
						</>
					)}

					<Divider />

					<ToolbarButton
						onClick={onExplain}
						icon={SparklesIcon}
						tooltip="Explain with FlowPilot"
					/>
					<ToolbarButton onClick={onCopy} icon={CopyIcon} tooltip="Copy" />
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

FlowNodeToolbar.displayName = "FlowNodeToolbar";

export { FlowNodeToolbar };

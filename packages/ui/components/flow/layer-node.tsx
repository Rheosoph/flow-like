"use client";
import { useDebounce } from "@uidotdev/usehooks";
import { type Node, type NodeProps, useReactFlow } from "@xyflow/react";
import {
	BanIcon,
	CircleXIcon,
	ScrollTextIcon,
	SquareCheckIcon,
	SquareFunctionIcon,
	TriangleAlertIcon,
	ZapIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import {
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import PuffLoader from "react-spinners/PuffLoader";
import { toast } from "sonner";
import type { IBoard, INode } from "../../lib";
import { logLevelFromNumber } from "../../lib/log-level";
import {
	type ILayer,
	ILayerType,
	ILogLevel,
	IPinType,
} from "../../lib/schema/flow/board";
import { useLogAggregation } from "../../state/log-aggregation-state";
import { useRunExecutionStore } from "../../state/run-execution-state";
import { AutoResizeText } from "./auto-resize-text";
import { CommentDialog } from "./comment-dialog";
import { FlowPin } from "./flow-pin";
import type { FlowSelectorDataRef } from "./flow-selector-data";
import { LayerEditMenu } from "./layer-editing-menu";
import { LayerNodeToolbar } from "./layer-node/layer-node-toolbar";
import { NameDialog } from "./name-dialog";

export type LayerNode = Node<
	{
		layer: ILayer;
		pinLookup: Record<string, INode>;
		boardId: string;
		hash: string;
		appId: string;
		boardRef?: RefObject<IBoard | undefined>;
		boardDataVersion?: string;
		selectorDataRef?: FlowSelectorDataRef;
		selectorDataVersion?: number;
		pushLayer(layer: ILayer): Promise<void>;
		onLayerUpdate(layer: ILayer): Promise<void>;
		onLayerRemove(layer: ILayer, preserve_nodes: boolean): Promise<void>;
		onExplain?: (nodeIds: string[]) => void;
	},
	"layerNode"
>;

export function LayerNode(props: NodeProps<LayerNode>) {
	const divRef = useRef<HTMLDivElement>(null);
	const { getNodes } = useReactFlow();
	const [comment, setComment] = useState<string | undefined>();
	const [name, setName] = useState<string | undefined>();
	const [editing, setEditing] = useState(false);
	const [isHovered, setIsHovered] = useState(false);
	const { resolvedTheme } = useTheme();

	const { currentMetadata } = useLogAggregation();
	const { runs } = useRunExecutionStore();
	const [executionState, setExecutionState] = useState<
		"done" | "running" | "none"
	>("none");
	const debouncedExecutionState = useDebounce(executionState, 100);
	const [runId, setRunId] = useState<string | undefined>(undefined);

	const fetchChildNodeIDs = useCallback(() => {
		const layers = props.data.boardRef?.current?.layers ?? {};
		const nodes = props.data.boardRef?.current?.nodes ?? {};
		const startId = props.data.layer.id;

		// Collect the start layer and all descendant layers (recursive).
		const collected = new Set<string>();
		const queue: string[] = [startId];

		while (queue.length) {
			const current = queue.shift()!;
			if (collected.has(current)) continue;
			collected.add(current);

			for (const l of Object.values(layers)) {
				// robustly detect layer id and common parent-field names
				const lid = (l as any).id ?? (l as any).layer ?? undefined;
				if (!lid) continue;

				const parentId =
					(l as any).parent ??
					(l as any).parent_id ??
					(l as any).parentLayer ??
					(l as any).parent?.id ??
					(l as any).layer_parent ??
					undefined;

				if (parentId === current && !collected.has(lid)) {
					queue.push(lid);
				}
			}
		}

		return Object.values(nodes)
			.filter((n) => n.layer && collected.has(n.layer))
			.map((n) => n.id);
	}, [props.data.layer.id, props.data.boardRef]);

	const [executed, severity] = useMemo(() => {
		const severity = ILogLevel.Debug;
		let childNodeExecuted = false;
		let worstSeverity = 0;

		const nodeIds = fetchChildNodeIDs();
		if (!currentMetadata) return [false, severity];
		currentMetadata.nodes?.forEach(([localNodeId, severity]) => {
			if (nodeIds.includes(localNodeId.toString())) {
				childNodeExecuted = true;
				worstSeverity = Math.max(worstSeverity, severity as number);
			}
		});

		if (childNodeExecuted) {
			return [true, logLevelFromNumber(worstSeverity)];
		}

		return [false, severity];
	}, [props.data.layer.id, currentMetadata]);

	useEffect(() => {
		const nodeIds = fetchChildNodeIDs();
		let isRunning = false;
		let already_executed = false;
		let foundRunId: string | undefined;

		for (const [rId, run] of runs) {
			if (nodeIds.some((nid) => run.nodes.has(nid))) {
				isRunning = true;
				foundRunId = rId;
				break;
			}

			if (nodeIds.some((nid) => run.already_executed.has(nid))) {
				already_executed = true;
				foundRunId = foundRunId ?? rId;
			}
		}

		if (foundRunId !== undefined) {
			setRunId(foundRunId);
		}

		if (isRunning) {
			setExecutionState("running");
			return;
		}

		if (already_executed) {
			setExecutionState("done");
			return;
		}

		setExecutionState("none");
	}, [runs, props.id, props.data.layer.nodes]);

	useEffect(() => {
		const height = Math.max(
			Object.values(props.data.layer.pins).filter(
				(pin) => pin.pin_type === IPinType.Input,
			).length,
			Object.values(props.data.layer.pins).filter(
				(pin) => pin.pin_type === IPinType.Output,
			).length,
		);

		if (divRef.current) {
			divRef.current.style.height = `calc(${height * 15}px + 1.25rem + 0.5rem)`;
			divRef.current.style.minHeight = "calc(15px + 1.25rem + 0.5rem)";
		}
	}, [props.data.hash]);

	const saveComment = useCallback(async () => {
		const node = getNodes().find((n) => n.id === props.id);
		if (!node) return;
		const layer = node.data.layer as ILayer;
		props.data.onLayerUpdate({ ...layer, comment: comment ?? "" });
		setComment(undefined);
	}, [props.id, comment]);

	const saveName = useCallback(async () => {
		const node = getNodes().find((n) => n.id === props.id);
		if (!node) return;
		const layer = node.data.layer as ILayer;
		props.data.onLayerUpdate({ ...layer, name: name ?? "Collapsed" });
		setName(undefined);
	}, [props.id, name]);

	const handleExplain = useCallback(() => {
		const selectedNodes = getNodes().filter((node) => node.selected);
		const nodeIds =
			selectedNodes.length > 0
				? selectedNodes.map((node) => node.id)
				: [props.id];
		props.data.onExplain?.(nodeIds);
	}, [getNodes, props.id, props.data.onExplain]);

	return (
		<>
			{typeof comment === "string" && (
				<CommentDialog
					onOpenChange={(open) => {
						if (!open) {
							saveComment();
						}
					}}
					comment={comment}
					open={typeof comment === "string"}
					onUpsert={(comment) => setComment(comment)}
				/>
			)}
			{typeof name === "string" && (
				<NameDialog
					onOpenChange={(open) => {
						if (!open) {
							saveName();
						}
					}}
					name={name}
					open={typeof name === "string"}
					onUpsert={(name) => setName(name)}
				/>
			)}
			<div
				className="relative"
				onMouseEnter={() => setIsHovered(true)}
				onMouseLeave={() => setIsHovered(false)}
			>
				{(props.selected || isHovered) && (
					<LayerNodeToolbar
						onRename={() => setName(props.data.layer.name ?? "")}
						onComment={() => setComment(props.data.layer.comment ?? "")}
						onEdit={() => setEditing(true)}
						onExtend={() => props.data.onLayerRemove(props.data.layer, true)}
						onDelete={() => props.data.onLayerRemove(props.data.layer, false)}
						onExplain={handleExplain}
					/>
				)}
				<div
					ref={divRef}
					key={`${props.data.hash}__node`}
					className={`p-1 flex flex-col justify-center items-center react-flow__node-default selectable focus:ring-2 relative bg-card! border-border! rounded-md! group ${executionState === "done" ? "opacity-60" : "opacity-100"} ${props.selected && "border-primary! border-2"}`}
				>
					{props.data.layer.comment && props.data.layer.comment !== "" && (
						<div className="absolute top-0 translate-y-[calc(-100%-0.5rem)] left-3 right-3 mb-2 text-center bg-foreground/70 text-background p-1 rounded-md">
							<small className="font-normal text-extra-small leading-extra-small">
								{props.data.layer.comment}
							</small>
							<div
								className="
                                            absolute
                                            -bottom-1
                                            left-1/2
                                            transform -translate-x-1/2
                                            w-0 h-0
                                            border-l-4 border-l-transparent
                                            border-r-4 border-r-transparent
                                            border-t-4 border-t-foreground/70
                                        "
							/>
						</div>
					)}
					{severity !== ILogLevel.Debug && (
						<div className="absolute top-0 z-10 translate-y-[calc(-50%)] translate-x-[calc(50%)] right-0 text-center bg-background rounded-full">
							{severity === ILogLevel.Fatal && (
								<BanIcon className="w-3 h-3 text-red-800" />
							)}
							{severity === ILogLevel.Error && (
								<CircleXIcon className="w-3 h-3 text-red-500" />
							)}
							{severity === ILogLevel.Warn && (
								<TriangleAlertIcon className="w-3 h-3 text-yellow-500" />
							)}
						</div>
					)}
					<div
						className={`header absolute top-0 left-0 right-0 h-4 gap-1 flex flex-row items-center border-b ${props.data.layer.type === ILayerType.Function ? "bg-primary/15 text-primary" : "bg-accent text-accent-foreground"} p-1 justify-between rounded-t-md`}
					>
						<div className="flex flex-row items-center gap-1 min-w-0">
							{props.data.layer.type === ILayerType.Function ? (
								<SquareFunctionIcon className="w-2 h-2 shrink-0" />
							) : (
								<ZapIcon className="w-2 h-2 shrink-0" />
							)}
							<small className="font-medium leading-none truncate">
								<AutoResizeText text={props.data.layer.name} maxChars={30} />
							</small>
						</div>
						<div className="flex flex-row items-center gap-1 shrink-0">
							{executed && (
								<ScrollTextIcon className="w-2 h-2 cursor-pointer hover:text-primary" />
							)}
							{useMemo(() => {
								if (debouncedExecutionState !== "running") return null;
								return (
									<PuffLoader
										color={resolvedTheme === "dark" ? "white" : "black"}
										size={10}
										speedMultiplier={1}
									/>
								);
							}, [debouncedExecutionState, resolvedTheme])}

							{useMemo(() => {
								return debouncedExecutionState === "done" ? (
									<SquareCheckIcon className="w-2 h-2 text-primary" />
								) : null;
							}, [debouncedExecutionState])}
						</div>
					</div>
					{Object.values(props.data.layer.pins)
						.filter((pin) => pin.pin_type === IPinType.Input)
						.toSorted((a, b) => a.index - b.index)
						.map((pin) => (
							<FlowPin
								appId={props.data.appId}
								node={props.data.pinLookup[pin.id] ?? props.data.layer}
								boardId={props.data.boardId}
								boardRef={props.data.boardRef}
								boardDataVersion={props.data.boardDataVersion}
								pin={pin}
								key={pin.id}
								skipOffset={true}
								onPinRemove={async () => {}}
								selectorDataRef={props.data.selectorDataRef}
								selectorDataVersion={props.data.selectorDataVersion}
							/>
						))}
					{Object.values(props.data.layer.pins)
						.filter((pin) => pin.pin_type === IPinType.Output)
						.toSorted((a, b) => a.index - b.index)
						.map((pin) => (
							<FlowPin
								appId={props.data.appId}
								node={props.data.pinLookup[pin.id] ?? props.data.layer}
								boardId={props.data.boardId}
								boardRef={props.data.boardRef}
								boardDataVersion={props.data.boardDataVersion}
								pin={pin}
								key={pin.id}
								skipOffset={true}
								onPinRemove={async () => {}}
								selectorDataRef={props.data.selectorDataRef}
								selectorDataVersion={props.data.selectorDataVersion}
							/>
						))}
				</div>
			</div>

			<LayerEditMenu
				open={editing}
				layer={props.data.layer}
				onOpenChange={setEditing}
				boardRef={props.data.boardRef}
				onApply={async (updated) => {
					const newLayer = {
						...props.data.layer,
						pins: updated.pins,
					};
					try {
						await props.data.onLayerUpdate(newLayer);
					} catch (error) {
						console.error(error);
						toast.error("Failed to update layer");
					}
					setEditing(false);
				}}
			/>
		</>
	);
}

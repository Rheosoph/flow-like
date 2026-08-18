"use client";
import { i18n as i18next } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { useDebounce } from "@uidotdev/usehooks";
import {
	Handle,
	type Node,
	type NodeProps,
	Position,
	useReactFlow,
	useStoreApi,
} from "@xyflow/react";
import {
	BanIcon,
	BoxIcon,
	CircleStopIcon,
	CircleXIcon,
	ClockIcon,
	CloudCog,
	DatabaseIcon,
	MonitorIcon,
	PlayCircleIcon,
	ScrollTextIcon,
	SquareCheckIcon,
	TriangleAlertIcon,
	WorkflowIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import {
	type RefObject,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import PuffLoader from "react-spinners/PuffLoader";
import { useLogAggregation } from "../..";
import { useInvalidateInvoke } from "../../hooks";
import { type PeerUserInfo, colorFromSub } from "../../hooks/use-peer-users";
import {
	getActivityColorClasses,
	useRunActivity,
} from "../../hooks/use-run-activity";
import {
	IExecutionMode,
	type IGenericCommand,
	ILogLevel,
	IPinType,
	IValueType,
	cacheIndicatorLabel,
	formatCacheTtl,
	moveNodeCommand,
	removeNodeCommand,
	updateNodeCommand,
	upsertLayerCommand,
	upsertPinCommand,
} from "../../lib";
import type { INode } from "../../lib";
import { logLevelFromNumber } from "../../lib/log-level";
import { isWebkitLite } from "../../lib/platform";
import type {
	IBoard,
	IComment,
	ILayer,
	ILayerCache,
} from "../../lib/schema/flow/board";
import { ILayerType } from "../../lib/schema/flow/board/commands/upsert-layer";
import { type IPin, IVariableType } from "../../lib/schema/flow/pin";
import { convertJsonToUint8Array } from "../../lib/uint8";
import { useBackendStore } from "../../state/backend-state";
import { useRunExecutionStore } from "../../state/run-execution-state";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "../ui";
import { DynamicImage } from "../ui";
import { AutoResizeText } from "./auto-resize-text";
import { useUndoRedo } from "./flow-history";
import { EventPayloadForm } from "./flow-node/event-payload-form";
import { FlowNodeCommentMenu } from "./flow-node/flow-node-comment-menu";
import { FlowPinAction } from "./flow-node/flow-node-pin-action";
import { FlowNodeRenameMenu } from "./flow-node/flow-node-rename-menu";
import { FlowNodeToolbar } from "./flow-node/flow-node-toolbar";
import { FlowPin } from "./flow-pin";
import type { FlowSelectorDataRef } from "./flow-selector-data";
import { LayerEditMenu } from "./layer-editing-menu";
import { typeToColor } from "./utils";

export interface RemoteSelectionParticipant {
	clientId: number;
	/** The sub (subject) from the auth token - use to resolve user info via API */
	sub?: string;
	/** Whether this user just actively clicked this node */
	isActive?: boolean;
}

export interface IPinAction {
	action: "create";
	pin: IPin;
	onAction: (pin: IPin) => Promise<void>;
}

export type FlowNode = Node<
	{
		hash: string;
		node: INode;
		boardId: string;
		appId: string;
		transparent?: boolean;
		boardRef: RefObject<IBoard | undefined>;
		boardDataVersion?: string;
		fnRefsHash?: string;
		version?: [number, number, number];
		onExecute: (node: INode, payload?: object) => Promise<void>;
		onRemoteExecute?: (node: INode, payload?: object) => Promise<void>;
		isOffline?: boolean;
		onCopy: () => Promise<void>;
		remoteSelections?: RemoteSelectionParticipant[];
		peerUsers?: Map<string, PeerUserInfo>;
		onOpenInfo?: (node: INode) => void;
		onExplain?: (nodeIds: string[]) => void;
		onFilterLogs?: (nodeId: string) => void;
		executionMode?: IExecutionMode;
		isUnavailable?: boolean;
		functionLayerId?: string;
		/** Set only when the referenced function caches its results. */
		functionCache?: ILayerCache;
		currentLayerId?: string;
		remoteExecuting?: boolean;
		selectorDataRef?: FlowSelectorDataRef;
		selectorDataVersion?: number;
	},
	"node"
>;

type FlowStoreApi = ReturnType<typeof useStoreApi>;
type NodeInternalsUpdates = Parameters<
	ReturnType<FlowStoreApi["getState"]>["updateNodeInternals"]
>[0];

const pendingInternalsUpdates = new WeakMap<FlowStoreApi, Set<string>>();

/**
 * React Flow's own hook schedules one animation frame per call, and each
 * store update re-runs every subscriber selector, so N nodes re-measuring in
 * the same tick cost O(N²). Collect the ids and flush them in one store call.
 */
function scheduleNodeInternalsUpdate(store: FlowStoreApi, nodeId: string) {
	const pending = pendingInternalsUpdates.get(store);
	if (pending) {
		pending.add(nodeId);
		return;
	}
	const ids = new Set([nodeId]);
	pendingInternalsUpdates.set(store, ids);
	requestAnimationFrame(() => {
		pendingInternalsUpdates.delete(store);
		const { domNode, updateNodeInternals } = store.getState();
		const updates: NodeInternalsUpdates = new Map();
		for (const id of ids) {
			const nodeElement = domNode?.querySelector<HTMLDivElement>(
				`.react-flow__node[data-id="${id}"]`,
			);
			if (nodeElement) updates.set(id, { id, nodeElement, force: true });
		}
		if (updates.size > 0) {
			updateNodeInternals(updates, { triggerFitView: false });
		}
	});
}

const FlowNodeInner = memo(
	({
		props,
	}: {
		props: NodeProps<FlowNode>;
	}) => {
		const { pushCommand } = useUndoRedo(props.data.appId, props.data.boardId);
		const { resolvedTheme } = useTheme();
		const invalidate = useInvalidateInvoke();
		// Field selectors: subscribing to the whole log store re-renders every
		// node on currentLogs/isLoading churn during runs — with hundreds of
		// nodes that bypasses the memo comparator on every log tick.
		const currentMetadata = useLogAggregation((state) => state.currentMetadata);
		const heatmapEnabled = useLogAggregation((state) => state.heatmapEnabled);
		const heatmap = useLogAggregation((state) => state.heatmap);

		const [payload, setPayload] = useState({
			open: false,
			payload: "",
		});
		const [executing, setExecuting] = useState(false);

		// Use separate selectors returning primitives to avoid infinite re-renders
		const executionStatus = useRunExecutionStore((state) => {
			for (const [, run] of state.runs) {
				if (run.nodes.has(props.id)) return "running" as const;
				if (run.already_executed.has(props.id)) return "done" as const;
			}
			return "none" as const;
		});

		const activeRunId = useRunExecutionStore((state) => {
			for (const [runId, run] of state.runs) {
				if (run.nodes.has(props.id) || run.already_executed.has(props.id)) {
					return runId;
				}
			}
			return undefined;
		});

		const debouncedExecutionState = useDebounce(executionStatus, 100);
		const runActivity = useRunActivity(activeRunId);
		const div = useRef<HTMLDivElement>(null);
		const reactFlow = useReactFlow();
		const { getNode } = useReactFlow();
		const flowStore = useStoreApi();
		const remoteSelections = props.data.remoteSelections ?? [];
		const displayedRemoteSelections = useMemo(
			() => remoteSelections.slice(0, 3),
			[remoteSelections],
		);
		const extraRemoteSelections =
			remoteSelections.length - displayedRemoteSelections.length;
		const [executed, severity] = useMemo(() => {
			const severity = ILogLevel.Debug;

			const nodeId = props.data.node.id;
			if (!currentMetadata) return [false, severity];
			const result = currentMetadata.nodes?.find(([localNodeId, severity]) => {
				if (localNodeId === nodeId) {
					return true;
				}
			}) as [string, number] | undefined;

			if (result) {
				return [true, logLevelFromNumber(result?.[1] ?? 0)];
			}

			return [false, severity];
		}, [props.data.node, currentMetadata]);

		// Aggregated many-runs activity for the heatmap overlay: visit count,
		// error count, and an intensity bucket relative to the busiest node.
		const nodeHeat = useMemo(() => {
			if (!heatmapEnabled || !heatmap) return undefined;
			const heat = heatmap.nodes[props.data.node.id];
			if (!heat) return undefined;
			const intensity =
				heatmap.maxVisits > 0 ? heat.visits / heatmap.maxVisits : 0;
			return { ...heat, intensity };
		}, [heatmapEnabled, heatmap, props.data.node.id]);

		const isReroute = useMemo(() => {
			return props.data.node.name === "reroute";
		}, [props.data.node.name]);

		const isWasmNode = useMemo(
			() => Boolean(props.data.node.wasm?.package_id),
			[props.data.node.wasm],
		);

		const firstPinType =
			Object.values(props.data.node.pins)?.[0]?.data_type ??
			IVariableType.Generic;
		const nodeStyle = useMemo(
			() => ({
				backgroundColor: props.selected ? typeToColor(firstPinType) : undefined,
				borderColor: typeToColor(firstPinType),
				borderWidth: "1px",
				borderStyle: "solid",
			}),
			[props.selected, firstPinType],
		);

		const sortPins = useCallback((a: IPin, b: IPin) => {
			// Step 1: Compare by type - Input comes before Output
			if (a.pin_type === "Input" && b.pin_type === "Output") return -1;
			if (a.pin_type === "Output" && b.pin_type === "Input") return 1;

			// Step 2: If types are the same, compare by index
			return a.index - b.index;
		}, []);

		// Execution state is now computed directly from the selector above

		const addPin = useCallback(
			async (node: INode, pin: IPin, index: number) => {
				if (typeof props.data.version !== "undefined") {
					return;
				}

				const backend = useBackendStore.getState().backend;
				if (!backend) return;
				const nodeGuard = reactFlow
					.getNodes()
					.find((node) => node.id === props.id);
				if (!nodeGuard) return;

				node = nodeGuard.data.node as INode;
				if (!node.pins) return;

				const newPin: IPin = {
					...pin,
					depends_on: [],
					connected_to: [],
					id: createId(),
					index: index,
				};

				const allPins = Object.values(node.pins);
				const inputPins = allPins
					.filter((p) => p.pin_type === "Input")
					.sort(sortPins);
				const outputPins = allPins
					.filter((p) => p.pin_type === "Output")
					.sort(sortPins);

				if (newPin.pin_type === "Input") {
					// Insert the new input pin at the specified index
					inputPins.splice(index - 1, 0, newPin); // Convert to 0-based index for splice

					// Update indices for input pins only, starting from the insertion point
					for (let i = index - 1; i < inputPins.length; i++) {
						inputPins[i].index = i + 1; // Convert back to 1-based index
					}
				} else {
					// Insert the new output pin at the specified index
					outputPins.splice(index - 1, 0, newPin); // Convert to 0-based index for splice

					// Update indices for output pins only, starting from the insertion point
					for (let i = index - 1; i < outputPins.length; i++) {
						outputPins[i].index = i + 1; // Convert back to 1-based index
					}
				}

				// Rebuild the pins object with updated pins
				node.pins = {};
				[...inputPins, ...outputPins].forEach((pin) => {
					node.pins[pin.id] = pin;
				});

				const command = updateNodeCommand({
					node: {
						...node,
						coordinates: [nodeGuard.position.x, nodeGuard.position.y, 0],
					},
				});

				const result = await backend.boardState.executeCommand(
					props.data.appId,
					props.data.boardId,
					command,
				);

				await pushCommand(result, false);

				await invalidate(backend.boardState.getBoard, [
					props.data.appId,
					props.data.boardId,
				]);
			},
			[
				reactFlow,
				sortPins,
				pushCommand,
				invalidate,
				props.id,
				props.data.version,
				props.data.appId,
				props.data.boardId,
			],
		);
		const pinRemoveCallback = useCallback(
			async (pinToRemove: IPin) => {
				if (typeof props.data.version !== "undefined") {
					return;
				}

				const backend = useBackendStore.getState().backend;
				if (!backend) return;

				const nodeGuard = getNode(props.id);
				if (!nodeGuard) return;

				if (!props?.data?.node?.pins) return;
				const node = nodeGuard?.data?.node as INode | undefined;
				if (!node) return;
				const allPins = Object.values(node.pins);

				const inputPins = allPins
					.filter((p) => p.pin_type === "Input" && p.id !== pinToRemove.id)
					.sort(sortPins)
					.map((p, i) => ({ ...p, index: i + 1 }));

				const outputPins = allPins
					.filter((p) => p.pin_type === "Output" && p.id !== pinToRemove.id)
					.sort(sortPins)
					.map((p, i) => ({ ...p, index: i + 1 }));

				const updatedPins: Record<string, IPin> = {};
				[...inputPins, ...outputPins].forEach((p) => {
					updatedPins[p.id] = p;
				});
				node.pins = updatedPins;

				const command = updateNodeCommand({
					node: {
						...node,
						coordinates: [nodeGuard.position.x, nodeGuard.position.y, 0],
					},
				});

				const result = await backend.boardState.executeCommand(
					props.data.appId,
					props.data.boardId,
					command,
				);

				await pushCommand(result, false);

				await invalidate(backend.boardState.getBoard, [
					props.data.appId,
					props.data.boardId,
				]);
			},
			[
				getNode,
				sortPins,
				pushCommand,
				invalidate,
				props.id,
				props.data.version,
				props.data.appId,
				props.data.boardId,
			],
		);

		const parsePins = useCallback(
			(pins: IPin[]) => {
				const inputPins: (IPin | IPinAction)[] = [];
				const outputPins: (IPin | IPinAction)[] = [];
				let isExec = false;

				let pastPinWithCount: [string, number, IPin | undefined] = [
					"",
					0,
					undefined,
				];

				Object.values(pins)
					.sort(sortPins)
					.forEach((pin, index) => {
						if (pin.data_type === "Execution") isExec = true;

						const pastPinId = `${pin.name}_${pin.pin_type}`;

						if (pastPinWithCount[0] === pastPinId) {
							pastPinWithCount[1] += 1;
						}

						if (pastPinWithCount[0] !== pastPinId && pastPinWithCount[1] > 0) {
							const action: IPinAction = {
								action: "create",
								pin: { ...pastPinWithCount[2]! },
								onAction: async (pin) => {
									await addPin(props.data.node, pin, index - 1);
								},
							};

							if (pastPinWithCount[2]?.pin_type === "Input") {
								inputPins.push(action);
							} else {
								outputPins.push(action);
							}
						}

						// update to past pin information
						if (pastPinWithCount[0] !== pastPinId)
							pastPinWithCount = [pastPinId, 0, pin];
						pin = { ...pin, dynamic: pastPinWithCount[1] > 1 };

						if (pin.pin_type === "Input") {
							inputPins.push(pin);
						} else {
							outputPins.push(pin);
						}
					});

				if (pastPinWithCount[1] > 0 && pastPinWithCount[2]) {
					const action: IPinAction = {
						action: "create",
						pin: { ...pastPinWithCount[2] },
						onAction: async (pin) => {
							await addPin(
								props.data.node,
								pin,
								Object.values(props.data.node?.pins || []).length,
							);
						},
					};

					if (pastPinWithCount[2].pin_type === "Input") {
						inputPins.push(action);
					} else {
						outputPins.push(action);
					}
				}

				return { inputPins, outputPins, isExec };
			},
			[addPin, sortPins, props.data.node],
		);

		// Parse pins when node pins change
		const visiblePins = useMemo(() => {
			const all = Object.values(props.data.node?.pins || []);
			if (props.data.node?.name !== "control_call_function") return all;
			let inputIdx = 0;
			return all
				.filter((p) => p.name !== "function_layer_id")
				.sort(sortPins)
				.map((p) => {
					if (p.pin_type === "Input") {
						inputIdx++;
						return { ...p, index: inputIdx };
					}
					return p;
				});
		}, [props.data.node?.pins, props.data.node?.name]);

		// Derive pins synchronously (avoids the extra render pass of effect+setState)
		const { inputPins, outputPins, isExec } = useMemo(
			() => parsePins(visiblePins),
			[parsePins, visiblePins],
		);

		const pinLayoutKey = useMemo(
			() =>
				visiblePins.map((p) => `${p.id}:${p.index}:${p.pin_type}`).join("|"),
			[visiblePins],
		);
		const measuredPinLayout = useRef<string | null>(null);
		useEffect(() => {
			// React Flow measures handles itself when the node mounts (its
			// ResizeObserver); only later pin layout changes need a re-measure.
			if (measuredPinLayout.current === pinLayoutKey) return;
			const isMount = measuredPinLayout.current === null;
			measuredPinLayout.current = pinLayoutKey;
			if (!isMount) scheduleNodeInternalsUpdate(flowStore, props.id);
		}, [pinLayoutKey, props.id, flowStore]);

		useEffect(() => {
			if (isReroute) return;
			const height = Math.max(inputPins.length, outputPins.length);
			if (div.current)
				div.current.style.height = `calc(${height * 15}px + 1.25rem + 0.5rem)`;
		}, [isReroute, inputPins, outputPins]);

		function isPinAction(pin: IPin | IPinAction): pin is IPinAction {
			return typeof (pin as IPinAction).onAction === "function";
		}

		const renderInputPins = useMemo(
			() =>
				!(props.data.node.start ?? false) &&
				inputPins
					.filter((pin) => isPinAction(pin) || pin.pin_type === "Input")
					.map((pin, arrayIndex) => {
						return isPinAction(pin) ? (
							<FlowPinAction
								key={`${pin.pin.id}__action`}
								action={pin}
								index={arrayIndex}
								input
							/>
						) : (
							<FlowPin
								appId={props.data.appId}
								key={pin.id}
								node={props.data.node}
								boardId={props.data.boardId}
								boardRef={props.data.boardRef}
								boardDataVersion={props.data.boardDataVersion}
								pin={pin}
								onPinRemove={pinRemoveCallback}
								skipOffset={isReroute}
								version={props.data.version}
								currentLayerId={props.data.currentLayerId}
								selectorDataRef={props.data.selectorDataRef}
								selectorDataVersion={props.data.selectorDataVersion}
							/>
						);
					}),
			[
				inputPins,
				props.data.node,
				props.data.boardId,
				props.data.boardDataVersion,
				pinRemoveCallback,
				isReroute,
				props.data.version,
				props.data.currentLayerId,
				props.data.selectorDataVersion,
			],
		);

		const renderOutputPins = useMemo(
			() =>
				outputPins.map((pin, arrayIndex) => {
					return isPinAction(pin) ? (
						<FlowPinAction
							action={pin}
							index={arrayIndex}
							input={false}
							key={`${pin.pin.id}__action`}
						/>
					) : (
						<FlowPin
							appId={props.data.appId}
							node={props.data.node}
							boardId={props.data.boardId}
							boardRef={props.data.boardRef}
							boardDataVersion={props.data.boardDataVersion}
							pin={pin}
							key={pin.id}
							onPinRemove={pinRemoveCallback}
							skipOffset={isReroute}
							version={props.data.version}
							currentLayerId={props.data.currentLayerId}
							selectorDataRef={props.data.selectorDataRef}
							selectorDataVersion={props.data.selectorDataVersion}
						/>
					);
				}),
			[
				outputPins,
				props.data.node,
				props.data.boardId,
				props.data.boardDataVersion,
				pinRemoveCallback,
				isReroute,
				props.data.version,
				props.data.currentLayerId,
				props.data.selectorDataVersion,
			],
		);

		// Compute connection states efficiently - only track the specific fn_refs we care about
		const refInConnected = useMemo(() => {
			const board = props.data.boardRef?.current;
			if (!board) return false;
			const currentNodeId = props.data.node.id;
			// Only check nodes, return boolean to avoid object reference changes
			return Object.values(board.nodes || {}).some((node) =>
				node.fn_refs?.fn_refs?.includes(currentNodeId),
			);
		}, [props.data.node.id, props.data.fnRefsHash]);

		const refOutConnected = useMemo(() => {
			return (props.data.node.fn_refs?.fn_refs?.length ?? 0) > 0;
		}, [props.data.node.fn_refs?.fn_refs?.length]);

		const renderFnRefInputs = useMemo(() => {
			const canBeReferencedByFns =
				props.data.node.fn_refs?.can_be_referenced_by_fns ?? false;
			if (!canBeReferencedByFns) return null;

			return (
				<Handle
					position={Position.Top}
					type={"target"}
					className={`relative ml-auto right-0 z-50 mt-2 -mr-1`}
					id={`ref_in_${props.data.node.id}`}
					style={{
						width: 12,
						height: 12,
						borderRadius: 2,
						background: refInConnected
							? isWebkitLite()
								? "var(--pin-fn-ref)"
								: `
				linear-gradient(
					135deg,
					var(--pin-fn-ref) 0%,
					color-mix(in oklch, var(--pin-fn-ref) 90%, white) 50%,
					var(--pin-fn-ref) 100%
				)
			`
							: "var(--background)",
						border: "1px solid var(--pin-fn-ref)",
						padding: 0,
						boxShadow:
							refInConnected && !isWebkitLite()
								? `
		0 0 6px color-mix(in oklch, var(--pin-fn-ref) 30%, transparent),
		inset 0 1px 1px color-mix(in oklch, white 15%, transparent)
	`
								: "none",
					}}
				/>
			);
		}, [
			props.data.node.fn_refs?.can_be_referenced_by_fns,
			refInConnected,
			props.data.node.id,
		]);
		const renderFnRefOutputs = useMemo(() => {
			const canBeReferencedByFns =
				props.data.node.fn_refs?.can_reference_fns ?? false;
			if (!canBeReferencedByFns) return null;

			return (
				<Handle
					position={Position.Bottom}
					type={"source"}
					className={`relative z-50`}
					id={`ref_out_${props.data.node.id}`}
					style={{
						width: 12,
						height: 12,
						borderRadius: 2,
						background: refOutConnected
							? isWebkitLite()
								? "var(--pin-fn-ref)"
								: `
			radial-gradient(
				circle at 30% 30%,
				color-mix(in oklch, var(--pin-fn-ref) 100%, white 20%),
				var(--pin-fn-ref) 70%
			)
		`
							: "var(--background)",
						border: "1px solid var(--pin-fn-ref)",
						padding: 0,
						boxShadow:
							refOutConnected && !isWebkitLite()
								? `
			0 0 8px color-mix(in oklch, var(--pin-fn-ref) 40%, transparent),
			0 1px 2px color-mix(in oklch, black 20%, transparent),
			inset 0 1px 1px color-mix(in oklch, white 20%, transparent)
		`
								: "none",
					}}
				/>
			);
		}, [
			props.data.node.fn_refs?.can_reference_fns,
			refOutConnected,
			props.data.node.id,
		]);
		const playNode = useMemo(() => {
			if (!props.data.node.start) return null;

			const executionMode = props.data.executionMode ?? IExecutionMode.Hybrid;
			const canRemoteExecuteBase =
				!props.data.isOffline && props.data.onRemoteExecute !== undefined;

			// Apply execution mode restrictions
			// only_offline nodes can never run remotely
			const canLocalExecute = executionMode !== IExecutionMode.Remote;
			const canRemoteExecute =
				canRemoteExecuteBase &&
				executionMode !== IExecutionMode.Local &&
				!props.data.node.only_offline;

			if (executionStatus === "done" || executing)
				return (
					<button
						className="bg-background hover:bg-card group/play transition-all rounded-md hover:rounded-lg border p-1 absolute left-0 top-0 translate-x-[calc(-120%)] opacity-200!"
						onClick={async (e) => {
							const backend = useBackendStore.getState().backend;
							if (!backend) return;
							if (activeRunId)
								await backend.eventState.cancelExecution(activeRunId);
						}}
					>
						<CircleStopIcon className="w-3 h-3 group-hover/play:scale-110 text-primary" />
					</button>
				);

			const handleLocalExecute = async (payloadObj?: object) => {
				if (executing) return;
				setExecuting(true);
				await props.data.onExecute(props.data.node, payloadObj);
				setExecuting(false);
			};

			const handleRemoteExecute = async (payloadObj?: object) => {
				if (executing || !props.data.onRemoteExecute) return;
				setExecuting(true);
				await props.data.onRemoteExecute(props.data.node, payloadObj);
				setExecuting(false);
			};

			if (Object.keys(props.data.node.pins).length <= 1)
				return (
					<div className="absolute left-0 top-0 translate-x-[calc(-120%)] flex flex-col gap-1">
						{canLocalExecute && (
							<button
								className="bg-background hover:bg-card group/play transition-all rounded-md hover:rounded-lg border p-1"
								onClick={() => handleLocalExecute()}
								title={i18next.t('executeLocally', 'Execute locally')}
							>
								<PlayCircleIcon className="w-3 h-3 group-hover/play:scale-110" />
							</button>
						)}
						{canRemoteExecute && (
							<button
								className="bg-background hover:bg-card group/play transition-all rounded-md hover:rounded-lg border p-1 relative"
								onClick={() => handleRemoteExecute()}
								title={i18next.t('executeOnServer', 'Execute on server')}
							>
								<CloudCog className="w-3 h-3 group-hover/play:scale-110" />
							</button>
						)}
					</div>
				);

			return (
				<Dialog
					open={payload.open}
					onOpenChange={(open) => setPayload((old) => ({ ...old, open }))}
				>
					<DialogTrigger asChild>
						<div className="absolute left-0 top-0 translate-x-[calc(-120%)] flex flex-col gap-1">
							<button
								className="bg-background hover:bg-card group/play transition-all rounded-md hover:rounded-lg border p-1"
								title={
									canLocalExecute ? "Execute locally" : i18next.t('executeOnServer', 'Execute on server')
								}
							>
								{canLocalExecute ? (
									<PlayCircleIcon className="w-3 h-3 group-hover/play:scale-110" />
								) : (
									<CloudCog className="w-3 h-3 group-hover/play:scale-110" />
								)}
							</button>
						</div>
					</DialogTrigger>
					<DialogContent className="max-w-lg">
						<DialogHeader>
							<DialogTitle>{i18next.t('executeFriendly_name', 'Execute {{friendly_name}}', { friendly_name: props.data.node.friendly_name })}</DialogTitle>
							<DialogDescription>
								{i18next.t('provideInputValuesForTheEventPayload', 'Provide input values for the event payload.')}
							</DialogDescription>
						</DialogHeader>
						<EventPayloadForm
							node={props.data.node}
							boardRef={props.data.boardRef}
							onLocalExecute={canLocalExecute ? handleLocalExecute : undefined}
							onRemoteExecute={
								canRemoteExecute ? handleRemoteExecute : undefined
							}
							canLocalExecute={canLocalExecute}
							canRemoteExecute={canRemoteExecute}
							onClose={() => setPayload((old) => ({ ...old, open: false }))}
						/>
					</DialogContent>
				</Dialog>
			);
		}, [
			props.data.node.start,
			payload,
			activeRunId,
			executing,
			executionStatus,
			props.data.onExecute,
			props.data.onRemoteExecute,
			props.data.isOffline,
			props.data.node,
			props.data.executionMode,
		]);

		return (
			<div
				key={`${props.id}__node`}
				ref={div}
				className={`bg-card! p-2 react-flow__node-default rounded-md! selectable focus:ring-2 relative group ${props.selected && "border-primary! border-2"} ${executionStatus === "done" ? "opacity-60" : "opacity-100"} ${props.data.isUnavailable && "opacity-50 border-dashed! border-destructive/60!"} ${isReroute && "w-4 max-w-4 max-h-3! overflow-y rounded-lg! p-[0.4rem]!"} ${!isReroute && "border-border!"}`}
				style={
					isReroute
						? nodeStyle
						: props.data.remoteExecuting
							? {
									boxShadow:
										"0 0 12px 2px rgba(59, 130, 246, 0.5), 0 0 4px 1px rgba(59, 130, 246, 0.3)",
								}
							: remoteSelections.length > 0
								? {
										boxShadow: `0 0 0 2px ${colorFromSub(remoteSelections[0]?.sub)}40, 0 0 12px 0 ${colorFromSub(remoteSelections[0]?.sub)}25`,
									}
								: {}
				}
			>
				{remoteSelections.length > 0 && (
					<div className="pointer-events-none absolute -top-5 left-0 flex items-center gap-0.5 z-10">
						<div className="flex items-center -space-x-1.5">
							{displayedRemoteSelections.map((participant) => {
								const color = colorFromSub(participant.sub);
								const userInfo = participant.sub
									? props.data.peerUsers?.get(participant.sub)
									: undefined;
								const name = userInfo?.truncatedName ?? "User";
								return (
									<div
										key={`${participant.clientId}-${participant.sub ?? "unknown"}`}
										className={`flex items-center gap-1 rounded-full border-2 bg-background/95 px-1 py-0.5 text-[0.5625rem] leading-none shadow-md backdrop-blur-sm transition-all duration-200 ${participant.isActive ? "animate-pulse scale-110 ring-2 ring-offset-1" : ""}`}
										style={{
											borderColor: color,
											...(participant.isActive ? { ringColor: color } : {}),
										}}
										title={name}
									>
										{userInfo?.avatarUrl ? (
											<img
												src={userInfo.avatarUrl}
												alt={name}
												className="h-3.5 w-3.5 rounded-full object-cover"
											/>
										) : (
											<span
												className="flex h-3.5 w-3.5 items-center justify-center rounded-full text-[8px] font-bold text-white"
												style={{
													background: `linear-gradient(135deg, ${color}, ${color}dd)`,
												}}
											>
												{name.charAt(0).toUpperCase()}
											</span>
										)}
										{displayedRemoteSelections.length <= 2 && (
											<span
												className="font-semibold max-w-14 truncate pr-0.5"
												style={{ color }}
											>
												{name}
											</span>
										)}
									</div>
								);
							})}
						</div>
						{extraRemoteSelections > 0 && (
							<div className="rounded-full border border-border bg-background/95 px-1.5 py-0.5 text-[0.5625rem] font-medium leading-none shadow-md">{`+${extraRemoteSelections}`}</div>
						)}
					</div>
				)}
				{props.data.remoteExecuting && (
					<div className="absolute inset-0 rounded-md pointer-events-none animate-pulse ring-2 ring-blue-400/60" />
				)}
				{playNode}
				{props.data.node.long_running && (
					<div className="absolute top-0 z-10 translate-y-[calc(-50%)] translate-x-[calc(-50%)] left-0 text-center bg-background rounded-full">
						<ClockIcon className="w-2 h-2 text-foreground" />
					</div>
				)}
				{props.data.node.only_offline && (
					<div
						className="absolute bottom-0 z-10 translate-y-[calc(50%)] translate-x-[calc(-50%)] left-0 text-center bg-background rounded-full"
						title={i18next.t('thisNodeCanOnlyRunLocally', 'This node can only run locally')}
					>
						<MonitorIcon className="w-2 h-2 text-blue-500" />
					</div>
				)}
				{isWasmNode && !isReroute && (
					<div
						className="absolute bottom-0 z-10 translate-y-[calc(50%)] translate-x-[calc(50%)] right-0 text-center bg-background rounded-full"
						title={i18next.t('wasmSandboxNodePackageVal', 'WASM sandbox node — package: {{val}}', { val: props.data.node.wasm?.package_id })}
					>
						<BoxIcon className="w-2 h-2 text-amber-500" />
					</div>
				)}
				{props.data.isUnavailable && !isReroute && (
					<div
						className="absolute top-0 z-10 translate-y-[calc(-50%)] translate-x-[calc(-50%)] left-1/2 text-center bg-destructive rounded-full p-0.5"
						title={i18next.t('thisNodesPackageIsNoLongerAvailable', 'This node\'s package is no longer available')}
					>
						<TriangleAlertIcon className="w-2 h-2 text-destructive-foreground" />
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
				{nodeHeat && !isReroute && (
					<>
						<div
							className="pointer-events-none absolute inset-0 rounded-md"
							style={{
								boxShadow: `inset 0 0 0 2px color-mix(in srgb, var(--primary) ${Math.round(
									20 + nodeHeat.intensity * 80,
								)}%, transparent)`,
							}}
						/>
						<div
							className="absolute bottom-0 left-0 z-10 flex translate-y-[calc(50%)] translate-x-[calc(-30%)] items-center gap-1"
							title={i18next.t('countRunsVisitedThisNode', {
								defaultValue_one: '{{count}} run visited this Node{{errors}}',
								defaultValue_other: '{{count}} runs visited this Node{{errors}}',
								count: nodeHeat.visits,
								errors:
									nodeHeat.errors > 0
										? ` · ${i18next.t('countRunsWithErrors', {
												defaultValue_one: '{{count}} with an error',
												defaultValue_other: '{{count}} with errors',
												count: nodeHeat.errors,
											})}`
										: '',
							})}
						>
							<span className="rounded-full bg-primary px-1.5 py-0.5 text-[8px] font-semibold leading-none tabular-nums text-primary-foreground">{`${nodeHeat.visits}×`}</span>
							{nodeHeat.errors > 0 && (
								<span className="rounded-full bg-destructive px-1.5 py-0.5 text-[8px] font-semibold leading-none tabular-nums text-destructive-foreground">{`${nodeHeat.errors}!`}</span>
							)}
						</div>
					</>
				)}
				{props.data.node.comment && (
					<div className="absolute top-0 translate-y-[calc(-100%-0.5rem)] left-3 right-3 mb-2 text-center bg-foreground/70 text-background p-1 rounded-md">
						<small className="font-normal text-extra-small leading-extra-small">
							{props.data.node.comment}
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
				{props.data.node.error && (
					<div className="absolute bottom-0 translate-y-[calc(100%+1rem)] left-3 right-3 mb-2 text-destructive-foreground bg-destructive p-1 rounded-md">
						<small className="font-normal text-extra-small leading-extra-small">
							{props.data.node.error}
						</small>
					</div>
				)}
				{renderInputPins}
				{renderFnRefInputs}
				{renderFnRefOutputs}
				{!isReroute && (
					<div
						className={`header absolute top-0 left-0 right-0 h-4 gap-1 flex flex-row items-center border-b p-1 justify-between rounded-md rounded-b-none bg-card ${props.data.functionLayerId && "bg-linear-to-r from-card via-violet-500/50 to-violet-500"} ${props.data.node.event_callback && "bg-linear-to-l  from-card via-primary/50 to-primary"} ${!isExec && !props.data.functionLayerId && "bg-linear-to-r  from-card via-tertiary/50 to-tertiary"} ${props.data.node.start && "bg-linear-to-r  from-card via-primary/50 to-primary"} ${isReroute && "w-6"}`}
					>
						<div className={"flex flex-row items-center gap-1 min-w-0"}>
							{props.data.node?.icon ? (
								<DynamicImage
									className="w-2 h-2 bg-foreground shrink-0"
									url={props.data.node.icon}
								/>
							) : (
								<WorkflowIcon className="w-2 h-2 shrink-0" />
							)}
							<small className="font-medium leading-none text-start truncate">
								<AutoResizeText
									text={props.data.node?.friendly_name}
									maxChars={30}
								/>
							</small>
						</div>
						<div className="flex flex-row items-center gap-1">
							{props.data.functionCache && (
								<span
									className="flex flex-row items-center gap-0.5 shrink-0 text-violet-100"
									title={cacheIndicatorLabel(props.data.functionCache)}
								>
									<DatabaseIcon className="w-2 h-2" />
									{props.data.functionCache.ttl_seconds ? (
										<span className="text-[7px] leading-none">
											{formatCacheTtl(props.data.functionCache.ttl_seconds)}
										</span>
									) : null}
								</span>
							)}
							{executed && (
								<ScrollTextIcon
									onClick={(e) => {
										e.stopPropagation();
										props.data.onFilterLogs?.(props.data.node.id);
									}}
									className="w-2 h-2 cursor-pointer hover:text-primary"
								/>
							)}
							{debouncedExecutionState === "running" && (
								<PuffLoader
									color={resolvedTheme === "dark" ? "white" : "black"}
									size={10}
									speedMultiplier={1}
								/>
							)}
							{debouncedExecutionState === "running" && (
								<span
									className={`text-[8px] ${getActivityColorClasses(runActivity.status).text}`}
								>
									{runActivity.formattedTime}
								</span>
							)}
							{debouncedExecutionState === "done" && (
								<SquareCheckIcon className="w-2 h-2 text-primary" />
							)}
						</div>
					</div>
				)}
				{renderOutputPins}
			</div>
		);
	},
	(prev, next) =>
		prev.props.data.hash === next.props.data.hash &&
		prev.props.selected === next.props.selected &&
		prev.props.data.fnRefsHash === next.props.data.fnRefsHash &&
		// The node's own hash does not move when the function it calls changes its
		// caching, so the indicator needs its own comparison.
		prev.props.data.functionCache?.enabled ===
			next.props.data.functionCache?.enabled &&
		prev.props.data.functionCache?.ttl_seconds ===
			next.props.data.functionCache?.ttl_seconds &&
		prev.props.data.functionCache?.scope ===
			next.props.data.functionCache?.scope &&
		prev.props.data.functionCache?.prefix ===
			next.props.data.functionCache?.prefix &&
		prev.props.data.isUnavailable === next.props.data.isUnavailable &&
		prev.props.data.remoteExecuting === next.props.data.remoteExecuting &&
		prev.props.data.remoteSelections === next.props.data.remoteSelections &&
		prev.props.data.peerUsers === next.props.data.peerUsers &&
		prev.props.data.selectorDataVersion === next.props.data.selectorDataVersion,
);

function FlowNode(props: NodeProps<FlowNode>) {
	const [isHovered, setIsHovered] = useState(false);
	const [commentMenu, setCommentMenu] = useState(false);
	const [renameMenu, setRenameMenu] = useState(false);
	const [editingMenu, setEditingMenu] = useState(false);
	const flow = useReactFlow();
	const { pushCommand, pushCommands } = useUndoRedo(
		props.data.appId,
		props.data.boardId,
	);
	const invalidate = useInvalidateInvoke();

	const copy = useCallback(async () => {
		props.data.onCopy();
	}, [flow]);

	const handleError = useCallback(async () => {
		if (typeof props.data.version !== "undefined") {
			return;
		}

		const node = flow.getNodes().find((node) => node.id === props.id);
		if (!node) return;

		const innerNode = node.data.node as INode;

		const handleErrorPin = Object.values(innerNode.pins).find(
			(pin) =>
				pin.name === "auto_handle_error" && pin.pin_type === IPinType.Output,
		);

		if (handleErrorPin) {
			const backend = useBackendStore.getState().backend;
			if (!backend) return;
			const filteredPins = Object.values(innerNode.pins).filter(
				(pin) =>
					pin.name !== "auto_handle_error" &&
					pin.name !== "auto_handle_error_string",
			);
			innerNode.pins = {};
			filteredPins
				.toSorted((a, b) => a.index - b.index)
				.forEach(
					(pin, index) => (innerNode.pins[pin.id] = { ...pin, index: index }),
				);
			let updateNode = updateNodeCommand({
				node: {
					...innerNode,
				},
			});

			updateNode = await backend.boardState.executeCommand(
				props.data.appId,
				props.data.boardId,
				updateNode,
			);
			await pushCommand(updateNode, false);
			invalidate(backend.boardState.getBoard, [
				props.data.appId,
				props.data.boardId,
			]);
			return;
		}

		const newPin: IPin = {
			name: "auto_handle_error",
			description: i18next.t("handlesNodeErrorsForYou", "Handles Node Errors for you.", { ns: "flow" }),
			pin_type: IPinType.Output,
			value_type: IValueType.Normal,
			data_type: IVariableType.Execution,
			id: createId(),
			index: 0,
			connected_to: [],
			depends_on: [],
			friendly_name: "On Error",
			default_value: convertJsonToUint8Array(false),
		};

		const stringPin: IPin = {
			name: "auto_handle_error_string",
			description: i18next.t("handlesNodeErrorsForYou", "Handles Node Errors for you.", { ns: "flow" }),
			pin_type: IPinType.Output,
			value_type: IValueType.Normal,
			data_type: IVariableType.String,
			id: createId(),
			index: 0,
			connected_to: [],
			depends_on: [],
			friendly_name: "Error",
			default_value: convertJsonToUint8Array(""),
		};

		const command = upsertPinCommand({
			node_id: innerNode.id,
			pin: newPin,
		});

		const stringCommand = upsertPinCommand({
			node_id: innerNode.id,
			pin: stringPin,
		});

		const backend = useBackendStore.getState().backend;
		if (!backend) return;

		const commands = await backend.boardState.executeCommands(
			props.data.appId,
			props.data.boardId,
			[command, stringCommand],
		);

		await pushCommands(commands);

		invalidate(backend.boardState.getBoard, [
			props.data.appId,
			props.data.boardId,
		]);
	}, [props.data.node, props.data.appId, props.data.boardId, flow]);

	const handleCollapse = useCallback(
		async (_x: number, _y: number) => {
			if (typeof props.data.version !== "undefined") {
				return;
			}

			const selectedNodes = flow.getNodes().filter((node) => node.selected);
			if (selectedNodes.length <= 1) return;

			// Calculate bounding box of selected nodes to position the collapsed layer
			let minX = Number.POSITIVE_INFINITY;
			let minY = Number.POSITIVE_INFINITY;
			for (const node of selectedNodes) {
				const x = node.position.x;
				const y = node.position.y;
				if (x < minX) minX = x;
				if (y < minY) minY = y;
			}

			const nodeIds = selectedNodes.map((node) => {
				const isNode = node.data.node as INode;
				if (isNode) return isNode.id;
				const isLayer = node.data.layer as ILayer;
				if (isLayer) return isLayer.id;
				const isComment = node.data.comment as IComment;
				if (isComment) return isComment.id;
				return "";
			});
			const command = upsertLayerCommand({
				layer: {
					id: createId(),
					comments: {},
					nodes: {},
					pins: {},
					parent_id: (selectedNodes[0].data.node as INode).layer,
					coordinates: [minX, minY, 0],
					in_coordinates: undefined,
					name: i18next.t("collapsed", "Collapsed", { ns: "flow" }),
					type: ILayerType.Collapsed,
					variables: {},
				},
				node_ids: nodeIds,
				current_layer: (selectedNodes[0].data.node as INode).layer,
			});

			const backend = useBackendStore.getState().backend;
			if (!backend) return;

			const result = await backend.boardState.executeCommand(
				props.data.appId,
				props.data.boardId,
				command,
			);
			await pushCommand(result, false);
			await invalidate(backend.boardState.getBoard, [
				props.data.appId,
				props.data.boardId,
			]);
		},
		[props.data.node, invalidate, pushCommands, flow],
	);

	const deleteNodes = useCallback(async () => {
		if (typeof props.data.version !== "undefined") {
			return;
		}

		const nodes = flow.getNodes().filter((node) => node.selected);
		if (!nodes || nodes.length === 0) return;

		const commands = nodes.map((node) => {
			return removeNodeCommand({
				node: node.data.node as INode,
				connected_nodes: [],
			});
		});
		const backend = useBackendStore.getState().backend;
		if (!backend) return;
		const result = await backend.boardState.executeCommands(
			props.data.appId,
			props.data.boardId,
			commands,
		);
		await pushCommands(result);
		await invalidate(backend.boardState.getBoard, [
			props.data.appId,
			props.data.boardId,
		]);
	}, [props.data.node, invalidate, pushCommands, flow]);

	const orderNodes = useCallback(
		async (
			type: "align" | "justify",
			dir: "start" | "end" | "center" | "distribute",
		) => {
			if (typeof props.data.version !== "undefined") {
				return;
			}

			const selectedNodes = flow.getNodes().filter((node) => node.selected);
			if (selectedNodes.length <= 1) return;
			let currentLayer: string | undefined = undefined;

			let start = Number.POSITIVE_INFINITY;
			let end = Number.NEGATIVE_INFINITY;

			selectedNodes.forEach((node) => {
				const nodeData = node.data.node as INode;
				if (nodeData?.layer) currentLayer = nodeData.layer;

				start = Math.min(
					start,
					type === "align" ? node.position.x : node.position.y,
				);
				end = Math.max(
					end,
					type === "align" ? node.position.x : node.position.y,
				);
			});

			if (
				start === Number.POSITIVE_INFINITY ||
				end === Number.NEGATIVE_INFINITY
			)
				return;

			const center = (start + end) / 2;

			let commands: IGenericCommand[];

			if (dir === "distribute") {
				// Even spacing: sort nodes along the relevant axis and distribute evenly
				const sorted = [...selectedNodes].sort((a, b) =>
					type === "align"
						? a.position.x - b.position.x
						: a.position.y - b.position.y,
				);
				const count = sorted.length;
				const step = count > 1 ? (end - start) / (count - 1) : 0;

				commands = sorted.map((node, i) => {
					return moveNodeCommand({
						node_id: node.id,
						from_coordinates: [node.position.x, node.position.y, 0],
						to_coordinates: [
							type === "align" ? start + i * step : node.position.x,
							type === "justify" ? start + i * step : node.position.y,
							0,
						],
						current_layer: currentLayer,
					});
				});
			} else {
				commands = selectedNodes.map((node) => {
					return moveNodeCommand({
						node_id: node.id,
						from_coordinates: [node.position.x, node.position.y, 0],
						to_coordinates: [
							type === "align"
								? dir === "start"
									? start
									: dir === "end"
										? end
										: center
								: node.position.x,
							type === "align"
								? node.position.y
								: dir === "start"
									? start
									: dir === "end"
										? end
										: center,
							0,
						],
						current_layer: currentLayer,
					});
				});
			}

			const backend = useBackendStore.getState().backend;
			if (!backend) return;

			const result = await backend.boardState.executeCommands(
				props.data.appId,
				props.data.boardId,
				commands,
			);

			pushCommands(result);
			await invalidate(backend.boardState.getBoard, [
				props.data.appId,
				props.data.boardId,
			]);
		},
		[props.data.node, invalidate, pushCommands, flow],
	);

	const isReadOnly = typeof props.data.version !== "undefined";

	const handleOpenInfo = useCallback(() => {
		props.data.onOpenInfo?.(props.data.node);
	}, [props.data.onOpenInfo, props.data.node]);

	const handleExplain = useCallback(() => {
		const selectedNodes = flow.getNodes().filter((node) => node.selected);
		const nodeIds =
			selectedNodes.length > 0
				? selectedNodes.map((node) => node.id)
				: [props.data.node.id];
		props.data.onExplain?.(nodeIds);
	}, [flow, props.data.node.id, props.data.onExplain]);

	// Stable callbacks for the toolbar so the memoized ToolbarButtons (and their
	// Radix Tooltip/Popper subtrees, which measure the DOM on render) don't
	// re-render every time this node re-renders (e.g. after a drag/drop).
	const handleOpenComment = useCallback(() => setCommentMenu(true), []);
	const handleOpenRename = useCallback(() => setRenameMenu(true), []);
	const handleOpenEdit = useCallback(() => setEditingMenu(true), []);

	return (
		<>
			{commentMenu && (
				<FlowNodeCommentMenu
					appId={props.data.appId}
					boardId={props.data.boardId}
					node={props.data.node}
					open={commentMenu}
					onOpenChange={(open) => setCommentMenu(open)}
				/>
			)}
			{renameMenu && (
				<FlowNodeRenameMenu
					appId={props.data.appId}
					boardId={props.data.boardId}
					node={props.data.node}
					open={renameMenu}
					onOpenChange={(open) => setRenameMenu(open)}
				/>
			)}
			{editingMenu && props.data.node.name === "events_generic" && (
				<LayerEditMenu
					open={editingMenu}
					onOpenChange={setEditingMenu}
					node={props.data.node}
					boardRef={props.data.boardRef}
					onApply={async (updated) => {
						const backend = useBackendStore.getState().backend;
						if (!backend) return;

						const currentNode = flow.getNode(props.id);
						if (!currentNode) return;

						const updatedNode = updated as INode;
						const command = updateNodeCommand({
							node: {
								...updatedNode,
								coordinates: [
									currentNode.position.x,
									currentNode.position.y,
									0,
								],
							},
						});

						const result = await backend.boardState.executeCommand(
							props.data.appId,
							props.data.boardId,
							command,
						);

						await pushCommand(result, false);
						await invalidate(backend.boardState.getBoard, [
							props.data.appId,
							props.data.boardId,
						]);
						setEditingMenu(false);
					}}
					mode="node"
				/>
			)}
			<div
				className="relative"
				onMouseEnter={() => setIsHovered(true)}
				onMouseLeave={() => setIsHovered(false)}
			>
				{(isHovered || props.selected) && (
					<FlowNodeToolbar
						node={props.data.node}
						appId={props.data.appId}
						boardId={props.data.boardId}
						isReadOnly={isReadOnly}
						onCopy={copy}
						onDelete={deleteNodes}
						onComment={handleOpenComment}
						onRename={handleOpenRename}
						onEdit={handleOpenEdit}
						onInfo={handleOpenInfo}
						onHandleError={handleError}
						onCollapse={handleCollapse}
						onAlign={orderNodes}
						onExplain={handleExplain}
					/>
				)}
				<FlowNodeInner props={props} />
			</div>
		</>
	);
}

function flowNodeAreEqual(
	prev: NodeProps<FlowNode>,
	next: NodeProps<FlowNode>,
) {
	return (
		prev.data.hash === next.data.hash &&
		prev.selected === next.selected &&
		prev.data.fnRefsHash === next.data.fnRefsHash &&
		prev.data.isUnavailable === next.data.isUnavailable &&
		prev.data.remoteSelections === next.data.remoteSelections &&
		prev.data.peerUsers === next.data.peerUsers &&
		prev.data.remoteExecuting === next.data.remoteExecuting &&
		prev.data.currentLayerId === next.data.currentLayerId &&
		prev.data.selectorDataVersion === next.data.selectorDataVersion
	);
}

const node = memo(FlowNode, flowNodeAreEqual);
export { node as FlowNode };

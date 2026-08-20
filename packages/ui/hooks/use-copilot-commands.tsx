"use client";

import { createId } from "@paralleldrive/cuid2";
import type { UseQueryResult } from "@tanstack/react-query";
import { XIcon } from "lucide-react";
import { useCallback } from "react";
import type { BoardCommand } from "../components/flow/flow-copilot";
import {
	IValueType,
	addNodeCommand,
	connectPinsCommand,
	disconnectPinsCommand,
	moveNodeCommand,
	removeCommentCommand,
	removeNodeCommand,
	removeVariableCommand,
	updateNodeCommand,
	upsertCommentCommand,
	upsertLayerCommand,
	upsertVariableCommand,
} from "../lib";
import { expectedCopilotPinType } from "../lib/copilot-command-pins";
import { getErrorMessage } from "../lib/error-message";
import {
	type FlowPilotCommandApplyFailure,
	executeFlowPilotCommandBatch,
	throwFlowPilotCommandApplyError,
} from "../lib/flowpilot-command-apply";
import { flowPilotDebugLog } from "../lib/flowpilot-debug";
import { toastError } from "../lib/messages";
import type { IGenericCommand } from "../lib/schema";
import {
	type IBoard,
	type IComment,
	ICommentType,
	ILayerType,
	type IVariable,
} from "../lib/schema/flow/board";
import type { PlaceholderPinDef } from "../lib/schema/flow/copilot";
import { type INode, IVariableType } from "../lib/schema/flow/node";
import { type IPin, IPinType } from "../lib/schema/flow/pin";
import type { ILayer } from "../lib/schema/flow/run";
import { convertJsonToUint8Array } from "../lib/uint8";

const MAX_COPILOT_BATCH_COMMANDS = 100;

/// A pin write that cannot be applied YET because the node's own `on_update` has not minted its
/// target pin — the config value that creates it is still queued in this same batch. Distinct from
/// a genuine failure so the write is retried on a later pass instead of being discarded.
const DEFER_PIN_UPDATE = Symbol("defer-pin-update");
const MAX_COPILOT_BATCH_BYTES = 4 * 1024 * 1024;
const DEFAULT_OUTPUT_PIN_ALIASES = new Set([
	"result",
	"value",
	"output",
	"out",
]);

interface UseCopilotCommandsProps {
	board: UseQueryResult<IBoard | undefined, Error>;
	catalog: UseQueryResult<INode[] | undefined, Error>;
	executeCommands: (
		commands: IGenericCommand[],
		options?: { refetch?: boolean },
	) => Promise<unknown>;
	currentLayer: string | undefined;
}

type UpdateNodePinCommand = Extract<
	BoardCommand,
	{ command_type: "UpdateNodePin" }
>;

type PinConnectionCommand = Extract<
	BoardCommand,
	{ command_type: "ConnectPins" | "DisconnectPins" }
>;

function cloneNode(node: INode): INode {
	return JSON.parse(JSON.stringify(node)) as INode;
}

function jsonByteLength(value: unknown): number {
	const json = JSON.stringify(value);
	if (typeof TextEncoder === "undefined") return json.length;
	return new TextEncoder().encode(json).length;
}

function encodedJsonValue(value: unknown): number[] | null {
	if (value === null || value === undefined) return null;
	return Array.from(convertJsonToUint8Array(value) || []);
}

function layerTypeFromCommand(value?: string): ILayerType {
	switch (value) {
		case "Function":
			return ILayerType.Function;
		case "Macro":
			return ILayerType.Macro;
		default:
			return ILayerType.Collapsed;
	}
}

function pinsFromDefs(
	pinDefs: PlaceholderPinDef[] | undefined,
	includeDefaultExec: boolean,
): Record<string, IPin> {
	const pins: Record<string, IPin> = {};
	let pinIndex = 0;

	const addPin = (
		name: string,
		friendlyName: string,
		pinType: IPinType,
		dataType: IVariableType,
		valueType = IValueType.Normal,
		description = "",
		schema?: string,
		enforceSchema = false,
	) => {
		const pin: IPin = {
			id: createId(),
			name,
			friendly_name: friendlyName,
			connected_to: [],
			depends_on: [],
			description,
			index: pinIndex++,
			pin_type: pinType,
			value_type: valueType,
			data_type: dataType,
			default_value: null,
			schema: schema ?? null,
			options: enforceSchema ? { enforce_schema: true } : null,
		};
		pins[pin.id] = pin;
	};

	if (includeDefaultExec) {
		addPin("exec_in", "Exec In", IPinType.Input, IVariableType.Execution);
		addPin("exec_out", "Exec Out", IPinType.Output, IVariableType.Execution);
	}

	for (const pinDef of pinDefs ?? []) {
		addPin(
			pinDef.name,
			pinDef.friendly_name,
			pinDef.pin_type as IPinType,
			pinDef.data_type as IVariableType,
			(pinDef.value_type as IValueType) || IValueType.Normal,
			pinDef.description || "",
			pinDef.schema,
			pinDef.enforce_schema ?? false,
		);
	}

	return pins;
}

function appendAdditionalNodePins(
	node: INode,
	pinDefs: PlaceholderPinDef[] | undefined,
): INode {
	if (!pinDefs?.length) return node;
	if (node.name !== "events_generic") {
		throw new Error(
			"Additional catalog-node pins are only supported on events_generic",
		);
	}

	const pins = { ...node.pins };
	let outputCount = Object.values(pins).filter(
		(pin) => pin.pin_type === IPinType.Output,
	).length;

	for (const pinDef of pinDefs) {
		if (
			pinDef.pin_type !== IPinType.Output ||
			pinDef.data_type === IVariableType.Execution
		) {
			throw new Error(
				`Additional events_generic pin "${pinDef.name}" must be a non-execution Output`,
			);
		}
		if (
			Object.values(pins).some(
				(pin) => pin.pin_type === IPinType.Output && pin.name === pinDef.name,
			)
		) {
			throw new Error(
				`events_generic already has an output pin named "${pinDef.name}"`,
			);
		}

		const id = createId();
		pins[id] = {
			id,
			name: pinDef.name,
			friendly_name: pinDef.friendly_name,
			description: pinDef.description ?? "",
			pin_type: IPinType.Output,
			data_type: pinDef.data_type as IVariableType,
			value_type: (pinDef.value_type as IValueType) ?? IValueType.Normal,
			index: ++outputCount,
			connected_to: [],
			depends_on: [],
			default_value: null,
			schema: pinDef.schema ?? null,
			options: pinDef.enforce_schema ? { enforce_schema: true } : null,
		};
	}

	return { ...node, pins };
}

function isSetupLayerCommand(cmd: BoardCommand): boolean {
	return (
		cmd.command_type === "CreateLayer" &&
		((cmd.node_ids ?? []).length === 0 ||
			Boolean(cmd.ref_id) ||
			Boolean(cmd.pins?.length) ||
			cmd.layer_type === "Function")
	);
}

function normalizePinValue(value: unknown): unknown {
	if (
		typeof value === "string" &&
		value.startsWith('"') &&
		value.endsWith('"')
	) {
		return value.slice(1, -1);
	}

	return value;
}

function toFlowScriptCamelCase(input: string): string {
	let output = "";
	let uppercaseNext = false;
	let first = true;

	for (const char of input) {
		if (/^[a-zA-Z0-9]$/.test(char)) {
			if (first) {
				output += char.toLowerCase();
				first = false;
			} else if (uppercaseNext) {
				output += char.toUpperCase();
			} else {
				output += char;
			}
			uppercaseNext = false;
		} else if (!first) {
			uppercaseNext = true;
		}
	}

	return output || "node";
}

function pinLookupKeys(value: string | null | undefined): string[] {
	if (!value) return [];
	const camelCase = toFlowScriptCamelCase(value);
	return [
		...new Set([
			value,
			value.toLowerCase(),
			camelCase,
			camelCase.toLowerCase(),
		]),
	];
}

function addPinLookup(
	pinMap: Map<string, string>,
	key: string | null | undefined,
	pinId: string,
) {
	for (const lookupKey of pinLookupKeys(key)) {
		if (!pinMap.has(lookupKey)) {
			pinMap.set(lookupKey, pinId);
		}
	}
}

/// How closely a pin answers to `pinRef`: 0 when its own name matches, 1 when only its friendly
/// name does, undefined when neither. A pin's own name must win — `string_format`'s config pin is
/// named `format_string` but presented as "Input", so an `{input}` placeholder would otherwise
/// resolve to the format string itself and overwrite the template with the placeholder's value.
function pinRefMatchRank(pin: IPin, pinRef: string): number | undefined {
	const requestedKeys = new Set(pinLookupKeys(pinRef));
	if (pinLookupKeys(pin.name).some((key) => requestedKeys.has(key))) return 0;
	if (pinLookupKeys(pin.friendly_name).some((key) => requestedKeys.has(key)))
		return 1;
	return undefined;
}

function pinMatchesDirection(
	pin: IPin | undefined,
	expectedPinType?: IPinType,
): pin is IPin {
	return Boolean(pin && (!expectedPinType || pin.pin_type === expectedPinType));
}

function dataOutputPins(node: INode): IPin[] {
	return Object.values(node.pins ?? {})
		.filter(
			(pin) =>
				pin.pin_type === IPinType.Output &&
				pin.data_type !== IVariableType.Execution,
		)
		.sort((a, b) => a.index - b.index);
}

function defaultDataOutputPin(node: INode): IPin | undefined {
	const outputs = dataOutputPins(node);
	if (outputs.length === 1) return outputs[0];
	return outputs.find((pin) =>
		DEFAULT_OUTPUT_PIN_ALIASES.has(pin.name.toLowerCase()),
	);
}

export function useCopilotCommands({
	board,
	catalog,
	executeCommands,
	currentLayer,
}: UseCopilotCommandsProps) {
	const handleExecuteCommands = useCallback(
		async (commands: BoardCommand[]) => {
			const pendingConnectionCommands = commands.filter(
				(command): command is PinConnectionCommand =>
					command.command_type === "ConnectPins" ||
					command.command_type === "DisconnectPins",
			);
			let latestBoardNodes: Record<string, INode> = board.data?.nodes ?? {};
			let latestBoardLayers: Record<string, ILayer> = board.data?.layers ?? {};
			let latestBoardVariables: Record<string, IVariable> =
				board.data?.variables ?? {};
			let latestBoardComments: Record<string, IComment> =
				board.data?.comments ?? {};
			let appliedGenericCommandCount = 0;
			const commandFailures: FlowPilotCommandApplyFailure[] = [];
			const recordedFailureKeys = new Set<string>();
			const recordCommandFailure = (
				command: BoardCommand,
				phase: string,
				message: string,
			) => {
				const queueIndex = commands.indexOf(command);
				const failure: FlowPilotCommandApplyFailure = {
					queueIndex: queueIndex >= 0 ? queueIndex : undefined,
					phase,
					commandType: command.command_type,
					message,
				};
				const key = `${failure.queueIndex ?? "unknown"}:${phase}:${message}`;
				if (!recordedFailureKeys.has(key)) {
					recordedFailureKeys.add(key);
					commandFailures.push(failure);
				}
				toastError(message, <XIcon />);
			};

			const nodeReferenceMap = new Map<string, INode>();
			const ambiguousNodeRefs = new Set<string>();
			const pinIdMap = new Map<string, Map<string, string>>();

			const existingNodes = Object.values(latestBoardNodes);
			let baseX = 100;
			let baseY = 100;

			if (existingNodes.length > 0) {
				const rightmostNode = existingNodes.reduce((max, node) => {
					const x = node.coordinates?.[0] ?? 0;
					return x > (max.coordinates?.[0] ?? 0) ? node : max;
				});
				baseX = (rightmostNode.coordinates?.[0] ?? 0) + 300;
				baseY = rightmostNode.coordinates?.[1] ?? 100;
			}

			const layerAsNode = (layer: ILayer): INode =>
				({
					id: layer.id,
					name: layer.name,
					friendly_name: layer.name,
					pins: layer.pins,
					coordinates: layer.coordinates,
				}) as unknown as INode;

			const buildPinMapping = (nodeRef: string, node: INode) => {
				if (ambiguousNodeRefs.has(nodeRef)) return;
				const pinMap = new Map<string, string>();
				// Two passes, names first: `addPinLookup` is first-writer-wins, so registering a
				// pin's friendly name in the same pass lets it claim a key another pin owns by
				// its real name. See `pinRefMatchRank`.
				for (const pin of Object.values(node.pins ?? {})) {
					addPinLookup(pinMap, pin.name, pin.id);
				}
				for (const pin of Object.values(node.pins ?? {})) {
					addPinLookup(pinMap, pin.friendly_name, pin.id);
				}

				const defaultOutputPin = defaultDataOutputPin(node);
				if (defaultOutputPin) {
					for (const alias of DEFAULT_OUTPUT_PIN_ALIASES) {
						addPinLookup(pinMap, alias, defaultOutputPin.id);
					}
				}
				pinIdMap.set(nodeRef, pinMap);
			};

			const registerNodeRef = (ref: string, node: INode) => {
				if (ambiguousNodeRefs.has(ref)) return;

				const mapped = nodeReferenceMap.get(ref);
				if (mapped && mapped.id !== node.id) {
					nodeReferenceMap.delete(ref);
					pinIdMap.delete(ref);
					ambiguousNodeRefs.add(ref);
					return;
				}

				nodeReferenceMap.set(ref, node);
				buildPinMapping(ref, node);
			};

			const registerNodeRefs = (
				refs: Array<string | null | undefined>,
				node: INode,
			) => {
				for (const ref of [...new Set(refs.filter(Boolean) as string[])]) {
					registerNodeRef(ref, node);
				}
			};

			const replaceMappedNode = (node: INode) => {
				buildPinMapping(node.id, node);
				for (const [ref, mapped] of Array.from(nodeReferenceMap.entries())) {
					if (mapped.id === node.id) {
						nodeReferenceMap.set(ref, node);
						buildPinMapping(ref, node);
					}
				}
			};

			const removeMappedNode = (nodeId: string) => {
				pinIdMap.delete(nodeId);
				for (const [ref, mapped] of Array.from(nodeReferenceMap.entries())) {
					if (mapped.id === nodeId) {
						nodeReferenceMap.delete(ref);
						pinIdMap.delete(ref);
					}
				}
			};

			const resolveNode = (ref: string): INode | undefined => {
				if (latestBoardNodes[ref]) return latestBoardNodes[ref];
				if (ambiguousNodeRefs.has(ref)) return undefined;
				if (nodeReferenceMap.has(ref)) return nodeReferenceMap.get(ref);
				if (latestBoardLayers[ref]) return layerAsNode(latestBoardLayers[ref]);
				return undefined;
			};

			const resolveNodeId = (ref: string): string =>
				resolveNode(ref)?.id ?? nodeReferenceMap.get(ref)?.id ?? ref;

			const resolveLayerId = (ref?: string | null): string | undefined => {
				if (!ref) return undefined;
				if (latestBoardLayers[ref]) return ref;
				return nodeReferenceMap.get(ref)?.id ?? ref;
			};

			const resolveLayer = (ref: string): ILayer | undefined => {
				const layerId = resolveLayerId(ref);
				return layerId ? latestBoardLayers[layerId] : undefined;
			};

			const resolveNodeIds = (refs: string[] = []): string[] =>
				refs.map((ref) => resolveNodeId(ref));

			const resolvePinId = (
				nodeRef: string,
				pinRef: string,
				expectedPinType?: IPinType,
			): string | undefined => {
				const effectivePinType = expectedCopilotPinType(
					expectedPinType,
					Boolean(resolveLayer(nodeRef)),
				);
				const nodePinMap = pinIdMap.get(nodeRef);
				if (nodePinMap) {
					const node = resolveNode(nodeRef);
					for (const lookupKey of pinLookupKeys(pinRef)) {
						const mappedPinId = nodePinMap.get(lookupKey);
						const mappedPin = mappedPinId ? node?.pins[mappedPinId] : undefined;
						if (pinMatchesDirection(mappedPin, effectivePinType)) {
							return mappedPinId;
						}
					}
				}

				const node = resolveNode(nodeRef);
				if (!node) return undefined;

				const pinById = node.pins[pinRef];
				if (pinMatchesDirection(pinById, effectivePinType)) return pinRef;

				const ranked = Object.values(node.pins)
					.filter((pin) => pinMatchesDirection(pin, effectivePinType))
					.map((pin) => ({ pin, rank: pinRefMatchRank(pin, pinRef) }))
					.filter((entry) => entry.rank !== undefined)
					.sort((left, right) => (left.rank ?? 0) - (right.rank ?? 0));
				if (ranked.length > 0) return ranked[0].pin.id;

				if (
					effectivePinType !== IPinType.Input &&
					DEFAULT_OUTPUT_PIN_ALIASES.has(pinRef.toLowerCase())
				) {
					const defaultOutputPin = defaultDataOutputPin(node);
					if (pinMatchesDirection(defaultOutputPin, effectivePinType)) {
						return defaultOutputPin.id;
					}
				}

				console.warn(
					`Pin "${pinRef}" not found in node "${node.friendly_name || node.name}". Available pins:`,
					Object.values(node.pins).map((p) => ({
						id: p.id,
						name: p.name,
						type: p.pin_type,
					})),
				);
				return undefined;
			};

			const rebuildPinMappings = () => {
				pinIdMap.clear();
				for (const [nodeId, node] of Object.entries(latestBoardNodes)) {
					buildPinMapping(nodeId, node);
				}
				for (const [layerId, layer] of Object.entries(latestBoardLayers)) {
					buildPinMapping(layerId, layerAsNode(layer));
				}
				for (const [ref, node] of Array.from(nodeReferenceMap.entries())) {
					const freshNode = latestBoardNodes[node.id];
					if (freshNode) {
						nodeReferenceMap.set(ref, freshNode);
						buildPinMapping(ref, freshNode);
						continue;
					}

					const freshLayer = latestBoardLayers[node.id];
					if (freshLayer) {
						const freshLayerNode = layerAsNode(freshLayer);
						nodeReferenceMap.set(ref, freshLayerNode);
						buildPinMapping(ref, freshLayerNode);
						continue;
					}

					buildPinMapping(ref, node);
				}
			};

			const refreshBoardSnapshot = async () => {
				const freshBoard = await board.refetch();
				if (freshBoard.error) throw freshBoard.error;
				const data = freshBoard.data ?? board.data;
				latestBoardNodes = data?.nodes ?? latestBoardNodes;
				latestBoardLayers = data?.layers ?? latestBoardLayers;
				latestBoardVariables = data?.variables ?? latestBoardVariables;
				latestBoardComments = data?.comments ?? latestBoardComments;
				rebuildPinMappings();
				return freshBoard;
			};

			const applyExecutedCommandsToSnapshot = (
				executedCommands: IGenericCommand[],
			) => {
				for (const command of executedCommands) {
					switch (command.command_type) {
						case "AddNode":
						case "UpdateNode":
						case "MoveNode":
							if (command.node) {
								latestBoardNodes[command.node.id] = command.node;
								replaceMappedNode(command.node);
							}
							break;
						case "RemoveNode":
							if (command.node?.id) {
								delete latestBoardNodes[command.node.id];
								removeMappedNode(command.node.id);
							}
							break;
						case "UpsertLayer":
							if (command.layer) {
								latestBoardLayers[command.layer.id] = command.layer;
								registerNodeRefs(
									[command.layer.id, command.layer.name],
									layerAsNode(command.layer),
								);
							}
							break;
						case "RemoveLayer":
							if (command.layer?.id) {
								delete latestBoardLayers[command.layer.id];
								removeMappedNode(command.layer.id);
							}
							break;
						case "UpsertVariable":
							if (command.variable) {
								latestBoardVariables[command.variable.id] = command.variable;
							}
							break;
						case "RemoveVariable":
							if (command.variable?.id) {
								delete latestBoardVariables[command.variable.id];
							}
							break;
						case "UpsertComment":
							if (command.comment) {
								latestBoardComments[command.comment.id] = command.comment;
							}
							break;
						case "RemoveComment":
							if (command.comment?.id) {
								delete latestBoardComments[command.comment.id];
							}
							break;
					}
				}
				rebuildPinMappings();
			};

			rebuildPinMappings();
			let executedAnyCommands = false;
			let refreshedAfterLastExecution = false;

			const executeInBatches = async (
				genericCommands: IGenericCommand[],
				label: string,
				options: { refetch?: boolean } = {},
			): Promise<IGenericCommand[]> => {
				if (genericCommands.length === 0) return [];

				let batch: IGenericCommand[] = [];
				let batchBytes = 2;
				let batchIndex = 0;
				const executedCommands: IGenericCommand[] = [];

				const flush = async () => {
					if (batch.length === 0) return;
					batchIndex++;
					flowPilotDebugLog(
						`[FlowPilot] Executing ${label} batch ${batchIndex}`,
						{
							commands: batch.length,
							approxBytes: batchBytes,
						},
					);
					const executedBatch =
						await executeFlowPilotCommandBatch<IGenericCommand>({
							requestedCommands: commands.length,
							alreadyAppliedCommands: appliedGenericCommandCount,
							expectedBatchCommands: batch.length,
							phase: `${label} batch ${batchIndex}`,
							commandType: batch[0]?.command_type ?? "GenericCommandBatch",
							execute: () =>
								executeCommands([...batch], {
									refetch: options.refetch ?? false,
								}),
							refetch: refreshBoardSnapshot,
						});
					executedCommands.push(...executedBatch);
					appliedGenericCommandCount += executedBatch.length;
					batch = [];
					batchBytes = 2;
				};

				for (const command of genericCommands) {
					const commandBytes = jsonByteLength(command) + 1;
					if (
						batch.length > 0 &&
						(batch.length >= MAX_COPILOT_BATCH_COMMANDS ||
							batchBytes + commandBytes > MAX_COPILOT_BATCH_BYTES)
					) {
						await flush();
					}
					batch.push(command);
					batchBytes += commandBytes;
				}

				await flush();
				applyExecutedCommandsToSnapshot(executedCommands);
				return executedCommands;
			};

			const nodeCreateCommands: IGenericCommand[] = [];
			let nodeIndex = 0;

			for (const cmd of commands) {
				if (cmd.command_type === "CreateLayer" && isSetupLayerCommand(cmd)) {
					const layerId = createId();
					const layerType = layerTypeFromCommand(cmd.layer_type);
					if (cmd.cache != null && layerType !== ILayerType.Function) {
						recordCommandFailure(
							cmd,
							"layer creation",
							`Cannot configure function cache on non-Function layer "${cmd.name}"`,
						);
						continue;
					}
					const position = cmd.position || {
						x: baseX + (nodeIndex % 3) * 300,
						y: baseY + Math.floor(nodeIndex / 3) * 200,
					};
					const targetLayer = resolveLayerId(cmd.target_layer) ?? currentLayer;
					const layer: ILayer = {
						id: layerId,
						name: cmd.name,
						type: layerType,
						color: cmd.color || null,
						coordinates: [position.x, position.y, 0],
						nodes: {},
						variables: {},
						comments: {},
						pins: pinsFromDefs(cmd.pins, false),
						cache: cmd.cache ?? null,
						parent_id: targetLayer,
					};

					nodeCreateCommands.push(
						upsertLayerCommand({
							layer,
							node_ids: [],
							current_layer: targetLayer,
						}),
					);
					latestBoardLayers[layerId] = layer;
					registerNodeRefs(
						[cmd.ref_id, `$${nodeIndex}`, cmd.name, layerId],
						layerAsNode(layer),
					);
					flowPilotDebugLog(`[CreateLayer] Queued "${cmd.name}" (${layerId})`, {
						refs: [cmd.ref_id, `$${nodeIndex}`, cmd.name, layerId].filter(
							Boolean,
						),
					});
					nodeIndex++;
					continue;
				}

				if (cmd.command_type === "AddNode") {
					const catalogNode = catalog.data?.find(
						(node) => node.name === cmd.node_type,
					);
					if (!catalogNode) {
						recordCommandFailure(
							cmd,
							"node creation",
							`Node type "${cmd.node_type}" was not found in the current catalog`,
						);
						continue;
					}

					const position = cmd.position || {
						x: baseX + (nodeIndex % 3) * 300,
						y: baseY + Math.floor(nodeIndex / 3) * 200,
					};
					const targetLayer = resolveLayerId(cmd.target_layer) ?? currentLayer;

					let result: ReturnType<typeof addNodeCommand>;
					try {
						result = addNodeCommand({
							node: appendAdditionalNodePins(
								{
									...cloneNode(catalogNode),
									coordinates: [position.x, position.y, 0],
									friendly_name: cmd.friendly_name ?? catalogNode.friendly_name,
								},
								cmd.additional_pins,
							),
							current_layer: targetLayer,
						});
					} catch (error) {
						recordCommandFailure(
							cmd,
							"node creation",
							`Cannot create node "${cmd.node_type}": ${getErrorMessage(error)}`,
						);
						continue;
					}
					const plannedNode = result.node as INode;

					nodeCreateCommands.push(result.command);
					registerNodeRefs(
						[cmd.ref_id, `$${nodeIndex}`, cmd.node_type, plannedNode.id],
						plannedNode,
					);

					flowPilotDebugLog(
						`[AddNode] Queued "${plannedNode.friendly_name}" (${plannedNode.id})`,
						{
							refs: [
								cmd.ref_id,
								`$${nodeIndex}`,
								cmd.node_type,
								plannedNode.id,
							].filter(Boolean),
						},
					);
					nodeIndex++;
					continue;
				}

				if (cmd.command_type === "AddPlaceholder") {
					const layerId = createId();
					const position = cmd.position || {
						x: baseX + (nodeIndex % 3) * 300,
						y: baseY + Math.floor(nodeIndex / 3) * 200,
					};
					const targetLayer = resolveLayerId(cmd.target_layer) ?? currentLayer;

					const layer: ILayer = {
						id: layerId,
						name: cmd.name,
						type: ILayerType.Collapsed,
						coordinates: [position.x, position.y, 0],
						nodes: {},
						variables: {},
						comments: {},
						pins: pinsFromDefs(cmd.pins, true),
						parent_id: targetLayer,
					};

					nodeCreateCommands.push(
						upsertLayerCommand({
							layer,
							node_ids: [],
							current_layer: targetLayer,
						}),
					);
					latestBoardLayers[layerId] = layer;

					const placeholderNode = layerAsNode(layer);
					registerNodeRefs(
						[cmd.ref_id, `$${nodeIndex}`, cmd.name, layerId],
						placeholderNode,
					);
					flowPilotDebugLog(
						`[AddPlaceholder] Queued "${cmd.name}" (${layerId})`,
						{
							refs: [cmd.ref_id, `$${nodeIndex}`, cmd.name, layerId].filter(
								Boolean,
							),
						},
					);
					nodeIndex++;
				}
			}

			const executedNodeCreateCommands = await executeInBatches(
				nodeCreateCommands,
				"node creation",
			);
			if (executedNodeCreateCommands.length > 0) {
				executedAnyCommands = true;
				refreshedAfterLastExecution = false;
			}

			const variableCreateCommands: IGenericCommand[] = [];
			for (const cmd of commands) {
				if (cmd.command_type !== "CreateVariable") continue;

				const variableId = cmd.variable_id || createId();
				const targetLayer = resolveLayerId(cmd.target_layer) ?? null;
				const variable: IVariable = {
					id: variableId,
					name: cmd.name,
					data_type: (cmd.data_type as IVariableType) || IVariableType.String,
					value_type: (cmd.value_type as IValueType) || IValueType.Normal,
					default_value:
						"default_value" in cmd
							? (encodedJsonValue(cmd.default_value) ?? [])
							: null,
					description: cmd.description || null,
					category: cmd.category || null,
					schema: cmd.schema || null,
					editable: cmd.editable ?? true,
					exposed: cmd.exposed ?? false,
					secret: cmd.secret ?? false,
					runtime_configured: cmd.runtime_configured ?? false,
				};

				flowPilotDebugLog(
					`[CreateVariable] Queued ${cmd.name} (${cmd.data_type})`,
				);
				variableCreateCommands.push(
					upsertVariableCommand({ variable, layer_id: targetLayer }),
				);
				latestBoardVariables[variable.id] = variable;
			}

			const executedVariableCreateCommands = await executeInBatches(
				variableCreateCommands,
				"variable creation",
			);
			if (executedVariableCreateCommands.length > 0) {
				executedAnyCommands = true;
				refreshedAfterLastExecution = false;
			}

			const buildUpdateNodePinCommand = (
				cmd: UpdateNodePinCommand,
				{ deferrable }: { deferrable: boolean },
			): IGenericCommand | null | typeof DEFER_PIN_UPDATE => {
				const nodeId = resolveNodeId(cmd.node_id);
				const node = latestBoardNodes[nodeId] ?? resolveNode(cmd.node_id);

				if (!node) {
					console.error(
						`[UpdateNodePin] FAILED - Node not found: ${cmd.node_id}`,
						{
							command: cmd,
							availableNodeRefs: Array.from(nodeReferenceMap.keys()),
							boardNodeIds: Object.keys(latestBoardNodes),
						},
					);
					recordCommandFailure(
						cmd,
						"pin update",
						`Pin update failed: Node "${cmd.node_id}" was not found`,
					);
					return null;
				}

				const pinId = resolvePinId(cmd.node_id, cmd.pin_id, IPinType.Input);
				const pin = pinId ? node.pins[pinId] : undefined;

				if (!pin || !pinId) {
					if (deferrable) {
						// Dynamic pins (a `string_format` placeholder, a `$param`) exist only after the
						// config write in this same batch reaches the board and `on_update` runs. Hold
						// the write for a later pass rather than dropping it.
						return DEFER_PIN_UPDATE;
					}
					console.error(
						`[UpdateNodePin] FAILED - Pin not found: ${cmd.pin_id} in ${node.friendly_name}`,
						{
							command: cmd,
							pin_requested: cmd.pin_id,
							pinId_resolved: pinId,
							availablePins: Object.values(node.pins).map((p) => ({
								name: p.name,
								id: p.id,
								type: p.pin_type,
							})),
						},
					);
					recordCommandFailure(
						cmd,
						"pin update",
						`Pin update failed: Pin "${cmd.pin_id}" was not found in "${node.friendly_name}"`,
					);
					return null;
				}

				let encodedValue: number[] | null = null;
				if (cmd.value !== null && cmd.value !== undefined) {
					const encoded = convertJsonToUint8Array(normalizePinValue(cmd.value));
					if (!encoded) {
						console.error(
							"[UpdateNodePin] FAILED - Could not encode value:",
							cmd.value,
						);
						recordCommandFailure(
							cmd,
							"pin update",
							`Pin update failed: The value for "${cmd.pin_id}" could not be encoded`,
						);
						return null;
					}
					encodedValue = Array.from(encoded);
				}

				flowPilotDebugLog(
					`[UpdateNodePin] Queued ${node.friendly_name}.${cmd.pin_id} = ${JSON.stringify(cmd.value)}`,
					{ encodedValue, originalValue: cmd.value, pinId },
				);

				const updatedNode: INode = {
					...node,
					pins: {
						...node.pins,
						[pinId]: {
							...pin,
							default_value: encodedValue,
						},
					},
				};

				latestBoardNodes[updatedNode.id] = updatedNode;
				replaceMappedNode(updatedNode);

				return updateNodeCommand({
					node: updatedNode,
					old_node: node,
				});
			};

			const pendingPinUpdates = commands.filter(
				(cmd): cmd is UpdateNodePinCommand =>
					cmd.command_type === "UpdateNodePin",
			);

			let remainingPinUpdates = [...pendingPinUpdates];
			while (remainingPinUpdates.length > 0) {
				const usedNodeIds = new Set<string>();
				const consumedIndexes = new Set<number>();
				const pinUpdateBatch: IGenericCommand[] = [];

				for (let index = 0; index < remainingPinUpdates.length; index++) {
					const cmd = remainingPinUpdates[index];
					const nodeId = resolveNodeId(cmd.node_id);
					if (usedNodeIds.has(nodeId)) continue;

					const genericCommand = buildUpdateNodePinCommand(cmd, {
						deferrable: true,
					});
					// A deferred write neither leaves the queue nor claims this node's slot in the
					// pass, so the config write that mints its pin — usually the node's very next
					// command — can still land now and resolve it on the following pass.
					if (genericCommand === DEFER_PIN_UPDATE) continue;
					consumedIndexes.add(index);
					if (!genericCommand) continue;

					usedNodeIds.add(nodeId);
					pinUpdateBatch.push(genericCommand);
				}

				if (consumedIndexes.size === 0) {
					// No pass can make further progress, so the remaining writes are not waiting on
					// anything: surface them as failures instead of discarding them silently.
					for (const deferred of remainingPinUpdates) {
						buildUpdateNodePinCommand(deferred, { deferrable: false });
					}
					break;
				}

				if (pinUpdateBatch.length > 0) {
					const executedPinUpdateCommands = await executeInBatches(
						pinUpdateBatch,
						"pin update",
					);
					if (executedPinUpdateCommands.length > 0) {
						executedAnyCommands = true;
						refreshedAfterLastExecution = false;
					}
					// No per-pass board.refetch(): executeInBatches already updates the
					// optimistic snapshot that resolveNode/resolvePinId read from, so the
					// next pass resolves correctly. The single visible refetch happens once
					// at end-of-generation (fallback below), avoiding a full parseBoard of
					// all nodes on every pass.
				}

				remainingPinUpdates = remainingPinUpdates.filter(
					(_, index) => !consumedIndexes.has(index),
				);
			}

			const layerWithNodeIds = (layer: ILayer, nodeIds: string[]): ILayer => ({
				...layer,
				nodes: Object.fromEntries(
					nodeIds
						.map((nodeId) => [
							nodeId,
							latestBoardNodes[nodeId] ?? layer.nodes?.[nodeId],
						])
						.filter((entry): entry is [string, INode] => Boolean(entry[1])),
				),
			});

			const remainingGenericCommands: IGenericCommand[] = [];

			for (const cmd of commands) {
				switch (cmd.command_type) {
					case "AddNode":
					case "AddPlaceholder":
					case "CreateVariable":
					case "UpdateNodePin":
						break;

					case "RemoveNode": {
						const node = resolveNode(cmd.node_id);
						if (!node) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot remove node: "${cmd.node_id}" was not found`,
							);
							break;
						}

						remainingGenericCommands.push(
							removeNodeCommand({
								node,
								connected_nodes: [],
							}),
						);
						delete latestBoardNodes[node.id];
						removeMappedNode(node.id);
						break;
					}

					case "ConnectPins":
					case "DisconnectPins":
						// Resolve connection endpoints only after all contract-changing board edits
						// have executed and the dynamic node pins have been refreshed.
						break;

					case "MoveNode": {
						const node = resolveNode(cmd.node_id);
						if (!node) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot move node: "${cmd.node_id}" was not found`,
							);
							break;
						}

						const targetLayer =
							resolveLayerId(cmd.target_layer) ?? currentLayer;
						const movedNode: INode = {
							...node,
							coordinates: [cmd.position.x, cmd.position.y, 0],
						};

						remainingGenericCommands.push(
							moveNodeCommand({
								node_id: node.id,
								to_coordinates: [cmd.position.x, cmd.position.y, 0],
								current_layer: targetLayer,
							}),
						);
						latestBoardNodes[node.id] = movedNode;
						replaceMappedNode(movedNode);
						break;
					}

					case "RenameNode": {
						const node = resolveNode(cmd.node_id);
						if (!node) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot rename node: "${cmd.node_id}" was not found`,
							);
							break;
						}
						const renamedNode: INode = {
							...node,
							friendly_name: cmd.friendly_name,
						};
						remainingGenericCommands.push(
							updateNodeCommand({ node: renamedNode, old_node: node }),
						);
						latestBoardNodes[renamedNode.id] = renamedNode;
						replaceMappedNode(renamedNode);
						break;
					}

					case "SetNodeFunctionRefs":
						recordCommandFailure(
							cmd,
							"board edit",
							"Function references require the atomic FlowScript apply path and cannot be safely applied from a client-side command queue",
						);
						break;

					case "UpdateVariable": {
						const existingVariable = latestBoardVariables[cmd.variable_id];
						if (!existingVariable) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot update variable: "${cmd.variable_id}" was not found`,
							);
							break;
						}

						const updatedVariable: IVariable = {
							...existingVariable,
							name: cmd.name ?? existingVariable.name,
							data_type:
								(cmd.data_type as IVariableType | undefined) ??
								existingVariable.data_type,
							value_type:
								(cmd.value_type as IValueType | undefined) ??
								existingVariable.value_type,
							default_value: cmd.clear_default_value
								? null
								: "default_value" in cmd
									? (encodedJsonValue(cmd.default_value) ?? [])
									: "value" in cmd
										? (encodedJsonValue(cmd.value) ?? [])
										: existingVariable.default_value,
							description: cmd.clear_description
								? null
								: "description" in cmd
									? (cmd.description ?? null)
									: existingVariable.description,
							category: cmd.clear_category
								? null
								: "category" in cmd
									? (cmd.category ?? null)
									: existingVariable.category,
							schema: cmd.clear_schema
								? null
								: "schema" in cmd
									? (cmd.schema ?? null)
									: existingVariable.schema,
							exposed: cmd.exposed ?? existingVariable.exposed,
							secret: cmd.secret ?? existingVariable.secret,
							editable: cmd.editable ?? existingVariable.editable,
							runtime_configured:
								cmd.runtime_configured ?? existingVariable.runtime_configured,
						};

						flowPilotDebugLog(
							`[UpdateVariable] Queued ${existingVariable.name} = ${JSON.stringify(cmd.value)}`,
						);
						remainingGenericCommands.push(
							upsertVariableCommand({
								variable: updatedVariable,
								old_variable: existingVariable,
							}),
						);
						latestBoardVariables[updatedVariable.id] = updatedVariable;
						break;
					}

					case "DeleteVariable": {
						const variableToDelete = latestBoardVariables[cmd.variable_id];
						if (!variableToDelete) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot delete variable: "${cmd.variable_id}" was not found`,
							);
							break;
						}

						flowPilotDebugLog(
							`[DeleteVariable] Queued ${variableToDelete.name}`,
						);
						remainingGenericCommands.push(
							removeVariableCommand({ variable: variableToDelete }),
						);
						delete latestBoardVariables[cmd.variable_id];
						break;
					}

					case "CreateComment": {
						const commentId = createId();
						const comment: IComment = {
							id: commentId,
							content: cmd.content,
							comment_type: ICommentType.Text,
							coordinates: cmd.position
								? [cmd.position.x, cmd.position.y, 0]
								: [baseX, baseY, 0],
							width: cmd.width ?? 200,
							height: cmd.height ?? 100,
							color: cmd.color || null,
							timestamp: {
								nanos_since_epoch: 0,
								secs_since_epoch: Math.floor(Date.now() / 1000),
							},
							author: "copilot",
						};
						const targetLayer =
							resolveLayerId(cmd.target_layer) ?? currentLayer;

						flowPilotDebugLog(
							`[CreateComment] Queued "${cmd.content.slice(0, 30)}..."`,
						);
						remainingGenericCommands.push(
							upsertCommentCommand({ comment, current_layer: targetLayer }),
						);
						latestBoardComments[comment.id] = comment;
						break;
					}

					case "UpdateComment": {
						const existingComment = latestBoardComments[cmd.comment_id];
						if (!existingComment) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot update comment: "${cmd.comment_id}" was not found`,
							);
							break;
						}

						const updatedComment: IComment = {
							...existingComment,
							content: cmd.content ?? existingComment.content,
							color: cmd.color ?? existingComment.color,
						};

						flowPilotDebugLog(
							`[UpdateComment] Queued "${updatedComment.content.slice(0, 30)}..."`,
						);
						remainingGenericCommands.push(
							upsertCommentCommand({
								comment: updatedComment,
								current_layer: currentLayer,
								old_comment: existingComment,
							}),
						);
						latestBoardComments[updatedComment.id] = updatedComment;
						break;
					}

					case "DeleteComment": {
						const commentToDelete = latestBoardComments[cmd.comment_id];
						if (!commentToDelete) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot delete comment: "${cmd.comment_id}" was not found`,
							);
							break;
						}

						flowPilotDebugLog(
							`[DeleteComment] Queued "${commentToDelete.content.slice(0, 30)}..."`,
						);
						remainingGenericCommands.push(
							removeCommentCommand({ comment: commentToDelete }),
						);
						delete latestBoardComments[cmd.comment_id];
						break;
					}

					case "CreateLayer": {
						if (isSetupLayerCommand(cmd)) {
							break;
						}
						const layerId = createId();
						const layerType = layerTypeFromCommand(cmd.layer_type);
						if (cmd.cache != null && layerType !== ILayerType.Function) {
							recordCommandFailure(
								cmd,
								"layer creation",
								`Cannot configure function cache on non-Function layer "${cmd.name}"`,
							);
							break;
						}
						const targetLayer =
							resolveLayerId(cmd.target_layer) ?? currentLayer;
						const nodeIds = resolveNodeIds(cmd.node_ids || []);
						const position = cmd.position ?? { x: baseX, y: baseY };

						const layer: ILayer = {
							id: layerId,
							name: cmd.name,
							type: layerType,
							color: cmd.color || null,
							coordinates: [position.x, position.y, 0],
							nodes: {},
							variables: {},
							comments: {},
							pins: pinsFromDefs(cmd.pins, false),
							cache: cmd.cache ?? null,
							parent_id: targetLayer,
						};

						flowPilotDebugLog(
							`[CreateLayer] Queued "${cmd.name}" with ${nodeIds.length} nodes`,
						);
						remainingGenericCommands.push(
							upsertLayerCommand({
								layer,
								node_ids: nodeIds,
								current_layer: targetLayer,
							}),
						);
						latestBoardLayers[layerId] = layerWithNodeIds(layer, nodeIds);
						registerNodeRefs(
							[cmd.ref_id, cmd.name, layerId],
							layerAsNode(layer),
						);
						break;
					}

					case "UpdateLayerCache": {
						const existingLayer = resolveLayer(cmd.layer_id);
						if (!existingLayer) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot update function cache: "${cmd.layer_id}" was not found`,
							);
							break;
						}
						if (existingLayer.type !== ILayerType.Function) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot update function cache: "${cmd.layer_id}" is not a Function layer`,
							);
							break;
						}

						const updatedLayer: ILayer = {
							...existingLayer,
							cache: cmd.cache ?? null,
						};
						remainingGenericCommands.push(
							upsertLayerCommand({
								layer: updatedLayer,
								node_ids: [],
								current_layer: existingLayer.parent_id ?? null,
								old_layer: existingLayer,
							}),
						);
						latestBoardLayers[existingLayer.id] = updatedLayer;
						break;
					}

					case "AddNodesToLayer": {
						const existingLayer = resolveLayer(cmd.layer_id);
						if (!existingLayer) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot add nodes to layer: "${cmd.layer_id}" was not found`,
							);
							break;
						}

						const existingNodeIds = Object.keys(existingLayer.nodes || {});
						const nodeIds = resolveNodeIds(cmd.node_ids);
						const allNodeIds = [...new Set([...existingNodeIds, ...nodeIds])];
						const updatedLayer = layerWithNodeIds(existingLayer, allNodeIds);

						flowPilotDebugLog(
							`[AddNodesToLayer] Queued ${nodeIds.length} nodes for "${existingLayer.name}"`,
						);
						remainingGenericCommands.push(
							upsertLayerCommand({
								layer: updatedLayer,
								node_ids: allNodeIds,
								current_layer: currentLayer,
								old_layer: existingLayer,
							}),
						);
						latestBoardLayers[existingLayer.id] = updatedLayer;
						break;
					}

					case "RemoveNodesFromLayer": {
						const layerToUpdate = resolveLayer(cmd.layer_id);
						if (!layerToUpdate) {
							recordCommandFailure(
								cmd,
								"board edit",
								`Cannot remove nodes from layer: "${cmd.layer_id}" was not found`,
							);
							break;
						}

						const nodeIds = resolveNodeIds(cmd.node_ids);
						const currentNodeIds = Object.keys(layerToUpdate.nodes || {});
						const remainingNodeIds = currentNodeIds.filter(
							(id) => !nodeIds.includes(id),
						);
						const updatedLayer = layerWithNodeIds(
							layerToUpdate,
							remainingNodeIds,
						);

						flowPilotDebugLog(
							`[RemoveNodesFromLayer] Queued ${nodeIds.length} nodes from "${layerToUpdate.name}"`,
						);
						remainingGenericCommands.push(
							upsertLayerCommand({
								layer: updatedLayer,
								node_ids: remainingNodeIds,
								current_layer: currentLayer,
								old_layer: layerToUpdate,
							}),
						);
						latestBoardLayers[layerToUpdate.id] = updatedLayer;
						break;
					}
				}
			}

			const executedBoardEditCommands = await executeInBatches(
				remainingGenericCommands,
				"board edit",
			);
			if (executedBoardEditCommands.length > 0) {
				executedAnyCommands = true;
				refreshedAfterLastExecution = false;
				await refreshBoardSnapshot();
				refreshedAfterLastExecution = true;
			}

			// Setup edits (especially variable contract changes and configuration-driven dynamic
			// pins) must settle before endpoint names are resolved. Otherwise a stale optimistic
			// pin id can be queued in the same batch and then pruned by the post-update hook.
			if (
				pendingConnectionCommands.length > 0 &&
				executedAnyCommands &&
				!refreshedAfterLastExecution
			) {
				await refreshBoardSnapshot();
				refreshedAfterLastExecution = true;
			}

			const connectionGenericCommands: IGenericCommand[] = [];
			for (const cmd of pendingConnectionCommands) {
				const fromNode = resolveNode(cmd.from_node);
				const toNode = resolveNode(cmd.to_node);
				if (!fromNode || !toNode) {
					const missingNode = !fromNode ? cmd.from_node : cmd.to_node;
					if (cmd.command_type === "ConnectPins") {
						console.error(
							`[ConnectPins] FAILED - Node not found: "${missingNode}"`,
							{
								command: cmd,
								availableNodeRefs: Array.from(nodeReferenceMap.keys()),
								boardNodeIds: Object.keys(latestBoardNodes),
							},
						);
					}
					recordCommandFailure(
						cmd,
						"connection",
						`${cmd.command_type === "ConnectPins" ? "Connection" : "Disconnection"} failed: Node "${missingNode}" was not found`,
					);
					continue;
				}

				const fromPinId = resolvePinId(
					cmd.from_node,
					cmd.from_pin,
					IPinType.Output,
				);
				const toPinId = resolvePinId(cmd.to_node, cmd.to_pin, IPinType.Input);
				if (!fromPinId || !toPinId) {
					const missingPin = !fromPinId
						? `${fromNode.friendly_name}.${cmd.from_pin}`
						: `${toNode.friendly_name}.${cmd.to_pin}`;
					if (cmd.command_type === "ConnectPins") {
						console.error(
							`[ConnectPins] FAILED - Pin not found: "${missingPin}"`,
							{
								command: cmd,
								from_pin_requested: cmd.from_pin,
								to_pin_requested: cmd.to_pin,
								fromPinId_resolved: fromPinId,
								toPinId_resolved: toPinId,
								fromNodePins: Object.values(fromNode.pins).map((pin) => ({
									name: pin.name,
									id: pin.id,
									type: pin.pin_type,
								})),
								toNodePins: Object.values(toNode.pins).map((pin) => ({
									name: pin.name,
									id: pin.id,
									type: pin.pin_type,
								})),
							},
						);
					}
					recordCommandFailure(
						cmd,
						"connection",
						`${cmd.command_type === "ConnectPins" ? "Connection" : "Disconnection"} failed: Pin "${missingPin}" was not found`,
					);
					continue;
				}

				if (cmd.command_type === "ConnectPins") {
					flowPilotDebugLog(
						`[ConnectPins] Queued ${fromNode.friendly_name}.${cmd.from_pin} -> ${toNode.friendly_name}.${cmd.to_pin}`,
						{
							from_node_id: fromNode.id,
							from_pin_id: fromPinId,
							to_node_id: toNode.id,
							to_pin_id: toPinId,
						},
					);
					connectionGenericCommands.push(
						connectPinsCommand({
							from_node: fromNode.id,
							from_pin: fromPinId,
							to_node: toNode.id,
							to_pin: toPinId,
						}),
					);
				} else {
					connectionGenericCommands.push(
						disconnectPinsCommand({
							from_node: fromNode.id,
							from_pin: fromPinId,
							to_node: toNode.id,
							to_pin: toPinId,
						}),
					);
				}
			}

			const executedConnectionCommands = await executeInBatches(
				connectionGenericCommands,
				"connection",
			);
			if (executedConnectionCommands.length > 0) {
				executedAnyCommands = true;
				refreshedAfterLastExecution = false;
			}

			if (commandFailures.length > 0) {
				await throwFlowPilotCommandApplyError(
					{
						requestedCommands: commands.length,
						appliedCommands: appliedGenericCommandCount,
						failures: commandFailures,
					},
					refreshBoardSnapshot,
				);
			}

			if (executedAnyCommands && !refreshedAfterLastExecution) {
				await refreshBoardSnapshot();
			}

			flowPilotDebugLog(
				`[handleExecuteCommands] Completed ${commands.length} FlowPilot commands`,
			);
		},
		[
			catalog.data,
			executeCommands,
			board.data,
			currentLayer,
			board.refetch,
			board,
		],
	);

	return { handleExecuteCommands };
}

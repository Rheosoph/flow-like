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
import { toastError } from "../lib/messages";
import type { IGenericCommand } from "../lib/schema";
import {
	type IBoard,
	type IComment,
	ICommentType,
	ILayerType,
	type IVariable,
} from "../lib/schema/flow/board";
import { type INode, IVariableType } from "../lib/schema/flow/node";
import { type IPin, IPinType } from "../lib/schema/flow/pin";
import type { ILayer } from "../lib/schema/flow/run";
import { convertJsonToUint8Array } from "../lib/uint8";

interface UseCopilotCommandsProps {
	board: UseQueryResult<IBoard | undefined, Error>;
	catalog: UseQueryResult<INode[] | undefined, Error>;
	executeCommand: (
		command: IGenericCommand,
		append?: boolean,
	) => Promise<unknown>;
	currentLayer: string | undefined;
}

export function useCopilotCommands({
	board,
	catalog,
	executeCommand,
	currentLayer,
}: UseCopilotCommandsProps) {
	const handleExecuteCommands = useCallback(
		async (commands: BoardCommand[]) => {
			let latestBoardNodes = board.data?.nodes ?? {};
			let latestBoardLayers = board.data?.layers ?? {};

			// ===== MAPPING TABLES =====
			const nodeReferenceMap = new Map<string, INode>();
			const pinIdMap = new Map<string, Map<string, string>>();

			// ===== POSITION CALCULATION =====
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

			// Helper to convert a layer to node-like structure for pin resolution
			const layerAsNode = (layer: ILayer): INode =>
				({
					id: layer.id,
					name: layer.name,
					friendly_name: layer.name,
					pins: layer.pins,
					coordinates: layer.coordinates,
				}) as unknown as INode;

			// ===== HELPER FUNCTIONS =====
			const resolveNode = (ref: string): INode | undefined => {
				// Check regular nodes first
				if (latestBoardNodes[ref]) return latestBoardNodes[ref];
				// Check reference map (newly created nodes/layers)
				if (nodeReferenceMap.has(ref)) return nodeReferenceMap.get(ref);
				// Check existing layers (placeholders) - they can be connected to like nodes
				if (latestBoardLayers[ref]) return layerAsNode(latestBoardLayers[ref]);
				return undefined;
			};

			const resolvePinId = (
				nodeRef: string,
				pinRef: string,
			): string | undefined => {
				const nodePinMap = pinIdMap.get(nodeRef);
				if (nodePinMap) {
					if (nodePinMap.has(pinRef)) return nodePinMap.get(pinRef);
					if (nodePinMap.has(pinRef.toLowerCase()))
						return nodePinMap.get(pinRef.toLowerCase());
				}

				const node = resolveNode(nodeRef);
				if (!node) return undefined;

				if (node.pins[pinRef]) return pinRef;

				for (const pin of Object.values(node.pins)) {
					if (pin.name.toLowerCase() === pinRef.toLowerCase()) return pin.id;
					if (pin.friendly_name?.toLowerCase() === pinRef.toLowerCase())
						return pin.id;
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

			const buildPinMapping = (nodeRef: string, node: INode) => {
				const pinMap = new Map<string, string>();
				for (const pin of Object.values(node.pins)) {
					pinMap.set(pin.name, pin.id);
					pinMap.set(pin.name.toLowerCase(), pin.id);
					if (pin.friendly_name && pin.friendly_name !== pin.name) {
						pinMap.set(pin.friendly_name, pin.id);
						pinMap.set(pin.friendly_name.toLowerCase(), pin.id);
					}
				}
				pinIdMap.set(nodeRef, pinMap);
				return pinMap;
			};

			const refreshBoardSnapshot = async () => {
				const freshBoard = await board.refetch();
				latestBoardNodes = freshBoard.data?.nodes ?? latestBoardNodes;
				latestBoardLayers = freshBoard.data?.layers ?? latestBoardLayers;

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
						const layerNode = layerAsNode(freshLayer);
						nodeReferenceMap.set(ref, layerNode);
						buildPinMapping(ref, layerNode);
					}
				}
			};

			// Build pin mappings for existing board nodes
			for (const [nodeId, node] of Object.entries(latestBoardNodes)) {
				buildPinMapping(nodeId, node);
			}

			// Build pin mappings for existing layers (placeholders) so they can be connected to
			for (const [layerId, layer] of Object.entries(latestBoardLayers)) {
				buildPinMapping(layerId, layerAsNode(layer));
			}

			// ===== FIRST PASS: ADD NODES =====
			let nodeIndex = 0;

			for (const cmd of commands) {
				if (cmd.command_type === "AddNode") {
					const catalogNode = catalog.data?.find(
						(n) => n.name === cmd.node_type,
					);
					if (!catalogNode) {
						toastError(
							`Node type ${cmd.node_type} not found in catalog`,
							<XIcon />,
						);
						continue;
					}

					const position = cmd.position || {
						x: baseX + (nodeIndex % 3) * 300,
						y: baseY + Math.floor(nodeIndex / 3) * 200,
					};

					const targetLayer = cmd.target_layer ?? currentLayer;

					const result = addNodeCommand({
						node: {
							...catalogNode,
							coordinates: [position.x, position.y, 0],
							friendly_name: catalogNode.friendly_name,
						},
						current_layer: targetLayer,
					});

					const executedCommand = await executeCommand(result.command);

					if (!executedCommand) {
						console.error(
							`[AddNode] Command execution returned undefined - command may not have been executed`,
						);
						toastError(
							`Failed to add node "${catalogNode.friendly_name}" - check if you're editing the latest version`,
							<XIcon />,
						);
						continue;
					}

					const actualNode =
						((executedCommand as Record<string, unknown> | undefined)
							?.node as INode) ?? result.node;

					const refs = [
						cmd.ref_id,
						`$${nodeIndex}`,
						cmd.node_type,
						actualNode.id,
					].filter(Boolean) as string[];

					for (const ref of refs) {
						nodeReferenceMap.set(ref, actualNode);
						buildPinMapping(ref, actualNode);
					}

					console.log(
						`[AddNode] Created "${actualNode.friendly_name}" (${actualNode.id})`,
						{
							refs,
							pins: Object.values(actualNode.pins).map(
								(p) => `${p.name}:${p.id.slice(0, 8)}`,
							),
						},
					);

					nodeIndex++;
				}

				// Handle AddPlaceholder in first pass (creates layer nodes)
				if (cmd.command_type === "AddPlaceholder") {
					const layerId = createId();
					const position = cmd.position || {
						x: baseX + (nodeIndex % 3) * 300,
						y: baseY + Math.floor(nodeIndex / 3) * 200,
					};

					const targetLayer = cmd.target_layer ?? currentLayer;

					// Create default execution pins
					const execInPin: IPin = {
						id: createId(),
						name: "exec_in",
						friendly_name: "Exec In",
						connected_to: [],
						depends_on: [],
						description: "",
						index: 0,
						pin_type: IPinType.Input,
						value_type: IValueType.Normal,
						data_type: IVariableType.Execution,
						default_value: null,
					};

					const execOutPin: IPin = {
						...execInPin,
						id: createId(),
						pin_type: IPinType.Output,
						name: "exec_out",
						friendly_name: "Exec Out",
						index: 1,
					};

					// Build pins object starting with exec pins
					const pins: Record<string, IPin> = {
						[execInPin.id]: execInPin,
						[execOutPin.id]: execOutPin,
					};

					// Add custom pins if provided
					if (cmd.pins && cmd.pins.length > 0) {
						let pinIndex = 2;
						for (const pinDef of cmd.pins) {
							const pin: IPin = {
								id: createId(),
								name: pinDef.name,
								friendly_name: pinDef.friendly_name,
								description: pinDef.description || "",
								connected_to: [],
								depends_on: [],
								index: pinIndex++,
								pin_type: pinDef.pin_type as IPinType,
								value_type:
									(pinDef.value_type as IValueType) || IValueType.Normal,
								data_type: pinDef.data_type as IVariableType,
								default_value: null,
							};
							pins[pin.id] = pin;
						}
					}

					const layer: ILayer = {
						id: layerId,
						name: cmd.name,
						type: ILayerType.Collapsed,
						coordinates: [position.x, position.y, 0],
						nodes: {},
						variables: {},
						comments: {},
						pins,
						parent_id: targetLayer,
					};

					const result = await executeCommand(
						upsertLayerCommand({
							layer,
							node_ids: [],
							current_layer: targetLayer,
						}),
					);

					if (result) {
						// Treat the layer as a "node" for reference purposes
						const layerAsNode = {
							id: layerId,
							name: cmd.name,
							friendly_name: cmd.name,
							pins,
							coordinates: [position.x, position.y, 0],
						} as unknown as INode;

						const refs = [
							cmd.ref_id,
							`$${nodeIndex}`,
							cmd.name,
							layerId,
						].filter(Boolean) as string[];

						for (const ref of refs) {
							nodeReferenceMap.set(ref, layerAsNode);
							buildPinMapping(ref, layerAsNode);
						}

						console.log(`[AddPlaceholder] Created "${cmd.name}" (${layerId})`, {
							refs,
							pins: Object.values(pins).map(
								(p) => `${p.name}:${p.id.slice(0, 8)}`,
							),
						});
					}

					nodeIndex++;
				}
			}

			const delay = (ms: number) =>
				new Promise((resolve) => setTimeout(resolve, ms));

			type CreateVariableCommand = Extract<
				BoardCommand,
				{ command_type: "CreateVariable" }
			>;
			type UpdateNodePinCommand = Extract<
				BoardCommand,
				{ command_type: "UpdateNodePin" }
			>;

			const applyCreateVariable = async (cmd: CreateVariableCommand) => {
				const variableId = cmd.variable_id || createId();
				const targetLayer = cmd.target_layer ?? null;
				const variable: IVariable = {
					id: variableId,
					name: cmd.name,
					data_type: (cmd.data_type as IVariableType) || IVariableType.String,
					value_type: (cmd.value_type as IValueType) || IValueType.Normal,
					default_value:
						"default_value" in cmd
							? Array.from(convertJsonToUint8Array(cmd.default_value) || [])
							: null,
					description: cmd.description || null,
					category: cmd.category || null,
					schema: cmd.schema || null,
					editable: cmd.editable ?? true,
					exposed: cmd.exposed ?? false,
					secret: cmd.secret ?? false,
					runtime_configured: cmd.runtime_configured ?? false,
				};

				console.log(`[CreateVariable] ${cmd.name} (${cmd.data_type})`);
				await executeCommand(
					upsertVariableCommand({ variable, layer_id: targetLayer }),
				);
			};

			const applyUpdateNodePin = async (cmd: UpdateNodePinCommand) => {
				await refreshBoardSnapshot();

				const nodeId = nodeReferenceMap.get(cmd.node_id)?.id ?? cmd.node_id;
				const node = latestBoardNodes[nodeId] ?? resolveNode(cmd.node_id);

				if (!node) {
					console.error(
						`[UpdateNodePin] ❌ FAILED - Node not found: ${cmd.node_id}`,
						{
							command: cmd,
							availableNodeRefs: Array.from(nodeReferenceMap.keys()),
							boardNodeIds: Object.keys(latestBoardNodes),
						},
					);
					toastError(
						`Pin update failed: Node "${cmd.node_id}" not found`,
						<XIcon />,
					);
					return;
				}

				const pinId = resolvePinId(cmd.node_id, cmd.pin_id);
				const pin = pinId ? node.pins[pinId] : undefined;

				if (!pin || !pinId) {
					console.error(
						`[UpdateNodePin] ❌ FAILED - Pin not found: ${cmd.pin_id} in ${node.friendly_name}`,
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
					toastError(
						`Pin update failed: Pin "${cmd.pin_id}" not found in "${node.friendly_name}"`,
						<XIcon />,
					);
					return;
				}

				let encodedValue: number[] | null = null;
				if (cmd.value !== null && cmd.value !== undefined) {
					let valueToEncode = cmd.value;
					if (typeof cmd.value === "string") {
						if (cmd.value.startsWith('"') && cmd.value.endsWith('"')) {
							valueToEncode = cmd.value.slice(1, -1);
						}
					}
					const encoded = convertJsonToUint8Array(valueToEncode);
					if (encoded) {
						encodedValue = encoded;
					} else {
						console.error(
							`[UpdateNodePin] ❌ FAILED - Could not encode value:`,
							cmd.value,
						);
						toastError(`Pin update failed: Could not encode value`, <XIcon />);
						return;
					}
				}

				console.log(
					`[UpdateNodePin] ✓ Setting: ${node.friendly_name}.${cmd.pin_id} = ${JSON.stringify(cmd.value)}`,
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

				try {
					const result = await executeCommand(
						updateNodeCommand({
							node: updatedNode,
							old_node: node,
						}),
					);
					if (result) {
						console.log(`[UpdateNodePin] ✓ SUCCESS:`, result);
						await refreshBoardSnapshot();
					} else {
						console.error(
							`[UpdateNodePin] ❌ FAILED - executeCommand returned undefined/null`,
						);
						toastError(
							`Pin update failed: Command not executed (check version)`,
							<XIcon />,
						);
					}
				} catch (err) {
					console.error(`[UpdateNodePin] ❌ FAILED - Exception:`, err);
					toastError(`Pin update failed: ${err}`, <XIcon />);
				}
			};

			await refreshBoardSnapshot();

			for (const cmd of commands) {
				if (cmd.command_type === "CreateVariable") {
					await delay(50);
					await applyCreateVariable(cmd);
				}
			}

			for (const cmd of commands) {
				if (cmd.command_type === "UpdateNodePin") {
					await delay(50);
					await applyUpdateNodePin(cmd);
				}
			}

			await refreshBoardSnapshot();

			// ===== SECOND PASS: CONNECTIONS & REMAINING COMMANDS =====
			for (const cmd of commands) {
				await delay(50);

				switch (cmd.command_type) {
					case "AddNode":
						break;

					case "AddPlaceholder":
						// Already handled in first pass
						break;

					case "CreateVariable":
						// Applied before node configuration so variable get/set nodes can be wired safely.
						break;

					case "RemoveNode": {
						const node = resolveNode(cmd.node_id);
						if (node) {
							await executeCommand(
								removeNodeCommand({
									node,
									connected_nodes: [],
								}),
							);
						}
						break;
					}

					case "ConnectPins": {
						const fromNode = resolveNode(cmd.from_node);
						const toNode = resolveNode(cmd.to_node);

						if (!fromNode || !toNode) {
							const missingNode = !fromNode ? cmd.from_node : cmd.to_node;
							console.error(
								`[ConnectPins] ❌ FAILED - Node not found: "${missingNode}"`,
								{
									command: cmd,
									availableNodeRefs: Array.from(nodeReferenceMap.keys()),
									boardNodeIds: Object.keys(latestBoardNodes),
								},
							);
							toastError(
								`Connection failed: Node "${missingNode}" not found`,
								<XIcon />,
							);
							break;
						}

						const fromPinId = resolvePinId(cmd.from_node, cmd.from_pin);
						const toPinId = resolvePinId(cmd.to_node, cmd.to_pin);

						if (!fromPinId || !toPinId) {
							const missingPin = !fromPinId
								? `${fromNode.friendly_name}.${cmd.from_pin}`
								: `${toNode.friendly_name}.${cmd.to_pin}`;
							console.error(
								`[ConnectPins] ❌ FAILED - Pin not found: "${missingPin}"`,
								{
									command: cmd,
									from_pin_requested: cmd.from_pin,
									to_pin_requested: cmd.to_pin,
									fromPinId_resolved: fromPinId,
									toPinId_resolved: toPinId,
									fromNodePins: Object.values(fromNode.pins).map((p) => ({
										name: p.name,
										id: p.id,
										type: p.pin_type,
									})),
									toNodePins: Object.values(toNode.pins).map((p) => ({
										name: p.name,
										id: p.id,
										type: p.pin_type,
									})),
								},
							);
							toastError(
								`Connection failed: Pin "${missingPin}" not found`,
								<XIcon />,
							);
							break;
						}

						console.log(
							`[ConnectPins] ✓ Connecting: ${fromNode.friendly_name}.${cmd.from_pin} -> ${toNode.friendly_name}.${cmd.to_pin}`,
							{
								from_node_id: fromNode.id,
								from_pin_id: fromPinId,
								to_node_id: toNode.id,
								to_pin_id: toPinId,
							},
						);

						try {
							const connectResult = await executeCommand(
								connectPinsCommand({
									from_node: fromNode.id,
									from_pin: fromPinId,
									to_node: toNode.id,
									to_pin: toPinId,
								}),
							);
							if (connectResult) {
								console.log(`[ConnectPins] ✓ SUCCESS:`, connectResult);
								await delay(100);
							} else {
								console.error(
									`[ConnectPins] ❌ FAILED - executeCommand returned undefined/null`,
								);
								toastError(
									`Connection failed: Command not executed (check version)`,
									<XIcon />,
								);
							}
						} catch (err) {
							console.error(`[ConnectPins] ❌ FAILED - Exception:`, err);
							toastError(`Connection failed: ${err}`, <XIcon />);
						}
						break;
					}

					case "DisconnectPins": {
						const fromNode = resolveNode(cmd.from_node);
						const toNode = resolveNode(cmd.to_node);
						if (!fromNode || !toNode) break;

						const fromPinId = resolvePinId(cmd.from_node, cmd.from_pin);
						const toPinId = resolvePinId(cmd.to_node, cmd.to_pin);
						if (!fromPinId || !toPinId) break;

						await executeCommand(
							disconnectPinsCommand({
								from_node: fromNode.id,
								from_pin: fromPinId,
								to_node: toNode.id,
								to_pin: toPinId,
							}),
						);
						break;
					}

					case "UpdateNodePin": {
						// Applied before connections so dynamic/configured pins exist.
						break;
					}

					case "MoveNode": {
						const node = resolveNode(cmd.node_id);
						if (!node) break;

						const targetLayer = cmd.target_layer ?? currentLayer;

						await executeCommand(
							moveNodeCommand({
								node_id: node.id,
								to_coordinates: [cmd.position.x, cmd.position.y, 0],
								current_layer: targetLayer,
							}),
						);
						break;
					}

					case "UpdateVariable": {
						const existingVariable = board.data?.variables?.[cmd.variable_id];
						if (!existingVariable) {
							toastError(
								`Cannot update variable: "${cmd.variable_id}" not found`,
								<XIcon />,
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
									? Array.from(convertJsonToUint8Array(cmd.default_value) || [])
									: "value" in cmd
										? Array.from(convertJsonToUint8Array(cmd.value) || [])
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

						console.log(
							`[UpdateVariable] ${existingVariable.name} = ${JSON.stringify(cmd.value)}`,
						);
						await executeCommand(
							upsertVariableCommand({
								variable: updatedVariable,
								old_variable: existingVariable,
							}),
						);
						break;
					}

					case "DeleteVariable": {
						const variableToDelete = board.data?.variables?.[cmd.variable_id];
						if (!variableToDelete) {
							toastError(
								`Cannot delete variable: "${cmd.variable_id}" not found`,
								<XIcon />,
							);
							break;
						}

						console.log(`[DeleteVariable] ${variableToDelete.name}`);
						await executeCommand(
							removeVariableCommand({ variable: variableToDelete }),
						);
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

						const targetLayer = cmd.target_layer ?? currentLayer;

						console.log(`[CreateComment] "${cmd.content.slice(0, 30)}..."`);
						await executeCommand(
							upsertCommentCommand({ comment, current_layer: targetLayer }),
						);
						break;
					}

					case "UpdateComment": {
						const existingComment = board.data?.comments?.[cmd.comment_id];
						if (!existingComment) {
							toastError(
								`Cannot update comment: "${cmd.comment_id}" not found`,
								<XIcon />,
							);
							break;
						}

						const updatedComment: IComment = {
							...existingComment,
							content: cmd.content ?? existingComment.content,
							color: cmd.color ?? existingComment.content,
						};

						console.log(
							`[UpdateComment] "${updatedComment.content.slice(0, 30)}..."`,
						);
						await executeCommand(
							upsertCommentCommand({
								comment: updatedComment,
								current_layer: currentLayer,
								old_comment: existingComment,
							}),
						);
						break;
					}

					case "DeleteComment": {
						const commentToDelete = board.data?.comments?.[cmd.comment_id];
						if (!commentToDelete) {
							toastError(
								`Cannot delete comment: "${cmd.comment_id}" not found`,
								<XIcon />,
							);
							break;
						}

						console.log(
							`[DeleteComment] "${commentToDelete.content.slice(0, 30)}..."`,
						);
						await executeCommand(
							removeCommentCommand({ comment: commentToDelete }),
						);
						break;
					}

					case "CreateLayer": {
						const layerId = createId();
						const targetLayer = cmd.target_layer ?? currentLayer;

						const layer: ILayer = {
							id: layerId,
							name: cmd.name,
							type: ILayerType.Collapsed,
							color: cmd.color || null,
							coordinates: [baseX, baseY, 0],
							nodes: {},
							variables: {},
							comments: {},
							pins: {},
							parent_id: targetLayer,
						};

						console.log(
							`[CreateLayer] "${cmd.name}" with ${cmd.node_ids?.length || 0} nodes`,
						);
						await executeCommand(
							upsertLayerCommand({
								layer,
								node_ids: cmd.node_ids || [],
								current_layer: targetLayer,
							}),
						);
						break;
					}

					case "AddNodesToLayer": {
						const existingLayer = board.data?.layers?.[cmd.layer_id];
						if (!existingLayer) {
							toastError(
								`Cannot add nodes to layer: "${cmd.layer_id}" not found`,
								<XIcon />,
							);
							break;
						}

						const existingNodeIds = Object.keys(existingLayer.nodes || {});
						const allNodeIds = [
							...new Set([...existingNodeIds, ...cmd.node_ids]),
						];

						console.log(
							`[AddNodesToLayer] Adding ${cmd.node_ids.length} nodes to "${existingLayer.name}"`,
						);
						await executeCommand(
							upsertLayerCommand({
								layer: existingLayer,
								node_ids: allNodeIds,
								current_layer: currentLayer,
								old_layer: existingLayer,
							}),
						);
						break;
					}

					case "RemoveNodesFromLayer": {
						const layerToUpdate = board.data?.layers?.[cmd.layer_id];
						if (!layerToUpdate) {
							toastError(
								`Cannot remove nodes from layer: "${cmd.layer_id}" not found`,
								<XIcon />,
							);
							break;
						}

						const currentNodeIds = Object.keys(layerToUpdate.nodes || {});
						const remainingNodeIds = currentNodeIds.filter(
							(id) => !cmd.node_ids.includes(id),
						);

						console.log(
							`[RemoveNodesFromLayer] Removing ${cmd.node_ids.length} nodes from "${layerToUpdate.name}"`,
						);
						await executeCommand(
							upsertLayerCommand({
								layer: layerToUpdate,
								node_ids: remainingNodeIds,
								current_layer: currentLayer,
								old_layer: layerToUpdate,
							}),
						);
						break;
					}
				}
			}

			console.log(
				`[handleExecuteCommands] Completed ${commands.length} commands`,
			);

			await board.refetch();
		},
		[
			catalog.data,
			executeCommand,
			board.data?.nodes,
			board.data?.variables,
			board.data?.comments,
			board.data?.layers,
			currentLayer,
			board.refetch,
			board,
		],
	);

	return { handleExecuteCommands };
}

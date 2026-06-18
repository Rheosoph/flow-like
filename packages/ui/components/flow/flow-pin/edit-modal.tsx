"use client";

import { useInternalNode } from "@xyflow/react";
import { isEqual } from "lodash-es";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useInvalidateInvoke, useInvoke } from "../../../hooks";
import { updateNodeCommand, upsertLayerCommand } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import useFlowControlState from "../../../state/flow-control-state";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui";
import { useUndoRedo } from "../flow-history";
import { VariablesMenuEdit } from "../variables/variables-menu-edit";

export function PinEditModal({
	appId,
	boardId,
	version,
}: Readonly<{
	appId: string;
	boardId: string;
	version?: [number, number, number];
}>) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const { pushCommand } = useUndoRedo(appId, boardId);
	const [defaultValueState, setDefaultValueState] = useState<any>(null);
	const { editedPin, stopEditPin } = useFlowControlState();

	// Fetch the board to get the original layer/node data
	const board = useInvoke(backend.boardState.getBoard, backend.boardState, [
		appId,
		boardId,
	]);

	// Try to find the node directly via React Flow (for regular nodes)
	const directNode = useInternalNode(editedPin?.node ?? "");

	// Determine if editedPin.node refers to a layer or a regular node
	const isLayerPin = useMemo(() => {
		if (!editedPin?.node || !board.data) return false;
		return !!board.data.layers?.[editedPin.node];
	}, [editedPin?.node, board.data]);

	// Get the layer from board data (if it's a layer pin)
	const layer = useMemo(() => {
		if (!isLayerPin || !editedPin?.node || !board.data?.layers) return null;
		return board.data.layers[editedPin.node];
	}, [isLayerPin, editedPin?.node, board.data?.layers]);

	// Get the node from board data (if it's a regular node pin)
	const node = useMemo(() => {
		if (isLayerPin || !editedPin?.node || !board.data?.nodes) return null;
		return board.data.nodes[editedPin.node];
	}, [isLayerPin, editedPin?.node, board.data?.nodes]);

	useEffect(() => {
		if (editedPin) {
			setDefaultValueState(editedPin.pin.default_value);
			return;
		}

		setDefaultValueState(null);
	}, [editedPin?.node, editedPin?.pin]);

	const refetchBoard = useCallback(async () => {
		invalidate(backend.boardState.getBoard, [appId, boardId]);
	}, [appId, boardId, backend, invalidate]);

	const onChangeDefaultValue = useCallback(async () => {
		if (typeof version !== "undefined") {
			stopEditPin();
			return;
		}

		if (!editedPin?.node || !editedPin?.pin) {
			stopEditPin();
			return;
		}

		const hasChanged = !isEqual(defaultValueState, editedPin.pin.default_value);

		if (!hasChanged) {
			stopEditPin();
			return;
		}

		// Handle layer pins
		if (isLayerPin && layer) {
			const originalPin = layer.pins[editedPin.pin.id];
			if (!originalPin) {
				stopEditPin();
				return;
			}

			const command = upsertLayerCommand({
				current_layer: null,
				layer: {
					...layer,
					pins: {
						...layer.pins,
						[editedPin.pin.id]: {
							...originalPin,
							default_value: defaultValueState,
						},
					},
				},
				node_ids: [],
			});

			const result = await backend.boardState.executeCommand(
				appId,
				boardId,
				command,
			);
			await pushCommand(result, false);
			await refetchBoard();
			stopEditPin();
			return;
		}

		// Handle regular node pins
		if (node && directNode) {
			const command = updateNodeCommand({
				node: {
					...node,
					coordinates: [directNode.position.x, directNode.position.y, 0],
					pins: {
						...node.pins,
						[editedPin.pin.id]: {
							...editedPin.pin,
							default_value: defaultValueState,
						},
					},
				},
			});

			const result = await backend.boardState.executeCommand(
				appId,
				boardId,
				command,
			);
			await pushCommand(result, false);
			await refetchBoard();
			stopEditPin();
			return;
		}

		stopEditPin();
	}, [
		appId,
		boardId,
		backend,
		defaultValueState,
		directNode,
		editedPin?.node,
		editedPin?.pin,
		isLayerPin,
		layer,
		node,
		pushCommand,
		refetchBoard,
		stopEditPin,
		version,
	]);

	// Check if we have enough data to show the modal
	const canShowModal = editedPin?.pin && (isLayerPin ? !!layer : !!node);

	if (!canShowModal) {
		return null;
	}

	return (
		<Dialog
			open={!!editedPin?.pin && canShowModal}
			onOpenChange={async (open) => {
				if (!open) {
					await onChangeDefaultValue();
				}
			}}
		>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>Set Default Value</DialogTitle>
					<DialogDescription>
						The default value will only be used if the pin is not connected.
					</DialogDescription>
				</DialogHeader>
				<div className="w-full">
					<VariablesMenuEdit
						variable={{
							data_type: editedPin.pin.data_type,
							default_value: defaultValueState,
							exposed: false,
							id: editedPin.pin.id,
							value_type: editedPin.pin.value_type,
							name: editedPin.pin.name,
							editable: editedPin.pin.editable,
							secret: editedPin.pin.secret,
							category: editedPin.pin.category,
							description: editedPin.pin.description,
						}}
						updateVariable={async (variable) => {
							setDefaultValueState(variable.default_value);
						}}
					/>
				</div>
			</DialogContent>
		</Dialog>
	);
}

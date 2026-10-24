import type { Edge, Node } from "@xyflow/react";
import type { ILayer } from "../../packages/ui/lib/schema/flow/board";
import {
	ICommandType,
	type IGenericCommand,
} from "../../packages/ui/lib/schema/flow/board/commands/generic-command";
import type { INode } from "../../packages/ui/lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../packages/ui/lib/schema/flow/pin";

export function makeBoard(count = 10): Record<string, INode> {
	const nodes: Record<string, INode> = {};
	const add = (
		id: string,
		kind: IVariableType,
		x: number,
		y: number,
		reference = false,
	) => {
		const pins: IPin[] = reference
			? []
			: [IPinType.Input, IPinType.Output].map((direction) => ({
					id: `${id}.${direction === IPinType.Input ? "in" : "out"}`,
					name: direction === IPinType.Input ? "input" : "output",
					friendly_name: kind,
					description: "",
					pin_type: direction,
					data_type: kind,
					value_type: IValueType.Normal,
					index: 1,
					connected_to: [],
					depends_on: [],
				}));
		nodes[id] = {
			id,
			name: `harness_${id}`,
			friendly_name: id,
			description: "Integrated renderer test fixture",
			category: "test",
			coordinates: [x, y, 0],
			pins: Object.fromEntries(pins.map((pin) => [pin.id, pin])),
			fn_refs: reference
				? {
						fn_refs: [],
						can_reference_fns: true,
						can_be_referenced_by_fns: true,
					}
				: undefined,
		};
	};
	for (const [row, prefix, kind, length] of [
		[0, "string", IVariableType.String, 3],
		[1, "integer", IVariableType.Integer, 2],
		[2, "exec", IVariableType.Execution, 3],
	] as const)
		for (let col = 0; col < length; col++)
			add(
				`${prefix}-${String.fromCharCode(97 + col)}`,
				kind,
				80 + col * 320,
				100 + row * 170,
			);
	add("ref-a", IVariableType.Generic, 80, 610, true);
	add("ref-b", IVariableType.Generic, 400, 610, true);
	for (let i = 10; i < count; i++) {
		add(
			`extra-${i}`,
			IVariableType.String,
			80 + ((i - 10) % 100) * 220,
			900 + Math.floor((i - 10) / 100) * 110,
		);
		if ((i - 10) % 100 > 0) {
			nodes[`extra-${i - 1}`].pins[`extra-${i - 1}.out`].connected_to.push(
				`extra-${i}.in`,
			);
			nodes[`extra-${i}`].pins[`extra-${i}.in`].depends_on.push(
				`extra-${i - 1}.out`,
			);
		}
	}
	for (const prefix of ["string", "exec"]) {
		nodes[`${prefix}-a`].pins[`${prefix}-a.out`].connected_to.push(
			`${prefix}-b.in`,
		);
		nodes[`${prefix}-b`].pins[`${prefix}-b.in`].depends_on.push(
			`${prefix}-a.out`,
		);
	}
	return nodes;
}

export function pinCache(nodes: Record<string, INode>) {
	return new Map<string, [IPin, INode | ILayer, boolean]>(
		Object.values(nodes).flatMap((node) =>
			Object.values(node.pins).map(
				(pin) =>
					[pin.id, [pin, node, true]] as [
						string,
						[IPin, INode | ILayer, boolean],
					],
			),
		),
	);
}

export function project(
	nodes: Record<string, INode>,
	previousNodes: Node[] = [],
	previousEdges: Edge[] = [],
) {
	const selectedNodes = new Set(
		previousNodes.filter((node) => node.selected).map((node) => node.id),
	);
	const selectedEdges = new Set(
		previousEdges.filter((edge) => edge.selected).map((edge) => edge.id),
	);
	const owners = new Map(
		Object.values(nodes).flatMap((node) =>
			Object.keys(node.pins).map((id) => [id, node.id] as const),
		),
	);
	const flowNodes: Node[] = Object.values(nodes).map((node) => ({
		id: node.id,
		type: "node",
		// The named fixture nodes exercise rich DOM editors; a custom class always keeps one.
		className: node.id.startsWith("extra-") ? undefined : "harness-rich-editor",
		position: { x: node.coordinates?.[0] ?? 0, y: node.coordinates?.[1] ?? 0 },
		selected: selectedNodes.has(node.id),
		data: { node },
	}));
	const edges: Edge[] = Object.values(nodes).flatMap((node) => [
		...Object.values(node.pins).flatMap((pin) =>
			pin.connected_to.flatMap((target) => {
				const targetNode = owners.get(target);
				if (!targetNode) return [];
				const id = `${pin.id}->${target}`;
				return [
					{
						id,
						source: node.id,
						target: targetNode,
						sourceHandle: pin.id,
						targetHandle: target,
						type:
							pin.data_type === IVariableType.Execution ? "execution" : "data",
						selected: selectedEdges.has(id),
						style: {
							stroke: `var(--${pin.data_type === IVariableType.Execution ? "foreground" : `pin-${pin.data_type.toLowerCase()}`})`,
						},
						data: { reduceMotion: true },
					},
				];
			}),
		),
		...(node.fn_refs?.fn_refs ?? []).map((target) => ({
			id: `ref:${node.id}->${target}`,
			source: node.id,
			target,
			sourceHandle: `ref_out_${node.id}`,
			targetHandle: `ref_in_${target}`,
			type: "data",
			selected: selectedEdges.has(`ref:${node.id}->${target}`),
		})),
	]);
	return { nodes: flowNodes, edges };
}

/** Simulates backend receipts and occupancy rules; no network backend is involved. */
export function applyCommands(
	nodes: Record<string, INode>,
	commands: IGenericCommand[],
): IGenericCommand[] {
	return commands.map((command) => {
		if (command.command_type === ICommandType.UpdateNode && command.node) {
			const old_node = structuredClone(nodes[command.node.id]);
			nodes[command.node.id] = structuredClone(command.node as INode);
			return { ...command, old_node };
		}
		const { from_node, from_pin, to_node, to_pin } = command;
		if (!from_node || !from_pin || !to_node || !to_pin)
			throw new Error("Missing command endpoint");
		const output = nodes[from_node].pins[from_pin];
		const input = nodes[to_node].pins[to_pin];
		if (command.command_type === ICommandType.DisconnectPin) {
			output.connected_to = output.connected_to.filter((id) => id !== input.id);
			input.depends_on = input.depends_on.filter((id) => id !== output.id);
		} else if (command.command_type === ICommandType.ConnectPin) {
			for (const node of Object.values(nodes))
				for (const pin of Object.values(node.pins)) {
					if (output.data_type === IVariableType.Execution)
						pin.depends_on = pin.depends_on.filter((id) => id !== output.id);
					else
						pin.connected_to = pin.connected_to.filter((id) => id !== input.id);
				}
			if (output.data_type === IVariableType.Execution)
				output.connected_to = [];
			else input.depends_on = [];
			if (!output.connected_to.includes(input.id))
				output.connected_to.push(input.id);
			if (!input.depends_on.includes(output.id))
				input.depends_on.push(output.id);
		} else
			throw new Error(`Unsupported simulated command: ${command.command_type}`);
		return command;
	});
}

export function inverse(receipts: IGenericCommand[]): IGenericCommand[] {
	return receipts.toReversed().map((command) =>
		command.command_type === ICommandType.UpdateNode
			? { ...command, node: command.old_node ?? undefined }
			: {
					...command,
					command_type:
						command.command_type === ICommandType.ConnectPin
							? ICommandType.DisconnectPin
							: ICommandType.ConnectPin,
				},
	);
}

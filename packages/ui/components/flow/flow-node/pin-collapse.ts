import { type IPin, IVariableType } from "../../../lib/schema/flow/pin";
import type { IPinAction } from "../flow-node";

export type PinRow = IPin | IPinAction;

export interface PinCollapseLayout {
	inputs: PinRow[];
	outputs: PinRow[];
	/** Display row (1-based) per rendered pin id. Empty while expanded, where `pin.index` drives placement. */
	slots: Record<string, number>;
	/** Pins removed from the rendered rows by the current collapse. */
	hiddenCount: number;
	/** Pins the latch would hide (equals `hiddenCount` while collapsed). */
	collapsibleCount: number;
}

export function isPinAction(row: PinRow): row is IPinAction {
	return typeof (row as IPinAction).onAction === "function";
}

export function isPinConnected(pin: IPin): boolean {
	return (
		(pin.connected_to?.length ?? 0) > 0 || (pin.depends_on?.length ?? 0) > 0
	);
}

/** Execution pins carry control flow and always stay visible. */
export function isPinCollapsible(pin: IPin): boolean {
	return pin.data_type !== IVariableType.Execution && !isPinConnected(pin);
}

function countCollapsible(rows: PinRow[]): number {
	let count = 0;
	for (const row of rows) {
		if (!isPinAction(row) && isPinCollapsible(row)) count += 1;
	}
	return count;
}

function packRows(
	rows: PinRow[],
	slots: Record<string, number>,
): { rows: PinRow[]; hidden: number } {
	const kept: PinRow[] = [];
	let hidden = 0;
	for (const row of rows) {
		if (isPinAction(row)) continue;
		if (isPinCollapsible(row)) {
			hidden += 1;
			continue;
		}
		slots[row.id] = kept.length + 1;
		kept.push(row);
	}
	return { rows: kept, hidden };
}

/**
 * Derives the rendered pin rows for a node. Collapsing only re-packs display
 * slots; the underlying `pin.index` values are never touched, so expanding
 * restores the original order exactly.
 */
export function layoutCollapsiblePins(
	inputs: PinRow[],
	outputs: PinRow[],
	collapsed: boolean,
): PinCollapseLayout {
	if (!collapsed) {
		return {
			inputs,
			outputs,
			slots: {},
			hiddenCount: 0,
			collapsibleCount: countCollapsible(inputs) + countCollapsible(outputs),
		};
	}
	const slots: Record<string, number> = {};
	const packedInputs = packRows(inputs, slots);
	const packedOutputs = packRows(outputs, slots);
	const hiddenCount = packedInputs.hidden + packedOutputs.hidden;
	return {
		inputs: packedInputs.rows,
		outputs: packedOutputs.rows,
		slots,
		hiddenCount,
		collapsibleCount: hiddenCount,
	};
}

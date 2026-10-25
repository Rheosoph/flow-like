import { createId } from "@paralleldrive/cuid2";
import type { PlaceholderPinDef } from "./schema/flow/copilot";
import { IVariableType } from "./schema/flow/node";
import { type IPin, IPinType, IValueType } from "./schema/flow/pin";

function pinTypeFromDef(def: PlaceholderPinDef): IPinType {
	return def.pin_type === IPinType.Output ? IPinType.Output : IPinType.Input;
}

/** A fresh, unwired layer pin built from a FlowPilot pin def. Mirrors `insert_placeholder_pin` in
 * `packages/core/editor/src/flow/ast/apply.rs`. */
export function placeholderPin(def: PlaceholderPinDef, index: number): IPin {
	return {
		id: createId(),
		name: def.name,
		friendly_name: def.friendly_name,
		connected_to: [],
		depends_on: [],
		description: def.description || "",
		index,
		pin_type: pinTypeFromDef(def),
		value_type: (def.value_type as IValueType) || IValueType.Normal,
		data_type: def.data_type as IVariableType,
		default_value: null,
		schema: def.schema ?? null,
		options: def.enforce_schema ? { enforce_schema: true } : null,
	};
}

/** A Function layer's pins once its data boundary becomes `boundary` (parameters as `Input`, then
 * returns as `Output`). Execution pins stay. A data pin whose name and direction survive keeps its
 * id, and with it its wiring, and takes the new type; every other pin is new. Mirrors
 * `migrated_boundary_pins` in `packages/core/editor/src/flow/ast/apply.rs`. */
export function migratedBoundaryPins(
	current: Record<string, IPin>,
	boundary: readonly PlaceholderPinDef[],
): Record<string, IPin> {
	const currentPins = Object.values(current).sort((a, b) => a.index - b.index);
	const pins: Record<string, IPin> = Object.fromEntries(
		currentPins
			.filter((pin) => pin.data_type === IVariableType.Execution)
			.map((pin) => [pin.id, pin]),
	);
	const firstIndex = Object.keys(pins).length;

	boundary.forEach((def, offset) => {
		const pinType = pinTypeFromDef(def);
		const index = firstIndex + offset;
		const kept = currentPins.find(
			(pin) =>
				pin.pin_type === pinType &&
				pin.data_type !== IVariableType.Execution &&
				pin.name === def.name &&
				!pins[pin.id],
		);
		if (!kept) {
			const pin = placeholderPin(def, index);
			pins[pin.id] = pin;
			return;
		}
		const { enforce_schema: _cleared, ...otherOptions } = kept.options ?? {};
		pins[kept.id] = {
			...kept,
			data_type: def.data_type as IVariableType,
			value_type: (def.value_type as IValueType) || IValueType.Normal,
			schema: def.schema ?? null,
			index,
			options: def.enforce_schema
				? { ...otherOptions, enforce_schema: true }
				: kept.options && otherOptions,
		};
	});

	return pins;
}

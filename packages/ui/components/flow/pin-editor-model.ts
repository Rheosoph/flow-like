import { i18n as i18next } from "@flow-like/locales";
import type { RefObject } from "react";
import type { IPin, IPinOptions } from "../../lib";
import {
	type IBoard,
	type ILayer,
	IPinType,
} from "../../lib/schema/flow/board";
import type { INode } from "../../lib/schema/flow/node";
import { IValueType, IVariableType } from "../../lib/schema/flow/variable";

export type PinEdit = {
	id: string;
	name: string;
	friendly_name: string;
	description: string;
	data_type: IVariableType;
	options?: IPinOptions | null;
	schema?: string | null;
	pin_type: IPinType;
	index: number;
	value_type: IValueType;
	default_value?: number[] | null;
};

export const normalizeValueType = (vt: unknown): IValueType => {
	const s = String(vt ?? "").toLowerCase();
	if (s === "array") return IValueType.Array;
	if (s === "hashmap" || s === "map") return IValueType.HashMap;
	if (s === "hashset" || s === "set") return IValueType.HashSet;
	return IValueType.Normal;
};

export const toMachineName = (s: string) =>
	s.trim().toLowerCase().replace(/\s+/g, "_");

export const sortByIndex = <T extends { index: number }>(arr: T[]) =>
	[...arr].sort((a, b) => a.index - b.index);

export const reindex = <T extends { index: number }>(arr: T[]) =>
	arr.map((p, i) => ({ ...p, index: i + 1 }));

export const isPayloadPin = (pin: PinEdit, isGenericEvent: boolean) =>
	isGenericEvent && pin.name === "payload";

type NamedPin = Pick<IPin, "id" | "name" | "friendly_name">;

const storedFriendlyName = (pin: NamedPin) =>
	pin.friendly_name ?? pin.name ?? pin.id;

/**
 * Call Function / Call Reference mirror a signature by pin `name` and drop pins whose
 * name disappears — wires included — so a stored name only changes with its label.
 */
export const pinNameForLabel = (
	stored: NamedPin | undefined,
	friendlyName: string,
): string =>
	stored?.name && friendlyName === storedFriendlyName(stored)
		? stored.name
		: toMachineName(friendlyName);

export const buildInitialEdits = (
	entity: ILayer | INode,
	boardRef?: RefObject<IBoard | undefined>,
): Record<string, PinEdit> => {
	const out: Record<string, PinEdit> = {};
	const isNodeMode = "friendly_name" in entity;
	const isGenericEvent =
		isNodeMode && (entity as INode).name === "events_generic";

	for (const pin of Object.values(entity.pins)) {
		const p: typeof pin & { valueType?: unknown } = pin;
		const friendly = storedFriendlyName(p);
		let description = p.description ?? "";
		let schema = p.schema ?? null;

		// Resolve description ref if it's a hash
		const descRef = boardRef?.current?.refs?.[description];
		if (descRef) {
			description = descRef;
		}

		// Resolve schema ref if it's a hash
		if (schema && typeof schema === "string") {
			const schemaRef = boardRef?.current?.refs?.[schema];
			if (schemaRef) {
				schema = schemaRef;
			}
		}

		// Hash for empty string, empty string will not be taken into the refs.
		if (description === "16248035215404677707") {
			description = "";
		}

		// Override description for payload pin on generic_event nodes
		if (isGenericEvent && p.name === "payload") {
			description = i18next.t(
				"catchAllForAdditionalMetadata",
				"(Catch all for additional metadata)",
			);
		}

		out[pin.id] = {
			id: pin.id,
			name: pinNameForLabel(p, friendly),
			friendly_name: friendly,
			description: description,
			data_type: p.data_type ?? IVariableType.Generic,
			options: p.options ?? null,
			schema: schema,
			pin_type: pin.pin_type,
			index: pin.index ?? 1,
			value_type: normalizeValueType(p.value_type ?? p.valueType),
			default_value: p.default_value ?? null,
		};
	}
	return out;
};

/** Serializes the edits back onto the entity's pin map, re-indexed per group. */
export const serializePinEdits = (
	original: Record<string, IPin>,
	edits: Record<string, PinEdit>,
	isGenericEvent: boolean,
): Record<string, IPin> => {
	const nextPins: Record<string, IPin> = {};
	const zeroIndexed = Object.values(edits).find((p) => p.index <= 0);

	for (const edit of Object.values(edits)) {
		const prev = original[edit.id];

		nextPins[edit.id] = {
			...(prev as IPin),
			id: edit.id,
			pin_type: edit.pin_type,
			index: zeroIndexed ? (edit.index ?? 0) + 1 : (edit.index ?? 1),
			connected_to: prev?.connected_to ?? [],
			depends_on: prev?.depends_on ?? [],
			default_value: edit.default_value ?? null,
			data_type: edit.data_type,
			// The payload pin shows a localized hint in place of its description; saving
			// that hint would persist UI text as the pin's contract.
			description: isPayloadPin(edit, isGenericEvent)
				? (prev?.description ?? "")
				: (edit.description ?? ""),
			friendly_name: edit.friendly_name ?? edit.name,
			name: edit.name || toMachineName(edit.friendly_name),
			options: edit.options ?? null,
			schema: edit.schema ?? null,
			value_type: edit.value_type ?? IValueType.Normal,
		};
	}

	const nextInputs = reindex(
		sortByIndex(
			Object.values(nextPins).filter((p) => p.pin_type === IPinType.Input),
		),
	);
	const nextOutputs = reindex(
		sortByIndex(
			Object.values(nextPins).filter((p) => p.pin_type === IPinType.Output),
		),
	);
	for (const p of nextInputs)
		nextPins[p.id] = { ...nextPins[p.id], index: p.index };
	for (const p of nextOutputs)
		nextPins[p.id] = { ...nextPins[p.id], index: p.index };

	return nextPins;
};

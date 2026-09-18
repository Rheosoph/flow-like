import type { IPin } from "../../../lib/schema/flow/pin";
import { parseUint8ArrayToJson } from "../../../lib/uint8";
import { type PinRow, isPinAction } from "../flow-node/pin-collapse";
import { resolvePinEditorKind } from "./pin-editor-kind";

/**
 * Label geometry inside the fixed 150px `.react-flow__node-default` (padding-box x,
 * W = 148, 146 when selected): the input label container starts at x=4 (handle box
 * [-6,6] + ml-2.5) and grows right; the output container ends at W-2.8 (handle box
 * [W-6,W+6], translate-x -100%+0.2rem) and grows left. A both-sided row therefore
 * needs capIn + capOut ≤ 134 to keep ≥ 7px clear between the containers (≥ 11px
 * between glyphs once the label insets count); a one-sided cap of 134 stays clear of
 * the opposite handle box and its value-type icon on both sides.
 */
export const PIN_FONT_PX = 9.6;
export const PIN_ROW_LABEL_BUDGET = 134;
export const PIN_LABEL_CAP_SOLO = PIN_ROW_LABEL_BUDGET;
export const PIN_LABEL_CAP_SHARED = PIN_ROW_LABEL_BUDGET / 2;

const LABEL_INSET = 4;
const ITEM_GAP = 4;
const MENU_BUTTON = 8;
const CHECKBOX = 10;
const CHEVRON = 10;
const DELETE_BUTTON = 12;
const SELECT_TEXT_RESERVE = 72;
const WIDTH_SAFETY = 1.02;

/**
 * Per-glyph-class advance in em, the wider of Inter (light theme) and Open Sans (dark
 * theme) measured with canvas measureText, so an estimate errs toward the short side
 * keeping its full label.
 */
const CHAR_EM: readonly (readonly [RegExp, number])[] = [
	[/ /, 0.28],
	[/[iljtfrI.,:;'|!()[\]{}-]/, 0.33],
	[/[mwMW@]/, 0.91],
	[/[A-Z]/, 0.68],
	[/[0-9]/, 0.62],
];

export function estimateTextPx(text: string, fontPx = PIN_FONT_PX): number {
	let em = 0;
	for (const ch of text) {
		const rule = CHAR_EM.find(([re]) => re.test(ch));
		if (rule) em += rule[1];
		else em += (ch.codePointAt(0) ?? 0) > 0x2e7f ? 1 : 0.58;
	}
	return Math.ceil(em * fontPx * WIDTH_SAFETY);
}

function enumText(pin: IPin): string {
	const value = parseUint8ArrayToJson(pin.default_value);
	return typeof value === "string" ? value : `Select ${pin.friendly_name}`;
}

/** Estimated px the rendered label row of `pin` wants; 0 when no label renders. */
export function pinLabelNeed(pin: IPin, nodeName?: string): number {
	if (pin.name === "exec_in" || pin.name === "exec_out") return 0;
	let need = LABEL_INSET;
	switch (resolvePinEditorKind(pin, nodeName)) {
		case "label":
			need += estimateTextPx(pin.friendly_name);
			break;
		case "plain":
			need += estimateTextPx(pin.friendly_name);
			if (pin.connected_to.length === 0) need += ITEM_GAP + MENU_BUTTON;
			break;
		case "boolean":
			need += estimateTextPx(pin.friendly_name) + ITEM_GAP + CHECKBOX;
			break;
		case "enum":
			need += estimateTextPx(enumText(pin)) + CHEVRON;
			break;
		default:
			need += SELECT_TEXT_RESERVE + CHEVRON;
	}
	if (pin.dynamic) need += ITEM_GAP + DELETE_BUTTON;
	return need;
}

/** Shorter side gets its need (up to half the budget); the longer side gets the rest. */
export function splitRowCaps(
	needIn: number,
	needOut: number,
): [number, number] {
	if (needIn === 0 || needOut === 0)
		return [PIN_LABEL_CAP_SOLO, PIN_LABEL_CAP_SOLO];
	const half = PIN_ROW_LABEL_BUDGET / 2;
	if (needIn <= needOut) {
		const capIn = Math.min(needIn, half);
		return [capIn, PIN_ROW_LABEL_BUDGET - capIn];
	}
	const capOut = Math.min(needOut, half);
	return [PIN_ROW_LABEL_BUDGET - capOut, capOut];
}

export type PinLabelCaps = Readonly<Record<string, number>>;

interface RowNeeds {
	input?: IPin;
	output?: IPin;
	needIn: number;
	needOut: number;
}

/**
 * Per-pin `max-width` (px) for the label container. Rows are keyed by the display
 * row (`slots[id] ?? pin.index`, mirroring flow-pin.tsx), so collapsed nodes pair
 * the rows they actually render. IPinAction rows and exec_in/exec_out never hold a label.
 */
export function planPinLabelCaps(
	inputs: readonly PinRow[],
	outputs: readonly PinRow[],
	slots: Readonly<Record<string, number>> = {},
	nodeNameOf: (pin: IPin) => string | undefined = () => undefined,
): PinLabelCaps {
	const rows = new Map<number, RowNeeds>();
	const collect = (list: readonly PinRow[], side: "input" | "output") => {
		for (const row of list) {
			if (isPinAction(row)) continue;
			const need = pinLabelNeed(row, nodeNameOf(row));
			if (need === 0) continue;
			const key = slots[row.id] ?? row.index;
			const entry = rows.get(key) ?? { needIn: 0, needOut: 0 };
			if (side === "input") {
				entry.input = row;
				entry.needIn = need;
			} else {
				entry.output = row;
				entry.needOut = need;
			}
			rows.set(key, entry);
		}
	};
	collect(inputs, "input");
	collect(outputs, "output");
	const caps: Record<string, number> = {};
	for (const entry of rows.values()) {
		const [capIn, capOut] = splitRowCaps(entry.needIn, entry.needOut);
		if (entry.input) caps[entry.input.id] = capIn;
		if (entry.output) caps[entry.output.id] = capOut;
	}
	return caps;
}

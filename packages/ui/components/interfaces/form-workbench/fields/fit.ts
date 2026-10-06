import type { WorkbenchField } from "../contracts";

const HALF_CELL_PX = 150;
const CHAR_PX = 6.6;
const OPTIONAL_PX = 60;
const DOT_PX = 12;

/** Whether a property's label line (label, "Optional", dot) fits half of the group: only then it shares a row. */
export function fitsHalf(
	prop: Pick<WorkbenchField, "short" | "label">,
	optional: boolean,
): boolean {
	if (!prop.short) return false;
	const width =
		prop.label.length * CHAR_PX + (optional ? OPTIONAL_PX : 0) + DOT_PX;
	return width <= HALF_CELL_PX;
}

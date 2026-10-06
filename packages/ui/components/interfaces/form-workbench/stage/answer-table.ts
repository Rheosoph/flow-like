/*
 * Tables inside the answer (SURFACE §Tables, canvas answer table): numeric columns are right-aligned in
 * mono. The shared table viewer draws the cells, so the stage marks the cells of numeric columns with
 * NUMERIC_ATTR after each render and styles them from the answer's wrapper.
 */

export const NUMERIC_ATTR = "data-fw-num";

const CURRENCY = String.raw`(?:[€$£¥₹₽₩]|CHF|EUR|USD|GBP|JPY)`;
const DIGITS = String.raw`(?:\d{1,3}(?:[,.'’   ]\d{3})+|\d+)(?:[.,]\d+)?`;
/** "12", "€ 4,920.00", "1.234,50 €", "-3.5 %", "CHF 1’200": a number with its currency or percent sign. */
const NUMERIC_TEXT = new RegExp(
	String.raw`^(?:${CURRENCY}\s?)?[-+−]?(?:${CURRENCY}\s?)?${DIGITS}(?:\s?(?:%|${CURRENCY}))?$`,
	"u",
);
/** A running index ("#", "No."): numbers, but read as labels, as the canvas sets them. */
const INDEX_HEADER = /^(?:#|no\.?|nr\.?|n°|pos\.?)$/i;

export const isNumericText = (text: string) => NUMERIC_TEXT.test(text.trim());

/** Per column: every filled body cell is a number, at least one is filled, and the header is no index. */
export function numericColumnsOf(
	head: readonly string[],
	rows: readonly (readonly string[])[],
): readonly boolean[] {
	const width = rows.reduce(
		(widest, row) => Math.max(widest, row.length),
		head.length,
	);
	return Array.from({ length: width }, (_, column) => {
		if (INDEX_HEADER.test((head[column] ?? "").trim())) return false;
		const filled = rows
			.map((row) => (row[column] ?? "").trim())
			.filter((text) => text !== "");
		return filled.length > 0 && filled.every(isNumericText);
	});
}

const cellsOf = (row: Element) =>
	Array.from(row.children).filter(
		(cell) => cell.tagName === "TD" || cell.tagName === "TH",
	);

const textsOf = (row: Element) =>
	cellsOf(row).map((cell) => cell.textContent ?? "");

function markRow(row: Element, numeric: readonly boolean[]) {
	cellsOf(row).forEach((cell, column) => {
		const wanted = numeric[column] === true;
		if (wanted === cell.hasAttribute(NUMERIC_ATTR)) return;
		if (wanted) cell.setAttribute(NUMERIC_ATTR, "");
		else cell.removeAttribute(NUMERIC_ATTR);
	});
}

/** Marks the cells of numeric columns in every shown table under `root` (the hidden tracking tables are skipped). */
export function markNumericColumns(root: ParentNode) {
	for (const table of Array.from(root.querySelectorAll("table"))) {
		if (table.getAttribute("aria-hidden") === "true") continue;
		const headRows = Array.from(table.querySelectorAll(":scope > thead > tr"));
		const bodyRows = Array.from(table.querySelectorAll(":scope > tbody > tr"));
		const numeric = numericColumnsOf(
			headRows[0] ? textsOf(headRows[0]) : [],
			bodyRows.map(textsOf),
		);
		for (const row of [...headRows, ...bodyRows]) markRow(row, numeric);
	}
}

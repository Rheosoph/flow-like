import type { TEquationElement } from "platejs";

/** Stored documents can carry a number, boolean or nothing here (GHSA-p8g2-cf33-p28j). */
export function getEquationExpression({
	texExpression,
}: TEquationElement): string {
	const value: unknown = texExpression;
	return typeof value === "string" ||
		typeof value === "number" ||
		typeof value === "boolean"
		? String(value)
		: "";
}

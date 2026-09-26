/**
 * A saved page surface cannot keep the run-scoped Page actions its controls were built with: they
 * are bound to the run that minted them. The surface cache stores this marker in their place, and
 * the control stays inert until a fresh load run replaces the component.
 */
export const PENDING_PAGE_ACTION_KEY = "pendingPageAction";

/** Set on the rendered element of a component whose action is pending; the renderer styles it inert. */
export const PENDING_PAGE_ACTION_ATTRIBUTE = "data-a2ui-action-pending";

const PENDING_JSON_FRAGMENT = `"${PENDING_PAGE_ACTION_KEY}":true`;

const pendingByValue = new WeakMap<object, boolean>();

function isPendingEntry([key, child]: [string, unknown]): boolean {
	return (
		(key === PENDING_PAGE_ACTION_KEY && child === true) ||
		containsPendingPageAction(child)
	);
}

function containsPendingPageAction(value: unknown): boolean {
	if (typeof value === "string") return value.includes(PENDING_JSON_FRAGMENT);
	if (!value || typeof value !== "object") return false;
	return Object.entries(value).some(isPendingEntry);
}

/** Whether a component still carries an action that only the next load run can supply. */
export function hasPendingPageAction(value: unknown): boolean {
	if (!value || typeof value !== "object") return false;
	const known = pendingByValue.get(value);
	if (known !== undefined) return known;
	const pending = containsPendingPageAction(value);
	pendingByValue.set(value, pending);
	return pending;
}

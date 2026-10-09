/** Encode one identifier without allowing it to change the route hierarchy. */
export function segment(value: string): string {
	if (!value || value === "." || value === "..")
		throw new Error("A nonempty resource identifier is required");
	return encodeURIComponent(value);
}
export const appPath = (id: string) => `/apps/${segment(id)}`;
export const boardPath = (app: string, board: string) =>
	`${appPath(app)}/board/${segment(board)}`;
export const eventPath = (app: string, event: string) =>
	`${appPath(app)}/events/${segment(event)}`;

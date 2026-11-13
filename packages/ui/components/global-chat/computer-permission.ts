/** A bot's computer tool failed because macOS has not granted Flow Like a permission. */
export interface ComputerPermissionNeed {
	permission: ComputerPermission;
	/** Granted after Flow Like started; macOS applies it only to a new process. */
	restartRequired: boolean;
}

export type ComputerPermission = "screen_recording" | "accessibility";

export const PERMISSION_MISSING = "permission_missing";

/** The window event the desktop's `RpaPermissionProvider` answers with its permission dialog. */
export const RPA_PERMISSIONS_REQUIRED = "flow:rpa-permissions-required";

const RPA_CAPABILITY: Record<ComputerPermission, string> = {
	screen_recording: "screen_capture",
	accessibility: "accessibility",
};

function isPermission(value: unknown): value is ComputerPermission {
	return value === "screen_recording" || value === "accessibility";
}

/** Reads the tool envelope `{error: {code}, permission, restart_required}` of a failed call. */
export function computerPermissionNeed(
	result: Record<string, unknown>,
): ComputerPermissionNeed | undefined {
	const error = result.error;
	if (
		!error ||
		typeof error !== "object" ||
		(error as { code?: unknown }).code !== PERMISSION_MISSING ||
		!isPermission(result.permission)
	)
		return undefined;
	return {
		permission: result.permission,
		restartRequired: result.restart_required === true,
	};
}

function requestId(): string {
	return (
		globalThis.crypto?.randomUUID?.() ??
		`${Date.now()}-${Math.random().toString(36).slice(2)}`
	);
}

/** Opens the desktop's permission dialog for `permission`; a request id keeps it from retrying a flow run. */
export function requestComputerPermission(
	permission: ComputerPermission,
	target: EventTarget = window,
): void {
	target.dispatchEvent(
		new CustomEvent(RPA_PERMISSIONS_REQUIRED, {
			detail: {
				requestId: requestId(),
				required: [RPA_CAPABILITY[permission]],
			},
		}),
	);
}

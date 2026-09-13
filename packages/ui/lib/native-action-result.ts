export interface NativeActionRequest {
	id: string;
	scope: string;
	responseMode?: "text" | "result";
	responseDeadline?: string;
}

export interface NativeActionOutcome {
	status: "success" | "error" | "interaction_required";
	text?: string;
	json?: string;
	error?: string;
}

export interface NativeActionResult extends NativeActionOutcome {
	id: string;
	scope: string;
}

export interface NativeActionCompletionContext {
	getCurrentScope: () => string | undefined;
	invoke?: (
		command: string,
		args: { result: NativeActionResult },
	) => Promise<unknown>;
	now?: () => number;
}

export const MAX_NATIVE_RESULT_BYTES = 256 * 1024;

export function assertNativeActionCurrent(
	request: NativeActionRequest,
	currentScope: string | undefined,
	now = Date.now(),
): void {
	if (!request.id || !request.scope || request.scope !== currentScope)
		throw new Error(
			"This request belongs to a different account or workspace.",
		);
	if (request.responseDeadline !== undefined) {
		const deadline = Date.parse(request.responseDeadline);
		if (!Number.isFinite(deadline) || deadline <= now)
			throw new Error("This native request has expired. Please try again.");
	}
}

export function nativeActionErrorOutcome(error: unknown): NativeActionOutcome {
	return {
		status: "error",
		error: (error instanceof Error ? error.message : String(error)).slice(
			0,
			4000,
		),
	};
}

/** The spoken answer contains only final message text, never plan or diagnostic fields. */
export function nativeChatOutcome(message: {
	inner: { content: unknown };
	error?: { title?: string; message?: string };
	run_context?: { outcome?: string };
}): NativeActionOutcome {
	if (message.error)
		return {
			status: "error",
			error: [message.error.title, message.error.message]
				.filter(Boolean)
				.join(": "),
		};
	if (
		["error", "failed", "cancelled", "interrupted"].includes(
			message.run_context?.outcome ?? "",
		)
	)
		return {
			status: "error",
			error: "FlowPilot did not finish this response.",
		};
	const content = message.inner.content;
	const text =
		typeof content === "string"
			? content
			: Array.isArray(content)
				? content
						.filter(
							(part) => part?.type === "text" && typeof part.text === "string",
						)
						.map((part) => part.text)
						.join("\n")
				: "";
	return text.trim()
		? { status: "success", text }
		: {
				status: "interaction_required",
				error: "Open FlowPilot to view the result or continue this request.",
			};
}

/** Late or cross-account completions are deliberately ignored by both bridge layers. */
export async function completeNativeAction(
	request: NativeActionRequest,
	outcome: NativeActionOutcome,
	context: NativeActionCompletionContext,
): Promise<boolean> {
	if (!request.responseMode) return false;
	try {
		const check = () =>
			assertNativeActionCurrent(
				request,
				context.getCurrentScope(),
				context.now?.(),
			);
		check();
		let result: NativeActionResult = {
			...outcome,
			id: request.id,
			scope: request.scope,
		};
		try {
			const bytes = (value: string) => new TextEncoder().encode(value).length;
			if (
				(result.text !== undefined && bytes(result.text) > 192 * 1024) ||
				(result.json !== undefined && bytes(result.json) > 192 * 1024) ||
				(result.error !== undefined && bytes(result.error) > 4 * 1024)
			)
				throw new Error("Result field is too large");
			if (result.json !== undefined) JSON.parse(result.json);
			if (
				new TextEncoder().encode(JSON.stringify(result)).length >
				MAX_NATIVE_RESULT_BYTES
			)
				throw new Error("Result is too large");
		} catch {
			result = {
				id: request.id,
				scope: request.scope,
				status: "error",
				error:
					"The response is too large for Shortcuts. Open Flow Like to view it.",
			};
		}
		const invoke =
			context.invoke ?? (await import("@tauri-apps/api/core")).invoke;
		check();
		await invoke("native_complete_action", { result });
		return true;
	} catch {
		return false;
	}
}

/** Run preparation and execution share a live identity guard across every awaited boundary. */
export async function runNativeAction(
	request: NativeActionRequest,
	context: NativeActionCompletionContext & {
		run: (checkCurrent: () => void) => Promise<NativeActionOutcome | undefined>;
	},
): Promise<void> {
	const check = () =>
		assertNativeActionCurrent(
			request,
			context.getCurrentScope(),
			context.now?.(),
		);
	let outcome: NativeActionOutcome | undefined;
	try {
		check();
		outcome = await context.run(check);
	} catch (error) {
		outcome = nativeActionErrorOutcome(error);
	}
	if (outcome) await completeNativeAction(request, outcome, context);
}

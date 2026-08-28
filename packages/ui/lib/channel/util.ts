export const MAX_ERROR_BODY_CHARS = 1_000;

export interface ChannelPushOptions {
	/** Aborts the delivery; an aborted push is never retried through the fallback. */
	signal?: AbortSignal;
}

export function errorMessage(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

export function bodyExcerpt(text: string): string {
	return text.trim().slice(0, MAX_ERROR_BODY_CHARS);
}

export function unixSeconds(): number {
	return Math.floor(Date.now() / 1000);
}

export interface TimeoutSignal {
	signal: AbortSignal;
	timedOut: () => boolean;
	dispose: () => void;
}

/** A signal that fires after `timeoutMs` or when `parent` aborts, whichever comes first. */
export function timeoutSignal(
	timeoutMs: number,
	parent?: AbortSignal,
): TimeoutSignal {
	const controller = new AbortController();
	let timedOut = false;
	const timer = setTimeout(() => {
		timedOut = true;
		controller.abort();
	}, timeoutMs);
	const onParentAbort = () => controller.abort();
	if (parent) {
		if (parent.aborted) controller.abort();
		else parent.addEventListener("abort", onParentAbort, { once: true });
	}
	return {
		signal: controller.signal,
		timedOut: () => timedOut,
		dispose: () => {
			clearTimeout(timer);
			parent?.removeEventListener("abort", onParentAbort);
		},
	};
}

export async function readBodyExcerpt(response: Response): Promise<string> {
	try {
		return bodyExcerpt(await response.text());
	} catch {
		return "";
	}
}

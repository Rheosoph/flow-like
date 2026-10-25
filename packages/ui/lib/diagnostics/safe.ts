export const TIMED_OUT = Symbol("timed-out");

export function attempt<T>(probe: () => T, fallback: T): T {
	try {
		return probe();
	} catch {
		return fallback;
	}
}

export async function attemptAsync<T>(
	probe: () => Promise<T> | T,
	fallback: T,
): Promise<T> {
	try {
		return await probe();
	} catch {
		return fallback;
	}
}

export function withTimeout<T>(
	promise: PromiseLike<T>,
	ms: number,
): Promise<T | typeof TIMED_OUT> {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<typeof TIMED_OUT>((resolve) => {
		timer = setTimeout(() => resolve(TIMED_OUT), ms);
	});
	return Promise.race([Promise.resolve(promise), timeout]).finally(() =>
		clearTimeout(timer),
	);
}

export function errorMessage(error: unknown): string {
	if (error instanceof Error) return error.message || error.name;
	return attempt(() => String(error), "unknown error");
}

export function readGlobal<T>(name: string): T | undefined {
	return attempt(
		() => Reflect.get(globalThis, name) as T | undefined,
		undefined,
	);
}

export function readProperty<T>(
	target: unknown,
	key: PropertyKey,
): T | undefined {
	if (target === null || target === undefined) return undefined;
	return attempt(
		() => Reflect.get(Object(target), key) as T | undefined,
		undefined,
	);
}

export function nonEmptyString(value: unknown): string | null {
	if (typeof value !== "string") return null;
	const trimmed = value.trim();
	return trimmed ? trimmed : null;
}

export function finiteNumber(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function optionalBoolean(value: unknown): boolean | null {
	return typeof value === "boolean" ? value : null;
}

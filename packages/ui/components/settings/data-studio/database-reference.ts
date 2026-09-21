import type { IDatabaseSelector } from "../../../state/backend-state/db-state";

export function databaseSelectorKey(selector?: IDatabaseSelector): string {
	return JSON.stringify([
		selector?.branch ?? "main",
		selector?.version ?? null,
		selector?.tag ?? null,
		selector?.read_only ?? false,
	]);
}

export function isDatabaseSnapshot(selector?: IDatabaseSelector): boolean {
	return Boolean(
		selector?.read_only || selector?.version !== undefined || selector?.tag,
	);
}

export function databaseSelectorFromParams(params: {
	get(name: string): string | null;
}): IDatabaseSelector {
	const branch = params.get("branch") ?? undefined;
	const tag = params.get("tag");
	if (tag !== null) {
		if (!tag.trim()) throw new Error("The tag in this link is empty.");
		return { tag };
	}
	if (branch !== undefined && !branch.trim()) {
		throw new Error("The branch in this link is empty.");
	}
	const rawVersion = params.get("version");
	const version = rawVersion === null ? undefined : Number(rawVersion);
	if (
		rawVersion !== null &&
		(!/^\d+$/.test(rawVersion) ||
			!Number.isSafeInteger(version) ||
			Number(version) < 1)
	) {
		throw new Error(
			"The version in this link is invalid or exceeds the supported integer range.",
		);
	}
	return {
		...(branch && { branch }),
		...(version !== undefined ? { version } : {}),
	};
}

export function setDatabaseSelectorParams(
	params: URLSearchParams,
	selector: IDatabaseSelector,
): void {
	for (const name of ["branch", "version", "tag", "page"]) params.delete(name);
	if (selector.tag) {
		params.set("tag", selector.tag);
		return;
	}
	if (selector.branch && selector.branch !== "main") {
		params.set("branch", selector.branch);
	}
	if (selector.version !== undefined) {
		params.set("version", String(selector.version));
	}
}

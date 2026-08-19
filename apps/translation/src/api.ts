import type { Bundles, LocaleConfig } from "./lib/keys";

async function request<T>(url: string, init?: RequestInit): Promise<T> {
	const response = await fetch(url, init);
	const body = await response.json().catch(() => ({}));
	if (!response.ok) {
		throw new Error(
			(body as { error?: string }).error ??
				`${init?.method ?? "GET"} ${url} failed with ${response.status}`,
		);
	}
	return body as T;
}

export function loadLocales(): Promise<{
	config: LocaleConfig;
	bundles: Bundles;
}> {
	return request("/api/locales");
}

export function saveNamespace(
	language: string,
	namespace: string,
	tree: Record<string, unknown>,
): Promise<{ ok: true }> {
	return request(
		`/api/locales/${encodeURIComponent(language)}/${encodeURIComponent(namespace)}`,
		{
			method: "PUT",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(tree),
		},
	);
}

export function addLanguage(language: string): Promise<LocaleConfig> {
	return request("/api/languages", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ language }),
	});
}

export interface Usage {
	file: string;
	line: number;
	text: string;
}

export function findUsages(keys: string[]): Promise<Usage[]> {
	const query = new URLSearchParams();
	for (const key of keys) query.append("key", key);
	return request(`/api/usages?${query.toString()}`);
}

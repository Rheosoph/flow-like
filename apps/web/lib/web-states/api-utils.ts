import type { QueryClient } from "@tanstack/react-query";
import type { IProfile } from "@tm9657/flow-like-ui";
import type { AuthContextProps } from "react-oidc-context";

export interface WebBackendRef {
	profile?: IProfile;
	auth?: AuthContextProps;
	queryClient?: QueryClient;
}

export function getApiBaseUrl(): string {
	return process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com";
}

export function constructApiUrl(path: string): string {
	const baseUrl = getApiBaseUrl();
	const cleanBase = baseUrl.endsWith("/") ? baseUrl.slice(0, -1) : baseUrl;
	const cleanPath = path.replace(/^\/+/, "");
	return `${cleanBase}/api/v1/${cleanPath}`;
}

function jsonStringify(value: unknown): string {
	return JSON.stringify(value, (_key, v) =>
		typeof v === "bigint" ? Number(v) : v,
	);
}

export async function apiFetch<T>(
	path: string,
	options?: RequestInit,
	auth?: AuthContextProps,
): Promise<T> {
	const headers: HeadersInit = {
		"Content-Type": "application/json",
	};

	if (auth?.user?.access_token) {
		headers["Authorization"] = `Bearer ${auth.user.access_token}`;
	}

	const url = constructApiUrl(path);
	const response = await fetch(url, {
		...options,
		headers: {
			...headers,
			...options?.headers,
		},
	});

	if (!response.ok) {
		if (response.status === 401 && auth?.isAuthenticated) {
			auth.startSilentRenew();
		}
		const errorText = await response.text();
		console.error(`API error ${response.status} for ${path}:`, errorText);
		throw new Error(`API error: ${response.status}`);
	}

	const text = await response.text();
	if (!text) return undefined as T;

	try {
		return JSON.parse(text) as T;
	} catch {
		return text as T;
	}
}

export async function apiGet<T>(
	path: string,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(path, { method: "GET" }, auth);
}

export async function apiPost<T>(
	path: string,
	body?: unknown,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "POST",
			body: body ? jsonStringify(body) : undefined,
		},
		auth,
	);
}

export async function apiPut<T>(
	path: string,
	body?: unknown,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "PUT",
			body: body ? jsonStringify(body) : undefined,
		},
		auth,
	);
}

export async function apiPatch<T>(
	path: string,
	body?: unknown,
	auth?: AuthContextProps,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "PATCH",
			body: body ? jsonStringify(body) : undefined,
		},
		auth,
	);
}

export async function apiDelete<T>(
	path: string,
	auth?: AuthContextProps,
	body?: unknown,
): Promise<T> {
	return apiFetch<T>(
		path,
		{
			method: "DELETE",
			...(body
				? {
						headers: { "Content-Type": "application/json" },
						body: JSON.stringify(body),
					}
				: {}),
		},
		auth,
	);
}

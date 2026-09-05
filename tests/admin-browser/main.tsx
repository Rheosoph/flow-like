import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "sonner";
import { AdminDashboardPage } from "../../packages/ui/components/pages/admin/admin-dashboard-page";
import {
	type IBackendState,
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import "../../packages/ui/global.css";
import { responseFor } from "./responses";

const params = new URLSearchParams(location.search);
document.documentElement.classList.toggle("dark", !params.has("light"));
const profile = {
	id: "admin-fixture",
	hub: "https://fixture.invalid",
	name: "Platform team",
};
const calls: { method: string; path: string; status: string }[] = [];
const subscribers = new Set<() => void>();
const emit = () => {
	for (const callback of subscribers) callback();
};
const fixture = {
	calls,
	error: params.has("error"),
	loading: params.has("loading"),
	empty: params.has("empty"),
	restricted: params.has("restricted"),
	releases: [] as (() => void)[],
};
Object.assign(window, { adminQa: fixture });

async function request(method: string, path: string, body?: unknown) {
	const call = { method, path, status: "pending" };
	calls.push(call);
	emit();
	if (fixture.loading && path.startsWith("admin/")) {
		await new Promise<void>((resolve) => fixture.releases.push(resolve));
	}
	await new Promise((resolve) => setTimeout(resolve, 100));
	try {
		if (fixture.error && path.startsWith("admin/")) {
			throw new Error("Fixture service is unavailable. Try again.");
		}
		const response = responseFor(path, fixture.empty, method, body);
		call.status = "success";
		return response;
	} catch (error) {
		call.status = error instanceof Error ? error.message : "error";
		throw error;
	} finally {
		emit();
	}
}

function RequestLog() {
	useSyncExternalStore(
		(callback) => {
			subscribers.add(callback);
			return () => {
				subscribers.delete(callback);
			};
		},
		() => JSON.stringify(calls),
	);
	return (
		<details className="fixed bottom-3 right-3 z-50 max-w-[calc(100vw-1.5rem)] rounded-lg border bg-background p-3 text-xs shadow-lg">
			<summary className="cursor-pointer font-medium">
				Fixture requests ({calls.length})
			</summary>
			<div className="mt-3 flex gap-2">
				<button
					type="button"
					className="rounded border px-2 py-1"
					onClick={() => {
						fixture.error = false;
						fixture.loading = false;
						for (const release of fixture.releases.splice(0)) release();
					}}
				>
					Restore service
				</button>
				<button
					type="button"
					className="rounded border px-2 py-1"
					onClick={() => {
						calls.splice(0);
						emit();
					}}
				>
					Clear log
				</button>
			</div>
			<ol
				className="mt-2 max-h-64 space-y-1 overflow-auto"
				aria-label="API request log"
			>
				{calls.map((call, index) => (
					<li key={`${index}-${call.path}`} className="break-all">
						<span className="font-mono">
							{call.method} {call.path}
						</span>{" "}
						<span
							className={
								call.status === "success"
									? "text-emerald-600"
									: "text-amber-600"
							}
						>
							{call.status}
						</span>
					</li>
				))}
			</ol>
		</details>
	);
}

function Fixture() {
	const currentBackend = useBackend();
	const [backend] = useState(currentBackend);
	const [ready, setReady] = useState(false);
	useEffect(() => {
		useBackendStore.getState().setBackend({
			...backend,
			userState: {
				...backend.userState,
				getProfile: async function getProfile() {
					return profile;
				},
				getInfo: async function getInfo() {
					return {
						id: "fixture-user",
						name: "Alex Morgan",
						permission: fixture.restricted ? 2050 : 1,
					};
				},
				getSettingsProfile: async function getSettingsProfile() {
					return { hub_profile: profile };
				},
				lookupUsers: async function lookupUsers() {
					return [];
				},
			},
			apiState: {
				...backend.apiState,
				get: async (_: unknown, path: string) => request("GET", path),
				post: async (_: unknown, path: string, body: unknown) =>
					request("POST", path, body),
				put: async (_: unknown, path: string, body: unknown) =>
					request("PUT", path, body),
			},
		} as unknown as IBackendState);
		setReady(true);
	}, [backend]);
	return (
		<div className="flex h-dvh flex-col bg-background text-foreground">
			<div className="shrink-0 border-b px-4 py-2 text-xs text-muted-foreground">
				Local verification · fixture data
				{fixture.restricted
					? " · publication and solution permissions only"
					: ""}
			</div>
			{ready && <AdminDashboardPage />}
			<RequestLog />
			<Toaster />
		</div>
	);
}

const client = new QueryClient({
	defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});
const container = document.getElementById("root");
if (!container) throw new Error("Fixture root is missing");
createRoot(container).render(
	<QueryClientProvider client={client}>
		<Fixture />
	</QueryClientProvider>,
);

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { AuthContext, type AuthContextProps } from "react-oidc-context";
import DesktopAppsPage from "../../apps/desktop/app/store/explore/apps/page";
import DesktopPackagesPage from "../../apps/desktop/app/store/packages/page";
import type { GenericFetcher } from "../../packages/ui/components/pages/store/store-package-detail";
import { ExploreAppsPage } from "../../packages/ui/components/store/explore-apps-page";
import { PackagesStorePage } from "../../packages/ui/components/store/packages-store-page";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import { IAppSearchSort } from "../../packages/ui/lib/schema/app/app-search-query";
import {
	type IBackendState,
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import "./fixture.css";
import { apps, groups, packages } from "./data";
import { configureDesktopBoundaries } from "./desktop-boundaries";
import { desktopNativeResponse } from "./desktop-installed";
import { router, usePathname, useSearchParams } from "./next-navigation";

const params = new URLSearchParams(location.search);
document.documentElement.classList.toggle("dark", !params.has("light"));
const profile = {
	id: "explore-fixture",
	name: "Community workspace",
	hub: "fixture.invalid",
};
const calls: { method: string; args: unknown[]; status: string }[] = [];
const subscribers = new Set<() => void>();
const emit = () => {
	for (const callback of subscribers) callback();
};
const fixture = {
	calls,
	error: params.has("error"),
	loading: params.has("loading"),
	empty: params.has("empty"),
	developer: params.has("developer"),
	desktop: params.has("desktop"),
	runnable: params.has("runnable"),
	suitesDelay: params.has("slow-suites") ? 2000 : 120,
	appDelay: params.has("slow-apps") ? 2000 : 120,
	packagesDelay: params.has("slow-packages") ? 2000 : 120,
	deferredProfile: params.has("deferred-profile"),
	releases: [] as (() => void)[],
	restore() {
		fixture.error = false;
		fixture.loading = false;
		fixture.deferredProfile = false;
		for (const release of fixture.releases.splice(0)) release();
	},
};
Object.assign(window, { exploreQa: fixture });

async function request<T>(
	method: string,
	args: unknown[],
	response: () => T,
	options: { delay?: number; deferred?: boolean; catalog?: boolean } = {},
): Promise<T> {
	const call = { method, args, status: "pending" };
	calls.push(call);
	emit();
	if ((fixture.loading && options.catalog !== false) || options.deferred)
		await new Promise<void>((resolve) => fixture.releases.push(resolve));
	await new Promise((resolve) => setTimeout(resolve, options.delay ?? 120));
	try {
		if (fixture.error && options.catalog !== false)
			throw new Error("The community catalog is temporarily unavailable.");
		const result = response();
		call.status = "success";
		return result;
	} catch (error) {
		call.status = "error";
		throw error;
	} finally {
		emit();
	}
}

const registryFetcher: GenericFetcher = async <T,>(
	_profile: unknown,
	path: string,
) =>
	request(
		"registry",
		[path],
		() => {
			const url = new URL(path, "https://fixture.invalid");
			if (url.pathname !== "/registry/search")
				throw new Error(`Unhandled fixture request: ${path}`);
			const query = url.searchParams.get("query")?.toLowerCase() ?? "";
			const verified = url.searchParams.get("verified_only") === "true";
			const category = url.searchParams.get("category");
			const sort = url.searchParams.get("sort_by") ?? "downloads";
			const descending = url.searchParams.get("sort_desc") !== "false";
			const offset = Math.max(0, Number(url.searchParams.get("offset") ?? 0));
			const limit = Math.max(1, Number(url.searchParams.get("limit") ?? 12));
			const filtered = fixture.empty
				? []
				: packages.filter(
						(pkg) =>
							(!verified || pkg.verified) &&
							(!category || pkg.primaryCategory === category) &&
							(!query ||
								`${pkg.name} ${pkg.metadata?.name} ${pkg.description} ${pkg.keywords.join(" ")}`
									.toLowerCase()
									.includes(query)),
					);
			filtered.sort((a, b) => {
				const order =
					sort === "name"
						? a.name.localeCompare(b.name)
						: sort === "updated_at"
							? ((packages.indexOf(a) * 7) % 23) -
								((packages.indexOf(b) * 7) % 23)
							: sort === "created_at"
								? packages.indexOf(b) - packages.indexOf(a)
								: a.downloadCount - b.downloadCount;
				return descending ? -order : order;
			});
			return {
				packages: filtered.slice(offset, offset + limit),
				totalCount: filtered.length,
				offset,
				limit,
			} as T;
		},
		{ delay: fixture.packagesDelay },
	);

configureDesktopBoundaries(
	registryFetcher,
	async <T,>(command: string, args?: Record<string, unknown>) =>
		request(
			"native",
			[command, args],
			() => desktopNativeResponse(command, args, fixture.empty) as T,
		),
);

const desktopAuth = {
	isAuthenticated: true,
	isLoading: false,
	user: {
		access_token: "fixture-only",
		profile: { sub: "fixture-user", name: "Alex Morgan" },
	},
} as unknown as AuthContextProps;

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
					onClick={fixture.restore}
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
					<li key={`${index}-${call.method}`} className="break-all">
						<span className="font-mono">
							{call.method} {JSON.stringify(call.args)}
						</span>{" "}
						{call.status}
					</li>
				))}
			</ol>
		</details>
	);
}

function Destination() {
	const pathname = usePathname();
	const search = useSearchParams();
	if (fixture.desktop) {
		if (pathname === "/" || pathname === "/store/explore/apps")
			return <DesktopAppsPage />;
		if (pathname === "/store/packages" && !search.has("id"))
			return <DesktopPackagesPage />;
	}
	if (pathname === "/" || pathname === "/store/explore/apps")
		return <ExploreAppsPage />;
	if (pathname === "/store/packages" && !search.has("id"))
		return <PackagesStorePage fetcher={registryFetcher} auth={null} />;
	return (
		<main className="flex-1 p-8">
			<h1 className="text-2xl font-semibold">Navigation reached {pathname}</h1>
			<p className="mt-2 text-muted-foreground">{search.toString()}</p>
			<button
				type="button"
				className="mt-6 rounded border px-4 py-2"
				onClick={() => router.back()}
			>
				Back to Explore
			</button>
		</main>
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
				getSettingsProfile: async function getSettingsProfile() {
					return request(
						"getSettingsProfile",
						[],
						() => ({ hub_profile: profile }),
						{ catalog: false, deferred: fixture.deferredProfile },
					);
				},
				getInfo: async function getInfo() {
					return { id: "fixture-user", dev_mode: fixture.developer };
				},
				updateUser: async function updateUser() {},
			},
			routeState: {
				...backend.routeState,
				getRoutes: async function getRoutes() {
					return [];
				},
			},
			eventState: {
				...backend.eventState,
				getEvents: async function getEvents(appId: string) {
					return fixture.runnable
						? [
								{
									id: `${appId}-event`,
									name: "Open workspace",
									active: true,
									default_page_id: "fixture-app-page",
									event_type: "fixture-custom-interface",
								},
							]
						: [];
				},
			},
			appState: {
				...backend.appState,
				getApps: async function getApps() {
					return [apps[0], apps[4], apps[8]];
				},
				getStoreGroups: async function getStoreGroups(offset = 0, limit = 12) {
					return request(
						"getStoreGroups",
						[offset, limit],
						() => (fixture.empty ? [] : groups.slice(offset, offset + limit)),
						{ delay: fixture.suitesDelay },
					);
				},
				searchApps: async function searchApps(
					id?: string,
					query?: string,
					language?: string,
					category?: string,
					author?: string,
					sort?: IAppSearchSort,
					tag?: string,
					offset = 0,
					limit = 50,
				) {
					return request(
						"searchApps",
						[id, query, language, category, author, sort, tag, offset, limit],
						() => {
							if (fixture.empty) return [];
							const result = apps.filter(
								([app, meta]) =>
									(!id || app.id === id) &&
									(!category || app.primary_category === category) &&
									(!query ||
										`${meta.name} ${meta.description} ${meta.tags.join(" ")}`
											.toLowerCase()
											.includes(query.toLowerCase())),
							);
							result.sort(([a], [b]) =>
								sort === IAppSearchSort.BestRated
									? (b.avg_rating ?? 0) - (a.avg_rating ?? 0)
									: sort === IAppSearchSort.NewestCreated
										? b.created_at.secs_since_epoch -
											a.created_at.secs_since_epoch
										: sort === IAppSearchSort.NewestUpdated
											? b.updated_at.secs_since_epoch -
												a.updated_at.secs_since_epoch
											: b.download_count - a.download_count,
							);
							return result.slice(offset, offset + limit);
						},
						{ delay: fixture.appDelay },
					);
				},
			},
		} as unknown as IBackendState);
		setReady(true);
	}, [backend]);
	return (
		<TooltipProvider>
			<div className="flex h-dvh flex-col bg-background text-foreground">
				<div className="shrink-0 border-b px-4 py-2 text-xs text-muted-foreground">
					Local verification · fixture data
				</div>
				{ready && <Destination />}
				<RequestLog />
			</div>
		</TooltipProvider>
	);
}

const client = new QueryClient({
	defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});
const container = document.getElementById("root");
if (!container) throw new Error("Fixture root is missing");
createRoot(container).render(
	<QueryClientProvider client={client}>
		<AuthContext.Provider value={desktopAuth}>
			<Fixture />
		</AuthContext.Provider>
	</QueryClientProvider>,
);

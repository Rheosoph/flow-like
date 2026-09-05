import { useEffect, useRef, useState } from "react";
import { AuthContext, useAuth } from "react-oidc-context";
import {
	createDefaultHomeLayout,
	createHomeWidget,
} from "../../packages/ui/components/home/catalog";
import { HomeEditor } from "../../packages/ui/components/home/home-editor";
import type { IHomeLayout } from "../../packages/ui/components/home/types";
import {
	IAppExecutionMode,
	IAppVisibility,
} from "../../packages/ui/lib/schema/app/app";
import type { IProfile } from "../../packages/ui/lib/schema/profile/profile";
import {
	type IBackendState,
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import { useGlobalChatStore } from "../../packages/ui/state/global-chat/global-chat-store";
import {
	defaultFixtureApps,
	defaultFixtureHistory,
	defaultFixtureModels,
	defaultFixtureNotifications,
	defaultFixturePackages,
} from "./default-fixture-data";

type Scenario = "returning" | "fresh" | "offline" | "guest";
const scenarios: Scenario[] = ["returning", "fresh", "offline", "guest"];

export default function DefaultFixture() {
	const auth = useAuth();
	const original = useRef(useBackend());
	const requested = new URLSearchParams(location.search).get(
		"scenario",
	) as Scenario;
	const [scenario, setScenario] = useState<Scenario>(
		scenarios.includes(requested) ? requested : "returning",
	);
	const authenticated = scenario === "returning" || scenario === "fresh";
	const fixtureUser = auth.user
		? {
				...auth.user,
				access_token: authenticated ? auth.user.access_token : "",
				profile: {
					...auth.user.profile,
					name: scenario === "offline" ? "Cached Alex" : undefined,
					given_name: undefined,
					preferred_username: undefined,
				},
			}
		: undefined;
	return (
		<AuthContext.Provider
			value={{
				...auth,
				isAuthenticated: authenticated,
				user:
					scenario === "guest" ? undefined : (fixtureUser as typeof auth.user),
			}}
		>
			<div className="flex h-screen min-h-0 flex-col bg-background text-foreground">
				<div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b px-4 py-2 text-[11px] text-muted-foreground">
					<span>
						Bundled default · local fixture data · remote requests blocked
					</span>
					<label className="flex items-center gap-2">
						Profile state
						<select
							aria-label="Profile state"
							className="rounded-md border bg-background px-2 py-1 text-foreground"
							value={scenario}
							onChange={(event) => setScenario(event.target.value as Scenario)}
						>
							<option value="returning">Returning user</option>
							<option value="fresh">Fresh profile</option>
							<option value="offline">Offline, no account</option>
							<option value="guest">Guest, online</option>
						</select>
					</label>
				</div>
				<DefaultScenario
					key={scenario}
					scenario={scenario}
					original={original.current}
				/>
			</div>
		</AuthContext.Provider>
	);
}

function DefaultScenario({
	scenario,
	original,
}: { scenario: Scenario; original: IBackendState }) {
	const persistedLayoutKey = new URLSearchParams(location.search).has("persist")
		? `home-default-browser-qa-${scenario}`
		: undefined;
	const [layout, setLayout] = useState<IHomeLayout>(() => {
		const saved = persistedLayoutKey
			? sessionStorage.getItem(persistedLayoutKey)
			: null;
		if (saved) return JSON.parse(saved);
		const initial = createDefaultHomeLayout();
		if (
			new URLSearchParams(location.search).has("customization") ||
			new URLSearchParams(location.search).get("selection") === "legacy"
		) {
			for (const preset of ["app-collection-feature", "app-ranking"]) {
				if (!initial.widgets.some((widget) => widget.type === preset)) {
					const widget = createHomeWidget(preset);
					initial.widgets.push({
						...widget,
						id: `customization-${preset}`,
						size: { ...widget.size, heightMode: "auto" },
					});
				}
			}
		}
		if (new URLSearchParams(location.search).get("selection") === "legacy") {
			const feature = initial.widgets.find(
				(widget) => widget.type === "app-collection-feature",
			);
			if (feature)
				feature.config = {
					...feature.config,
					source: "manual",
					appIds: ["default-fixture-app-5", "default-fixture-app-0"],
					category: "Business",
					query: "Invoice",
					tag: "finance",
				};
		}
		return initial;
	});
	const [ready, setReady] = useState(false);
	useEffect(() => {
		const offline = scenario === "offline";
		const catalog = new URLSearchParams(location.search).get("catalog");
		const fixtureApps = defaultFixtureApps.map(([app, metadata], index) =>
			catalog === "varied"
				? ([
						{
							...app,
							primary_category:
								index === 5 ? "Utilities" : app.primary_category,
						},
						{
							...metadata,
							name:
								index === 0
									? "Knowledge Chat for the whole team"
									: metadata.name,
							description: `${metadata.description} Keep shared context, detailed documents, and recent work together across every project in your workspace.`,
						},
					] as (typeof defaultFixtureApps)[number])
				: ([app, metadata] as (typeof defaultFixtureApps)[number]),
		);
		const fixtureModels =
			catalog === "varied"
				? defaultFixtureModels.map((model) => ({
						...model,
						parameters: { ...model.parameters, license: "Apache 2.0" },
						meta: {
							en: {
								...model.meta.en,
								name: "Workspace reasoning with documents and images",
								description:
									"A model for working through detailed documents, diagrams, shared knowledge, and complex questions from your team's daily work.",
							},
						},
					}))
				: defaultFixtureModels;
		const discoverable =
			catalog === "empty"
				? []
				: fixtureApps.map(
						([app, meta]) =>
							[
								catalog === "unrated"
									? {
											...app,
											rating_count: 0,
											rating_sum: 0,
											avg_rating: 0,
											download_count: 0,
										}
									: app,
								meta,
							] as (typeof defaultFixtureApps)[number],
					);
		const authenticated = scenario === "returning" || scenario === "fresh";
		const localApps = fixtureApps.slice(0, 2).map(
			([app, metadata]) =>
				[
					{
						...app,
						visibility: IAppVisibility.Offline,
						execution_mode: IAppExecutionMode.Local,
					},
					metadata,
				] as (typeof defaultFixtureApps)[number],
		);
		const owned =
			scenario === "returning"
				? fixtureApps.slice(0, 4)
				: offline
					? localApps
					: [];
		const profile: IProfile = {
			id: `default-fixture-${scenario}`,
			name: offline ? "Local workspace" : "Maker workspace",
			hub: location.origin,
			secure: false,
			bits: scenario === "returning" ? [defaultFixtureModels[0].id] : [],
			apps: owned.map(([app], index) => ({
				app_id: app.id,
				favorite: index < 2,
				pinned: index === 0,
			})),
			created: "2026-09-05",
			updated: "2026-09-05",
		};
		const calls: Record<string, number> = {};
		const record = (name: string) => {
			calls[name] = (calls[name] ?? 0) + 1;
		};
		const online = () => {
			if (offline) throw new Error("This fixture is offline.");
		};
		const account = () => {
			if (!authenticated)
				throw new Error("This fixture has no account session.");
		};
		const history = scenario === "returning" ? defaultFixtureHistory : [];
		const notifications =
			scenario === "returning"
				? structuredClone(defaultFixtureNotifications)
				: [];
		Object.assign(window, {
			defaultHomeQa: {
				scenario,
				calls,
				saved: null,
				profile,
				flowPilotState: () => useGlobalChatStore.getState().mode,
			},
		});
		useBackendStore.getState().setBackend({
			...original,
			profile,
			capabilities: () => ({
				needsSignIn: !offline,
				canExecuteLocally: offline,
				canHostEmbeddings: offline,
				canHostLlamaCPP: offline,
				canHostMLX: false,
			}),
			isOffline: async () => offline,
			appState: {
				...original.appState,
				getApps: async () => {
					record("library");
					if (new URLSearchParams(location.search).has("ownership")) {
						await new Promise<void>((resolve) => {
							Object.assign(Reflect.get(window, "defaultHomeQa"), {
								resolveOwnership: resolve,
							});
						});
					}
					return owned;
				},
				searchApps: async (
					_id: string | undefined,
					query: string | undefined,
					_publisher: string | undefined,
					category: string | undefined,
					_type: string | undefined,
					sort: string | undefined,
					tag: string | undefined,
					offset = 0,
					limit = 20,
				) => {
					record("discovery");
					online();
					return [...discoverable]
						.filter(
							([app, meta]) =>
								(!query ||
									`${meta.name} ${meta.description}`
										.toLowerCase()
										.includes(query.toLowerCase())) &&
								(!category || app.primary_category === category) &&
								(!tag || meta.tags?.includes(tag)),
						)
						.sort((a, b) =>
							String(sort).toLowerCase().includes("popular")
								? b[0].rating_sum - a[0].rating_sum
								: b[0].created_at.secs_since_epoch -
									a[0].created_at.secs_since_epoch,
						)
						.slice(offset, offset + limit);
				},
				getApp: async (id: string) => {
					const app = (offline ? localApps : fixtureApps).find(
						([app]) => app.id === id,
					)?.[0];
					if (!app) throw new Error("Fixture app not found.");
					return app;
				},
				getAppMeta: async (id: string) =>
					fixtureApps.find(([app]) => app.id === id)?.[1],
			},
			userState: {
				...original.userState,
				getProfile: async () => profile,
				getSettingsProfile: async () => ({ hub_profile: profile }),
				getAllSettingsProfiles: async () => [{ hub_profile: profile }],
				getInfo: async () => {
					record("account");
					account();
					return { name: "Felix", tier: "FREE" };
				},
				getNotifications: async () => {
					record("notification-count");
					account();
					return {
						notifications_count: notifications.length,
						unread_count: notifications.filter((item) => !item.read).length,
					};
				},
				listNotifications: async (unreadOnly?: boolean) => {
					record("notifications");
					account();
					return notifications.filter((item) => !unreadOnly || !item.read);
				},
				markNotificationRead: async (id: string) => {
					const item = notifications.find((item) => item.id === id);
					if (item) item.read = true;
				},
			},
			registryState: {
				...original.registryState,
				searchPackages: async ({
					query = "",
					limit = 20,
					offset = 0,
				}: { query?: string; limit?: number; offset?: number }) => {
					record("packages");
					online();
					const packages = (
						catalog === "empty" ? [] : defaultFixturePackages
					).filter((item) =>
						`${item.name} ${item.description}`
							.toLowerCase()
							.includes(query.toLowerCase()),
					);
					return {
						packages: packages.slice(offset, offset + limit),
						totalCount: packages.length,
						offset,
						limit,
					};
				},
			},
			bitState: {
				...original.bitState,
				getProfileBits: async () =>
					scenario === "returning" ? fixtureModels : [],
				searchBits: async () => {
					online();
					return catalog === "empty" ? [] : fixtureModels;
				},
				getBit: async (id: string, hub?: string) => {
					const model =
						catalog === "empty"
							? undefined
							: fixtureModels.find(
									(model) => model.id === id && (!hub || model.hub === hub),
								);
					if (!model) throw new Error("Fixture model not found.");
					return model;
				},
				isBitInstalled: async () => true,
				getBitSize: async () => 0,
			},
			usageState: offline
				? undefined
				: {
						getExecutionHistory: async (
							page = 0,
							pageSize = 100,
							appId?: string,
						) => {
							record("history");
							account();
							const items = history.filter(
								(row) => !appId || row.app_id === appId,
							);
							return {
								items: items.slice(page * pageSize, (page + 1) * pageSize),
								total: items.length,
								page,
								page_size: pageSize,
							};
						},
						getUsageSummary: async () => {
							record("usage");
							account();
							return {
								total_executions: history.length,
								total_llm_invocations: history.length ? 148 : 0,
								total_embedding_invocations: history.length ? 62 : 0,
								total_llm_price: history.length ? 1_240_000 : 0,
								total_embedding_price: history.length ? 180_000 : 0,
							};
						},
						getLlmHistory: async () => ({
							items: [],
							total: 0,
							page: 0,
							page_size: 100,
						}),
						getEmbeddingHistory: async () => ({
							items: [],
							total: 0,
							page: 0,
							page_size: 100,
						}),
					},
		} as unknown as IBackendState);
		setReady(true);
	}, [scenario, original]);
	if (!ready) return null;
	return (
		<HomeEditor
			draftKey={`default-fixture-${scenario}`}
			layout={layout}
			defaultLayout={createDefaultHomeLayout()}
			sourceLabel={scenario === "offline" ? "Local profile" : "Default home"}
			onSave={async (next) => {
				const qa = Reflect.get(window, "defaultHomeQa");
				qa.calls.saves = (qa.calls.saves ?? 0) + 1;
				Object.assign(window, {
					defaultHomeQa: {
						...Reflect.get(window, "defaultHomeQa"),
						saved: next,
					},
				});
				setLayout(next);
				if (persistedLayoutKey)
					sessionStorage.setItem(persistedLayoutKey, JSON.stringify(next));
			}}
			onReset={async () => {
				const qa = Reflect.get(window, "defaultHomeQa");
				qa.calls.resets = (qa.calls.resets ?? 0) + 1;
				setLayout(createDefaultHomeLayout());
				if (persistedLayoutKey) sessionStorage.removeItem(persistedLayoutKey);
			}}
		/>
	);
}

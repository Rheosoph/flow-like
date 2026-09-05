import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { lazy, Suspense, useState } from "react";
import { createRoot } from "react-dom/client";
import { AuthContext } from "react-oidc-context";
import { HomeEditor } from "../../packages/ui/components/home/home-editor";
import {
	createDefaultHomeLayout,
	createHomeWidget,
} from "../../packages/ui/components/home/catalog";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import { MobileHeaderProvider } from "../../packages/ui/components/ui/mobile-header";
import { Toaster } from "sonner";
import "../../packages/ui/global.css";

const DataFixture = lazy(() => import("./data-fixture"));
const ActivityFixture = lazy(() => import("./activity-fixture"));
const ProfileFixture = lazy(() => import("./profile-fixture"));

const at = { secs_since_epoch: 1788566400, nanos_since_epoch: 0 };
const apps = [
	"Knowledge Chat",
	"Invoice OCR",
	"Sheet Sync",
	"Webhook Relay",
].map((name, index) => [
	{
		id: `fixture-app-${index}`,
		authors: [],
		bits: [],
		boards: [],
		events: [],
		page_ids: [],
		widget_ids: [],
		templates: [],
		status: "Active",
		visibility: "Private",
		execution_mode: "Remote",
		primary_category: "Productivity",
		created_at: at,
		updated_at: { ...at, secs_since_epoch: at.secs_since_epoch + index },
		download_count: 0,
		rating_count: 0,
		rating_sum: 0,
	},
	{
		name,
		description: [
			"Find answers in your team's documents.",
			"Extract and review incoming invoices.",
			"Keep your spreadsheet work in sync.",
			"Connect the tools you already use.",
		][index],
		tags: ["productivity"],
		created_at: at,
		updated_at: at,
		preview_media: [],
	},
]);
const profile = {
	id: "qa-profile",
	name: "Fixture profile",
	hub: "fixture.invalid",
	bits: [],
	apps: apps.map(([app], i) => ({
		app_id: app.id,
		favorite: i < 2,
		pinned: false,
	})),
};
const events = ["/", "/reports", "/details"].map((route, i) => ({
	id: `fixture-event-${i}`,
	name: ["Landing page", "Reports", "Details"][i],
	route,
	is_default: i === 0,
	default_page_id: `fixture-page-${i}`,
	active: true,
	board_id: "fixture-board",
	board_version: [1, 0, 0],
	config: [],
	event_type: "quick_action",
	event_version: [1, 0, 0],
	node_id: "fixture-node",
	priority: 0,
	description: "Fixture app page",
	variables: {},
	created_at: at,
	updated_at: at,
}));
const counters = {
	bootstrap: 0,
	executions: 0,
	saves: 0,
	resets: 0,
	saveAttempts: 0,
};
(window as any).homeQa = {
	counters,
	getSaved: () => (window as any).homeQa.saved,
};
const component = (id: string, value: any) => ({
	id,
	component: { id, ...value },
});
function page(event: any) {
	return {
		id: event.default_page_id,
		name: event.name,
		layoutType: "stack",
		content: [],
		createdAt: "2026-09-05",
		updatedAt: "2026-09-05",
		canvasSettings: { padding: "24px" },
		components: [
			component("root", {
				type: "column",
				gap: { literalString: "16px" },
				children: { explicitList: ["heading", "body", "navigate"] },
			}),
			component("heading", {
				type: "text",
				content: { literalString: `Fixture ${event.name}` },
				size: { literalString: "xl" },
				weight: { literalString: "semibold" },
			}),
			component("body", {
				type: "text",
				content: {
					literalString:
						"This native app page uses a fixture backend. Navigation stays inside this widget.",
				},
			}),
			component("navigate", {
				type: "button",
				label: { literalString: "Show details in this widget" },
				eventHandlers: {
					click: [
						{
							name: "navigate_page",
							context: { route: "/details", queryParams: { item: "42" } },
						},
					],
				},
			}),
		],
	};
}

function Harness() {
	const original = useBackend();
	const [ready] = useState(() => {
		useBackendStore.getState().setBackend({
			...original,
			profile,
			capabilities: () => ({
				needsSignIn: false,
				canExecuteLocally: false,
				canHostEmbeddings: false,
				canHostLlamaCPP: false,
				canHostMLX: false,
			}),
			isOffline: async () => false,
			appState: {
				...original.appState,
				getApps: async () => apps,
				searchApps: async (_id: any, query: string) =>
					apps.filter(
						([, meta]) =>
							!query || meta.name.toLowerCase().includes(query.toLowerCase()),
					),
				getApp: async (id: string) => apps.find(([app]) => app.id === id)?.[0],
				getAppMeta: async (id: string) =>
					apps.find(([app]) => app.id === id)?.[1],
			},
			userState: {
				...original.userState,
				getProfile: async () => profile,
				getSettingsProfile: async () => ({ hub_profile: profile }),
				getAllSettingsProfiles: async () => [{ hub_profile: profile }],
				listNotifications: async () => [],
			},
			eventState: {
				...original.eventState,
				getEvents: async () => events,
				getEvent: async (_id: string, eventId: string) =>
					events.find((event) => event.id === eventId),
			},
			routeState: {
				...original.routeState,
				getRoutes: async () =>
					events.map((event) => ({ path: event.route, eventId: event.id })),
			},
			pageState: {
				...original.pageState,
				getPageBootstrap: async (
					_id: string,
					route: string,
					eventId?: string,
				) => {
					counters.bootstrap++;
					const event =
						events.find((event) =>
							eventId ? event.id === eventId : event.route === route,
						) ?? events[0];
					return {
						event,
						page: page(event),
						revision: "fixture-content-v1",
						executionRevision: "fixture-auth-v1",
						canonicalRoute: event.route,
					};
				},
			},
			boardState: {
				...original.boardState,
				getBoard: async () => ({ id: "fixture-board", nodes: {}, pins: {} }),
			},
			bitState: {
				...original.bitState,
				getProfileBits: async () => [],
				searchBits: async () => [],
			},
			registryState: {
				...original.registryState,
				searchPackages: async () => ({ packages: [], totalCount: 0 }),
			},
			usageState: {
				getExecutionHistory: async () => ({
					items: [],
					total: 0,
					page: 0,
					page_size: 10,
				}),
				getUsageSummary: async () => ({
					total_executions: 0,
					total_llm_invocations: 0,
					total_embedding_invocations: 0,
				}),
			},
		} as any);
		return true;
	});
	const [defaultLayout] = useState<any>(() =>
		new URLSearchParams(location.search).has("default")
			? createDefaultHomeLayout()
			: {
					version: 1,
					widgets: [
						createHomeWidget("greeting"),
						createHomeWidget("flowpilot-bar"),
						createHomeWidget("recent-apps"),
						{
							...createHomeWidget("info"),
							config: {
								mode: "markdown",
								body: "## Your workspace, your way\nKeep useful apps and context close. Choose **Customize** to try the production editor.",
							},
						},
					],
				},
	);
	const [layout, setLayout] = useState(defaultLayout);
	const [editorKey, setEditorKey] = useState(0);
	(window as any).homeQa.remount = () => setEditorKey((key) => key + 1);
	return (
		<div style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
			<div className="shrink-0 border-b bg-card px-5 py-2 text-[11px] text-muted-foreground">
				Local UI verification · fixture data · remote requests blocked
			</div>
			<HomeEditor
				key={editorKey}
				draftKey="fixture-profile-home"
				layout={layout}
				defaultLayout={defaultLayout}
				sourceLabel="Fixture profile"
				onReset={async () => {
					counters.resets++;
					setLayout(structuredClone(defaultLayout));
				}}
				onSave={async (value) => {
					counters.saveAttempts++;
					if ((window as any).homeQa.holdSave)
						await new Promise<void>((resolve) => {
							(window as any).homeQa.releaseSave = resolve;
						});
					if ((window as any).homeQa.failSave)
						throw new Error(
							"Fixture save failed. Your changes are still here.",
						);
					counters.saves++;
					(window as any).homeQa.saved = structuredClone(value);
					setLayout(value);
				}}
			/>
			<Toaster />
		</div>
	);
}
const queryClient = new QueryClient({
	defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});
createRoot(document.getElementById("root")!).render(
	<AuthContext.Provider
		value={
			{
				isAuthenticated: true,
				isLoading: false,
				user: {
					access_token: "fixture-only",
					profile: {
						sub: "fixture-user",
						name: "Alex Example",
						given_name: "Alex",
					},
				},
			} as any
		}
	>
		<QueryClientProvider client={queryClient}>
			<TooltipProvider>
				<MobileHeaderProvider>
					<Suspense fallback={<p>Loading local fixture…</p>}>
						{location.pathname.startsWith("/admin/profiles") ? (
							<ProfileFixture />
						) : location.pathname === "/data-fixture" ? (
							<DataFixture />
						) : location.pathname === "/activity-fixture" ? (
							<ActivityFixture />
						) : (
							<Harness />
						)}
					</Suspense>
				</MobileHeaderProvider>
			</TooltipProvider>
		</QueryClientProvider>
	</AuthContext.Provider>,
);

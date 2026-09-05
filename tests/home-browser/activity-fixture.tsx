import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { HomeWidgetContent } from "../../packages/ui/components/home/home-widget-content";
import type { IHomeWidget } from "../../packages/ui/components/home/types";
import type { IApp } from "../../packages/ui/lib/schema/app/app";
import type { IMetadata } from "../../packages/ui/lib/schema/bit/bit";
import type { IExecutionUsageRecord } from "../../packages/ui/lib/schema/usage/tracking";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import type { INotification } from "../../packages/ui/state/backend-state/types";

const now = Date.now();
const rows: IExecutionUsageRecord[] = Array.from(
	{ length: 100 },
	(_, index) => ({
		id: `fixture-execution-${index}`,
		app_id: index % 10 === 0 ? null : `fixture-app-${index % 3}`,
		created_at: new Date(now - (index + 1) * 80 * 60_000).toISOString(),
		status: ["Info", "Info", "Warn", "Error", "Debug", "Fatal"][index % 6],
		microseconds: (index + 1) * 1000,
		board_id: "fixture-board",
		node_id: "fixture-node",
		version: "1",
		instance: null,
		technical_user_id: null,
	}),
);
const notifications: INotification[] = [
	{
		id: "fixture-notification",
		user_id: "fixture-user",
		title: "Fixture invoice needs review",
		description: "Local test notification. No production data is loaded.",
		link: "/notifications",
		notification_type: "WORKFLOW",
		read: false,
		created_at: new Date(now).toISOString(),
	},
];

function widget(type: string, days: number): IHomeWidget {
	return {
		id: type,
		type,
		size: { columns: 6, rows: 5 },
		appearance: { variant: "card", accent: "purple" },
		config: { days, limit: 5 },
	};
}

export default function ActivityFixture() {
	const original = useBackend();
	const queryClient = useQueryClient();
	const [scenario, setScenario] = useState("populated");
	const scenarioRef = useRef(scenario);
	const [days, setDays] = useState(7);
	const [ready] = useState(() => {
		const checkScenario = () => {
			if (scenarioRef.current === "error")
				throw new Error("Fixture history unavailable");
		};
		useBackendStore.getState().setBackend({
			...original,
			profile: {
				id: "activity-fixture",
				hub: "fixture.invalid",
				secure: true,
				name: "Activity fixture",
			},
			appState: {
				...original.appState,
				getApps: async () =>
					["Fixture Chat", "Fixture Invoice OCR", "Fixture Sheet Sync"].map(
						(name, index): [IApp, IMetadata] => [
							{ id: `fixture-app-${index}` } as IApp,
							{ name } as IMetadata,
						],
					),
			},
			userState: {
				...original.userState,
				getNotifications: async () => ({
					notifications_count: notifications.length,
					unread_count: notifications.filter((item) => !item.read).length,
				}),
				listNotifications: async (unreadOnly) => {
					checkScenario();
					return scenarioRef.current === "empty"
						? []
						: notifications.filter((item) => !unreadOnly || !item.read);
				},
				markNotificationRead: async (id) => {
					const item = notifications.find(
						(notification) => notification.id === id,
					);
					if (item) item.read = true;
				},
			},
			usageState: {
				getExecutionHistory: async (
					_page?: number,
					pageSize = 100,
					appId?: string,
				) => {
					checkScenario();
					return {
						items:
							scenarioRef.current === "empty"
								? []
								: rows
										.filter((row) => !appId || row.app_id === appId)
										.slice(0, pageSize),
						total: scenarioRef.current === "empty" ? 0 : 243,
						page: 0,
						page_size: pageSize,
					};
				},
				getUsageSummary: async () => {
					checkScenario();
					const empty = scenarioRef.current === "empty";
					return {
						total_executions: empty ? 0 : 243,
						total_llm_invocations: empty ? 0 : 345,
						total_embedding_invocations: empty ? 0 : 120,
						total_llm_price: empty ? 0 : 1_250_000,
						total_embedding_price: empty ? 0 : 350_000,
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
		});
		return true;
	});
	if (!ready) return null;
	return (
		<main className="min-h-screen bg-background p-5 text-foreground">
			<header className="mb-5 flex flex-wrap items-center justify-between gap-4">
				<div className="min-w-0">
					<h1 className="text-xl font-semibold">
						Activity widgets · local fixture
					</h1>
					<p className="text-xs text-muted-foreground">
						Production renderers with mock account history. No remote data.
					</p>
				</div>
				<div className="flex flex-wrap gap-4">
					<label className="flex items-center gap-2 text-sm">
						Scenario
						<select
							aria-label="Scenario"
							className="rounded border bg-background p-2"
							value={scenario}
							onChange={(event) => {
								scenarioRef.current = event.target.value;
								setScenario(event.target.value);
								void queryClient.invalidateQueries({ queryKey: ["home"] });
							}}
						>
							<option value="populated">Populated</option>
							<option value="empty">Empty</option>
							<option value="error">Access error</option>
						</select>
					</label>
					<label className="flex items-center gap-2 text-sm">
						Period
						<select
							aria-label="Period"
							className="rounded border bg-background p-2"
							value={days}
							onChange={(event) => setDays(Number(event.target.value))}
						>
							<option value={1}>Today</option>
							<option value={7}>Last 7 days</option>
							<option value={30}>Last 30 days</option>
						</select>
					</label>
				</div>
			</header>
			<div
				style={{
					display: "grid",
					gridTemplateColumns: "repeat(auto-fit,minmax(min(100%,380px),1fr))",
					gap: 16,
				}}
			>
				{[
					"run-activity",
					"executions-by-app",
					"ai-usage",
					"needs-attention",
				].map((type) => (
					<section
						key={type}
						data-testid={`activity-${type}`}
						className="flex min-w-0 flex-col overflow-hidden rounded-xl border bg-card"
						style={{ height: type === "needs-attention" ? 520 : 360 }}
					>
						<h2 className="shrink-0 border-b px-4 py-3 text-base font-semibold">
							{type}
						</h2>
						<div className="min-h-0 flex-1">
							<HomeWidgetContent widget={widget(type, days)} />
						</div>
					</section>
				))}
			</div>
		</main>
	);
}

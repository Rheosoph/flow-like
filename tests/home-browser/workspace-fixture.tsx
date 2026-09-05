import { useEffect, useRef, useState } from "react";
import { AuthContext, useAuth } from "react-oidc-context";
import { HomeWorkspacePulse } from "../../packages/ui/components/home/home-content/workspace-overview";
import type { IHomeWidget } from "../../packages/ui/components/home/types";
import type { IApp } from "../../packages/ui/lib/schema/app/app";
import type { IMetadata } from "../../packages/ui/lib/schema/bit/bit";
import type { IExecutionUsageRecord } from "../../packages/ui/lib/schema/usage/tracking";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";

const now = Date.now();
const records: IExecutionUsageRecord[] = Array.from(
	{ length: 60 },
	(_, index) => ({
		id: `record-${index}`,
		app_id: `app-${index % 3}`,
		status: ["Info", "Warn", "Info", "Error", "Info", "Fatal"][index % 6],
		created_at: new Date(now - index * 2 * 3_600_000).toISOString(),
		instance: null,
		technical_user_id: null,
		board_id: "board",
		node_id: "node",
		version: "1",
		microseconds: 1000,
	}),
);
const baseWidget: IHomeWidget = {
	id: "workspace",
	type: "workspace-pulse",
	size: { columns: 4, rows: 4 },
	appearance: { variant: "card", accent: "purple" },
	config: { days: 7 },
};

export default function WorkspaceFixture() {
	const backend = useBackend();
	const original = useRef(backend);
	const auth = useAuth();
	const [scenario, setScenario] = useState("populated");
	const [applied, setApplied] = useState("");
	const [calls, setCalls] = useState(0);
	const [days, setDays] = useState(7);
	useEffect(() => {
		const value = original.current;
		useBackendStore.getState().setBackend({
			...value,
			profile: {
				...value.profile,
				id: `pulse-${scenario}`,
				name: "Operations workspace",
				hub: "fixture.invalid",
				secure: true,
			},
			appState: {
				...value.appState,
				getApps: async () =>
					scenario === "empty"
						? []
						: ["Knowledge Chat", "Invoice OCR", "Sheet Sync"].map(
								(name, index): [IApp, IMetadata] => [
									{ id: `app-${index}` } as IApp,
									{ name } as IMetadata,
								],
							),
			},
			userState: {
				...value.userState,
				getProfile: async () => ({
					id: `pulse-${scenario}`,
					name: "Operations workspace",
					apps:
						scenario === "empty"
							? null
							: [0, 1, 2].map((index) => ({
									app_id: `app-${index}`,
									favorite: false,
									pinned: false,
								})),
				}),
			},
			usageState:
				scenario === "offline"
					? undefined
					: {
							...value.usageState,
							getExecutionHistory: async () => {
								setCalls((count) => count + 1);
								await new Promise((resolve) => setTimeout(resolve, 80));
								if (scenario === "error")
									throw new Error("Fixture history is unavailable.");
								return {
									items: scenario === "empty" ? [] : records,
									total: scenario === "empty" ? 0 : 240,
									page: 0,
									page_size: 100,
								};
							},
						},
		} as typeof value);
		setApplied(scenario);
	}, [scenario]);
	return (
		<AuthContext.Provider
			value={{
				...auth,
				isAuthenticated: scenario !== "guest",
				user: scenario === "guest" ? undefined : auth.user,
			}}
		>
			<main className="min-h-screen bg-background p-4 text-foreground">
				<header className="mb-6 flex flex-wrap items-center gap-4">
					<h1 className="text-xl font-semibold">
						Workspace pulse · local fixture
					</h1>
					<label className="flex items-center gap-2 text-sm">
						Scenario
						<select
							className="rounded border bg-background p-2"
							aria-label="Scenario"
							value={scenario}
							onChange={(event) => setScenario(event.target.value)}
						>
							{["populated", "empty", "offline", "guest", "error"].map(
								(value) => (
									<option key={value}>{value}</option>
								),
							)}
						</select>
					</label>
					<label className="flex items-center gap-2 text-sm">
						Days
						<select
							className="rounded border bg-background p-2"
							aria-label="Days"
							value={days}
							onChange={(event) => setDays(Number(event.target.value))}
						>
							{[1, 7, 30].map((value) => (
								<option key={value}>{value}</option>
							))}
						</select>
					</label>
					<output data-testid="usage-calls" className="text-xs">
						{calls}
					</output>
				</header>
				<div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,320px),1fr))] items-start gap-4">
					{applied === scenario &&
						[4, 6, 12].map((width) => (
							<section
								data-testid={`pulse-${width}`}
								key={`${scenario}:${width}`}
								className="min-w-0 overflow-hidden rounded-2xl border border-border/60 bg-card/70"
							>
								<HomeWorkspacePulse
									widget={{
										...baseWidget,
										id: `pulse-${width}`,
										config: { days, showAttention: width !== 6 },
									}}
								/>
							</section>
						))}
				</div>
			</main>
		</AuthContext.Provider>
	);
}

import { loader } from "@monaco-editor/react";
import { ThemeProvider, useTheme } from "next-themes";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { ForecastWorkspace } from "../../packages/ui/components/settings/data-studio/forecasting/forecast-workspace";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import {
	BackendContext,
	type IBackendState,
	useBackend,
} from "../../packages/ui/state/backend-state";
import type {
	SavedStudioForecast,
	StudioForecastPayload,
} from "../../packages/ui/state/backend-state/query-state";
import { forecast, models, preview } from "./data";
import "./fixture.css";

declare const __FORECAST_MONACO_PATH__: string;
loader.config({ paths: { vs: __FORECAST_MONACO_PATH__ } });

const calls: { method: string; payload?: unknown }[] = [];
const snapshots: SavedStudioForecast[] = [];
Object.assign(window, { forecastingQa: { calls, snapshots } });
const pause = () => new Promise((resolve) => setTimeout(resolve, 150));

function Fixture() {
	const base = useBackend();
	const { resolvedTheme, setTheme } = useTheme();
	const [userScoped, setUserScoped] = useState(false);
	const [savedForecastId, setSavedForecastId] = useState<string>();
	const [backend] = useState<IBackendState>(() => ({
		...base,
		queryState: {
			...base.queryState,
			async executeSql(_appId, payload) {
				calls.push({ method: "executeSql", payload });
				await pause();
				return preview(payload.sql);
			},
			async forecast(_appId, payload: StudioForecastPayload) {
				calls.push({ method: "forecast", payload });
				await pause();
				return forecast(payload);
			},
			async listForecasts() {
				return snapshots.map(({ request, result, ...summary }) => ({
					...summary,
					model_bit_id: request.model_bit_id,
					horizon: request.horizon,
					run_id: result.run_id,
				}));
			},
			async createForecast(appId, payload) {
				const saved: SavedStudioForecast = {
					...payload,
					id: `fixture-saved-${snapshots.length + 1}`,
					app_id: appId,
					created_by: "fixture-user",
					created_at: "2026-10-16T00:00:00Z",
				};
				snapshots.push(saved);
				return saved;
			},
			async getForecast(_appId, id) {
				const result = snapshots.find((snapshot) => snapshot.id === id);
				if (!result) throw new Error("Fixture prediction not found");
				return result;
			},
		},
	}));
	return (
		<BackendContext.Provider value={backend}>
			<TooltipProvider>
				<div className="min-h-dvh bg-background text-foreground">
					<div className="border-b bg-card px-4 py-2 text-center text-xs text-muted-foreground">
						Local preview · Synthetic sensor data and forecast results ·
						Production UI components
					</div>
					<main className="mx-auto max-w-[1440px] p-3 sm:p-6">
						<header className="mb-5 flex flex-wrap items-center justify-between gap-3 border-b px-1 pb-4">
							<div>
								<p className="text-xs text-muted-foreground">
									Sensor operations / Data Studio
								</p>
								<h1 className="mt-2 text-2xl font-semibold">
									Machine temperature predictions
								</h1>
							</div>
							<button
								type="button"
								className="rounded-md border bg-card px-3 py-2 text-xs"
								onClick={() =>
									setTheme(resolvedTheme === "dark" ? "light" : "dark")
								}
							>
								Toggle color theme
							</button>
						</header>
						<div className="overflow-hidden rounded-xl border">
							<ForecastWorkspace
								appId="fixture-sensors"
								profileId="fixture-profile"
								projectTables={["sensor_readings", "machine_schedule"]}
								userTables={["my_sensor_readings"]}
								userScoped={userScoped}
								onScopeChange={setUserScoped}
								models={models}
								canWrite
								savedForecastId={savedForecastId}
								onSavedForecastChange={setSavedForecastId}
								onAskFlowPilot={() => calls.push({ method: "askFlowPilot" })}
							/>
						</div>
					</main>
				</div>
			</TooltipProvider>
		</BackendContext.Provider>
	);
}

const root = document.getElementById("root");
if (!root) throw new Error("Fixture root is missing");
createRoot(root).render(
	<ThemeProvider attribute="class" defaultTheme="dark" enableSystem={false}>
		<Fixture />
	</ThemeProvider>,
);

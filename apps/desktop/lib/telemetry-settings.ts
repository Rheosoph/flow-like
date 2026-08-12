import { invoke } from "@tauri-apps/api/core";

export interface ITelemetrySettings {
	enabled: boolean | null;
	crashReports: boolean | null;
	anonId: string | null;
}

const SETTINGS_EVENT = "flow-like:telemetry-settings";

export async function getTelemetrySettings(): Promise<ITelemetrySettings> {
	return invoke<ITelemetrySettings>("get_telemetry_settings");
}

function publish(settings: ITelemetrySettings): ITelemetrySettings {
	window.dispatchEvent(
		new CustomEvent<ITelemetrySettings>(SETTINGS_EVENT, { detail: settings }),
	);
	return settings;
}

export async function setTelemetryEnabled(
	enabled: boolean,
): Promise<ITelemetrySettings> {
	return publish(
		await invoke<ITelemetrySettings>("set_telemetry_enabled", { enabled }),
	);
}

export async function setCrashReportsEnabled(
	enabled: boolean,
): Promise<ITelemetrySettings> {
	return publish(
		await invoke<ITelemetrySettings>("set_crash_reports_enabled", { enabled }),
	);
}

/**
 * Crash reporting defaults on: only an explicit `false` disables it. Settings
 * that have not loaded yet count as off so nothing is captured before the
 * stored decision is known.
 */
export function isCrashReportingEnabled(
	settings: ITelemetrySettings | undefined,
): boolean {
	return settings != null && settings.crashReports !== false;
}

export function isUsageTelemetryEnabled(
	settings: ITelemetrySettings | undefined,
): boolean {
	return settings?.enabled === true;
}

export function onTelemetrySettingsChange(
	listener: (settings: ITelemetrySettings) => void,
): () => void {
	const handler = (event: Event) =>
		listener((event as CustomEvent<ITelemetrySettings>).detail);
	window.addEventListener(SETTINGS_EVENT, handler);
	return () => window.removeEventListener(SETTINGS_EVENT, handler);
}

"use client";
import type { IProfile } from "../../../lib/schema/profile/profile";
import { DashboardChainWidget, DashboardErrorWidget } from "./logs";
import { DashboardResourcesWidget } from "./resources/resources-dashboard-widget";
import { DashboardTelemetryAlertsWidget } from "./telemetry/alerts-dashboard-widget";
import { DashboardTelemetryWidget } from "./telemetry/dashboard-telemetry-widget";
import { DashboardTelemetryIssuesWidget } from "./telemetry/issues-dashboard-widget";
import { DashboardTelemetryTracesWidget } from "./telemetry/traces-dashboard-widget";
export default function AdminDashboardSystem({
	profile,
	admin,
	logs,
	telemetry,
}: {
	profile: IProfile | undefined;
	admin: boolean;
	logs: boolean;
	telemetry: boolean;
}) {
	return (
		<div className="space-y-5">
			{admin && <DashboardResourcesWidget profile={profile} />}
			{logs && (
				<div className="grid gap-5 lg:grid-cols-2">
					<DashboardErrorWidget profile={profile} />
					<DashboardChainWidget profile={profile} />
				</div>
			)}
			{telemetry && (
				<>
					<DashboardTelemetryAlertsWidget profile={profile} />
					<DashboardTelemetryIssuesWidget profile={profile} />
					<DashboardTelemetryWidget profile={profile} />
					<DashboardTelemetryTracesWidget profile={profile} />
				</>
			)}
		</div>
	);
}

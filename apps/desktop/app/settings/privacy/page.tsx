"use client";

import { useTranslation } from "@flow-like/locales";
import { PrivacySettingsPage, useFeatures } from "@flow-like/flow-like-ui";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import {
	type ITelemetrySettings,
	getTelemetrySettings,
	isCrashReportingEnabled,
	onTelemetrySettingsChange,
	setCrashReportsEnabled,
	setTelemetryEnabled,
} from "../../../lib/telemetry-settings";

export default function PrivacyPage() {
	const { t } = useTranslation("common");
	const features = useFeatures();
	const [settings, setSettings] = useState<ITelemetrySettings | undefined>();

	useEffect(() => {
		getTelemetrySettings()
			.then(setSettings)
			.catch((error) =>
				console.warn("Failed to load telemetry settings:", error),
			);
		return onTelemetrySettingsChange(setSettings);
	}, []);

	const handleChange = useCallback(async (enabled: boolean) => {
		try {
			await setTelemetryEnabled(enabled);
		} catch (error) {
			console.warn("Failed to update telemetry settings:", error);
			toast.error("Failed to update telemetry settings");
		}
	}, []);

	const handleCrashReportsChange = useCallback(async (enabled: boolean) => {
		try {
			await setCrashReportsEnabled(enabled);
		} catch (error) {
			console.warn("Failed to update crash report settings:", error);
			toast.error("Failed to update crash report settings");
		}
	}, []);

	return (
		<div className="h-full min-h-0 overflow-auto">
			<div className="container mx-auto flex max-w-5xl flex-col gap-6 px-2 pb-4">
				<div className="flex flex-col gap-1 pt-2">
					<h1 className="text-3xl font-bold tracking-tight">
						{t('privacyAmpTelemetry', "Privacy & Telemetry")}
					</h1>
					<p className="text-muted-foreground">
						{t('decideWhatAnonymousDiagnosticsAndUsageDataMayBeShared', 'Decide what anonymous diagnostics and usage data may be shared')}
					</p>
				</div>
				<PrivacySettingsPage
					available={features.data?.telemetry === true}
					enabled={settings?.enabled ?? undefined}
					onChange={handleChange}
					crashReportsEnabled={isCrashReportingEnabled(settings)}
					onCrashReportsChange={handleCrashReportsChange}
				/>
			</div>
		</div>
	);
}

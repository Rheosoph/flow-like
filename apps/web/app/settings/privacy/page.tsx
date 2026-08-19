"use client";

import {
	type TelemetryConsent,
	getCrashReportsEnabled,
	getTelemetryConsent,
	onTelemetryConsentChange,
	setCrashReportsEnabled,
	setTelemetryConsent,
} from "@/lib/telemetry-consent";
import { PrivacySettingsPage, useFeatures } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useState } from "react";

export default function PrivacyPage() {
	const { t } = useTranslation("common");
	const features = useFeatures();
	const [consent, setConsent] = useState<TelemetryConsent>(undefined);
	const [crashReports, setCrashReports] = useState(false);

	const sync = useCallback(() => {
		setConsent(getTelemetryConsent());
		setCrashReports(getCrashReportsEnabled());
	}, []);

	useEffect(() => {
		sync();
		return onTelemetryConsentChange(sync);
	}, [sync]);

	return (
		<div className="h-full min-h-0 overflow-auto">
			<div className="container mx-auto flex max-w-5xl flex-col gap-6 px-2 pb-4">
				<div className="flex flex-col gap-1 pt-2">
					<h1 className="text-3xl font-bold tracking-tight">
						{t("privacyAmpTelemetry", "Privacy & Telemetry")}
					</h1>
					<p className="text-muted-foreground">
						{t(
							"decideWhatAnonymousDiagnosticsAndUsageDataMayBeShared",
							"Decide what anonymous diagnostics and usage data may be shared",
						)}
					</p>
				</div>
				<PrivacySettingsPage
					available={features.data?.telemetry === true}
					enabled={consent === undefined ? undefined : consent === "granted"}
					onChange={setTelemetryConsent}
					crashReportsEnabled={crashReports}
					onCrashReportsChange={setCrashReportsEnabled}
				/>
			</div>
		</div>
	);
}

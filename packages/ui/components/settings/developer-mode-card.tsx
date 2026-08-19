"use client";

import { useTranslation } from "@flow-like/locales";
import { Code2 } from "lucide-react";
import { useDeveloperMode } from "../../hooks/use-developer-mode";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";

/**
 * Toggle for the per-user developer mode. Off, the UI hides builder surfaces
 * (Developer Tools, Flows, Events, Data Studio, package registries, …) so new
 * users only see what they need to use apps; on, everything is visible.
 */
export function DeveloperModeCard() {
	const { t } = useTranslation("settings");
	const { developerMode, setDeveloperMode } = useDeveloperMode();

	return (
		<Card id="developer-mode-card">
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<Code2 className="h-5 w-5" />
					{t("developerMode", "Developer Mode")}
				</CardTitle>
				<CardDescription>
					{t(
						"showAdvancedBuildingToolsAcrossTheApp",
						"Show advanced building tools across the app",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<div className="flex items-center justify-between">
					<div className="space-y-0.5">
						<Label htmlFor="developer-mode">
							{t("enableDeveloperMode", "Enable developer mode")}
						</Label>
						<p className="text-sm text-muted-foreground">
							{t(
								"unhidesFlowsEventsDataToolingAndPackageRegistriesSyncedAcrossYourDevices",
								"Unhides flows, events, data tooling, and package registries. Synced across your devices.",
							)}
						</p>
					</div>
					<Switch
						id="developer-mode"
						checked={developerMode}
						onCheckedChange={setDeveloperMode}
					/>
				</div>
			</CardContent>
		</Card>
	);
}

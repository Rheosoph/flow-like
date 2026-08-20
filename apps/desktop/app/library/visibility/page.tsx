"use client";

import {
	AppCard,
	Button,
	EmptyState,
	type IApp,
	type IMetadata,
	Separator,
	useBackend,
	useInvoke,
	useSpotlightStore,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { EyeIcon, FilesIcon, LayoutGridIcon, Sparkles } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

export default function Page() {
	const { t } = useTranslation("common");
	const router = useRouter();
	const backend = useBackend();
	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const [selectedApps, setSelectedApps] = useState<Set<string>>(new Set());
	const allItems = useMemo(() => {
		const map = new Map<string, IMetadata & { id: string; app: IApp }>();
		apps.data?.forEach(([app, meta]) => {
			if (meta) map.set(app.id, { ...meta, id: app.id, app });
		});
		return Array.from(map.values());
	}, [apps.data, currentProfile.data]);

	useEffect(() => {
		const selected = new Set<string>();

		currentProfile.data?.hub_profile.apps?.forEach((app) => {
			selected.add(app.app_id);
		});

		setSelectedApps(selected);
	}, [currentProfile.data]);

	return (
		<main className="flex flex-col w-full p-6 bg-gradient-to-br from-background to-muted/20 flex-1 min-h-0">
			<div className="mb-4 flex items-center justify-between">
				<div className="flex items-center space-x-3">
					<div className="p-2 rounded-xl bg-primary/10 text-primary">
						<EyeIcon className="h-8 w-8" />
					</div>
					<div>
						<h1 className="text-4xl font-bold tracking-tight bg-gradient-to-r from-foreground to-foreground/70 bg-clip-text">
							{t("manageAppVisibility", "Manage App Visibility")}
						</h1>
						<p className="text-muted-foreground mt-1">
							{t(
								"manageWhichAppsYouWantToSeeInThisProfile",
								"Manage which Apps you want to see in this Profile!",
							)}
						</p>
					</div>
				</div>
				<Button
					onClick={async () => {
						if (!currentProfile.data) return;
						const appsToRemove: string[] = [];
						const appsToAdd: string[] = [];
						const oldApps = new Set();

						currentProfile.data?.hub_profile.apps?.forEach((app) => {
							if (!selectedApps.has(app.app_id)) {
								appsToRemove.push(app.app_id);
							} else {
								oldApps.add(app.app_id);
							}
						});

						selectedApps.forEach((app_id) => {
							if (!oldApps.has(app_id)) {
								appsToAdd.push(app_id);
							}
						});

						for await (const app of Array.from(appsToAdd)) {
							await backend.userState.updateProfileApp(
								currentProfile.data,
								{
									app_id: app,
									favorite: false,
									pinned: false,
								},
								"Upsert",
							);
						}

						for await (const app of Array.from(appsToRemove)) {
							await backend.userState.updateProfileApp(
								currentProfile.data,
								{
									app_id: app,
									favorite: false,
									pinned: false,
								},
								"Remove",
							);
						}

						toast.success("Profile updated successfully!");
						await currentProfile.refetch();
						await apps.refetch();
					}}
				>
					{t("save", "Save")}
				</Button>
			</div>
			<Separator className="mb-4 mt-8" />
			<div className="flex-1 overflow-auto">
				{allItems.length === 0 && (
					<EmptyState
						action={[
							{
								label: "Create Your First App",
								onClick: () => {
									useSpotlightStore.getState().open();
									useSpotlightStore.getState().setMode("quick-create");
								},
							},
						]}
						icons={[Sparkles, LayoutGridIcon, FilesIcon]}
						className="min-w-full min-h-full flex-grow h-full border-2 border-dashed border-border/50 rounded-xl bg-muted/20"
						title={t("welcomeToYourLibrary", "Welcome to Your Library")}
						description={t(
							"createPowerfulCustomApplicationsBasedOnYourDataGetStartedWithYourFirstAppTodayItsFreeAndSecure",
							"Create powerful custom applications based on your data. Get started with your first app today - it's free and secure.",
						)}
					/>
				)}

				<div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-2 px-2 mt-5">
					{allItems.map((meta) => (
						<div key={`left${meta.id}`} className="group">
							<AppCard
								isOwned
								app={meta.app}
								metadata={meta as IMetadata}
								variant="small"
								onClick={() => {
									if (selectedApps.has(meta.app.id)) {
										selectedApps.delete(meta.app.id);
									} else {
										selectedApps.add(meta.app.id);
									}
									setSelectedApps(new Set(selectedApps));
								}}
								multiSelected={selectedApps.has(meta.app.id)}
								className="w-full"
							/>
						</div>
					))}
				</div>
			</div>
		</main>
	);
}

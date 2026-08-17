"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	CrashReportDialog,
	type ProjectQuickLink,
	type SpotlightItem,
	SpotlightProvider,
	handleUpgradeRequiredError,
	nowSystemTime,
	useBackend,
	useFeatures,
	useInvalidateInvoke,
	useInvoke,
	useSpotlightStore,
} from "@flow-like/flow-like-ui";
import { useLiveQuery } from "dexie-react-hooks";
import {
	Bookmark,
	BookmarkMinus,
	BookmarkPlus,
	ExternalLink,
} from "lucide-react";
import { useTheme } from "next-themes";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { type IShortcut, appsDB } from "../lib/apps-db";
import { currentRelativeUrl } from "../lib/return-url";
import {
	getCrashReportsEnabled,
	onTelemetryConsentChange,
} from "../lib/telemetry-consent";

interface SpotlightWrapperProps {
	children: React.ReactNode;
}

export function SpotlightWrapper({ children }: SpotlightWrapperProps) {
	const { t } = useTranslation("common");
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const { setTheme } = useTheme();
	const auth = useAuth();
	const backend = useBackend();
	const features = useFeatures();
	const invalidate = useInvalidateInvoke();
	const [crashReportOpen, setCrashReportOpen] = useState(false);
	const [crashReportsAllowed, setCrashReportsAllowed] = useState(false);

	useEffect(() => {
		const sync = () => setCrashReportsAllowed(getCrashReportsEnabled());
		sync();
		return onTelemetryConsentChange(sync);
	}, []);

	const handleReportBug = useCallback(() => {
		setCrashReportOpen(true);
	}, []);

	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const appMetadata = useInvoke(backend.appState.getApps, backend.appState, []);

	const openBoards = useInvoke(
		backend.boardState.getOpenBoards,
		backend.boardState,
		[],
	);

	const shortcuts = useLiveQuery(async () => {
		if (!currentProfile.data?.hub_profile.id) return [];
		return await appsDB.shortcuts
			.where("profileId")
			.equals(currentProfile.data.hub_profile.id)
			.sortBy("order");
	}, [currentProfile.data?.hub_profile.id]);

	const isCurrentPageShortcut = useMemo(() => {
		if (!shortcuts) return false;
		const fullPath = searchParams.toString()
			? `${pathname}?${searchParams.toString()}`
			: pathname;
		return shortcuts.some((s) => s.path === fullPath || s.path === pathname);
	}, [shortcuts, pathname, searchParams]);

	const syncProfileShortcuts = useCallback(async () => {
		if (!currentProfile.data?.hub_profile.id) return;

		try {
			const nextShortcuts = await appsDB.shortcuts
				.where("profileId")
				.equals(currentProfile.data.hub_profile.id)
				.sortBy("order");
			await backend.userState.updateProfileShortcuts(
				currentProfile.data,
				nextShortcuts,
			);
		} catch (error) {
			console.warn("Failed to sync shortcuts:", error);
		}
	}, [backend.userState, currentProfile.data]);

	const projects = useMemo<ProjectQuickLink[]>(() => {
		if (!appMetadata.data || !currentProfile.data) return [];

		const profileAppIds = new Set(
			(currentProfile.data.hub_profile.apps ?? []).map((a) => a.app_id),
		);

		return appMetadata.data
			.filter(([app]) => profileAppIds.has(app.id))
			.map(([app, meta]) => ({
				id: app.id,
				name: meta?.name || "Unnamed Project",
				icon: meta?.icon || meta?.preview_media?.[0]?.toString(),
				links: {
					flows: `/library/config/flows?id=${app.id}`,
					storage: `/library/config/storage?id=${app.id}`,
					events: `/library/config/events?id=${app.id}`,
					explore: `/library/config/explore?id=${app.id}`,
					settings: `/library/config?id=${app.id}`,
				},
			}));
	}, [appMetadata.data, currentProfile.data]);

	const openBoardItems = useMemo<SpotlightItem[]>(() => {
		if (!openBoards.data) return [];

		return openBoards.data.map(([appId, boardId, boardName]) => ({
			id: `open-board-${boardId}`,
			type: "dynamic" as const,
			label: boardName,
			description: t('openFlowBoard', 'Open flow board'),
			group: "open-flows",
			keywords: ["flow", "board", boardName.toLowerCase()],
			priority: 180,
			action: () => router.push(`/flow?id=${boardId}&app=${appId}`),
		}));
	}, [openBoards.data, router]);

	const handleNavigate = useCallback(
		(path: string) => {
			router.push(path);
		},
		[router],
	);

	const handleCreateProject = useCallback(() => {
		useSpotlightStore.getState().open();
		useSpotlightStore.getState().setMode("quick-create");
	}, []);

	const handleToggleTheme = useCallback(
		(theme: "light" | "dark" | "system") => {
			setTheme(theme);
			toast.success(`Theme set to ${theme}`);
		},
		[setTheme],
	);

	const handleOpenDocs = useCallback(() => {
		window.open("https://docs.flow-like.com", "_blank");
	}, []);

	const handleFlowPilotMessage = useCallback(
		async (message: string): Promise<string> => {
			const responses: Record<string, string> = {
				'how do i create a flow?':
					t('toCreateAFlowGoToLibraryNewProjectGiveItANameAndChooseOnlineModeYoullBeTakenDirectlyToTheFlowEditorWhereYouCanStartAddingNodes', 'To create a flow, go to Library > New Project, give it a name, and choose Online mode. You\'ll be taken directly to the flow editor where you can start adding nodes!'),
				'what are nodes?':
					t('nodesAreTheBuildingBlocksOfYourWorkflowsEachNodePerformsASpecificActionLikeFetchingDataProcessingTextOrCallingAiModelsConnectThemTogetherToCreatePowerfulAutomations', 'Nodes are the building blocks of your workflows. Each node performs a specific action - like fetching data, processing text, or calling AI models. Connect them together to create powerful automations!'),
				'help with storage':
					`Storage in Flow-Like lets you persist data between flow runs. You can store files, JSON data, and more. Access it from your project's Storage tab.`,
			};

			const lowerMessage = message.toLowerCase();
			for (const [key, response] of Object.entries(responses)) {
				if (lowerMessage.includes(key.split(" ").slice(0, 3).join(" "))) {
					return response;
				}
			}

			return t('thanksForYourQuestionAboutMessageFlowlikeIsAVisualWorkflowAutomationToolYouCanCreateFlowsWithDraganddropNodesConnectToAiModelsForIntelligentAutomationStoreAndProcessDataDeployOnlineForDetailedDocsVisitDocsflowlikecom', "Thanks for your question about \"{{message}}\"! Flow-Like is a visual workflow automation tool. You can: • Create flows with drag-and-drop nodes • Connect to AI models for intelligent automation • Store and process data • Deploy online For detailed docs, visit docs.flow-like.com", { message });
		},
		[],
	);

	const handleAddShortcut = useCallback(async () => {
		if (!currentProfile.data?.hub_profile.id) {
			toast.error("No profile selected");
			return;
		}

		const existingShortcuts = await appsDB.shortcuts
			.where("profileId")
			.equals(currentProfile.data.hub_profile.id)
			.toArray();

		const fullPath = searchParams.toString()
			? `${pathname}?${searchParams.toString()}`
			: pathname;

		const pageTitle =
			document.title.replace(i18next.t('flowlike', "| Flow-Like"), "").trim() || "Current Page";

		const appId =
			searchParams.get("app") ||
			(pathname === "/flow" ? null : searchParams.get("id"));

		let icon: string | undefined;
		if (appId && appMetadata.data) {
			const appData = appMetadata.data.find(([app]) => app.id === appId);
			icon = appData?.[1]?.icon || appData?.[1]?.preview_media?.[0]?.toString();
		}

		const newShortcut: IShortcut = {
			id: crypto.randomUUID(),
			profileId: currentProfile.data.hub_profile.id,
			label: pageTitle,
			path: fullPath,
			appId: appId || undefined,
			icon,
			order: existingShortcuts.length,
			createdAt: new Date().toISOString(),
		};

		const existingShortcut = existingShortcuts.find(
			(shortcut) => shortcut.path === fullPath,
		);
		if (existingShortcut) {
			await appsDB.shortcuts.update(existingShortcut.id, {
				label: pageTitle,
				appId: appId || undefined,
				icon,
			});
		} else {
			await appsDB.shortcuts.add(newShortcut);
		}
		await syncProfileShortcuts();
		toast.success("Page added to shortcuts");
	}, [
		currentProfile.data?.hub_profile.id,
		currentProfile.data,
		pathname,
		searchParams,
		appMetadata.data,
		syncProfileShortcuts,
		backend.userState,
	]);

	const handleRemoveShortcut = useCallback(async () => {
		if (!shortcuts) return;

		const fullPath = searchParams.toString()
			? `${pathname}?${searchParams.toString()}`
			: pathname;
		const shortcut = shortcuts.find(
			(s) => s.path === fullPath || s.path === pathname,
		);
		if (shortcut) {
			await appsDB.shortcuts.delete(shortcut.id);
			await syncProfileShortcuts();
			toast.success("Page removed from shortcuts");
		}
	}, [shortcuts, pathname, searchParams, syncProfileShortcuts]);

	const additionalItems = useMemo<SpotlightItem[]>(() => {
		const items: SpotlightItem[] = [...openBoardItems];

		if (isCurrentPageShortcut) {
			items.push({
				id: "action-remove-shortcut",
				type: "action",
				label: `Remove from Shortcuts`,
				description: `Remove this page from your quick access shortcuts`,
				icon: BookmarkMinus,
				group: "shortcuts",
				keywords: ["shortcut", "remove", "bookmark", "unpin"],
				priority: 250,
				action: handleRemoveShortcut,
			});
		} else {
			items.push({
				id: "action-add-shortcut",
				type: "action",
				label: i18next.t('addToShortcuts', 'Add to Shortcuts'),
				description: i18next.t('addThisPageToYourQuickAccessShortcuts', 'Add this page to your quick access shortcuts'),
				icon: BookmarkPlus,
				group: "shortcuts",
				keywords: ["shortcut", "add", "bookmark", "pin", "save"],
				priority: 250,
				action: handleAddShortcut,
			});
		}

		if (shortcuts && shortcuts.length > 0) {
			for (const shortcut of shortcuts.slice(0, 5)) {
				let iconUrl = shortcut.icon;
				if (!iconUrl && shortcut.appId && appMetadata.data) {
					const appData = appMetadata.data.find(
						([app]) => app.id === shortcut.appId,
					);
					iconUrl =
						appData?.[1]?.icon || appData?.[1]?.preview_media?.[0]?.toString();
				}

				items.push({
					id: `shortcut-${shortcut.id}`,
					type: "shortcut" as const,
					label: shortcut.label,
					description: shortcut.path,
					icon: Bookmark,
					iconUrl,
					group: "shortcuts",
					keywords: ["shortcut", "bookmark", shortcut.label.toLowerCase()],
					priority: 200,
					action: () => router.push(shortcut.path),
				});
			}
		}

		if (auth.isAuthenticated) {
			items.push({
				id: "action-logout",
				type: "action",
				label: i18next.t('signOut', 'Sign Out'),
				description: i18next.t('signOutOfYourAccount', 'Sign out of your account'),
				group: "account",
				keywords: ["logout", i18next.t('signOut2', 'sign out'), "account", "exit"],
				priority: 30,
				action: () => auth.signoutRedirect(),
			});

			items.push({
				id: "nav-account",
				type: "navigation",
				label: i18next.t('accountSettings', 'Account Settings'),
				description: i18next.t('manageYourAccountSettings', 'Manage your account settings'),
				group: "navigation",
				keywords: ["account", "profile", "user", "settings"],
				priority: 70,
				action: () => router.push("/account"),
			});

			items.push({
				id: "nav-notifications",
				type: "navigation",
				label: i18next.t('notifications', 'Notifications'),
				description: i18next.t('viewYourNotifications', 'View your notifications'),
				group: "navigation",
				keywords: ["notifications", "alerts", "messages", "invites"],
				priority: 65,
				action: () => router.push("/notifications"),
			});
		} else {
			items.push({
				id: "action-login",
				type: "action",
				label: i18next.t('signIn', 'Sign In'),
				description: i18next.t('signInToYourAccount', 'Sign in to your account'),
				group: "account",
				keywords: ["login", i18next.t('signIn2', 'sign in'), "account", "authenticate"],
				priority: 40,
				action: () => auth.signinRedirect({ url_state: currentRelativeUrl() }),
			});
		}

		items.push({
			id: "nav-profile-settings",
			type: "navigation",
			label: i18next.t('profileSettings', 'Profile Settings'),
			description: i18next.t('editYourProfileConfiguration', 'Edit your profile configuration'),
			group: "navigation",
			keywords: ["profile", "settings", "configuration", "preferences"],
			priority: 60,
			action: () => router.push("/settings/profiles"),
		});

		// FlowPilot Documentation items
		items.push({
			id: "flowpilot-docs",
			type: "action" as const,
			label: i18next.t('flowpilotDocumentation', 'FlowPilot Documentation'),
			description: i18next.t('learnHowToUseFlowpilotAiAssistant', 'Learn how to use FlowPilot AI assistant'),
			icon: ExternalLink,
			group: "flowpilot",
			keywords: [
				"flowpilot",
				"ai",
				"assistant",
				"help",
				"docs",
				"documentation",
				"copilot",
			],
			priority: 140,
			action: () => {
				window.open("https://docs.flow-like.com/guides/flowpilot", "_blank");
			},
		});

		items.push({
			id: "docs-quick-start",
			type: "action" as const,
			label: i18next.t('quickStartGuide', 'Quick Start Guide'),
			description: i18next.t('getStartedWithFlowlike', 'Get started with Flow-Like'),
			icon: ExternalLink,
			group: "flowpilot",
			keywords: ["docs", "quick start", "guide", "tutorial", "begin"],
			priority: 130,
			action: () => {
				window.open("https://docs.flow-like.com/start/quickstart", "_blank");
			},
		});

		items.push({
			id: "docs-concepts",
			type: "action" as const,
			label: i18next.t('coreConcepts', 'Core Concepts'),
			description: i18next.t('learnAboutFlowsNodesAndMore', 'Learn about flows, nodes, and more'),
			icon: ExternalLink,
			group: "flowpilot",
			keywords: ["docs", "concepts", "flows", "nodes", "learn"],
			priority: 125,
			action: () => {
				window.open("https://docs.flow-like.com/concepts", "_blank");
			},
		});

		return items;
	}, [
		openBoardItems,
		auth,
		router,
		isCurrentPageShortcut,
		handleAddShortcut,
		handleRemoveShortcut,
		shortcuts,
		appMetadata.data,
	]);

	const handleQuickCreateProject = useCallback(
		async (
			name: string,
			isOffline: boolean,
		): Promise<{ appId: string; boardId: string } | null> => {
			try {
				const meta = {
					name,
					description: i18next.t('quickcreatedProjectName', 'Quick-created project: {{name}}', { name }),
					tags: [],
					use_case: "",
					created_at: nowSystemTime(),
					updated_at: nowSystemTime(),
					preview_media: [],
				};

				// Web apps are always online
				const app = await backend.appState.createApp(meta, [], true, undefined);

				if (currentProfile.data) {
					await backend.userState.updateProfileApp(
						currentProfile.data,
						{
							app_id: app.id,
							favorite: false,
							pinned: false,
						},
						"Upsert",
					);
				}

				const boards = await backend.boardState.getBoardSummaries(app.id);
				const boardId = boards?.[0]?.id || "";

				toast.success(`Project "${name}" created! 🎉`);

				if (boardId) {
					router.push(`/flow?id=${boardId}&app=${app.id}`);
				}

				return { appId: app.id, boardId };
			} catch (error) {
				console.error("Failed to create project:", error);
				if (handleUpgradeRequiredError(error, "project-limit")) {
					useSpotlightStore.getState().close();
				} else {
					toast.error(
						error instanceof Error ? error.message : i18next.t('failedToCreateProject', 'Failed to create project'),
					);
				}
				return null;
			}
		},
		[backend, currentProfile.data, router],
	);

	return (
		<SpotlightProvider
			navigate={handleNavigate}
			projects={projects}
			onCreateProject={handleCreateProject}
			onToggleTheme={handleToggleTheme}
			onOpenDocs={handleOpenDocs}
			onReportBug={handleReportBug}
			additionalStaticItems={additionalItems}
			onFlowPilotMessage={handleFlowPilotMessage}
			onQuickCreateProject={handleQuickCreateProject}
		>
			{children}
			<CrashReportDialog
				open={crashReportOpen}
				onOpenChange={setCrashReportOpen}
				reportingEnabled={
					features.data?.telemetry === true && crashReportsAllowed
				}
			/>
		</SpotlightProvider>
	);
}

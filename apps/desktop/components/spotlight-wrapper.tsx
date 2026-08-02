"use client";

import {
	CrashReportDialog,
	IBitTypes,
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
import type { ISettingsProfile } from "@flow-like/flow-like-ui/types";
import { invoke } from "@tauri-apps/api/core";
import { useLiveQuery } from "dexie-react-hooks";
import {
	Bookmark,
	BookmarkMinus,
	BookmarkPlus,
	Bot,
	ExternalLink,
	Users,
} from "lucide-react";
import { useTheme } from "next-themes";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { type IShortcut, appsDB } from "../lib/apps-db";
import {
	type ITelemetrySettings,
	getTelemetrySettings,
	isCrashReportingEnabled,
	onTelemetrySettingsChange,
} from "../lib/telemetry-settings";
import { useTauriInvoke } from "./useInvoke";

interface SpotlightWrapperProps {
	children: React.ReactNode;
}

export function SpotlightWrapper({ children }: SpotlightWrapperProps) {
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const { setTheme } = useTheme();
	const auth = useAuth();
	const backend = useBackend();
	const features = useFeatures();
	const invalidate = useInvalidateInvoke();
	const [crashReportOpen, setCrashReportOpen] = useState(false);
	const [telemetrySettings, setTelemetrySettings] = useState<
		ITelemetrySettings | undefined
	>();

	useEffect(() => {
		getTelemetrySettings()
			.then(setTelemetrySettings)
			.catch(() => undefined);
		return onTelemetrySettingsChange(setTelemetrySettings);
	}, []);

	const crashReportingEnabled =
		features.data?.telemetry === true &&
		isCrashReportingEnabled(telemetrySettings);

	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const profiles = useTauriInvoke<Record<string, ISettingsProfile>>(
		"get_profiles",
		{},
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
			description: "Open flow board",
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
			// For now, return a helpful response about Flow-Like
			// This can be connected to a real AI backend later
			const responses: Record<string, string> = {
				"how do i create a flow?":
					"To create a flow, go to Library > New Project, give it a name, and choose Online/Offline mode. You'll be taken directly to the flow editor where you can start adding nodes!",
				"what are nodes?":
					"Nodes are the building blocks of your workflows. Each node performs a specific action - like fetching data, processing text, or calling AI models. Connect them together to create powerful automations!",
				"help with storage":
					"Storage in Flow-Like lets you persist data between flow runs. You can store files, JSON data, and more. Access it from your project's Storage tab.",
			};

			const lowerMessage = message.toLowerCase();
			for (const [key, response] of Object.entries(responses)) {
				if (lowerMessage.includes(key.split(" ").slice(0, 3).join(" "))) {
					return response;
				}
			}

			return `Thanks for your question about "${message}"! Flow-Like is a visual workflow automation tool. You can:\n\n• Create flows with drag-and-drop nodes\n• Connect to AI models for intelligent automation\n• Store and process data\n• Deploy online or keep offline\n\nFor detailed docs, visit docs.flow-like.com`;
		},
		[],
	);

	const handleProfileChange = useCallback(
		async (profileId: string) => {
			try {
				await invoke("set_current_profile", { profileId });
				await Promise.allSettled([
					invalidate(backend.userState.getProfile, []),
					invalidate(backend.userState.getSettingsProfile, []),
					invalidate(backend.appState.getApps, []),
					invalidate(backend.bitState.searchBits, [
						{
							bit_types: [
								IBitTypes.Llm,
								IBitTypes.Vlm,
								IBitTypes.Tts,
								IBitTypes.Stt,
								IBitTypes.Embedding,
								IBitTypes.ImageEmbedding,
							],
						},
					]),
					invalidate(backend.bitState.searchBits, [
						{
							bit_types: [IBitTypes.Template],
						},
					]),
					profiles.refetch(),
					currentProfile.refetch(),
				]);
				toast.success("Profile switched successfully");
			} catch (error) {
				console.error("Failed to switch profile:", error);
				toast.error("Failed to switch profile");
			}
		},
		[invalidate, backend, profiles, currentProfile],
	);

	const handleReportBug = useCallback(() => {
		setCrashReportOpen(true);
	}, []);

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
			document.title.replace(" | Flow-Like", "").trim() || "Current Page";

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
				label: "Remove from Shortcuts",
				description: "Remove this page from your quick access shortcuts",
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
				label: "Add to Shortcuts",
				description: "Add this page to your quick access shortcuts",
				icon: BookmarkPlus,
				group: "shortcuts",
				keywords: ["shortcut", "add", "bookmark", "pin", "save"],
				priority: 250,
				action: handleAddShortcut,
			});
		}

		if (shortcuts && shortcuts.length > 0) {
			for (const shortcut of shortcuts.slice(0, 5)) {
				// Get icon from shortcut or from app metadata
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
				label: "Sign Out",
				description: "Sign out of your account",
				group: "account",
				keywords: ["logout", "sign out", "account", "exit"],
				priority: 30,
				action: () => auth.signoutRedirect(),
			});

			items.push({
				id: "nav-account",
				type: "navigation",
				label: "Account Settings",
				description: "Manage your account settings",
				group: "navigation",
				keywords: ["account", "profile", "user", "settings"],
				priority: 70,
				action: () => router.push("/account"),
			});

			items.push({
				id: "nav-notifications",
				type: "navigation",
				label: "Notifications",
				description: "View your notifications",
				group: "navigation",
				keywords: ["notifications", "alerts", "messages", "invites"],
				priority: 65,
				action: () => router.push("/notifications"),
			});
		} else {
			items.push({
				id: "action-login",
				type: "action",
				label: "Sign In",
				description: "Sign in to your account",
				group: "account",
				keywords: ["login", "sign in", "account", "authenticate"],
				priority: 40,
				action: () => auth.signinRedirect(),
			});
		}

		items.push({
			id: "nav-profile-settings",
			type: "navigation",
			label: "Profile Settings",
			description: "Edit your profile configuration",
			group: "navigation",
			keywords: ["profile", "settings", "configuration", "preferences"],
			priority: 60,
			action: () => router.push("/settings/profiles"),
		});

		// Profile switching items
		const profileValues = profiles.data ? Object.values(profiles.data) : [];
		if (profileValues.length > 1) {
			for (const profile of profileValues) {
				const isCurrentProfile =
					profile.hub_profile.id === currentProfile.data?.hub_profile.id;
				if (isCurrentProfile) continue;

				items.push({
					id: `switch-profile-${profile.hub_profile.id}`,
					type: "action" as const,
					label: `Switch to ${profile.hub_profile.name}`,
					description:
						profile.hub_profile.hub?.replaceAll("https://", "") ||
						"Local profile",
					icon: Users,
					iconUrl: profile.hub_profile.icon ?? undefined,
					group: "profiles",
					keywords: [
						"profile",
						"switch",
						"change",
						profile.hub_profile.name?.toLowerCase() || "",
					],
					priority: 120,
					action: () => handleProfileChange(profile.hub_profile.id!),
				});
			}
		}

		// FlowPilot Documentation items
		items.push({
			id: "flowpilot-docs",
			type: "action" as const,
			label: "FlowPilot Documentation",
			description: "Learn how to use FlowPilot AI assistant",
			icon: Bot,
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
			subItems: [
				{
					id: "flowpilot-docs-intro",
					type: "action" as const,
					label: "Getting Started with FlowPilot",
					description: "Introduction to the AI assistant",
					icon: ExternalLink,
					group: "flowpilot",
					keywords: ["flowpilot", "intro", "getting started", "tutorial"],
					priority: 139,
					action: () => {
						window.open(
							"https://docs.flow-like.com/guides/flowpilot",
							"_blank",
						);
					},
				},
				{
					id: "flowpilot-docs-nodes",
					type: "action" as const,
					label: "Node Creation with AI",
					description: "Use FlowPilot to create flow nodes",
					icon: ExternalLink,
					group: "flowpilot",
					keywords: ["flowpilot", "nodes", "create", "ai", "generate"],
					priority: 138,
					action: () => {
						window.open("https://docs.flow-like.com/guides/nodes", "_blank");
					},
				},
				{
					id: "flowpilot-docs-workflows",
					type: "action" as const,
					label: "Building Workflows",
					description: "Create AI-powered automation workflows",
					icon: ExternalLink,
					group: "flowpilot",
					keywords: ["flowpilot", "workflows", "automation", "ai"],
					priority: 137,
					action: () => {
						window.open(
							"https://docs.flow-like.com/guides/workflows",
							"_blank",
						);
					},
				},
			],
		});

		// General docs quick links
		items.push({
			id: "docs-quick-start",
			type: "action" as const,
			label: "Quick Start Guide",
			description: "Get started with Flow-Like",
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
			label: "Core Concepts",
			description: "Learn about flows, nodes, and more",
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
		profiles.data,
		currentProfile.data,
		handleProfileChange,
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
					description: `Quick-created project: ${name}`,
					tags: [],
					use_case: "",
					created_at: nowSystemTime(),
					updated_at: nowSystemTime(),
					preview_media: [],
				};

				const app = await backend.appState.createApp(
					meta,
					[],
					!isOffline,
					undefined,
				);

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

				const boards = await backend.boardState.getBoards(app.id);
				const boardId = boards?.[0]?.[0] || "";

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
						error instanceof Error ? error.message : "Failed to create project",
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
				reportingEnabled={crashReportingEnabled}
			/>
		</SpotlightProvider>
	);
}

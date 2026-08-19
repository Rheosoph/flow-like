"use client";

import {
	Button,
	type IApp,
	IAppVisibility,
	LibraryPage,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ImportIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { appsDB } from "./../../lib/apps-db";
import { isMobileDevice as detectMobileDevice } from "./../../lib/platform";
import ImportEncryptedDialog from "./components/ImportEncryptedDialog";

export default function DesktopLibraryPage() {
	const { t } = useTranslation("common");
	const router = useRouter();
	const auth = useAuth();
	const [importDialogOpen, setImportDialogOpen] = useState(false);
	const [encryptedImportPath, setEncryptedImportPath] = useState<string | null>(
		null,
	);

	const isMobileDevice = useMemo(detectMobileDevice, []);

	const normalizePickerPath = useCallback((input: string): string => {
		if (!input.startsWith("file://")) return input;
		try {
			const url = new URL(input);
			let pathname = decodeURIComponent(url.pathname);
			if (/^[A-Za-z]:/.test(pathname.slice(1, 3))) {
				pathname = pathname.slice(1);
			}
			return pathname || input;
		} catch {
			const withoutScheme = input.replace(/^file:\/\//, "");
			return withoutScheme.startsWith("/")
				? withoutScheme
				: `/${withoutScheme}`;
		}
	}, []);

	const resolveSelectedPath = useCallback(
		(selected: unknown): string | null => {
			const resolve = (value: unknown): string | null => {
				if (!value) return null;
				if (typeof value === "string") return value;
				if (Array.isArray(value)) return resolve(value[0]);
				if (typeof value === "object") {
					const candidate = value as { path?: unknown; uri?: unknown };
					if (typeof candidate.path === "string") {
						return normalizePickerPath(candidate.path);
					}
					if (typeof candidate.uri === "string") {
						return normalizePickerPath(candidate.uri);
					}
				}
				return null;
			};

			return resolve(selected);
		},
		[normalizePickerPath],
	);

	const importApp = useCallback(async (path: string) => {
		if (path.toLowerCase().endsWith(".enc.flow-app")) {
			setEncryptedImportPath(path);
			setImportDialogOpen(true);
			return;
		}
		const toastId = toast.loading(t("importingApp", "Importing app..."), {
			description: t("pleaseWait", "Please wait."),
		});
		try {
			const app = await invoke<IApp>("import_app_from_file", { path });
			await appsDB.visibility.put({
				visibility: app.visibility ?? IAppVisibility.Offline,
				appId: app.id,
			});
			toast.success(
				t("appImportedSuccessfully", "App imported successfully!"),
				{ id: toastId },
			);
		} catch (err) {
			console.error(err);
			toast.error(`Failed to import app`, { id: toastId });
		}
	}, []);

	const pickImportFile = useCallback(async () => {
		type Filter = { name: string; extensions: string[] };
		const filtersOption: Filter[] | undefined = isMobileDevice
			? undefined
			: [{ name: t("flowApp", "Flow App"), extensions: ["flow-app"] }];

		const selection = await open({
			multiple: false,
			directory: false,
			...(filtersOption ? { filters: filtersOption } : {}),
		});
		const path = resolveSelectedPath(selection);
		if (!path) {
			toast.error("Unable to open selected file.");
			return;
		}
		await importApp(path);
	}, [importApp, isMobileDevice, resolveSelectedPath]);

	useEffect(() => {
		const unlistenPromise = listen<{ path: string }>(
			"import/file",
			async (event) => {
				const path = event.payload.path;
				if (!path) return;
				await importApp(path);
			},
		);

		return () => {
			unlistenPromise.then((unsub) => unsub()).catch(() => void 0);
		};
	}, [importApp]);

	// No cache invalidation here: the library is still mounted while the router
	// transitions, so wiping the cache made it refetch and reorder its own grid
	// under the pointer. The app route refetches what it needs on mount.
	const handleAppClick = useCallback(
		(appId: string) => {
			router.push(`/use?id=${appId}`);
		},
		[router],
	);

	const importButton = useMemo(
		() => (
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						className="h-8 w-8 rounded-full text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30"
						onClick={pickImportFile}
					>
						<ImportIcon className="h-4 w-4" />
					</Button>
				</TooltipTrigger>
				<TooltipContent>{t("importApp", "Import app")}</TooltipContent>
			</Tooltip>
		),
		[pickImportFile],
	);

	const mobileImportButton = useMemo(
		() => (
			<Button
				key="import"
				size="icon"
				variant="outline"
				onClick={pickImportFile}
			>
				<ImportIcon className="h-4 w-4" />
			</Button>
		),
		[pickImportFile],
	);

	return (
		<LibraryPage
			onAppClick={handleAppClick}
			extraToolbarActions={importButton}
			extraMobileActions={[mobileImportButton]}
			isAuthenticated={auth.isAuthenticated}
			renderExtras={({ refetchApps }) => (
				<ImportEncryptedDialog
					open={importDialogOpen}
					onOpenChange={(o) => {
						setImportDialogOpen(o);
						if (!o) setEncryptedImportPath(null);
					}}
					path={encryptedImportPath}
					onImported={refetchApps}
				/>
			)}
		/>
	);
}

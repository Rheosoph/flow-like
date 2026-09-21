"use client";

import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";

import {
	Button,
	LibraryPage,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ImportIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { isMobileDevice as detectMobileDevice } from "./../../lib/platform";
import ImportArchiveDialog from "./components/ImportArchiveDialog";

export default function DesktopLibraryPage() {
	const { t } = useTranslation("common");
	const router = useClientRouter();
	const auth = useAuth();
	const [importDialogOpen, setImportDialogOpen] = useState(false);
	const [importPath, setImportPath] = useState<string | null>(null);
	const importDialogOpenRef = useRef(false);

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

	const importApp = useCallback(
		(path: string) => {
			if (importDialogOpenRef.current) {
				toast.info(
					t("importAlreadyInProgress", "An import is already in progress."),
				);
				return;
			}
			importDialogOpenRef.current = true;
			setImportPath(path);
			setImportDialogOpen(true);
		},
		[t],
	);

	const handleImportDialogOpenChange = useCallback((next: boolean) => {
		importDialogOpenRef.current = next;
		setImportDialogOpen(next);
		if (!next) setImportPath(null);
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
		importApp(path);
	}, [importApp, isMobileDevice, resolveSelectedPath, t]);

	useEffect(() => {
		const unlistenPromise = listen<{ path: string }>("import/file", (event) => {
			const path = event.payload.path;
			if (!path) return;
			importApp(path);
		});

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
		[pickImportFile, t],
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
				<ImportArchiveDialog
					open={importDialogOpen}
					onOpenChange={handleImportDialogOpenChange}
					path={importPath}
					onImported={refetchApps}
				/>
			)}
		/>
	);
}

"use client";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
	Button,
	LibraryPage,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	useNetworkStatus,
	useQueryClient,
} from "@tm9657/flow-like-ui";
import { ImportIcon } from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import ImportEncryptedDialog from "./components/ImportEncryptedDialog";

export default function DesktopLibraryPage() {
	const isOnline = useNetworkStatus();
	const queryClient = useQueryClient();
	const router = useRouter();
	const [importDialogOpen, setImportDialogOpen] = useState(false);
	const [encryptedImportPath, setEncryptedImportPath] = useState<string | null>(
		null,
	);

	const isMobileDevice = useMemo(() => {
		if (typeof navigator === "undefined") return false;
		if (
			/Android|iPhone|iPad|iPod|Opera Mini|IEMobile|WPDesktop/i.test(
				navigator.userAgent,
			)
		)
			return true;
		const platform = navigator.platform?.toLowerCase() ?? "";
		const maxTouchPoints =
			(navigator as Navigator & { maxTouchPoints?: number }).maxTouchPoints ??
			0;
		return /mac/.test(platform) && maxTouchPoints > 1;
	}, []);

	const normalizePickerPath = (input: string): string => {
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
	};

	const resolveSelectedPath = (selected: unknown): string | null => {
		if (!selected) return null;
		if (typeof selected === "string") return selected;
		if (Array.isArray(selected)) return resolveSelectedPath(selected[0]);
		if (typeof selected === "object") {
			const candidate = selected as { path?: unknown; uri?: unknown };
			if (typeof candidate.path === "string")
				return normalizePickerPath(candidate.path);
			if (typeof candidate.uri === "string")
				return normalizePickerPath(candidate.uri);
		}
		return null;
	};

	const importApp = useCallback(async (path: string) => {
		if (path.toLowerCase().endsWith(".enc.flow-app")) {
			setEncryptedImportPath(path);
			setImportDialogOpen(true);
			return;
		}
		const toastId = toast.loading("Importing app...", {
			description: "Please wait.",
		});
		try {
			await invoke("import_app_from_file", { path });
			toast.success("App imported successfully!", { id: toastId });
		} catch (err) {
			console.error(err);
			toast.error("Failed to import app", { id: toastId });
		}
	}, []);

	const pickImportFile = useCallback(async () => {
		type Filter = { name: string; extensions: string[] };
		const filtersOption: Filter[] | undefined = isMobileDevice
			? undefined
			: [{ name: "Flow App", extensions: ["flow-app"] }];

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
	}, [importApp, isMobileDevice]);

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

	const handleAppClick = useCallback(
		(appId: string) => {
			if (isOnline) queryClient.invalidateQueries();
			router.push(`/use?id=${appId}`);
		},
		[isOnline, queryClient, router],
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
				<TooltipContent>Import app</TooltipContent>
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

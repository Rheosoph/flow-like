"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Loader2Icon,
	PaletteIcon,
	RotateCcwIcon,
	SaveIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import {
	type AppearanceMode,
	type AppearanceState,
	DEFAULT_APPEARANCE_STATE,
	buildAppearanceBlock,
	cloneAppearanceState,
	composeAppearanceSheet,
	parseAppearanceSheet,
} from "../../../lib/appearance/appearance-theme";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { MonacoCodeEditor } from "../../ui/monaco-code-editor";
import { Skeleton } from "../../ui/skeleton";
import {
	AppearancePreview,
	type AppearancePreviewView,
} from "./appearance-preview";
import { AppearanceRail } from "./appearance-rail";

type Panel = "controls" | "preview" | "css";

const WIDTHS = [
	{ id: "desktop", px: 960 },
	{ id: "tablet", px: 640 },
	{ id: "phone", px: 390 },
] as const;

/**
 * The Appearance editor: controls on the left, the surfaces the sheet lands on in the
 * middle, the sheet itself on the right.
 *
 * Saving is explicit, never debounced — `App::save` re-serializes and re-writes every board
 * in the process registry before it writes the manifest, so an autosave on a typing pause
 * would rewrite board files on every keystroke lull.
 */
export function AppearanceStudio({
	appId,
	className,
}: Readonly<{ appId: string; className?: string }>) {
	const { t } = useTranslation("flow");
	const backend = useBackend();
	const { resolvedTheme } = useTheme();

	const [state, setState] = useState<AppearanceState>(() =>
		cloneAppearanceState(DEFAULT_APPEARANCE_STATE),
	);
	const [tail, setTail] = useState("");
	const [sheet, setSheet] = useState("");
	const [saved, setSaved] = useState("");
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const [panel, setPanel] = useState<Panel>("preview");
	const [view, setView] = useState<AppearancePreviewView>("dashboard");
	const [mode, setMode] = useState<AppearanceMode>("dark");
	const [width, setWidth] = useState<number>(WIDTHS[0].px);
	const [replayKey, setReplayKey] = useState(0);

	const dirty = sheet !== saved;
	const dirtyRef = useRef(dirty);
	dirtyRef.current = dirty;

	useEffect(() => {
		setMode(resolvedTheme === "light" ? "light" : "dark");
	}, [resolvedTheme]);

	const load = useCallback(async () => {
		setLoading(true);
		setError(null);
		try {
			const css = (await backend.appState.getAppStylesheet(appId)) ?? "";
			const parsed = parseAppearanceSheet(css);
			setState(parsed.state);
			setTail(parsed.tail);
			const next = parsed.managed
				? css
				: composeAppearanceSheet(
						buildAppearanceBlock(parsed.state),
						parsed.tail,
					);
			setSheet(next);
			// Seeding the managed block is not an edit; it is written with the first real one.
			setSaved(next);
		} catch (cause) {
			console.error("Failed to load the app stylesheet", appId, cause);
			setError(
				cause instanceof Error ? cause.message : String(cause ?? "unknown"),
			);
		} finally {
			setLoading(false);
		}
	}, [appId, backend.appState]);

	useEffect(() => {
		void load();
	}, [load]);

	const save = useCallback(async () => {
		if (!dirtyRef.current) return;
		setSaving(true);
		try {
			await backend.appState.setAppStylesheet(appId, sheet);
			setSaved(sheet);
			toast.success(t("appStylesheetSaved", "App stylesheet saved"));
		} catch (cause) {
			console.error("Failed to save the app stylesheet", appId, cause);
			toast.error(
				cause instanceof Error ? cause.message : String(cause ?? "unknown"),
			);
		} finally {
			setSaving(false);
		}
	}, [appId, backend.appState, sheet, t]);

	useEffect(() => {
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.key !== "s" || !(event.metaKey || event.ctrlKey)) return;
			event.preventDefault();
			void save();
		};
		window.addEventListener("keydown", onKeyDown);
		return () => window.removeEventListener("keydown", onKeyDown);
	}, [save]);

	// A tab closing is not a React unmount we can block, so the browser prompt is the only
	// thing standing between an unsaved sheet and a lost afternoon.
	useEffect(() => {
		const onBeforeUnload = (event: BeforeUnloadEvent) => {
			if (!dirtyRef.current) return;
			event.preventDefault();
			event.returnValue = "";
		};
		window.addEventListener("beforeunload", onBeforeUnload);
		return () => window.removeEventListener("beforeunload", onBeforeUnload);
	}, []);

	/** A control moved: the managed block is rewritten, the author's tail is not. */
	const applyState = useCallback(
		(next: AppearanceState) => {
			setState(next);
			setSheet(composeAppearanceSheet(buildAppearanceBlock(next), tail));
		},
		[tail],
	);

	/** The sheet was typed into: read it back instead of overwriting what was typed. */
	const applySheet = useCallback((next: string) => {
		setSheet(next);
		const parsed = parseAppearanceSheet(next);
		setState(parsed.state);
		setTail(parsed.tail);
	}, []);

	const resetTokens = useCallback(() => {
		applyState(cloneAppearanceState(DEFAULT_APPEARANCE_STATE));
		setReplayKey((key) => key + 1);
	}, [applyState]);

	const previewSheet = useMemo(
		() =>
			composeAppearanceSheet(
				buildAppearanceBlock(state, { mode, boost: true }),
				tail,
			),
		[state, tail, mode],
	);

	const bytes = useMemo(
		() => (typeof Blob === "undefined" ? sheet.length : new Blob([sheet]).size),
		[sheet],
	);

	if (error) {
		return (
			<div
				className={cn(
					"flex h-full min-h-0 flex-col items-center justify-center gap-2 p-6 text-muted-foreground text-sm",
					className,
				)}
			>
				<TriangleAlertIcon className="size-5 text-destructive" />
				<p>
					{t(
						"couldNotOpenTheAppStylesheet",
						"Could not open the app stylesheet",
					)}
				</p>
				<p className="max-w-md text-center text-xs">{error}</p>
				<Button size="sm" variant="outline" onClick={() => void load()}>
					{t("tryAgain", "Try again")}
				</Button>
			</div>
		);
	}

	return (
		<div className={cn("flex h-full min-h-0 flex-col bg-card", className)}>
			<header className="flex shrink-0 flex-wrap items-center gap-2 border-b bg-muted/30 px-3 py-2">
				<span className="grid size-8 shrink-0 place-items-center rounded-lg border bg-background">
					<PaletteIcon className="size-4 text-primary" />
				</span>
				<div className="min-w-0">
					<div className="flex items-center gap-2">
						<span className="truncate font-mono text-xs font-medium">
							app.css
						</span>
						{dirty && (
							<Badge
								variant="outline"
								className="h-4 shrink-0 gap-1 px-1.5 font-normal text-[10px] text-muted-foreground"
								title={t("unsavedChanges", "Unsaved changes")}
							>
								<span className="size-1.5 rounded-full bg-primary" />
								{t("unsaved", "Unsaved")}
							</Badge>
						)}
					</div>
					<p className="truncate text-[11px] text-muted-foreground">
						{t(
							"appStylesheetAppliesToEveryPage",
							"Applies to every page in this app",
						)}
					</p>
				</div>

				<span className="flex-1" />

				<PillGroup
					className="lg:hidden"
					label={t("appearancePanels", "Panels")}
					options={[
						{ id: "controls", label: t("appearanceControls", "Controls") },
						{ id: "preview", label: t("appearancePreview", "Preview") },
						{ id: "css", label: "CSS" },
					]}
					value={panel}
					onChange={(next) => setPanel(next as Panel)}
				/>

				<Button
					size="sm"
					variant="ghost"
					className="h-7 gap-1.5 text-xs"
					onClick={resetTokens}
				>
					<RotateCcwIcon className="size-3.5" />
					{t("appearanceReset", "Reset")}
				</Button>
				<Button
					size="sm"
					className="h-7 gap-1.5 text-xs"
					disabled={!dirty || saving || loading}
					onClick={() => void save()}
				>
					{saving ? (
						<Loader2Icon className="size-3.5 animate-spin" />
					) : (
						<SaveIcon className="size-3.5" />
					)}
					{saving ? t("savingEllipsis", "Saving…") : t("save", "Save")}
				</Button>
			</header>

			<div className="grid min-h-0 flex-1 lg:grid-cols-[296px_minmax(0,1fr)_min(380px,34vw)]">
				<div
					className={cn(
						"min-h-0 overflow-y-auto border-b lg:block lg:border-r lg:border-b-0",
						panel === "controls" ? "block" : "hidden lg:block",
					)}
				>
					{loading ? (
						<div className="flex flex-col gap-3 p-4">
							<Skeleton className="h-5 w-24" />
							<Skeleton className="h-28 w-full" />
							<Skeleton className="h-40 w-full" />
						</div>
					) : (
						<AppearanceRail state={state} mode={mode} onChange={applyState} />
					)}
				</div>

				<div
					className={cn(
						"flex min-h-0 flex-col",
						panel === "preview" ? "flex" : "hidden lg:flex",
					)}
				>
					<div className="flex shrink-0 flex-wrap items-center gap-2 border-b px-3 py-2">
						<PillGroup
							label={t("appearancePreviewSurface", "Preview surface")}
							options={[
								{
									id: "dashboard",
									label: t("appearanceViewDashboard", "dashboard"),
								},
								{ id: "chat", label: t("appearanceViewChat", "chat") },
							]}
							value={view}
							onChange={(next) => {
								setView(next as AppearancePreviewView);
								setReplayKey((key) => key + 1);
							}}
						/>
						<PillGroup
							label={t("appearancePreviewMode", "Preview theme")}
							options={[
								{ id: "light", label: t("appearanceModeLight", "light") },
								{ id: "dark", label: t("appearanceModeDark", "dark") },
							]}
							value={mode}
							onChange={(next) => setMode(next as AppearanceMode)}
						/>
						<PillGroup
							label={t("appearancePreviewWidth", "Preview width")}
							options={[
								{
									id: String(WIDTHS[0].px),
									label: t("appearanceWidthDesktop", "desktop"),
								},
								{
									id: String(WIDTHS[1].px),
									label: t("appearanceWidthTablet", "tablet"),
								},
								{
									id: String(WIDTHS[2].px),
									label: t("appearanceWidthPhone", "phone"),
								},
							]}
							value={String(width)}
							onChange={(next) => setWidth(Number(next))}
						/>
						<span className="flex-1" />
						<Button
							size="sm"
							variant="ghost"
							className="h-7 text-xs"
							onClick={() => setReplayKey((key) => key + 1)}
						>
							{t("appearanceReplay", "Replay motion")}
						</Button>
					</div>

					<div className="min-h-0 flex-1 overflow-auto bg-muted/40 p-4">
						<div className="mx-auto w-full" style={{ maxWidth: width }}>
							{loading ? (
								<Skeleton className="h-72 w-full" />
							) : (
								<AppearancePreview
									key={`${replayKey}-${view}-${mode}`}
									view={view}
									mode={mode}
									sheet={previewSheet}
								/>
							)}
						</div>
					</div>
				</div>

				<div
					className={cn(
						"flex min-h-0 flex-col border-t lg:border-t-0 lg:border-l",
						panel === "css" ? "flex" : "hidden lg:flex",
					)}
				>
					<div className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
						<span className="font-mono text-[11px]">app.css</span>
						<span className="flex-1" />
						<span className="font-mono text-[10px] text-muted-foreground tabular-nums">
							{t("appearanceBytes", "{{count}} B", { count: bytes })}
						</span>
					</div>
					<div className="min-h-0 flex-1">
						{loading ? (
							<Skeleton className="h-full w-full" />
						) : (
							<MonacoCodeEditor
								value={sheet}
								onChange={applySheet}
								language="css"
								height="100%"
								autoFocus={false}
								allowFullscreen
								className="h-full w-full rounded-none border-0 bg-transparent"
							/>
						)}
					</div>
					<p className="shrink-0 border-t px-3 py-2 text-[11px] text-muted-foreground leading-relaxed">
						{t(
							"appearanceManagedBlockNote",
							"The controls own the block between the start and end markers. Write below the end marker and it survives every change; edit inside the block and the controls read your values back.",
						)}
					</p>
				</div>
			</div>
		</div>
	);
}

function PillGroup({
	label,
	options,
	value,
	onChange,
	className,
}: Readonly<{
	label: string;
	options: readonly { id: string; label: string }[];
	value: string;
	onChange: (id: string) => void;
	className?: string;
}>) {
	return (
		<fieldset
			aria-label={label}
			className={cn("flex overflow-hidden rounded-lg border", className)}
		>
			{options.map((option) => (
				<button
					key={option.id}
					type="button"
					aria-pressed={value === option.id}
					onClick={() => onChange(option.id)}
					className={cn(
						"px-2.5 py-1 font-mono text-[11px] text-muted-foreground transition-colors",
						value === option.id && "bg-primary/10 text-foreground",
					)}
				>
					{option.label}
				</button>
			))}
		</fieldset>
	);
}

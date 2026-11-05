"use client";

import {
	Copy,
	FolderOpen,
	Loader2,
	LockKeyhole,
	Plus,
	RefreshCw,
	Save,
	Share2,
	Trash2,
	Users,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { getApiOrigin } from "../../../../lib/api-url";
import { getErrorMessage } from "../../../../lib/error-message";
import { resolveHubWebOrigin } from "../../../../lib/public-web-origin";
import { useBackend } from "../../../../state/backend-state";
import type {
	SavedStudioForecast,
	StudioForecastSummary,
} from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../ui/dialog";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Textarea } from "../../../ui/textarea";
import {
	type ForecastRunSnapshot,
	forecastNeedsWebOrigin,
	forecastShareLink,
	forecastSummary,
} from "./forecast-snapshots";

export function ForecastLibrary({
	appId,
	userScoped,
	canWrite,
	run,
	running,
	stale,
	savedForecastId,
	onSavedForecastChange,
	onOpen,
	onNew,
	onOpeningChange,
}: {
	appId: string;
	userScoped: boolean;
	canWrite: boolean;
	run: ForecastRunSnapshot | null;
	running: boolean;
	stale: boolean;
	savedForecastId?: string;
	onSavedForecastChange?: (id?: string) => void;
	onOpen: (saved: SavedStudioForecast) => void;
	onNew: () => void;
	onOpeningChange: (opening: boolean) => void;
}) {
	const backend = useBackend();
	const [items, setItems] = useState<StudioForecastSummary[]>([]);
	const [active, setActive] = useState<SavedStudioForecast | null>(null);
	const activeRef = useRef(active);
	const [listing, setListing] = useState(false);
	const [opening, setOpening] = useState(false);
	const [listError, setListError] = useState<string | null>(null);
	const [openError, setOpenError] = useState<string | null>(null);
	const [saveRun, setSaveRun] = useState<ForecastRunSnapshot | null>(null);
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [saving, setSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const [deleting, setDeleting] = useState(false);
	const [deleteOpen, setDeleteOpen] = useState(false);
	const [deleteError, setDeleteError] = useState<string | null>(null);
	const [shareLink, setShareLink] = useState<string | null>(null);
	const [sharing, setSharing] = useState(false);
	const [shareError, setShareError] = useState<string | null>(null);
	const [copied, setCopied] = useState(false);
	const [copyError, setCopyError] = useState<string | null>(null);
	const alive = useRef(true);
	const listSequence = useRef(0);
	const navigation = useRef(0);
	const callbacks = useRef({
		onOpen,
		onNew,
		onSavedForecastChange,
		onOpeningChange,
	});
	callbacks.current = { onOpen, onNew, onSavedForecastChange, onOpeningChange };
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
			listSequence.current++;
			navigation.current++;
		};
	}, []);

	const activate = useCallback((saved: SavedStudioForecast | null) => {
		activeRef.current = saved;
		setActive(saved);
	}, []);
	const refresh = useCallback(async () => {
		const sequence = ++listSequence.current;
		setListing(true);
		setListError(null);
		try {
			const value = await backend.queryState.listForecasts(appId, userScoped);
			if (alive.current && sequence === listSequence.current) setItems(value);
		} catch (error) {
			if (alive.current && sequence === listSequence.current)
				setListError(getErrorMessage(error));
		} finally {
			if (alive.current && sequence === listSequence.current) setListing(false);
		}
	}, [appId, backend.queryState, userScoped]);
	useEffect(() => {
		void refresh();
	}, [refresh]);
	useEffect(() => {
		const saved = (event: Event) => {
			const detail = (event as CustomEvent).detail;
			if (detail?.appId === appId && detail.userScoped === userScoped)
				void refresh();
		};
		window.addEventListener("flow-like:forecast-saved", saved);
		return () => window.removeEventListener("flow-like:forecast-saved", saved);
	}, [appId, userScoped, refresh]);

	const open = useCallback(
		async (id: string, updateLocation = true) => {
			const sequence = ++navigation.current;
			setOpening(true);
			callbacks.current.onOpeningChange(true);
			setOpenError(null);
			setSaveRun(null);
			setDeleteOpen(false);
			setShareLink(null);
			setShareError(null);
			try {
				const saved = await backend.queryState.getForecast(
					appId,
					id,
					userScoped,
				);
				if (!alive.current || sequence !== navigation.current) return;
				activate(saved);
				setItems((current) =>
					current.some((item) => item.id === saved.id)
						? current
						: [forecastSummary(saved), ...current],
				);
				callbacks.current.onOpen(saved);
				if (updateLocation) callbacks.current.onSavedForecastChange?.(saved.id);
			} catch (error) {
				if (alive.current && sequence === navigation.current)
					setOpenError(getErrorMessage(error));
			} finally {
				if (alive.current && sequence === navigation.current) {
					setOpening(false);
					callbacks.current.onOpeningChange(false);
				}
			}
		},
		[activate, appId, backend.queryState, userScoped],
	);

	useEffect(() => {
		navigation.current++;
		setOpening(false);
		callbacks.current.onOpeningChange(false);
		if (savedForecastId === activeRef.current?.id) return;
		if (savedForecastId) void open(savedForecastId, false);
		else if (activeRef.current) {
			activate(null);
			callbacks.current.onNew();
		}
	}, [savedForecastId, open, activate]);

	const startNew = () => {
		navigation.current++;
		setOpening(false);
		callbacks.current.onOpeningChange(false);
		setOpenError(null);
		setShareError(null);
		activate(null);
		callbacks.current.onNew();
		callbacks.current.onSavedForecastChange?.();
	};
	const showSave = () => {
		if (!run || !canWrite) return;
		setSaveRun(run);
		setName(
			active
				? `${active.name} (copy)`
				: `${run.request.mapping.targets.join(", ")} forecast`,
		);
		setDescription(active?.description ?? "");
		setSaveError(null);
	};
	const save = async () => {
		if (!saveRun || !canWrite || !name.trim() || saving) return;
		const sequence = navigation.current;
		setSaving(true);
		setSaveError(null);
		try {
			const saved = await backend.queryState.createForecast(
				appId,
				{
					name: name.trim(),
					description: description.trim() || undefined,
					request: saveRun.request,
					result: saveRun.result,
				},
				userScoped,
			);
			if (!alive.current) return;
			setSaveRun(null);
			if (sequence === navigation.current) {
				activate(saved);
				callbacks.current.onSavedForecastChange?.(saved.id);
			}
			void refresh();
		} catch (error) {
			if (alive.current) setSaveError(getErrorMessage(error));
		} finally {
			if (alive.current) setSaving(false);
		}
	};
	const remove = async () => {
		if (!active || !canWrite || deleting) return;
		const id = active.id;
		const sequence = navigation.current;
		setDeleting(true);
		setDeleteError(null);
		try {
			await backend.queryState.deleteForecast(appId, id, userScoped);
			if (!alive.current) return;
			setDeleteOpen(false);
			setItems((current) => current.filter((item) => item.id !== id));
			if (sequence === navigation.current && activeRef.current?.id === id)
				startNew();
			void refresh();
		} catch (error) {
			if (alive.current) setDeleteError(getErrorMessage(error));
		} finally {
			if (alive.current) setDeleting(false);
		}
	};
	const share = async () => {
		if (!active || sharing) return;
		const sequence = navigation.current;
		setSharing(true);
		setShareError(null);
		try {
			const location = window.location.href;
			const desktop =
				"__TAURI__" in window ||
				"__TAURI_INTERNALS__" in window ||
				forecastNeedsWebOrigin(location);
			const webOrigin = desktop
				? await resolveHubWebOrigin(getApiOrigin(backend.profile))
				: undefined;
			if (!alive.current || sequence !== navigation.current) return;
			setShareLink(
				forecastShareLink(location, active.id, userScoped, webOrigin),
			);
			setCopied(false);
			setCopyError(null);
		} catch (error) {
			if (alive.current && sequence === navigation.current)
				setShareError(getErrorMessage(error));
		} finally {
			if (alive.current) setSharing(false);
		}
	};
	const busy = running || opening || saving || deleting || sharing;
	const displayedIsSaved = active?.result.run_id === run?.result.run_id;
	const permissionHint = canWrite
		? undefined
		: "Your role can view predictions but cannot save or delete them.";
	const selection =
		active && !items.some((item) => item.id === active.id)
			? [forecastSummary(active), ...items]
			: items;

	return (
		<section
			className="space-y-3 rounded-xl border bg-card p-4"
			aria-label="Saved predictions"
		>
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div className="flex items-center gap-2">
					<FolderOpen className="size-4 text-muted-foreground" />
					<h3 className="text-sm font-medium">Predictions</h3>
					<span className="flex items-center gap-1 rounded-full bg-muted px-2 py-1 text-[10px] text-muted-foreground">
						{userScoped ? (
							<LockKeyhole className="size-3" />
						) : (
							<Users className="size-3" />
						)}
						{userScoped ? "Private to you" : "Shared with project"}
					</span>
				</div>
				<div className="flex flex-wrap items-center gap-2">
					<Button
						size="sm"
						variant="outline"
						onClick={startNew}
						disabled={busy}
						className="h-8 gap-1.5 text-xs"
					>
						<Plus className="size-3.5" />
						New prediction
					</Button>
					<Button
						size="sm"
						variant="outline"
						onClick={showSave}
						disabled={!run || !canWrite || busy}
						title={permissionHint}
						className="h-8 gap-1.5 text-xs"
					>
						<Save className="size-3.5" />
						{active ? "Save as new" : "Save prediction"}
					</Button>
				</div>
			</div>
			<div className="flex flex-wrap items-center gap-2">
				<Select
					value={active?.id ?? ""}
					onValueChange={(id) => void open(id)}
					disabled={busy || selection.length === 0}
				>
					<SelectTrigger
						className="h-9 w-full min-w-0 basis-full text-xs @xl/ds:w-auto @xl/ds:basis-auto @xl/ds:flex-1"
						aria-label="Open a saved prediction"
					>
						<SelectValue
							placeholder={
								listing ? "Loading predictions…" : "Open a saved prediction"
							}
						/>
					</SelectTrigger>
					<SelectContent>
						{selection.map((item) => (
							<SelectItem key={item.id} value={item.id}>
								{item.name} · {item.horizon} steps
							</SelectItem>
						))}
					</SelectContent>
				</Select>
				<Button
					size="icon"
					variant="ghost"
					className="size-8"
					onClick={() => void refresh()}
					disabled={listing}
					aria-label="Refresh saved predictions"
				>
					{listing ? (
						<Loader2 className="size-3.5 animate-spin" />
					) : (
						<RefreshCw className="size-3.5" />
					)}
				</Button>
				{active && (
					<>
						<Button
							size="sm"
							variant="ghost"
							className="h-8 gap-1.5 text-xs"
							disabled={busy}
							onClick={() => void share()}
						>
							{sharing ? (
								<Loader2 className="size-3.5 animate-spin" />
							) : (
								<Share2 className="size-3.5" />
							)}
							{userScoped ? "Link" : "Share"}
						</Button>
						<Button
							size="icon"
							variant="ghost"
							className="size-8 text-muted-foreground hover:text-destructive"
							disabled={!canWrite || busy}
							title={permissionHint}
							aria-label={`Delete ${active.name}`}
							onClick={() => {
								setDeleteError(null);
								setDeleteOpen(true);
							}}
						>
							<Trash2 className="size-3.5" />
						</Button>
					</>
				)}
			</div>
			{opening ? (
				<output className="flex items-center gap-2 text-xs leading-normal text-muted-foreground">
					<Loader2 className="size-3.5 animate-spin" />
					Opening saved prediction…
				</output>
			) : active ? (
				<div className="space-y-1 text-xs text-muted-foreground">
					<p className="leading-normal">
						{displayedIsSaved
							? "Viewing saved result"
							: "Saved prediction selected; the displayed run has not been saved"}{" "}
						· {new Date(active.created_at).toLocaleString()}
					</p>
					{active.description && (
						<p className="whitespace-pre-wrap leading-normal">
							{active.description}
						</p>
					)}
				</div>
			) : !listing && items.length === 0 && !listError ? (
				<p className="text-xs leading-normal text-muted-foreground">
					Run a forecast and save a named prediction to compare results later.
				</p>
			) : null}
			{!canWrite && (
				<p className="text-[11px] leading-normal text-muted-foreground">
					{permissionHint}
				</p>
			)}
			{(listError || openError || shareError) && (
				<div role="alert" className="text-xs text-destructive">
					{openError || shareError || listError}
					{openError && savedForecastId && (
						<Button
							variant="link"
							size="sm"
							className="ml-2 h-auto p-0 text-xs"
							onClick={() => void open(savedForecastId, false)}
						>
							Retry opening
						</Button>
					)}
				</div>
			)}
			<Dialog
				open={saveRun !== null}
				onOpenChange={(value) => {
					if (!value && !saving) setSaveRun(null);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Save prediction</DialogTitle>
						<DialogDescription>
							{userScoped
								? "This prediction is private to your personal data scope."
								: "Project members who can read its source data can open this prediction."}{" "}
							Saving creates a new, fixed result.
						</DialogDescription>
					</DialogHeader>
					<div className="space-y-4">
						<div className="space-y-1.5">
							<Label htmlFor="forecast-save-name">Name</Label>
							<Input
								id="forecast-save-name"
								value={name}
								onChange={(event) => setName(event.target.value)}
								maxLength={160}
								disabled={saving}
								autoFocus
							/>
						</div>
						<div className="space-y-1.5">
							<Label htmlFor="forecast-save-description">
								Description{" "}
								<span className="text-muted-foreground">(optional)</span>
							</Label>
							<Textarea
								id="forecast-save-description"
								value={description}
								onChange={(event) => setDescription(event.target.value)}
								maxLength={4000}
								disabled={saving}
							/>
						</div>
						{saveRun && (
							<p className="rounded-md bg-muted p-3 text-xs leading-normal text-muted-foreground">
								{saveRun.result.model.family} · {saveRun.request.horizon} future
								steps · {saveRun.request.mapping.targets.join(", ")}
								{stale
									? ". Saves the displayed run with its original settings; draft changes are excluded."
									: ""}
							</p>
						)}
						{saveError && (
							<p role="alert" className="text-sm text-destructive">
								{saveError}
							</p>
						)}
					</div>
					<DialogFooter>
						<Button
							variant="outline"
							disabled={saving}
							onClick={() => setSaveRun(null)}
						>
							Cancel
						</Button>
						<Button
							disabled={saving || !name.trim() || !canWrite}
							onClick={() => void save()}
						>
							{saving && <Loader2 className="mr-2 size-4 animate-spin" />}Save
							prediction
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
			<Dialog
				open={deleteOpen}
				onOpenChange={(value) => {
					if (!deleting) setDeleteOpen(value);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>Delete “{active?.name}”?</DialogTitle>
						<DialogDescription>
							{userScoped
								? "This removes the saved prediction from your personal data."
								: "This removes the shared prediction for everyone in the project."}{" "}
							Its link will stop working.
						</DialogDescription>
					</DialogHeader>
					{deleteError && (
						<p role="alert" className="text-sm text-destructive">
							{deleteError}
						</p>
					)}
					<DialogFooter>
						<Button
							variant="outline"
							disabled={deleting}
							onClick={() => setDeleteOpen(false)}
						>
							Cancel
						</Button>
						<Button
							variant="destructive"
							disabled={deleting || !canWrite}
							onClick={() => void remove()}
						>
							{deleting && <Loader2 className="mr-2 size-4 animate-spin" />}
							Delete prediction
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
			<Dialog
				open={shareLink !== null}
				onOpenChange={(value) => {
					if (!value) setShareLink(null);
				}}
			>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>
							{userScoped ? "Private prediction link" : "Share prediction"}
						</DialogTitle>
						<DialogDescription>
							{userScoped
								? "Only your account can open this personal prediction. Copying its link does not grant access."
								: "Project members who can read its source data can open this saved prediction. The link does not grant new permissions."}
						</DialogDescription>
					</DialogHeader>
					<Input
						aria-label="Saved prediction link"
						value={shareLink ?? ""}
						readOnly
						onFocus={(event) => event.target.select()}
					/>
					{copyError && (
						<p role="alert" className="text-xs text-destructive">
							{copyError}
						</p>
					)}
					<DialogFooter>
						<Button variant="outline" onClick={() => setShareLink(null)}>
							Close
						</Button>
						<Button
							onClick={async () => {
								if (!shareLink) return;
								try {
									await navigator.clipboard.writeText(shareLink);
									if (alive.current) setCopied(true);
								} catch {
									if (alive.current)
										setCopyError(
											"Copy is unavailable. Select and copy the link above.",
										);
								}
							}}
						>
							<Copy className="mr-2 size-3.5" />
							{copied ? "Copied" : "Copy link"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</section>
	);
}

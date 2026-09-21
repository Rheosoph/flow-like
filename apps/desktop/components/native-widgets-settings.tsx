"use client";

import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";

import { classifyAppEventInterface } from "@flow-like/flow-like-ui/components/global-chat/app-event-interface";
import { HomeDataWidgetSettings } from "@flow-like/flow-like-ui/components/home/data-widget-settings";
import { normalizeHomeDataConfig } from "@flow-like/flow-like-ui/components/home/home-data-query";
import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import { Input } from "@flow-like/flow-like-ui/components/ui/input";
import { useInvoke } from "@flow-like/flow-like-ui/hooks/use-invoke";
import {
	appRouteUrl,
	parseAppRouteTarget,
} from "@flow-like/flow-like-ui/lib/app-route-url";
import {
	MAX_NATIVE_WIDGETS,
	NATIVE_WIDGETS_CHANGED,
	NATIVE_WIDGETS_REFRESH,
	NATIVE_WIDGETS_UPDATED,
	type NativeCustomWidget,
	type NativeWidgetAccent,
	type NativeWidgetDefinition,
	type NativeWidgetPageDefinition,
	nativeWidgetScope,
	newNativeWidgetDefinition,
	normalizeNativeWidgetDefinition,
	readNativeWidgetDefinitions,
	saveNativeWidgetDefinitions,
} from "@flow-like/flow-like-ui/lib/native-widget";
import {
	useBackend,
	useBackendReady,
} from "@flow-like/flow-like-ui/state/backend-state";
import { useQuery } from "@tanstack/react-query";
import {
	BarChart3,
	ChevronLeft,
	ExternalLink,
	LayoutTemplate,
	Loader2,
	Plus,
	RefreshCw,
	Smartphone,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import {
	previewNativeCustomWidget,
	readNativeCustomWidgetCache,
} from "../lib/native-custom-widgets";
import { NativeWidgetPreview } from "./native-widget-preview";

const selectClass =
	"h-9 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm";
function Field({
	label,
	children,
	hint,
}: { label: string; children: (id: string) => ReactNode; hint?: string }) {
	const id = useId();
	return (
		<div className="min-w-0 space-y-1.5">
			<label htmlFor={id} className="text-xs font-medium">
				{label}
			</label>
			{children(id)}
			{hint && <p className="text-xs text-muted-foreground">{hint}</p>}
		</div>
	);
}

function PageSettings({
	definition,
	onChange,
	apps,
}: {
	definition: NativeWidgetPageDefinition;
	onChange: (definition: NativeWidgetPageDefinition) => void;
	apps: { id: string; name: string }[];
}) {
	const backend = useBackend();
	const [routes, setRoutes] = useState<{ path: string; name: string }[]>([]);
	const [containers, setContainers] = useState<{ id: string; name: string }[]>(
		[],
	);
	const [error, setError] = useState<string>();
	const [loading, setLoading] = useState(false);
	useEffect(() => {
		let active = true;
		setRoutes([]);
		if (!definition.appId) return;
		void backend.eventState
			.getEvents(definition.appId)
			.then((events) => {
				if (active)
					setRoutes(
						events
							.filter(
								(event) =>
									event.active &&
									classifyAppEventInterface(event) === "page" &&
									(event.route || event.is_default),
							)
							.map((event) => ({
								path: event.route || "/",
								name: event.name || event.route || "/",
							})),
					);
			})
			.catch(() => {
				if (active)
					setError(
						"Pages could not be listed. You can enter a published app path below.",
					);
			});
		return () => {
			active = false;
		};
	}, [backend.eventState, definition.appId]);
	useEffect(() => {
		let active = true;
		setContainers([]);
		setError(undefined);
		setLoading(false);
		if (!definition.appId) return;
		const timer = setTimeout(() => {
			setLoading(true);
			void (async () => {
				try {
					const target = parseAppRouteTarget(
						definition.path,
						definition.queryParams,
					);
					const result = await backend.pageState.getPageBootstrap(
						definition.appId,
						target.path ?? "/",
					);
					if (result.routeMiss || !result.page)
						throw new Error("This path does not have a published app page.");
					if (active)
						setContainers(
							result.page.components
								.filter((component) =>
									[
										"row",
										"column",
										"grid",
										"stack",
										"card",
										"box",
										"center",
									].includes(component.component.type),
								)
								.map((component) => ({
									id: component.id,
									name: `${component.component.type} · ${component.id}`,
								})),
						);
				} catch (reason) {
					if (active)
						setError(
							reason instanceof Error
								? reason.message
								: "The page could not be loaded.",
						);
				} finally {
					if (active) setLoading(false);
				}
			})();
		}, 300);
		return () => {
			active = false;
			clearTimeout(timer);
		};
	}, [
		backend.pageState,
		definition.appId,
		definition.path,
		definition.queryParams,
	]);
	return (
		<div className="space-y-4">
			<Field label="App">
				{(id) => (
					<select
						id={id}
						className={selectClass}
						value={definition.appId}
						onChange={(event) =>
							onChange({
								...definition,
								appId: event.target.value,
								path: "/",
								queryParams: [],
								containerId: undefined,
							})
						}
					>
						<option value="">Choose an app</option>
						{apps.map((app) => (
							<option key={app.id} value={app.id}>
								{app.name}
							</option>
						))}
					</select>
				)}
			</Field>
			{routes.length > 0 && (
				<Field label="Published page">
					{(id) => (
						<select
							id={id}
							className={selectClass}
							value={
								routes.some((route) => route.path === definition.path)
									? definition.path
									: ""
							}
							onChange={(event) => {
								if (event.target.value)
									onChange({
										...definition,
										path: event.target.value,
										containerId: undefined,
									});
							}}
						>
							<option value="">Custom path</option>
							{routes.map((route) => (
								<option key={route.path} value={route.path}>
									{route.name} ({route.path})
								</option>
							))}
						</select>
					)}
				</Field>
			)}
			<Field
				label="Page container"
				hint="Choose a compact part of the page when the full layout does not fit."
			>
				{(id) => (
					<select
						id={id}
						className={selectClass}
						value={definition.containerId ?? ""}
						onChange={(event) =>
							onChange({
								...definition,
								containerId: event.target.value || undefined,
							})
						}
					>
						<option value="">Whole page</option>
						{containers.map((container) => (
							<option key={container.id} value={container.id}>
								{container.name}
							</option>
						))}
						{definition.containerId &&
							!containers.some(
								(container) => container.id === definition.containerId,
							) && (
								<option value={definition.containerId}>
									{definition.containerId} (unavailable)
								</option>
							)}
					</select>
				)}
			</Field>
			{loading && (
				<p className="flex items-center gap-2 text-xs text-muted-foreground">
					<Loader2 className="size-3 animate-spin" />
					Checking the page…
				</p>
			)}
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			<p className="text-xs leading-relaxed text-muted-foreground">
				Widgets support text, layouts, images, progress, compact tables, and
				charts. Page buttons open the app. Camera, video, forms, custom HTML,
				and other interactive content stay in the app.
			</p>
		</div>
	);
}

function WidgetEditor({
	initial,
	saved,
	scope,
	viewerId,
	apps,
	onClose,
}: {
	initial: NativeWidgetDefinition;
	saved: NativeWidgetDefinition[];
	scope: string;
	viewerId?: string;
	apps: { id: string; name: string }[];
	onClose: () => void;
}) {
	const backend = useBackend();
	const router = useClientRouter();
	const [draft, setDraft] = useState(initial);
	const [preview, setPreview] = useState<NativeCustomWidget>();
	const [error, setError] = useState<string>();
	const [loading, setLoading] = useState(false);
	const [family, setFamily] = useState<"small" | "medium" | "large">("medium");
	const request = useRef<AbortController | undefined>(undefined);
	const signature = JSON.stringify(draft);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Edited settings invalidate an in-flight preview.
	useEffect(() => {
		request.current?.abort();
		request.current = undefined;
		setPreview(undefined);
		setError(undefined);
		setLoading(false);
	}, [signature]);
	useEffect(() => () => request.current?.abort(), []);
	const validate = () =>
		normalizeNativeWidgetDefinition({
			...draft,
			updatedAt: new Date().toISOString(),
		});
	const showPreview = async () => {
		request.current?.abort();
		const controller = new AbortController();
		request.current = controller;
		setLoading(true);
		setError(undefined);
		const timer = setTimeout(() => controller.abort(), 30_000);
		try {
			const definition = validate();
			const cached = readNativeCustomWidgetCache(scope).find(
				(entry) =>
					entry.widget.id === definition.id &&
					entry.revision === draft.updatedAt,
			);
			const result = await previewNativeCustomWidget(backend, definition, {
				viewerId,
				signal: controller.signal,
				cached:
					JSON.stringify(draft) === JSON.stringify(initial)
						? cached
						: undefined,
			});
			if (!controller.signal.aborted) setPreview(result);
		} catch (reason) {
			if (request.current === controller)
				setError(
					controller.signal.aborted
						? "The preview timed out. Try again when the app is connected."
						: reason instanceof Error
							? reason.message
							: "The preview could not be loaded.",
				);
		} finally {
			clearTimeout(timer);
			if (request.current === controller) setLoading(false);
		}
	};
	const save = () => {
		try {
			const definition = validate();
			const current = readNativeWidgetDefinitions(scope);
			saveNativeWidgetDefinitions(
				scope,
				current.some((item) => item.id === definition.id)
					? current.map((item) =>
							item.id === definition.id ? definition : item,
						)
					: [...current, definition],
			);
			setDraft(definition);
			toast.success("Widget saved. Select it in the Flow Like widget picker.");
		} catch (reason) {
			setError(
				reason instanceof Error
					? reason.message
					: "The widget could not be saved on this device.",
			);
		}
	};
	return (
		<div className="min-w-0 rounded-2xl border bg-card">
			<div className="flex items-center justify-between gap-3 border-b p-4">
				<div className="flex items-center gap-2">
					<Button
						variant="ghost"
						size="icon"
						onClick={onClose}
						aria-label="Back to saved widgets"
					>
						<ChevronLeft className="size-4" />
					</Button>
					<h2 className="text-lg font-semibold">
						{saved.some((item) => item.id === draft.id)
							? "Edit widget"
							: "New widget"}
					</h2>
				</div>
				<Button onClick={save}>Save widget</Button>
			</div>
			<div className="grid min-w-0 gap-6 p-4 lg:grid-cols-[minmax(0,1fr)_minmax(280px,0.85fr)] lg:p-6">
				<div className="min-w-0 space-y-5">
					<Field label="Widget name">
						{(id) => (
							<Input
								id={id}
								value={draft.title}
								maxLength={120}
								onChange={(event) =>
									setDraft({ ...draft, title: event.target.value })
								}
							/>
						)}
					</Field>
					{draft.kind === "chart" ? (
						<HomeDataWidgetSettings
							native
							widget={{
								id: draft.id,
								type: "data",
								title: draft.title,
								size: { columns: 1, rows: 1 },
								appearance: { variant: "default", accent: draft.accent },
								config: { ...draft.data },
							}}
							onChange={(data) => {
								const config = normalizeHomeDataConfig(data);
								setDraft({ ...draft, appId: config.appId, data: config });
							}}
						/>
					) : (
						<PageSettings definition={draft} onChange={setDraft} apps={apps} />
					)}
					<div className="space-y-3 rounded-xl border border-border/60 p-3">
						<p className="text-sm font-medium">
							{draft.kind === "page" ? "Page address" : "Open on tap"}
						</p>
						<Field
							label="App path"
							hint="Enter a path such as /orders. You can include ?status=open; Flow Like handles encoding."
						>
							{(id) => (
								<Input
									id={id}
									value={draft.path}
									placeholder="/"
									maxLength={4096}
									onChange={(event) =>
										setDraft({ ...draft, path: event.target.value })
									}
								/>
							)}
						</Field>
						{draft.queryParams.map((pair, index) => (
							// biome-ignore lint/suspicious/noArrayIndexKey: Controlled positional pairs preserve repeated query names and have no row-local state.
							<div key={index} className="flex items-end gap-2">
								<Field label={`Parameter ${index + 1}`}>
									{(id) => (
										<Input
											id={id}
											value={pair.name}
											placeholder="status"
											onChange={(event) =>
												setDraft({
													...draft,
													queryParams: draft.queryParams.map((value, i) =>
														i === index
															? { ...value, name: event.target.value }
															: value,
													),
												})
											}
										/>
									)}
								</Field>
								<Field label="Value">
									{(id) => (
										<Input
											id={id}
											value={pair.value}
											placeholder="open"
											onChange={(event) =>
												setDraft({
													...draft,
													queryParams: draft.queryParams.map((value, i) =>
														i === index
															? { ...value, value: event.target.value }
															: value,
													),
												})
											}
										/>
									)}
								</Field>
								<Button
									variant="ghost"
									size="icon"
									aria-label={`Remove parameter ${index + 1}`}
									onClick={() =>
										setDraft({
											...draft,
											queryParams: draft.queryParams.filter(
												(_, i) => i !== index,
											),
										})
									}
								>
									<Trash2 className="size-4" />
								</Button>
							</div>
						))}
						<Button
							variant="outline"
							size="sm"
							disabled={draft.queryParams.length >= 32}
							onClick={() =>
								setDraft({
									...draft,
									queryParams: [...draft.queryParams, { name: "", value: "" }],
								})
							}
						>
							<Plus className="size-3" />
							Query parameter
						</Button>
					</div>
					<div className="grid grid-cols-2 gap-3">
						<Field label="Accent color">
							{(id) => (
								<select
									id={id}
									className={selectClass}
									value={draft.accent}
									onChange={(event) =>
										setDraft({
											...draft,
											accent: event.target.value as NativeWidgetAccent,
										})
									}
								>
									<option value="orange">Flow Like orange</option>
									<option value="blue">Blue</option>
									<option value="teal">Teal</option>
									<option value="purple">Purple</option>
								</select>
							)}
						</Field>
						<Field label="Refresh when app is open">
							{(id) => (
								<select
									id={id}
									className={selectClass}
									value={draft.refreshMinutes}
									onChange={(event) =>
										setDraft({
											...draft,
											refreshMinutes: Number(event.target.value),
										})
									}
								>
									{[
										[15, "15 minutes"],
										[30, "30 minutes"],
										[60, "Every hour"],
										[180, "3 hours"],
										[360, "6 hours"],
									].map(([value, name]) => (
										<option key={value} value={value}>
											{name}
										</option>
									))}
								</select>
							)}
						</Field>
					</div>
				</div>
				<div className="min-w-0 space-y-4 lg:sticky lg:top-4 lg:self-start">
					<div className="flex items-center justify-between gap-2">
						<h3 className="text-sm font-semibold">Content preview</h3>
						<select
							aria-label="Preview size"
							className={`${selectClass} max-w-32`}
							value={family}
							onChange={(event) =>
								setFamily(event.target.value as typeof family)
							}
						>
							<option value="small">Small</option>
							<option value="medium">Medium</option>
							<option value="large">Large</option>
						</select>
					</div>
					<div className="rounded-2xl bg-muted/40 p-3 sm:p-5">
						{preview ? (
							<NativeWidgetPreview widget={preview} family={family} />
						) : (
							<div className="flex min-h-44 flex-col items-center justify-center gap-2 text-center text-muted-foreground">
								<Smartphone className="size-8 opacity-40" />
								<p className="text-sm">Preview your selected content</p>
								<p className="max-w-64 text-xs">
									Use your actual data to check what fits before adding the
									widget.
								</p>
							</div>
						)}
					</div>
					<Button
						variant="outline"
						className="w-full"
						disabled={loading}
						onClick={() => void showPreview()}
					>
						{loading ? (
							<Loader2 className="size-4 animate-spin" />
						) : (
							<RefreshCw className="size-4" />
						)}
						{loading ? "Loading preview…" : "Preview latest content"}
					</Button>
					{error && (
						<p
							role="alert"
							className="rounded-lg bg-destructive/10 p-3 text-sm text-destructive"
						>
							{error}
						</p>
					)}
					{preview?.message && (
						<p className="text-sm text-muted-foreground">{preview.message}</p>
					)}
					{Boolean(preview?.warnings?.length) && (
						<div className="space-y-2 rounded-xl border border-amber-500/25 bg-amber-500/5 p-3">
							<p className="text-sm font-medium">Widget compatibility</p>
							{preview?.warnings?.map((warning) => (
								<p key={warning} className="text-xs text-muted-foreground">
									{warning}
								</p>
							))}
						</div>
					)}
					{draft.appId && (
						<Button
							variant="ghost"
							className="w-full"
							onClick={() => {
								try {
									router.push(
										appRouteUrl(
											draft.appId,
											parseAppRouteTarget(draft.path, draft.queryParams),
										),
									);
								} catch (reason) {
									setError(
										reason instanceof Error
											? reason.message
											: "Check the page address.",
									);
								}
							}}
						>
							<ExternalLink className="size-4" />
							Open app page
						</Button>
					)}
					<p className="text-xs leading-relaxed text-muted-foreground">
						iOS uses its own typography and controls when the widget is placed.
						Cached content stays available when Flow Like is closed, with its
						last update time. iOS controls when new content appears.
					</p>
					{draft.kind === "page" && (
						<p className="text-xs leading-relaxed text-muted-foreground">
							Open and use the selected page to capture workflow-driven content.
							Widget refreshes do not run page workflows.
						</p>
					)}
				</div>
			</div>
		</div>
	);
}

function NativeWidgetsForScope({
	scope,
	viewerId,
}: { scope: string; viewerId?: string }) {
	const backend = useBackend();
	const [definitions, setDefinitions] = useState(() =>
		readNativeWidgetDefinitions(scope),
	);
	const [selected, setSelected] = useState<NativeWidgetDefinition>();
	const [content, setContent] = useState(() =>
		readNativeCustomWidgetCache(scope),
	);
	const apps = useQuery({
		queryKey: ["native-widget-settings-apps", scope],
		queryFn: async () => {
			const [profile, apps] = await Promise.all([
				backend.userState.getProfile(),
				backend.appState.getApps(),
			]);
			return apps
				.filter(([app]) =>
					profile.apps?.some((visible) => visible.app_id === app.id),
				)
				.map(([app, meta]) => ({ id: app.id, name: meta?.name || app.id }));
		},
	});
	useEffect(() => {
		const update = (event: Event) => {
			if ((event as CustomEvent<{ scope: string }>).detail?.scope === scope) {
				setDefinitions(readNativeWidgetDefinitions(scope));
				setContent(readNativeCustomWidgetCache(scope));
			}
		};
		window.addEventListener(NATIVE_WIDGETS_CHANGED, update);
		window.addEventListener(NATIVE_WIDGETS_UPDATED, update);
		return () => {
			window.removeEventListener(NATIVE_WIDGETS_CHANGED, update);
			window.removeEventListener(NATIVE_WIDGETS_UPDATED, update);
		};
	}, [scope]);
	const remove = (id: string) => {
		try {
			saveNativeWidgetDefinitions(
				scope,
				readNativeWidgetDefinitions(scope).filter(
					(definition) => definition.id !== id,
				),
			);
			if (selected?.id === id) setSelected(undefined);
			toast.success("Widget removed from the picker.");
		} catch {
			toast.error("The widget could not be removed on this device.");
		}
	};
	return (
		<div className="h-full min-h-0 overflow-auto">
			<div className="mx-auto max-w-6xl space-y-6 px-3 py-4 sm:px-6">
				<Link
					href="/settings"
					className="inline-flex items-center gap-1 text-sm text-muted-foreground"
				>
					<ChevronLeft className="size-4" />
					Settings
				</Link>
				<div className="flex flex-wrap items-start justify-between gap-4">
					<div>
						<h1 className="text-2xl font-semibold tracking-tight">
							Native widgets
						</h1>
						<p className="mt-1 text-sm text-muted-foreground">
							Your data and app pages on the iOS Home Screen.
						</p>
					</div>
					<div className="flex gap-2">
						<Button
							variant="outline"
							disabled={definitions.length >= MAX_NATIVE_WIDGETS}
							onClick={() => setSelected(newNativeWidgetDefinition("page"))}
						>
							<LayoutTemplate className="size-4" />
							App Page
						</Button>
						<Button
							disabled={definitions.length >= MAX_NATIVE_WIDGETS}
							onClick={() => setSelected(newNativeWidgetDefinition("chart"))}
						>
							<BarChart3 className="size-4" />
							Data Chart
						</Button>
					</div>
				</div>
				<div className="flex items-start gap-3 rounded-xl border border-orange-500/20 bg-orange-500/5 p-4">
					<Smartphone className="mt-0.5 size-5 shrink-0 text-orange-500" />
					<p className="text-sm leading-relaxed">
						Save a widget here. On your iPhone, hold the Home Screen, choose{" "}
						<strong>Edit → Add Widget</strong>, and search for{" "}
						<strong>Flow Like</strong>. Add <strong>Data Chart</strong> or{" "}
						<strong>App Page</strong>, then edit it and choose your saved
						widget. Settings are stored for this account and profile on this
						device.
					</p>
				</div>
				{selected ? (
					<WidgetEditor
						key={selected.id}
						initial={selected}
						saved={definitions}
						scope={scope}
						viewerId={viewerId}
						apps={apps.data ?? []}
						onClose={() => setSelected(undefined)}
					/>
				) : (
					<>
						<div className="flex items-center justify-between">
							<h2 className="text-sm font-medium">
								Saved widgets · {definitions.length}/{MAX_NATIVE_WIDGETS}
							</h2>
							<Button
								variant="ghost"
								size="sm"
								disabled={!definitions.length}
								onClick={() => {
									window.dispatchEvent(
										new CustomEvent(NATIVE_WIDGETS_REFRESH, {
											detail: { scope },
										}),
									);
									toast.info(
										"Refreshing widget content while Flow Like is open.",
									);
								}}
							>
								<RefreshCw className="size-3.5" />
								Refresh all
							</Button>
						</div>
						{definitions.length ? (
							<div className="grid gap-3 sm:grid-cols-2">
								{definitions.map((definition) => {
									const state = content.find(
										(entry) =>
											entry.widget.id === definition.id &&
											entry.revision === definition.updatedAt,
									)?.widget;
									return (
										<div
											key={definition.id}
											className="flex items-start gap-3 rounded-xl border bg-card p-4"
										>
											<div className="rounded-lg bg-muted p-2 text-orange-500">
												{definition.kind === "chart" ? (
													<BarChart3 className="size-5" />
												) : (
													<LayoutTemplate className="size-5" />
												)}
											</div>
											<button
												type="button"
												className="min-w-0 flex-1 text-left"
												onClick={() => setSelected(definition)}
											>
												<h3 className="truncate font-medium">
													{definition.title}
												</h3>
												<p className="mt-1 truncate text-xs text-muted-foreground">
													{definition.kind === "chart"
														? "Data Chart"
														: "App Page"}{" "}
													·{" "}
													{apps.data?.find((app) => app.id === definition.appId)
														?.name ?? definition.appId}
												</p>
												<p className="mt-2 text-xs text-muted-foreground">
													{state
														? state.state === "ready"
															? `Updated ${new Date(state.updatedAt).toLocaleString()}`
															: (state.message ?? "Open the app to refresh.")
														: "Waiting for first update…"}
												</p>
											</button>
											<Button
												variant="ghost"
												size="icon"
												aria-label={`Delete ${definition.title}`}
												onClick={() => remove(definition.id)}
											>
												<Trash2 className="size-4" />
											</Button>
										</div>
									);
								})}
							</div>
						) : (
							<div className="rounded-2xl border border-dashed p-10 text-center">
								<BarChart3 className="mx-auto mb-3 size-9 text-orange-500" />
								<h2 className="font-medium">Keep useful information in view</h2>
								<p className="mx-auto mt-2 max-w-md text-sm text-muted-foreground">
									Create a chart from a table or ontology, or display a compact
									part of an app page.
								</p>
							</div>
						)}
					</>
				)}
			</div>
		</div>
	);
}

export function NativeWidgetsSettings() {
	const backend = useBackend();
	const ready = useBackendReady();
	const auth = useAuth();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		ready && !auth.isLoading,
		[],
		0,
	);
	if (ready && !auth.isLoading && profile.isError)
		return (
			<div className="flex h-64 flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
				<p>Your workspace could not be loaded.</p>
				<Button variant="outline" onClick={() => void profile.refetch()}>
					Try again
				</Button>
			</div>
		);
	if (!ready || auth.isLoading || !profile.isSuccess || !profile.data?.id)
		return (
			<div className="flex h-64 items-center justify-center gap-2 text-muted-foreground">
				<Loader2 className="size-5 animate-spin" />
				Loading workspace…
			</div>
		);
	const viewerId = auth.isAuthenticated ? auth.user?.profile.sub : undefined;
	const scope = nativeWidgetScope({ profile: profile.data }, viewerId);
	return (
		<NativeWidgetsForScope key={scope} scope={scope} viewerId={viewerId} />
	);
}

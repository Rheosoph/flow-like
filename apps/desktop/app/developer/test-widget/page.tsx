"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	cn,
	getComponentRenderer,
} from "@flow-like/flow-like-ui";
import type { MicroWidgetInstanceComponent } from "@flow-like/flow-like-ui";
import type {
	WidgetInspection,
	WidgetPreviewBundle,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import { contractDefaults } from "@flow-like/widget-sdk";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
	AlertTriangle,
	ArrowLeft,
	FolderOpen,
	LayoutTemplate,
	Loader2,
	RefreshCw,
	Terminal,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import {
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import type {
	WidgetPropDraftValue,
	WidgetPropsDraft,
} from "../../../lib/widget-props-form";
import {
	createWidgetPropsDraft,
	parseWidgetPropsDraft,
} from "../../../lib/widget-props-form";
import { WidgetPropsForm } from "./props-form";

function contractSummary(widget: WidgetInspection): string {
	return `${widget.inputCount} inputs · ${widget.eventCount} events · ${widget.queryCount} queries`;
}

function WidgetListItem({
	widget,
	selected,
	onSelect,
}: {
	widget: WidgetInspection;
	selected: boolean;
	onSelect: () => void;
}) {
	return (
		<button
			type="button"
			onClick={onSelect}
			aria-pressed={selected}
			className={cn(
				"w-full text-left rounded-lg border p-3 transition-colors",
				selected
					? "border-primary/40 bg-primary/5"
					: "border-border/20 bg-card/50 hover:bg-muted/10 hover:border-border/40",
			)}
		>
			<p className="text-sm font-medium">{widget.name}</p>
			{widget.description && (
				<p className="text-xs text-muted-foreground/70 line-clamp-2 mt-0.5">
					{widget.description}
				</p>
			)}
			<Badge variant="outline" className="mt-2 text-[10px] font-normal">
				{contractSummary(widget)}
			</Badge>
		</button>
	);
}

function WidgetPreviewFrame({
	bundle,
	widget,
	props,
}: {
	bundle: WidgetPreviewBundle;
	widget: WidgetInspection;
	props: Record<string, unknown>;
}) {
	const Renderer = useMemo(
		() => getComponentRenderer("microWidgetInstance"),
		[],
	);
	const instanceId = useMemo(
		() => `dev-preview-${widget.id}-${bundle.bundleHash.slice(0, 8)}`,
		[widget.id, bundle.bundleHash],
	);

	const component = useMemo<MicroWidgetInstanceComponent>(
		() => ({
			id: instanceId,
			type: "microWidgetInstance",
			instanceId,
			packageId: bundle.packageId,
			widgetId: widget.id,
			packageVersion: bundle.packageVersion,
			bundleHash: bundle.bundleHash,
			contract: widget.contract,
			props,
			preview: false,
		}),
		[instanceId, bundle, widget, props],
	);

	if (!Renderer) {
		return (
			<p className="text-sm text-destructive">
				The micro widget renderer is not registered.
			</p>
		);
	}

	return (
		<Renderer
			key={instanceId}
			component={component}
			componentId={instanceId}
			surfaceId="developer-test-widget"
			renderChild={() => null}
		/>
	);
}

function TestWidgetPageContent() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const [projectDir, setProjectDir] = useState(
		searchParams.get("project") ?? "",
	);
	const [bundle, setBundle] = useState<WidgetPreviewBundle | null>(null);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [props, setProps] = useState<Record<string, unknown>>({});
	const [propsDraft, setPropsDraft] = useState<WidgetPropsDraft>({});

	const selectWidget = useCallback((widget: WidgetInspection) => {
		setSelectedId(widget.id);
		const defaults = contractDefaults(widget.contract);
		setProps(defaults);
		setPropsDraft(createWidgetPropsDraft(widget.contract, defaults));
	}, []);

	const loadBundle = useCallback(
		async (dir: string) => {
			if (!dir) return;
			setLoading(true);
			setError(null);
			try {
				const result = await invoke<WidgetPreviewBundle>(
					"developer_prepare_widget_preview",
					{ projectDir: dir },
				);
				setBundle(result);
				const first = result.widgets[0];
				if (first) selectWidget(first);
				else {
					setSelectedId(null);
					setProps({});
					setPropsDraft({});
				}
			} catch (err) {
				setBundle(null);
				setSelectedId(null);
				setProps({});
				setPropsDraft({});
				setError(String(err));
			} finally {
				setLoading(false);
			}
		},
		[selectWidget],
	);

	const initialLoadRef = useRef(false);
	useEffect(() => {
		if (initialLoadRef.current) return;
		initialLoadRef.current = true;
		if (projectDir) void loadBundle(projectDir);
	}, [projectDir, loadBundle]);

	const selectDirectory = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (selected) {
			setProjectDir(selected);
			void loadBundle(selected);
		}
	}, [loadBundle]);

	const selectedWidget =
		bundle?.widgets.find((widget) => widget.id === selectedId) ?? null;
	const selectedInputCount = Object.keys(
		selectedWidget?.contract.inputs ?? {},
	).length;
	const propsValidation = useMemo(
		() =>
			selectedWidget
				? parseWidgetPropsDraft(selectedWidget.contract, propsDraft)
				: { props: {}, errors: {}, valid: false },
		[selectedWidget, propsDraft],
	);

	const updatePropsDraft = useCallback(
		(key: string, value: WidgetPropDraftValue) => {
			setPropsDraft((current) => ({ ...current, [key]: value }));
		},
		[],
	);

	const applyProps = useCallback(() => {
		if (!propsValidation.valid) {
			toast.error("Fix the invalid prop values before applying");
			return;
		}
		setProps(propsValidation.props);
		toast.success("Props applied");
	}, [propsValidation]);

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between py-6">
				<div className="flex items-center gap-4">
					<button
						type="button"
						onClick={() => router.push("/developer")}
						aria-label="Back to developer projects"
						className="h-8 w-8 rounded-full flex items-center justify-center text-muted-foreground/60 hover:text-foreground/80 hover:bg-muted/30 transition-colors"
					>
						<ArrowLeft className="h-4 w-4" />
					</button>
					<div>
						<h1 className="text-2xl font-semibold tracking-tight flex items-center gap-2">
							<LayoutTemplate className="h-6 w-6" />
							Test Widget
						</h1>
						<p className="text-sm text-muted-foreground/70">
							Render your project's built widgets through the real sandboxed
							host
						</p>
					</div>
				</div>
				<div className="flex items-center gap-2">
					<Button
						variant="outline"
						size="sm"
						onClick={selectDirectory}
						className="gap-1.5"
					>
						<FolderOpen className="h-4 w-4" />
						{projectDir ? "Change Project" : "Select Project"}
					</Button>
					{projectDir && (
						<Button
							variant="outline"
							size="sm"
							onClick={() => void loadBundle(projectDir)}
							disabled={loading}
							className="gap-1.5"
						>
							{loading ? (
								<Loader2 className="h-4 w-4 animate-spin" />
							) : (
								<RefreshCw className="h-4 w-4" />
							)}
							Reload
						</Button>
					)}
				</div>
			</div>

			{projectDir && (
				<p className="text-xs text-muted-foreground/60 font-mono pb-4 truncate">
					{projectDir}
				</p>
			)}

			<div className="flex-1 overflow-y-auto min-h-0 pb-12">
				{!projectDir && (
					<Card className="max-w-md mx-auto mt-12">
						<CardHeader>
							<CardTitle>No Project Selected</CardTitle>
							<CardDescription>
								Pick a local project directory containing a built{" "}
								<code>widgets.flwb</code> to preview its widgets.
							</CardDescription>
						</CardHeader>
						<CardContent>
							<Button onClick={selectDirectory} className="gap-1.5">
								<FolderOpen className="h-4 w-4" />
								Select Project Directory
							</Button>
						</CardContent>
					</Card>
				)}

				{projectDir && error && !loading && (
					<Card className="max-w-xl mx-auto mt-12 border-destructive/40 bg-destructive/5">
						<CardContent className="flex items-start gap-2 p-4 text-sm">
							<AlertTriangle className="h-5 w-5 text-destructive mt-0.5 shrink-0" />
							<div className="min-w-0">
								<p className="font-medium text-destructive">
									Failed to prepare widget preview
								</p>
								<p className="text-muted-foreground mt-1 wrap-break-word">
									{error}
								</p>
								<p className="text-muted-foreground mt-2">
									Build the bundle first: <code>mise run build</code> in the
									project directory.
								</p>
							</div>
						</CardContent>
					</Card>
				)}

				{loading && !bundle && (
					<div className="flex justify-center mt-12">
						<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
					</div>
				)}

				{bundle && (
					<div className="grid grid-cols-[260px_1fr] gap-6 items-start">
						<div className="space-y-3">
							<div className="flex items-baseline justify-between">
								<h3 className="text-sm font-medium">Widgets</h3>
								<span className="text-xs text-muted-foreground/60">
									{bundle.widgets.length}
								</span>
							</div>
							{bundle.widgets.map((widget) => (
								<WidgetListItem
									key={widget.id}
									widget={widget}
									selected={widget.id === selectedId}
									onSelect={() => selectWidget(widget)}
								/>
							))}
							<div className="rounded-lg border border-border/20 bg-muted/5 p-3 text-xs text-muted-foreground/70 flex items-start gap-2">
								<Terminal className="h-3.5 w-3.5 mt-0.5 shrink-0" />
								<span>
									For the rich dev loop (HMR, props panel, event log) run{" "}
									<code>flow-like-widgets dev</code> in the project.
								</span>
							</div>
						</div>

						<div className="space-y-4 min-w-0">
							{selectedWidget ? (
								<>
									<div className="rounded-xl border border-border/20 bg-card/50 p-4">
										<WidgetPreviewFrame
											bundle={bundle}
											widget={selectedWidget}
											props={props}
										/>
									</div>

									<form
										className="space-y-3"
										noValidate
										onSubmit={(event) => {
											event.preventDefault();
											applyProps();
										}}
									>
										<div className="flex items-end justify-between gap-4">
											<div className="space-y-1">
												<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
													Props
												</h2>
												<p className="text-xs text-muted-foreground/60">
													Generated from the widget's bundled type contract.
												</p>
											</div>
											<Button
												type="submit"
												size="sm"
												variant="outline"
												disabled={
													selectedInputCount === 0 || !propsValidation.valid
												}
											>
												Apply
											</Button>
										</div>
										<WidgetPropsForm
											contract={selectedWidget.contract}
											draft={propsDraft}
											errors={propsValidation.errors}
											onChange={updatePropsDraft}
										/>
										<p className="text-xs text-muted-foreground/60">
											Structured props use JSON and are checked against their
											schema before the update is applied.
										</p>
									</form>
								</>
							) : (
								<Card>
									<CardContent className="p-6 text-sm text-muted-foreground">
										The bundle contains no widgets.
									</CardContent>
								</Card>
							)}
						</div>
					</div>
				)}
			</div>
		</div>
	);
}

export default function TestWidgetPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<Loader2 className="h-6 w-6 animate-spin text-muted-foreground/60" />
				</div>
			}
		>
			<TestWidgetPageContent />
		</Suspense>
	);
}

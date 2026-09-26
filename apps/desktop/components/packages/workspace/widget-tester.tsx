"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	cn,
	getComponentRenderer,
} from "@flow-like/flow-like-ui";
import type { MicroWidgetInstanceComponent } from "@flow-like/flow-like-ui";
import type {
	WidgetInspection,
	WidgetPreviewBundle,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import { Trans, i18n as i18next, useTranslation } from "@flow-like/locales";
import { contractDefaults } from "@flow-like/widget-sdk";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Loader2, RefreshCw, Terminal } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
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
	return [
		i18next.t("countInputs", {
			defaultValue_one: "{{count}} input",
			defaultValue_other: "{{count}} inputs",
			count: widget.inputCount,
		}),
		i18next.t("countEvents", {
			defaultValue_one: "{{count}} event",
			defaultValue_other: "{{count}} events",
			count: widget.eventCount,
		}),
		i18next.t("countQueries", {
			defaultValue_one: "{{count}} query",
			defaultValue_other: "{{count}} queries",
			count: widget.queryCount,
		}),
	].join(" · ");
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
	const { t } = useTranslation("common");
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
				{t(
					"theMicroWidgetRendererIsNotRegistered",
					"The micro widget renderer is not registered.",
				)}
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

export interface WidgetTesterProps {
	projectPath: string;
}

/** Renders the checkout's built `widgets.flwb` through the real sandboxed host. */
export function WidgetTester({ projectPath }: Readonly<WidgetTesterProps>) {
	const { t } = useTranslation("common");
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

	const clearSelection = useCallback(() => {
		setSelectedId(null);
		setProps({});
		setPropsDraft({});
	}, []);

	const loadBundle = useCallback(async () => {
		setLoading(true);
		setError(null);
		try {
			const result = await invoke<WidgetPreviewBundle>(
				"developer_prepare_widget_preview",
				{ projectDir: projectPath },
			);
			setBundle(result);
			const first = result.widgets[0];
			if (first) selectWidget(first);
			else clearSelection();
		} catch (err) {
			setBundle(null);
			clearSelection();
			setError(String(err));
		} finally {
			setLoading(false);
		}
	}, [projectPath, selectWidget, clearSelection]);

	useEffect(() => {
		void loadBundle();
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
			toast.error(
				t(
					"fixTheInvalidPropValuesBeforeApplying",
					"Fix the invalid prop values before applying",
				),
			);
			return;
		}
		setProps(propsValidation.props);
		toast.success(t("propsApplied", "Props applied"));
	}, [propsValidation, t]);

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between gap-3">
				<h3 className="text-sm font-medium">
					{t("widgets", "Widgets")}
					{bundle && (
						<span className="ml-2 text-xs text-muted-foreground/60">
							{bundle.widgets.length}
						</span>
					)}
				</h3>
				<Button
					variant="outline"
					size="sm"
					onClick={() => void loadBundle()}
					disabled={loading}
					className="gap-1.5"
				>
					{loading ? (
						<Loader2 className="h-4 w-4 animate-spin" />
					) : (
						<RefreshCw className="h-4 w-4" />
					)}
					{t("reload", "Reload")}
				</Button>
			</div>

			{error && !loading && (
				<Card className="border-destructive/40 bg-destructive/5">
					<CardContent className="flex items-start gap-2 p-4 text-sm">
						<AlertTriangle className="h-5 w-5 text-destructive mt-0.5 shrink-0" />
						<div className="min-w-0">
							<p className="font-medium text-destructive">
								{t(
									"failedToPrepareWidgetPreview",
									"Failed to prepare widget preview",
								)}
							</p>
							<p className="text-muted-foreground mt-1 wrap-break-word">
								{error}
							</p>
							<p className="text-muted-foreground mt-2">
								<Trans i18nKey="buildTheBundleFirstCodemiseRunBuildcodeInTheProjectDirectory">
									Build the bundle first: <code>mise run build</code> in the
									project directory.
								</Trans>
							</p>
						</div>
					</CardContent>
				</Card>
			)}

			{loading && !bundle && (
				<div className="flex justify-center py-12">
					<Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
				</div>
			)}

			{bundle && (
				<div className="grid grid-cols-1 items-start gap-6 md:grid-cols-[260px_1fr]">
					<div className="space-y-3">
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
								{t(
									"forTheRichDevLoopHmrPropsPanelEventLogRun",
									"For the rich dev loop (HMR, props panel, event log) run",
								)}{" "}
								<Trans i18nKey="codeflowlikewidgetsDevcodeInTheProject">
									<code>flow-like-widgets dev</code> in the project.
								</Trans>
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
												{t("props", "Props")}
											</h2>
											<p className="text-xs text-muted-foreground/60">
												{`Generated from the widget's bundled type contract.`}
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
											{t("apply", "Apply")}
										</Button>
									</div>
									<WidgetPropsForm
										contract={selectedWidget.contract}
										draft={propsDraft}
										errors={propsValidation.errors}
										onChange={updatePropsDraft}
									/>
									<p className="text-xs text-muted-foreground/60">
										{t(
											"structuredPropsUseJsonAndAreCheckedAgainstTheirSchemaBeforeTheUpdateIsApplied",
											"Structured props use JSON and are checked against their schema before the update is applied.",
										)}
									</p>
								</form>
							</>
						) : (
							<Card>
								<CardContent className="p-6 text-sm text-muted-foreground">
									{t(
										"theBundleContainsNoWidgets",
										"The bundle contains no widgets.",
									)}
								</CardContent>
							</Card>
						)}
					</div>
				</div>
			)}
		</div>
	);
}

"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import {
	BinaryIcon,
	ChevronRightIcon,
	CpuIcon,
	DownloadIcon,
	type LucideIcon,
	MicIcon,
	ShieldAlertIcon,
	ShieldCheckIcon,
	ShieldOffIcon,
	TriangleAlertIcon,
	Volume2Icon,
} from "lucide-react";
import { Fragment, useEffect, useId, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../ui/collapsible";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "../ui/dialog";
import {
	WIDGET_CAPABILITY_TONE,
	WIDGET_CONSENT_VISIBLE_KNOWN_CARDS,
	WIDGET_FULL_ROW_CAPABILITIES,
	type WidgetConsentSource,
	type WidgetConsentView,
	buildWidgetConsentView,
	widgetConsentViewSignature,
} from "./micro-widget-consent-view";
import {
	type Translate,
	WIDGET_CAPABILITY_COPY,
	WIDGET_DIRECTIVE_LABELS,
	widgetBannerLead,
	widgetBannerSuffix,
	widgetConsentTitle,
	widgetPolicySourceLabel,
	widgetSourceAbout,
	widgetSourceRisk,
} from "./micro-widget-network-copy";
import type {
	MicroWidgetCapability,
	WidgetPolicy,
} from "./micro-widget-policy";
import {
	WidgetHostCopy,
	WidgetPurposeCard,
	WidgetSourceLevelBadge,
} from "./micro-widget-purpose-card";
import {
	type MicroWidgetCspSource,
	describeMicroWidgetSandbox,
	isMicroWidgetLocalSource,
} from "./micro-widget-sandbox-policy";
import type {
	MicroWidgetConsentPrompt,
	MicroWidgetGrantActions,
} from "./use-micro-widget-grant";
import { useMicroWidgetInertWindow } from "./use-micro-widget-inert-window";

const CAPABILITY_ICONS: Readonly<Record<MicroWidgetCapability, LucideIcon>> = {
	microphone: MicIcon,
	downloads: DownloadIcon,
	media: Volume2Icon,
	workers: CpuIcon,
	wasm: BinaryIcon,
};

function distinct(values: readonly string[]): string[] {
	return [...new Set(values)];
}

function ConsentBanner({ view }: { view: WidgetConsentView }) {
	const { t } = useTranslation("common");
	const text = [
		widgetBannerLead(view.lead, t),
		...(view.hasNetwork ? [widgetBannerSuffix(t)] : []),
	].join(" ");
	const broad = view.networkLevel === "broad";
	return (
		<DialogPrimitive.Description asChild>
			{broad ? (
				<div
					className="flex items-start gap-2 rounded-md border border-destructive/50 px-3 py-2 text-sm text-destructive"
					data-widget-consent-banner="broad"
				>
					<TriangleAlertIcon
						aria-hidden="true"
						className="mt-0.5 size-4 shrink-0"
					/>
					<span>{text}</span>
				</div>
			) : (
				<p className="text-sm" data-widget-consent-banner={view.tone}>
					{text}
				</p>
			)}
		</DialogPrimitive.Description>
	);
}

function CapabilityRows({
	capabilities,
}: {
	capabilities: readonly MicroWidgetCapability[];
}) {
	const { t } = useTranslation("common");
	const full = capabilities.filter((name) =>
		WIDGET_FULL_ROW_CAPABILITIES.includes(name),
	);
	const compact = capabilities.filter(
		(name) => !WIDGET_FULL_ROW_CAPABILITIES.includes(name),
	);
	return (
		<>
			{full.map((name) => {
				const Icon = CAPABILITY_ICONS[name];
				return (
					<div
						key={name}
						className="flex items-start gap-2 rounded-md border p-2.5"
						data-widget-capability={name}
					>
						<Icon
							aria-hidden="true"
							className={cn(
								"mt-0.5 size-4 shrink-0",
								WIDGET_CAPABILITY_TONE[name] === "broad"
									? "text-destructive"
									: "text-muted-foreground",
							)}
						/>
						<div className="flex min-w-0 flex-col gap-0.5">
							<p className="text-sm font-medium">
								{WIDGET_CAPABILITY_COPY[name].label(t)}
							</p>
							<p className="text-xs text-muted-foreground">
								{WIDGET_CAPABILITY_COPY[name].description(t)}
							</p>
						</div>
					</div>
				);
			})}
			{compact.length > 0 && (
				<div
					className="flex flex-wrap items-center gap-1.5 text-xs"
					data-widget-capabilities-also
				>
					<span className="text-muted-foreground">
						{t("widgetCapabilityAlsoAsksFor", "Also asks for")}
					</span>
					{compact.map((name) => {
						const Icon = CAPABILITY_ICONS[name];
						return (
							<Badge key={name} variant="outline" data-widget-capability={name}>
								<Icon aria-hidden="true" />
								{WIDGET_CAPABILITY_COPY[name].label(t)}
							</Badge>
						);
					})}
				</div>
			)}
		</>
	);
}

function sourceLabel(source: MicroWidgetCspSource, t: Translate): string {
	return source === "bundle"
		? t("widgetCapabilitySourceBundle", "widget bundle")
		: source;
}

function EffectiveSandbox({
	policy,
	localMedia,
}: {
	policy: WidgetPolicy;
	localMedia: boolean | undefined;
}) {
	const { t } = useTranslation("common");
	const sandbox = useMemo(
		() =>
			describeMicroWidgetSandbox(
				policy,
				localMedia === undefined ? {} : { localMedia },
			),
		[policy, localMedia],
	);
	return (
		<section className="flex flex-col gap-2">
			<h3 className="text-xs font-medium">
				{t("widgetCapabilityEffectiveSandbox", "Effective sandbox")}
			</h3>
			<div className="flex flex-wrap items-center gap-1.5 text-xs">
				<span className="font-mono text-muted-foreground">
					{t("widgetCapabilityIframeSandbox", "iframe sandbox")}
				</span>
				{sandbox.sandbox.map(({ token, grantedBy }) => (
					<Badge
						key={token}
						variant={grantedBy ? "secondary" : "outline"}
						className="font-mono"
					>
						{token}
					</Badge>
				))}
			</div>
			<div className="overflow-x-auto rounded-md border">
				<table className="w-full text-xs">
					<thead className="bg-muted/50 text-muted-foreground">
						<tr>
							<th className="px-2 py-1.5 text-left font-medium">
								{t("widgetCapabilityCspDirective", "Directive")}
							</th>
							<th className="px-2 py-1.5 text-left font-medium">
								{t("widgetCapabilityCspAllowed", "Allowed sources")}
							</th>
						</tr>
					</thead>
					<tbody>
						{sandbox.csp.map(({ directive, sources, grantedBy }) => {
							const lastLocal = sources.findLastIndex(isMicroWidgetLocalSource);
							return (
								<tr key={directive} className="border-t">
									<td className="px-2 py-1.5 align-top font-mono whitespace-nowrap">
										{directive}
									</td>
									<td className="px-2 py-1.5">
										<div className="flex flex-wrap items-center gap-1">
											{sources.map((source, index) => (
												<Fragment key={source}>
													<Badge
														variant={grantedBy ? "secondary" : "outline"}
														className="font-mono break-all whitespace-normal"
													>
														{sourceLabel(source, t)}
													</Badge>
													{index === lastLocal && (
														<span
															className="text-muted-foreground"
															data-widget-local-only
														>
															{t(
																"widgetCapabilitySourceLocalOnly",
																"local data only (no network)",
															)}
														</span>
													)}
												</Fragment>
											))}
											{grantedBy && (
												<span className="text-muted-foreground">
													{t(
														"widgetCapabilityGrantedBy",
														"granted by {{capability}}",
														{
															capability:
																WIDGET_CAPABILITY_COPY[grantedBy].label(t),
														},
													)}
												</span>
											)}
										</div>
									</td>
								</tr>
							);
						})}
					</tbody>
				</table>
			</div>
			<p className="text-xs text-muted-foreground" data-widget-sandbox-note>
				{t(
					"widgetCapabilitySandboxNote",
					"The policy above limits which sites the widget can load from or send data to. Browsers cannot block every channel, for example WebRTC, preconnect or file downloads, so only allow widgets from publishers you trust.",
				)}
			</p>
		</section>
	);
}

function SourceRow({ source }: { source: WidgetConsentSource }) {
	const { t } = useTranslation("common");
	const about = widgetSourceAbout(source, t);
	const risk = widgetSourceRisk(source, t);
	return (
		<li
			className="flex flex-col gap-1 p-2 text-xs"
			data-widget-source-row={source.source}
		>
			<div className="flex flex-wrap items-center gap-1.5">
				<bdi dir="ltr" translate="no" className="font-mono break-all">
					{source.source}
				</bdi>
				<WidgetSourceLevelBadge level={source.level} />
				<span className="text-muted-foreground">
					{source.directives
						.map((key) => WIDGET_DIRECTIVE_LABELS[key](t))
						.join(" · ")}
				</span>
			</div>
			<WidgetHostCopy>{about ? `${about} ${risk}` : risk}</WidgetHostCopy>
			{source.runtime && (
				<>
					{source.slot && (
						<p className="text-muted-foreground">
							{t("widgetRuntimeInputLabel", "Inputs", { count: 1 })}{" "}
							<code className="rounded bg-muted px-1 font-mono text-foreground">
								{source.slot}
							</code>
						</p>
					)}
					<p className="text-muted-foreground">
						{t(
							"widgetRuntimeSourceDetail",
							"This address reached the widget while the app was running. It can come from the app's pages or workflows, including nodes from this widget's publisher. Neither Flow-Like nor the app's admins reviewed it. Your choice applies only to you.",
						)}
					</p>
				</>
			)}
		</li>
	);
}

function ConsentDetails({
	prompt,
	view,
}: {
	prompt: MicroWidgetConsentPrompt;
	view: WidgetConsentView;
}) {
	const { t } = useTranslation("common");
	const network = prompt.descriptor?.network;
	const asksForNetwork = view.sources.some((source) => source.shown);
	const compactCapabilities = view.capabilities.filter(
		(name) => !WIDGET_FULL_ROW_CAPABILITIES.includes(name),
	);
	return (
		<Collapsible>
			<CollapsibleTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="group h-auto min-h-6 w-fit gap-1 px-1 text-xs"
				>
					<ChevronRightIcon
						aria-hidden="true"
						className="size-3.5 group-data-[state=open]:rotate-90"
					/>
					{t("details", "Details")}
				</Button>
			</CollapsibleTrigger>
			<CollapsibleContent
				className="flex flex-col gap-3 pt-2"
				data-widget-consent-details
			>
				{view.sources.length > 0 && (
					<section className="flex flex-col gap-1.5">
						<h3 className="text-xs font-medium">
							{t("widgetDetailsSourcesHeading", "Every address")}
						</h3>
						<ul className="flex flex-col divide-y rounded-md border">
							{view.sources.map((source) => (
								<SourceRow
									key={`${source.runtime ? "runtime" : "declared"} ${source.source}`}
									source={source}
								/>
							))}
						</ul>
					</section>
				)}
				{asksForNetwork && (
					<ul className="flex list-disc flex-col gap-1 pl-4 text-xs text-muted-foreground">
						{view.loadsPassively && (
							<li>
								{t(
									"widgetNetworkPassiveLoadNote",
									"Loading images, fonts, media or stylesheets also sends data: the widget can put it in the request address.",
								)}
							</li>
						)}
						{view.usesWebSockets && (
							<li>
								{t(
									"widgetNetworkWebSocketNote",
									"On some platforms a wss:// site is also reachable over https://.",
								)}
							</li>
						)}
						<li>
							{t(
								"widgetNetworkCrossWidgetNote",
								"Other widgets on this page may be able to pass data to this widget, which it could then send to these sites.",
							)}
						</li>
					</ul>
				)}
				{compactCapabilities.length > 0 && (
					<ul className="flex flex-col gap-1 text-xs">
						{compactCapabilities.map((name) => (
							<li key={name}>
								<span className="font-medium">
									{WIDGET_CAPABILITY_COPY[name].label(t)}
								</span>
								<span className="text-muted-foreground">
									{" · "}
									{WIDGET_CAPABILITY_COPY[name].description(t)}
								</span>
							</li>
						))}
					</ul>
				)}
				<EffectiveSandbox
					policy={prompt.subject.policy}
					localMedia={prompt.descriptor?.engine?.localMedia}
				/>
				{network && (
					<p className="text-xs text-muted-foreground">
						{t(
							"widgetSourceDataVersion",
							"Classification data {{catalogVersion}} / {{pslVersion}}",
							{
								catalogVersion: network.catalogVersion,
								pslVersion: network.pslVersion,
							},
						)}
					</p>
				)}
			</CollapsibleContent>
		</Collapsible>
	);
}

function PurposeCards({ view }: { view: WidgetConsentView }) {
	const { t } = useTranslation("common");
	const [showAllKnown, setShowAllKnown] = useState(false);
	const collapsedKnown = view.cards
		.filter((card) => card.level === "known")
		.slice(WIDGET_CONSENT_VISIBLE_KNOWN_CARDS);
	const hidden = new Set(
		showAllKnown ? [] : collapsedKnown.map((card) => card.index),
	);
	const names = distinct(
		collapsedKnown.flatMap((card) =>
			card.sources.map((source) => source.provider ?? source.emphasis),
		),
	);
	return (
		<>
			{view.cards
				.filter((card) => !hidden.has(card.index))
				.map((card) => (
					<WidgetPurposeCard key={card.index} card={card} />
				))}
			{collapsedKnown.length > 0 && (
				<div className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
					<Button
						type="button"
						variant="link"
						size="sm"
						className="h-auto min-h-6 px-0 text-xs"
						aria-expanded={showAllKnown}
						onClick={() => setShowAllKnown((open) => !open)}
					>
						{showAllKnown
							? t("showLess", "Show less")
							: t(
									"widgetNetworkMoreServices",
									"{{count}} more identified services",
									{ count: collapsedKnown.length },
								)}
					</Button>
					{!showAllKnown && <span translate="no">{names.join(", ")}</span>}
				</div>
			)}
		</>
	);
}

export interface MicroWidgetConsentDialogProps {
	prompt: MicroWidgetConsentPrompt;
	actions: Pick<
		MicroWidgetGrantActions,
		| "allowOnce"
		| "allowForProject"
		| "dontAllow"
		| "stopAsking"
		| "setIncludeRuntime"
		| "showNewerPrompt"
	>;
	/** 1-based place in the document's consent queue. */
	position?: number;
	total?: number;
	/** Called once the dialog closed after a decision; focus the widget or its Review button. */
	onRestoreFocus?: () => void;
}

/**
 * Consent for what a package widget asks for (§14.5.2–§14.5.3): Flow-Like's
 * classification first, one card per purpose, then capabilities and Details.
 * Calm below `broad`; at `broad` Don't allow is primary and focused and
 * "Always allow" needs an inline confirm. Allow controls are inert for a
 * moment after anything could have moved them under the pointer. Closing the
 * dialog in any way counts as Don't allow.
 */
export function MicroWidgetConsentDialog({
	prompt,
	actions,
	position = 1,
	total = 1,
	onRestoreFocus,
}: MicroWidgetConsentDialogProps) {
	const { t } = useTranslation("common");
	const view = useMemo(() => buildWidgetConsentView(prompt), [prompt]);
	const broad = view.tone === "broad";
	const { inert, isInert, restart } = useMicroWidgetInertWindow();
	const [confirming, setConfirming] = useState(false);
	const showConfirm = confirming && broad;
	const decided = useRef(false);
	const returnToAlways = useRef(false);
	const titleRef = useRef<HTMLHeadingElement>(null);
	const dontAllowRef = useRef<HTMLButtonElement>(null);
	const alwaysRef = useRef<HTMLButtonElement>(null);
	const cancelRef = useRef<HTMLButtonElement>(null);
	const checkboxId = useId();

	const contentKey = widgetConsentViewSignature(prompt, view);
	useEffect(() => {
		if (contentKey) restart();
	}, [contentKey, restart]);

	useEffect(() => {
		if (showConfirm) {
			cancelRef.current?.focus();
			restart();
		} else if (returnToAlways.current) {
			returnToAlways.current = false;
			alwaysRef.current?.focus();
		}
	}, [showConfirm, restart]);

	const decide = (action: (promptKey: string) => void) => {
		decided.current = true;
		setConfirming(false);
		action(prompt.key);
	};
	const allow = (action: (promptKey: string) => void) => () => {
		if (!isInert()) decide(action);
	};
	const cancelConfirm = () => {
		returnToAlways.current = true;
		setConfirming(false);
	};
	const openConfirm = () => {
		if (!isInert()) setConfirming(true);
	};
	const showNewer = () => {
		actions.showNewerPrompt();
		titleRef.current?.focus();
		restart();
	};

	const { widgetId, packageId, source } = prompt.subject;
	const inertProps = inert ? { "aria-disabled": true } : {};
	const runtimeShown = view.cards.some((card) => card.runtime.length > 0);
	const confirmText =
		view.classified && view.top?.level === "broad"
			? t(
					"widgetConsentBroadConfirm",
					"Every time this project opens the widget, anyone who signs up with {{provider}} could receive what it sends.",
					{ provider: view.top.provider ?? view.top.emphasis },
				)
			: t(
					"widgetConsentBroadConfirmGeneric",
					"Every time this project opens the widget, it gets these permissions without asking again.",
				);

	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (open) return;
				if (showConfirm) cancelConfirm();
				else decide(actions.dontAllow);
			}}
		>
			<DialogContent
				className="gap-3 sm:max-w-xl"
				data-widget-consent-dialog={prompt.mode}
				data-widget-consent-tone={view.tone}
				onOpenAutoFocus={(event) => {
					event.preventDefault();
					(broad ? dontAllowRef.current : titleRef.current)?.focus();
				}}
				onCloseAutoFocus={(event) => {
					event.preventDefault();
					if (decided.current) onRestoreFocus?.();
				}}
				onPointerDownOutside={(event) => event.preventDefault()}
			>
				<DialogHeader className="gap-1 pr-8 text-left">
					<div className="flex items-start gap-2">
						<ShieldAlertIcon
							aria-hidden="true"
							className={cn(
								"mt-0.5 size-5 shrink-0",
								broad ? "text-destructive" : "text-primary",
							)}
						/>
						<DialogTitle
							ref={titleRef}
							tabIndex={-1}
							className="min-w-0 flex-1 text-base leading-snug outline-none"
							data-widget-consent-title={view.title}
						>
							{widgetConsentTitle(view.title, t)}
						</DialogTitle>
						{total > 1 && (
							<span
								className="shrink-0 pt-0.5 text-xs text-muted-foreground"
								data-widget-consent-position
							>
								{t("widgetConsentQueuePosition", "{{position}} of {{total}}", {
									position,
									total,
								})}
							</span>
						)}
					</div>
					<p
						className="text-xs wrap-break-word text-muted-foreground"
						data-widget-policy-source={source}
					>
						<Trans
							i18nKey="widgetConsentSubject"
							defaults="<0>{{widgetId}}</0> from <1>{{packageId}}</1>"
							values={{ widgetId, packageId }}
							components={[
								<bdi key="widget" className="font-mono" />,
								<bdi key="package" className="font-mono" />,
							]}
						/>
						<span aria-hidden="true"> · </span>
						{t("widgetPolicySource", "Source: {{source}}", {
							source: widgetPolicySourceLabel(source, t),
						})}
					</p>
				</DialogHeader>

				<ConsentBanner view={view} />

				<div className="-mx-1 flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto overscroll-contain px-1">
					<PurposeCards view={view} />
					{view.alreadyAllowed > 0 && (
						<p className="text-xs text-muted-foreground">
							{t(
								"widgetConsentAlreadyAllowed",
								"Already allowed: {{count}} sites",
								{
									count: view.alreadyAllowed,
								},
							)}
						</p>
					)}
					{view.wildcardUnsupported && (
						<p className="text-xs text-muted-foreground">
							{t(
								"widgetSourceEngineUnsupported",
								"This browser can't allow addresses with a wildcard, so the widget runs without them.",
							)}
						</p>
					)}
					<CapabilityRows capabilities={view.capabilities} />
					<ConsentDetails prompt={prompt} view={view} />
				</div>

				<div
					className="flex shrink-0 flex-col gap-2 border-t pt-3"
					data-widget-consent-footer
				>
					{prompt.hasRuntimeCheckbox && (
						<div className="flex min-h-6 items-center gap-2">
							<Checkbox
								id={checkboxId}
								checked={prompt.includeRuntime}
								onCheckedChange={(checked) =>
									actions.setIncludeRuntime(checked === true)
								}
								data-widget-runtime-checkbox
							/>
							<label htmlFor={checkboxId} className="text-sm">
								{t(
									"widgetRuntimeIncludeSources",
									"Also allow the {{count}} addresses provided while the app runs",
									{ count: view.runtimeCount },
								)}
							</label>
						</div>
					)}
					{prompt.newerAvailable && (
						<output
							className="flex flex-wrap items-center gap-x-1 text-xs"
							data-widget-consent-frozen
						>
							{t(
								"widgetRuntimeSourcesChanged",
								"The widget received different addresses while this was open.",
							)}
							<Button
								type="button"
								variant="link"
								size="sm"
								className="h-auto min-h-6 px-0 text-xs"
								onClick={showNewer}
							>
								{t("widgetRuntimeSourcesShowNew", "Show them")}
							</Button>
						</output>
					)}
					{showConfirm ? (
						<div className="flex flex-col gap-2" data-widget-consent-confirm>
							<p className="text-sm">{confirmText}</p>
							<div className="flex flex-col gap-2 sm:flex-row sm:justify-end">
								<Button
									ref={cancelRef}
									type="button"
									className="w-full sm:w-auto"
									onClick={cancelConfirm}
								>
									{t("cancel", "Cancel")}
								</Button>
								<Button
									type="button"
									variant="outline"
									className="w-full gap-1.5 sm:w-auto"
									onClick={allow(actions.allowForProject)}
									{...inertProps}
								>
									<ShieldCheckIcon aria-hidden="true" />
									{t("widgetConsentConfirmAlwaysAllow", "Always allow")}
								</Button>
							</div>
						</div>
					) : (
						<div className="flex flex-col gap-2 sm:flex-row sm:items-center">
							{prompt.canStopAsking && (
								<Button
									type="button"
									variant="link"
									size="sm"
									className="h-auto min-h-6 justify-start px-0 text-xs sm:mr-auto"
									onClick={() => decide(actions.stopAsking)}
								>
									{t("widgetRuntimeStopAsking", "Stop asking for this widget")}
								</Button>
							)}
							<div className="flex flex-col gap-2 sm:ml-auto sm:flex-row">
								<Button
									ref={dontAllowRef}
									type="button"
									variant={broad ? "default" : "outline"}
									className="w-full gap-1.5 sm:w-auto"
									onClick={() => decide(actions.dontAllow)}
								>
									<ShieldOffIcon aria-hidden="true" />
									{t("widgetConsentDontAllow", "Don't allow")}
								</Button>
								<Button
									type="button"
									variant={broad ? "outline" : "default"}
									className="w-full sm:w-auto"
									onClick={allow(actions.allowOnce)}
									{...inertProps}
								>
									{t("widgetCapabilityAllowOnce", "Allow this time")}
								</Button>
								{prompt.canAllowForProject && (
									<Button
										ref={alwaysRef}
										type="button"
										variant="outline"
										className="w-full gap-1.5 sm:w-auto"
										onClick={
											broad ? openConfirm : allow(actions.allowForProject)
										}
										{...inertProps}
									>
										<ShieldCheckIcon aria-hidden="true" />
										{broad
											? t(
													"widgetCapabilityAllowForProjectEllipsis",
													"Always allow for this project…",
												)
											: t(
													"widgetCapabilityAllowForProject",
													"Always allow for this project",
												)}
									</Button>
								)}
							</div>
						</div>
					)}
					<div className="flex flex-col gap-1 text-xs text-muted-foreground">
						{prompt.mode === "runtime" && (
							<p>
								{t("widgetRuntimeReloadNote", "Allowing reloads the widget.")}
							</p>
						)}
						{prompt.canAllowForProject ? (
							<>
								<p>
									{t(
										"widgetCapabilityRememberNote",
										'"Always allow" is stored on this device for this project only. If the widget later asks for another capability or site, you will be asked again. You can revoke it in the project\'s widget permissions.',
									)}
								</p>
								{runtimeShown && (
									<p>
										{t(
											"widgetConsentRememberRuntimeNote",
											"Addresses provided while the app runs are remembered for this project until you revoke them.",
										)}
									</p>
								)}
							</>
						) : (
							<p>
								{t(
									"widgetConsentSessionOnlyNote",
									"This widget is shown outside a project, so your choice lasts until you reload the page or restart Flow-Like.",
								)}
							</p>
						)}
					</div>
				</div>
			</DialogContent>
		</Dialog>
	);
}

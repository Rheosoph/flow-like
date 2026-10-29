"use client";

import { useTranslation } from "@flow-like/locales";
import {
	GlobeIcon,
	ShieldOffIcon,
	ShieldQuestionIcon,
	TriangleAlertIcon,
	XIcon,
} from "lucide-react";
import type { ReactNode, Ref } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import {
	type Translate,
	widgetDeclinedPermissions,
	widgetRuntimeIssueReason,
} from "./micro-widget-network-copy";
import type {
	MicroWidgetGrant,
	MicroWidgetGrantNotice,
	MicroWidgetRuntimeRequest,
} from "./use-micro-widget-grant";
import { useMicroWidgetInertWindow } from "./use-micro-widget-inert-window";

const BAR =
	"absolute top-2 left-2 right-2 z-40 flex items-center gap-2 rounded-md border bg-background/95 px-2 py-1 text-xs shadow-sm";

function DismissButton({ onDismiss }: { onDismiss: () => void }) {
	const { t } = useTranslation("common");
	return (
		<Button
			type="button"
			variant="ghost"
			size="icon"
			className="size-6 shrink-0"
			onClick={onDismiss}
			aria-label={t("close", "Close")}
		>
			<XIcon aria-hidden="true" className="size-3" />
		</Button>
	);
}

function LinkButton({
	children,
	onClick,
	...props
}: {
	children: ReactNode;
	onClick?: () => void;
	"aria-disabled"?: boolean;
	ref?: Ref<HTMLButtonElement>;
}) {
	return (
		<Button
			type="button"
			variant="link"
			size="sm"
			className="h-auto min-h-6 shrink-0 px-1 text-xs"
			onClick={onClick}
			{...props}
		>
			{children}
		</Button>
	);
}

/**
 * Non-modal request while the widget runs (§14.5.4): the frame keeps running
 * without the new addresses until the viewer reviews them. Review is inert
 * for a moment after the banner appears.
 */
export function MicroWidgetRuntimeBanner({
	request,
	onReview,
	onDismiss,
}: {
	request: MicroWidgetRuntimeRequest;
	onReview: () => void;
	onDismiss: () => void;
}) {
	const { t } = useTranslation("common");
	const { inert, isInert } = useMicroWidgetInertWindow();
	const broad = request.level === "broad";
	const Icon = broad ? TriangleAlertIcon : GlobeIcon;
	return (
		<output className={BAR} data-widget-runtime-request={request.count}>
			<Icon
				aria-hidden="true"
				className={cn(
					"size-3.5 shrink-0",
					broad ? "text-destructive" : "text-muted-foreground",
				)}
			/>
			<span className="min-w-0 flex-1">
				{t(
					"widgetRuntimeRequestBanner",
					"Wants to load from {{count}} new sites",
					{
						count: request.count,
					},
				)}
			</span>
			<LinkButton
				onClick={() => {
					if (!isInert()) onReview();
				}}
				{...(inert ? { "aria-disabled": true } : {})}
			>
				{t("widgetRuntimeReview", "Review")}
			</LinkButton>
			<DismissButton onDismiss={onDismiss} />
		</output>
	);
}

function hostOf(source: string): string {
	const separator = source.indexOf("://");
	const rest = separator < 0 ? source : source.slice(separator + 3);
	const end = rest.search(/[/?#]/);
	return end < 0 ? rest : rest.slice(0, end);
}

function skippedCount(
	notice: Extract<MicroWidgetGrantNotice, { kind: "runtime-skipped" }>,
): number {
	return (
		notice.rejected.length +
		notice.issues.reduce((sum, issue) => sum + issue.count, 0)
	);
}

/** One line per server rejection (`host: reason`) and per host issue (`n × reason`); never a URL. */
export function microWidgetSkippedLines(
	notice: Extract<MicroWidgetGrantNotice, { kind: "runtime-skipped" }>,
	t: Translate,
): string[] {
	const lines = notice.rejected.map((rejection) =>
		t("widgetRuntimeSkippedHost", "{{host}}: {{reason}}", {
			host: hostOf(rejection.source),
			reason: widgetRuntimeIssueReason(rejection.code, t),
		}),
	);
	const counts = new Map<string, number>();
	for (const issue of notice.issues) {
		const reason = widgetRuntimeIssueReason(issue.code, t);
		counts.set(reason, (counts.get(reason) ?? 0) + issue.count);
	}
	for (const [reason, amount] of counts) {
		lines.push(
			t("widgetRuntimeSkippedCount", "{{amount}} × {{reason}}", {
				amount,
				reason,
			}),
		);
	}
	if (notice.invalidReason) {
		lines.push(
			t("widgetRuntimeSkippedAll", "All addresses: {{reason}}", {
				reason: widgetRuntimeIssueReason(notice.invalidReason, t),
			}),
		);
	}
	return [...new Set(lines)];
}

function SkippedDetails({
	notice,
}: {
	notice: Extract<MicroWidgetGrantNotice, { kind: "runtime-skipped" }>;
}) {
	const { t } = useTranslation("common");
	const lines = microWidgetSkippedLines(notice, t);
	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="link"
					size="sm"
					className="h-auto min-h-6 shrink-0 px-1 text-xs"
				>
					{t("details", "Details")}
				</Button>
			</PopoverTrigger>
			<PopoverContent
				align="end"
				className="w-72 p-3 text-xs"
				data-widget-skipped-details
			>
				<ul className="flex flex-col gap-1">
					{lines.map((line) => (
						<li key={line} className="wrap-break-word">
							{line}
						</li>
					))}
				</ul>
			</PopoverContent>
		</Popover>
	);
}

function noticeMessage(
	notice: MicroWidgetGrantNotice,
	t: Translate,
	locale: string,
): string {
	switch (notice.kind) {
		case "invalid":
			return t(
				"widgetPolicyInvalidNotice",
				"This widget's permissions could not be verified, so it runs without them.",
			);
		case "baseline":
			return widgetDeclinedPermissions(notice.declined, t, locale);
		case "unavailable":
			return t(
				"widgetGrantUnavailableNotice",
				"This server cannot grant widget permissions, so the widget runs without them.",
			);
		case "runtime-unavailable":
			return t(
				"widgetRuntimeSourcesUnavailable",
				"This server can't allow addresses provided while the app runs. The widget runs without them.",
			);
		case "runtime-skipped": {
			const count = skippedCount(notice);
			return count > 0
				? t(
						"widgetRuntimeSourcesSkipped",
						"{{count}} addresses given to this widget can't be allowed",
						{ count },
					)
				: t(
						"widgetRuntimeSourcesSkippedAll",
						"Addresses given to this widget can't be allowed",
					);
		}
	}
}

function noticeIcon(notice: MicroWidgetGrantNotice) {
	switch (notice.kind) {
		case "baseline":
			return ShieldOffIcon;
		case "runtime-unavailable":
		case "runtime-skipped":
			return GlobeIcon;
		default:
			return ShieldQuestionIcon;
	}
}

/**
 * A mounted widget that runs with less than it asked for. A declined widget
 * offers Allow in place; it opens the consent dialog again.
 */
export function MicroWidgetNotice({
	notice,
	onReview,
	reviewRef,
	onDismiss,
}: {
	notice: MicroWidgetGrantNotice;
	onReview?: () => void;
	reviewRef?: Ref<HTMLButtonElement>;
	onDismiss: () => void;
}) {
	const { t, i18n } = useTranslation("common");
	const Icon = noticeIcon(notice);
	return (
		<output className={BAR} data-widget-notice={notice.kind}>
			<Icon
				aria-hidden="true"
				className="size-3.5 shrink-0 text-muted-foreground"
			/>
			<span className="min-w-0 flex-1">
				{noticeMessage(notice, t, i18n.language)}
			</span>
			{notice.kind === "runtime-skipped" && <SkippedDetails notice={notice} />}
			{onReview && (
				<LinkButton ref={reviewRef} onClick={onReview}>
					{t("widgetPermissionAllow", "Allow…")}
				</LinkButton>
			)}
			<DismissButton onDismiss={onDismiss} />
		</output>
	);
}

/**
 * The widget's single notice slot (§14.5.4), highest priority first: the
 * runtime request banner, then the controller's notices in their order.
 */
export function MicroWidgetNoticeSlot({
	grant,
	onReview,
	reviewRef,
}: {
	grant: Pick<
		MicroWidgetGrant,
		"runtimeRequest" | "notices" | "dismissRuntimeRequest" | "dismissNotice"
	>;
	onReview: () => void;
	/** Focus lands here once the consent dialog closes on a declined widget. */
	reviewRef?: Ref<HTMLButtonElement>;
}) {
	if (grant.runtimeRequest) {
		return (
			<MicroWidgetRuntimeBanner
				request={grant.runtimeRequest}
				onReview={onReview}
				onDismiss={grant.dismissRuntimeRequest}
			/>
		);
	}
	const notice = grant.notices[0];
	if (!notice) return null;
	return (
		<MicroWidgetNotice
			notice={notice}
			onReview={notice.kind === "baseline" ? onReview : undefined}
			reviewRef={reviewRef}
			onDismiss={() => grant.dismissNotice(notice.kind)}
		/>
	);
}

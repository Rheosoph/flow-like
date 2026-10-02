"use client";

import { useTranslation } from "@flow-like/locales";
import { OctagonX, TriangleAlert } from "lucide-react";
import { type ReactNode, useState } from "react";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import { DvButton } from "../primitives/dv-button";
import { TONE_CHIP, cx } from "../primitives/tone";
import { useAttentionCounts } from "../workspace";
import { AttentionPopover } from "./attention-popover";
import type { ChromeNavigate } from "./rail-row";

export interface AttentionButtonProps {
	scope: DevicesScope;
	route: DevicesRoute;
	onNavigate: ChromeNavigate;
	/** App scope: the app's name ("Needs you in Invoice AI"). */
	appName?: string;
	className?: string;
}

/** Top bar buttons: 32 px, label hidden below 720 px of the area, pressed while their layer is open. */
export const CHROME_BUTTON =
	"shrink-0 px-2.5 aria-expanded:bg-row-selected data-[state=open]:bg-row-selected";
export const CHROME_LABEL = "@max-[720px]/devices:hidden";
/** Popover surface of the area: popover fill, strong border, no blur or shadow (R13). */
export const CHROME_POPOVER =
	"max-h-[min(70vh,560px)] overflow-auto rounded-lg border-border-strong bg-popover p-0 text-ui shadow-none backdrop-blur-none";

interface CountPillProps {
	count: number;
	critical?: boolean;
}

/** SPEC §3.2: mono count; the critical one is square-cornered with the octagon glyph (R14). */
export function CountPill({
	count,
	critical = false,
}: Readonly<CountPillProps>) {
	return (
		<span
			aria-hidden
			data-count={critical ? "critical" : "total"}
			className={cx(
				"inline-flex h-4.5 min-w-5 items-center justify-center gap-0.5 px-1.25 font-mono text-xs font-medium tabular-nums",
				critical
					? cx("border", TONE_CHIP.critical)
					: "rounded-full bg-muted text-ink-2",
			)}
		>
			{critical ? <OctagonX aria-hidden className="size-3" /> : null}
			{count}
		</span>
	);
}

export interface AttentionButtonViewProps {
	critical: number;
	/** Counted items: Info never counts. */
	total: number;
	appName?: string;
	/** Fleet overview: jump to "Needs you" instead of opening the popover. */
	onDirect?: () => void;
	open?: boolean;
	onOpenChange?: (open: boolean) => void;
	/** The popover's content. */
	children?: ReactNode;
	className?: string;
}

function useAttentionName(props: Readonly<AttentionButtonViewProps>) {
	const { t } = useTranslation("devices");
	const { critical, total, appName } = props;
	return appName
		? t(
				"chrome.attention.nameApp",
				"Attention in {{app}}: {{critical, number}} critical, {{total, number}} in total",
				{ app: appName, critical, total },
			)
		: t(
				"chrome.attention.name",
				"Attention: {{critical, number}} critical, {{total, number}} in total",
				{ critical, total },
			);
}

/** The Attention button over plain counts. */
export function AttentionButtonView(props: Readonly<AttentionButtonViewProps>) {
	const { t } = useTranslation("devices");
	const name = useAttentionName(props);
	const { critical, total, onDirect } = props;
	const button = (
		<DvButton
			variant="ghost"
			icon={TriangleAlert}
			aria-label={name}
			aria-haspopup={onDirect ? undefined : "dialog"}
			data-chrome="attention"
			className={cx(CHROME_BUTTON, props.className)}
			onClick={onDirect}
		>
			<span className={CHROME_LABEL}>
				{t("chrome.attention.label", "Attention")}
			</span>
			{critical > 0 ? <CountPill count={critical} critical /> : null}
			<CountPill count={total} />
		</DvButton>
	);
	if (onDirect) return button;
	return (
		<Popover open={props.open} onOpenChange={props.onOpenChange}>
			<PopoverTrigger asChild>{button}</PopoverTrigger>
			<PopoverContent
				align="end"
				aria-label={t("chrome.attention.popover", "Needs you")}
				className={cx(CHROME_POPOVER, "w-[min(560px,calc(100vw-16px))]")}
			>
				{props.children}
			</PopoverContent>
		</Popover>
	);
}

/** SPEC §3.2 item 6: split attention count; opens the top-5 popover. */
export function AttentionButton({
	scope,
	route,
	onNavigate,
	appName,
	className,
}: Readonly<AttentionButtonProps>) {
	const [open, setOpen] = useState(false);
	const counts = useAttentionCounts(
		scope.kind === "app" ? { appId: scope.appId } : {},
	);
	const onFleet = route.screen === "fleet";
	return (
		<AttentionButtonView
			critical={counts.critical}
			total={counts.total}
			appName={scope.kind === "app" ? appName : undefined}
			open={open}
			onOpenChange={setOpen}
			onDirect={
				onFleet
					? () =>
							onNavigate({ ...route, focus: "attention" }, { replace: true })
					: undefined
			}
			className={className}
		>
			<AttentionPopover
				scope={scope}
				route={route}
				onNavigate={onNavigate}
				appName={appName}
				onClose={() => setOpen(false)}
			/>
		</AttentionButtonView>
	);
}

"use client";

import { useTranslation } from "@flow-like/locales";
import { LoaderCircle, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { enumLabel } from "../copy/enum-labels";
import { type AreaTime, type DevicesT, useAreaTime } from "./area-context";
import { TONE_ICON } from "./icons";
import {
	CONVERGENCE_LOOK,
	type ConvergenceChipKind,
	convergenceLabel,
} from "./status-chip";
import { TONE_TEXT, type Tone, cx } from "./tone";

export type DesiredRun = "running" | "stopped";
export type ObservedRun =
	| "unknown"
	| "starting"
	| "running"
	| "stopping"
	| "stopped"
	| "backoff"
	| "failed"
	| "removed";

/** "Requested Running, actual Restarting after a crash" (also the PairedPins name). */
export function requestedActualLabel(
	t: DevicesT,
	desired: DesiredRun,
	observed: ObservedRun,
): string {
	return t(
		"devices:view.dvo.aria",
		"Requested {{requested}}, actual {{actual}}",
		{
			requested: enumLabel(t, "desired", desired),
			actual: enumLabel(t, "observed", observed),
		},
	);
}

interface Shared {
	/** Second line ("Crashing · 3 h ago, last known"); built from `since` when omitted. */
	sub?: ReactNode;
	className?: string;
}

export interface ServiceStateProps extends Shared {
	desired: DesiredRun;
	observed: ObservedRun;
	conv: ConvergenceChipKind;
	/** Unix seconds of the last change, for the default second line. */
	since?: number;
	lastKnown?: boolean;
	versions?: undefined;
}

/** "Saved v2 → device has v1", "2 requested → 1 ready": neutral unless diverged. */
export interface VersionsProps extends Shared {
	versions: true;
	requested: ReactNode;
	actual: ReactNode;
	/** Accessible sentence for the pair. */
	label: string;
	tone?: Tone;
	icon?: LucideIcon;
}

export type RequestedActualProps = ServiceStateProps | VersionsProps;

function Pair({
	tone,
	icon: Icon,
	requested,
	actual,
	label,
	versions,
	attrs,
}: Readonly<{
	tone?: Tone;
	icon?: LucideIcon;
	requested: ReactNode;
	actual: ReactNode;
	label: string;
	versions: boolean;
	attrs?: Record<string, string>;
}>) {
	const critical = tone === "critical";
	return (
		<span
			role="img"
			aria-label={label}
			data-dvo={versions ? "versions" : "service"}
			data-tone={tone ?? "neutral"}
			{...attrs}
			className={cx(
				"inline-flex max-w-full items-stretch overflow-hidden rounded-md border bg-card align-middle text-xs in-[td]:flex-wrap",
				critical ? "border-critical-line" : "border-border",
			)}
		>
			<span className="shrink-0 bg-surface-sunken py-0.5 pl-1.5 font-sans whitespace-nowrap text-muted-foreground">
				{requested}
			</span>
			<span
				aria-hidden
				className="shrink-0 bg-surface-sunken py-0.5 pr-0.75 pl-1 text-muted-foreground"
			>
				→
			</span>
			<span
				className={cx(
					"inline-flex min-w-0 items-start gap-1 px-1.5 py-0.5 font-medium in-[td]:wrap-break-word",
					tone ? TONE_TEXT[tone] : "text-ink-2",
					critical && "bg-critical-bg",
				)}
			>
				{Icon ? <Icon aria-hidden className="mt-0.5 size-3 shrink-0" /> : null}
				<span className="min-w-0">{actual}</span>
			</span>
		</span>
	);
}

function Sub({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<span className="mt-0.75 block text-xs text-muted-foreground">
			{children}
		</span>
	);
}

function VersionsPair(props: Readonly<VersionsProps>) {
	return (
		<span className={cx("inline-block max-w-full", props.className)}>
			<Pair
				versions
				tone={props.tone}
				icon={props.icon ?? (props.tone ? TONE_ICON[props.tone] : undefined)}
				requested={props.requested}
				actual={props.actual}
				label={props.label}
			/>
			{props.sub ? <Sub>{props.sub}</Sub> : null}
		</span>
	);
}

const sinceLine = (
	t: DevicesT,
	time: AreaTime,
	{ conv, since, lastKnown }: Readonly<ServiceStateProps>,
) => {
	if (since === undefined) return null;
	const params = {
		state: convergenceLabel(t, conv),
		ago: time.ago(Math.min(since, time.nowS)),
	};
	return lastKnown
		? t(
				"devices:view.dvo.sinceLastKnown",
				"{{state}} · {{ago}}, last known",
				params,
			)
		: t("devices:view.dvo.since", "{{state}} · {{ago}}", params);
};

function ServiceState(props: Readonly<ServiceStateProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { desired, observed, conv } = props;
	const look = CONVERGENCE_LOOK[conv];
	const moving = observed === "starting" || observed === "stopping";
	const second = props.sub ?? sinceLine(t, time, props);
	return (
		<span className={cx("inline-block max-w-full", props.className)}>
			<Pair
				versions={false}
				tone={look.tone === "outline" ? undefined : look.tone}
				icon={moving ? LoaderCircle : look.icon}
				requested={enumLabel(t, "desired", desired)}
				actual={enumLabel(t, "observed", observed)}
				label={requestedActualLabel(t, desired, observed)}
				attrs={{ "data-conv": conv, "data-req": desired, "data-act": observed }}
			/>
			{second ? <Sub>{second}</Sub> : null}
		</span>
	);
}

/** SPEC §4.7 / IA §6.3.4: "Requested → Actual"; only the actual half takes the tone. */
export function RequestedActual(props: Readonly<RequestedActualProps>) {
	return props.versions ? (
		<VersionsPair {...props} />
	) : (
		<ServiceState {...props} />
	);
}

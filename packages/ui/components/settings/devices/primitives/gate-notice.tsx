"use client";

import {
	type MouseEvent,
	type ReactElement,
	type ReactNode,
	cloneElement,
	isValidElement,
	useId,
} from "react";
import { GATE_ICON, type GateKind } from "./icons";
import { cx } from "./tone";

const ICON_TONE: Partial<Record<GateKind, string>> = {
	locked: "text-locked",
	noaccess: "text-warning",
	policy: "text-warning",
};

/** SPEC §4.5 block form: a whole section is gated. `kind` = GateNoticeKind (model contracts). */
export function GateNotice({
	kind,
	title,
	text,
	have,
	actions,
	className,
}: Readonly<{
	kind: GateKind;
	/** The bold first sentence ("Unlock lab-gpu-02 to see its services."). */
	title: ReactNode;
	text?: ReactNode;
	/** What the viewer has ("Your access: View status, Read logs · Invoice AI"). */
	have?: ReactNode;
	actions?: ReactNode;
	className?: string;
}>) {
	const Icon = GATE_ICON[kind];
	return (
		<div
			data-gate={kind}
			className={cx(
				"flex items-start gap-2.5 rounded-lg border border-dashed bg-surface-sunken px-3.5 py-3 text-ui text-ink-2",
				kind === "locked" ? "border-locked-line" : "border-unknown-line",
				className,
			)}
		>
			<Icon
				aria-hidden
				className={cx(
					"mt-px size-4 shrink-0 text-muted-foreground",
					ICON_TONE[kind],
				)}
			/>
			<div className="flex max-w-[72ch] min-w-0 flex-col gap-1.5">
				<p className="text-ui">
					<b className="font-semibold text-foreground">{title}</b>
					{text ? <> {text}</> : null}
				</p>
				{have ? <p className="text-xs text-muted-foreground">{have}</p> : null}
				{actions ? (
					<div className="flex flex-wrap items-center gap-2">{actions}</div>
				) : null}
			</div>
		</div>
	);
}

/** SPEC §4.5 inline form: the always-visible one-line reason next to a disabled control (never tooltip-only). */
export function GateInline({
	kind,
	id,
	className,
	children,
}: Readonly<{
	kind: GateKind;
	id?: string;
	className?: string;
	children: ReactNode;
}>) {
	const Icon = GATE_ICON[kind];
	return (
		<span
			id={id}
			data-gate-inline={kind}
			className={cx(
				"inline-flex max-w-[42ch] min-w-[16ch] items-start gap-1 text-left text-xs whitespace-normal text-muted-foreground",
				className,
			)}
		>
			<Icon aria-hidden className="mt-0.5 size-3 shrink-0" />
			<span>{children}</span>
		</span>
	);
}

export interface Gate {
	kind: GateKind;
	reason: ReactNode;
}

const blockClick = (event: MouseEvent) => {
	event.preventDefault();
	event.stopPropagation();
};

/**
 * R7: a gated control stays visible and disabled (`aria-disabled`, still
 * focusable) with its reason underneath, linked by `aria-describedby`.
 * Without a gate the control renders unchanged.
 */
export function GatedAction({
	gate,
	className,
	children,
}: Readonly<{
	gate?: Gate | null;
	className?: string;
	children: ReactElement<Record<string, unknown>>;
}>) {
	const reasonId = useId();
	if (!gate) return children;
	const control = isValidElement(children)
		? cloneElement(children, {
				"aria-disabled": true,
				"aria-describedby": reasonId,
				"data-gated": gate.kind,
				onClick: blockClick,
			})
		: children;
	return (
		<span
			className={cx(
				"inline-flex max-w-full min-w-0 flex-col items-start gap-1",
				className,
			)}
		>
			{control}
			<GateInline kind={gate.kind} id={reasonId}>
				{gate.reason}
			</GateInline>
		</span>
	);
}

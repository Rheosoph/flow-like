"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "./consequence-preview";
import { DvButton } from "./dv-button";
import { InlineResult } from "./inline-result";
import { cx } from "./tone";

export interface InlineConfirmProps {
	/** Accessible name of the region ("Confirm restart"). */
	label: string;
	/** "Restart support-bot?" */
	title: ReactNode;
	/** "Support Portal · edge-berlin-01" */
	sub?: ReactNode;
	rows: ConsequenceRows;
	/** Verb + object: "Restart support-bot". */
	confirmLabel: ReactNode;
	tone?: "danger" | "default";
	/** Runs while the button is busy; a thrown error stays inline and the confirm stays open. */
	onConfirm(): Promise<void> | void;
	onCancel(): void;
	className?: string;
}

/**
 * The question takes the focus when it appears (keyboard and screen reader are
 * where it is asked) and hands it back to the control that opened it when it
 * goes, unless the person has moved on meanwhile.
 */
function useConfirmFocus() {
	const region = useRef<HTMLElement>(null);
	useLayoutEffect(() => {
		const node = region.current;
		const opener = document.activeElement;
		node?.querySelector("button")?.focus();
		return () => {
			if (
				node?.contains(document.activeElement) &&
				opener instanceof HTMLElement &&
				opener.isConnected
			)
				opener.focus();
		};
	}, []);
	return region;
}

/** SPEC §4.6 inline form for Start/Stop/Restart/scale in tables and on N3; replaced by an InlineResult after it runs. */
export function InlineConfirm({
	label,
	title,
	sub,
	rows,
	confirmLabel,
	tone = "danger",
	onConfirm,
	onCancel,
	className,
}: Readonly<InlineConfirmProps>) {
	const { t } = useTranslation("devices");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const inFlight = useRef(false);
	const region = useConfirmFocus();

	const submit = async () => {
		if (inFlight.current) return;
		inFlight.current = true;
		setBusy(true);
		setError(null);
		try {
			await onConfirm();
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			inFlight.current = false;
			setBusy(false);
		}
	};

	return (
		<section
			ref={region}
			aria-label={label}
			data-inline-confirm=""
			className={cx(
				"flex max-w-[860px] flex-col gap-2.5 rounded-lg border border-border-strong bg-card p-3",
				className,
			)}
		>
			<p className="text-ui">
				<b className="font-semibold">{title}</b>
				{sub ? <span className="text-muted-foreground"> {sub}</span> : null}
			</p>
			<ConsequencePreview rows={rows} compact />
			{error ? (
				<InlineResult tone="critical" onDismiss={() => setError(null)}>
					{error}
				</InlineResult>
			) : null}
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					size="sm"
					variant={tone === "danger" ? "danger" : "default"}
					busy={busy}
					onClick={submit}
				>
					{confirmLabel}
				</DvButton>
				<DvButton size="sm" disabled={busy} onClick={onCancel}>
					{t("common.confirm.cancel", "Cancel")}
				</DvButton>
			</div>
		</section>
	);
}

/** The same confirm as a table row directly under the row it acts on. */
export function InlineConfirmRow({
	colSpan,
	...props
}: Readonly<InlineConfirmProps & { colSpan: number }>) {
	return (
		<tr data-confirm-row="" className="hover:bg-transparent">
			<td colSpan={colSpan} className="border-t border-hairline px-4 py-2">
				<InlineConfirm {...props} />
			</td>
		</tr>
	);
}

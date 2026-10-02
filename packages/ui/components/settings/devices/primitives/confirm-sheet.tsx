"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import type { LucideIcon } from "lucide-react";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "./consequence-preview";
import { DvButton } from "./dv-button";
import { DvSheet } from "./dv-sheet";
import {
	CheckField,
	ChoiceCards,
	DvInput,
	DvTextarea,
	Field,
} from "./form-fields";
import type { ConseqKind } from "./icons";
import { InlineResult } from "./inline-result";

/** SPEC §4.6 / §6.5 confirmation strengths. */
export type ConfirmStrength = "none" | "check" | "typed" | "reason" | "review";

export interface ConfirmReason {
	value: string;
	label: ReactNode;
	hint?: ReactNode;
}

export interface ConfirmResult {
	ok: boolean;
	reason?: string;
	note?: string;
}

export interface ConfirmOptions {
	icon?: LucideIcon;
	/** Verb + object: "Stop support-bot?" */
	title: ReactNode;
	/** Object context: "Support Portal · on edge-berlin-01". */
	sub?: ReactNode;
	rows: ConsequenceRows;
	whoLabel?: "notices" | "loses";
	rowLabels?: Partial<Record<ConseqKind, ReactNode>>;
	strength?: ConfirmStrength;
	/** `check`; `reason` with `requireCheck`; `review` step 2 without `typed`. */
	checkLabel?: ReactNode;
	/** `typed` / `review`: the exact text to type (device name, service ID). */
	typed?: string;
	reasons?: readonly ConfirmReason[];
	/** `reason`: also require the acknowledgement (a change that was already attempted). */
	requireCheck?: boolean;
	/** Verb + object: "Stop support-bot". */
	confirmLabel: ReactNode;
	tone?: "danger" | "default";
	wide?: boolean;
	/** Extra body content under the consequence rows (options, prerequisites). */
	extra?: ReactNode;
	/** Runs while the button shows busy; a thrown error is shown inline and the sheet stays open. */
	onConfirm?: (result: ConfirmResult) => Promise<void> | void;
}

interface ConfirmInput {
	step: 1 | 2;
	checked: boolean;
	typedValue: string;
	reason?: string;
}

/** Whether the confirm button is enabled for the strength (typed = exact match only). */
export function confirmReady(
	options: Pick<
		ConfirmOptions,
		"strength" | "typed" | "checkLabel" | "requireCheck"
	>,
	input: ConfirmInput,
): boolean {
	const typedOk =
		options.typed !== undefined && input.typedValue === options.typed;
	const finalStep =
		options.typed !== undefined
			? typedOk
			: !options.checkLabel || input.checked;
	const rules: Record<ConfirmStrength, () => boolean> = {
		none: () => true,
		check: () => input.checked,
		typed: () => typedOk,
		reason: () => !!input.reason && (!options.requireCheck || input.checked),
		review: () => input.step === 2 && finalStep,
	};
	return rules[options.strength ?? "none"]();
}

interface ConfirmSheetProps {
	open: boolean;
	options: ConfirmOptions;
	onResolve(result: ConfirmResult): void;
}

/**
 * Every opening is its own session: ticks, typed text, the busy state and a
 * shown error never carry over when a screen keeps one sheet mounted and
 * toggles `open`.
 */
export function ConfirmSheet(props: Readonly<ConfirmSheetProps>) {
	const [session, setSession] = useState(0);
	const [wasOpen, setWasOpen] = useState(props.open);
	if (props.open !== wasOpen) {
		setWasOpen(props.open);
		if (props.open) setSession(session + 1);
	}
	return <ConfirmSheetSession key={session} {...props} />;
}

function ConfirmSheetSession({
	open,
	options,
	onResolve,
}: Readonly<ConfirmSheetProps>) {
	const { t } = useTranslation("devices");
	const strength = options.strength ?? "none";
	const [step, setStep] = useState<1 | 2>(1);
	const [checked, setChecked] = useState(false);
	const [typedValue, setTypedValue] = useState("");
	const [reason, setReason] = useState<string | undefined>();
	const [note, setNote] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const inFlight = useRef(false);

	const reviewing = strength === "review" && step === 1;
	const ready = confirmReady(options, { step, checked, typedValue, reason });

	const submit = async () => {
		if (!ready || inFlight.current) return;
		inFlight.current = true;
		setBusy(true);
		setError(null);
		const result: ConfirmResult = {
			ok: true,
			reason,
			note: note.trim() || undefined,
		};
		try {
			await options.onConfirm?.(result);
			onResolve(result);
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
			inFlight.current = false;
			setBusy(false);
		}
	};

	const cancel = () => {
		if (inFlight.current) return;
		onResolve({ ok: false });
	};

	const typedField =
		options.typed !== undefined ? (
			<Field
				id="dv-confirm-typed"
				label={
					<Trans
						t={t}
						i18nKey="common.confirm.typeToConfirm"
						defaults="Type <1/> to confirm"
						components={{
							1: <b className="font-mono font-semibold">{options.typed}</b>,
						}}
					/>
				}
			>
				<DvInput
					mono
					autoComplete="off"
					spellCheck={false}
					value={typedValue}
					onChange={(event) => setTypedValue(event.target.value)}
				/>
			</Field>
		) : null;

	const checkField = options.checkLabel ? (
		<CheckField
			id="dv-confirm-ack"
			checked={checked}
			onCheckedChange={setChecked}
		>
			{options.checkLabel}
		</CheckField>
	) : null;

	const footNote =
		strength === "review"
			? step === 1
				? t("common.confirm.stepReview", "Step 1 of 2 · Review")
				: t("common.confirm.stepConfirm", "Step 2 of 2 · Confirm")
			: undefined;

	return (
		<DvSheet
			open={open}
			onOpenChange={(next) => {
				if (!next) cancel();
			}}
			role="alertdialog"
			closeOnOutside={false}
			icon={options.icon}
			eyebrow={t("common.confirm.eyebrow", "Before this runs")}
			title={options.title}
			sub={options.sub}
			wide={options.wide}
			footNote={footNote}
			onBack={
				strength === "review" && step === 2 ? () => setStep(1) : undefined
			}
			foot={
				<>
					<DvButton onClick={cancel} disabled={busy}>
						{t("common.confirm.cancel", "Cancel")}
					</DvButton>
					{reviewing ? (
						<DvButton variant="primary" onClick={() => setStep(2)}>
							{t("common.confirm.continue", "Continue")}
						</DvButton>
					) : (
						<DvButton
							variant={options.tone === "danger" ? "danger" : "primary"}
							aria-disabled={!ready || undefined}
							busy={busy}
							data-confirm=""
							onClick={submit}
						>
							{options.confirmLabel}
						</DvButton>
					)}
				</>
			}
		>
			{strength !== "review" || step === 1 ? (
				<ConsequencePreview
					rows={options.rows}
					whoLabel={options.whoLabel}
					labels={options.rowLabels}
				/>
			) : null}
			{options.extra && (strength !== "review" || step === 1)
				? options.extra
				: null}
			{strength === "check" ? checkField : null}
			{strength === "typed" ? typedField : null}
			{strength === "reason" ? (
				<>
					<ChoiceCards
						id="dv-confirm-reason"
						legend={t("common.confirm.reason", "Why?")}
						value={reason}
						onValueChange={setReason}
						options={(options.reasons ?? []).map((item) => ({
							value: item.value,
							title: item.label,
							hint: item.hint,
						}))}
					/>
					<Field
						id="dv-confirm-note"
						label={t("common.confirm.note", "Note (optional)")}
					>
						<DvTextarea
							value={note}
							onChange={(event) => setNote(event.target.value)}
						/>
					</Field>
					{options.requireCheck ? checkField : null}
				</>
			) : null}
			{strength === "review" && step === 2 ? (typedField ?? checkField) : null}
			{error ? (
				<InlineResult tone="critical" onDismiss={() => setError(null)}>
					{error}
				</InlineResult>
			) : null}
		</DvSheet>
	);
}

type ConfirmFn = (options: ConfirmOptions) => Promise<ConfirmResult>;

interface PendingConfirm {
	id: number;
	options: ConfirmOptions;
	resolve(result: ConfirmResult): void;
	/** Its `onConfirm` is in flight: the sheet stays until that settles. */
	running: boolean;
}

const ConfirmContext = createContext<ConfirmFn | null>(null);

/** The request with `onConfirm` wrapped so the provider knows while it runs. */
function trackRunning(
	id: number,
	options: ConfirmOptions,
	resolve: PendingConfirm["resolve"],
): PendingConfirm {
	const { onConfirm } = options;
	const pending: PendingConfirm = { id, options, resolve, running: false };
	if (!onConfirm) return pending;
	pending.options = {
		...options,
		onConfirm: async (result) => {
			pending.running = true;
			try {
				await onConfirm(result);
			} finally {
				pending.running = false;
			}
		},
	};
	return pending;
}

/**
 * Hosts the single confirm sheet of the area; `useConfirm()` opens it. A new
 * request replaces an idle sheet (which resolves not ok) but is refused while
 * a confirmed action is still running, and only the sheet on screen can
 * resolve its own request.
 */
export function ConfirmProvider({
	children,
}: Readonly<{ children: ReactNode }>) {
	const [pending, setPending] = useState<PendingConfirm | null>(null);
	const pendingRef = useRef<PendingConfirm | null>(null);
	const sequence = useRef(0);

	const confirm = useCallback<ConfirmFn>(
		(options) =>
			new Promise<ConfirmResult>((resolve) => {
				const current = pendingRef.current;
				if (current?.running) {
					resolve({ ok: false });
					return;
				}
				current?.resolve({ ok: false });
				sequence.current += 1;
				const next = trackRunning(sequence.current, options, resolve);
				pendingRef.current = next;
				setPending(next);
			}),
		[],
	);

	const settle = useCallback((id: number, result: ConfirmResult) => {
		setPending((shown) => (shown?.id === id ? null : shown));
		const current = pendingRef.current;
		if (current?.id !== id) return;
		pendingRef.current = null;
		current.resolve(result);
	}, []);

	useEffect(
		() => () => {
			pendingRef.current?.resolve({ ok: false });
			pendingRef.current = null;
		},
		[],
	);

	return (
		<ConfirmContext.Provider value={confirm}>
			{children}
			{pending ? (
				<ConfirmSheet
					key={pending.id}
					open
					options={pending.options}
					onResolve={(result) => settle(pending.id, result)}
				/>
			) : null}
		</ConfirmContext.Provider>
	);
}

/** Promise API: `const { ok, reason } = await confirm({ … })`. Needs a `ConfirmProvider` above. */
export function useConfirm(): ConfirmFn {
	const confirm = useContext(ConfirmContext);
	return useCallback<ConfirmFn>(
		(options) => {
			if (!confirm) {
				return Promise.reject(
					new Error("useConfirm() was called outside a ConfirmProvider"),
				);
			}
			return confirm(options);
		},
		[confirm],
	);
}

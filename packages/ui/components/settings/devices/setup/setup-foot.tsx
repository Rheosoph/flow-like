"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	ChevronLeft,
	ChevronRight,
	type LucideIcon,
	Package,
	Plus,
	RefreshCw,
} from "lucide-react";
import type { ReactNode } from "react";
import type { SetupStep } from "../../../../lib/device-management/model/types";
import { DvButton } from "../primitives/dv-button";
import { GateInline } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { useSetup } from "./setup-context";
import { Mono } from "./setup-parts";
import {
	CREATE_STEP,
	START_STEP,
	STEP_COUNT,
	WAIT_STEP,
	lockedBefore,
} from "./setup-state";

const NEXT_REASON_ID = "dv-setup-next-reason";
const BACK_REASON_ID = "dv-setup-back-reason";

interface Primary {
	label: string;
	icon?: LucideIcon;
	/** "Continue ›": the chevron trails the label. */
	trailing: boolean;
}

/** The step's one coral button (R2): its label names what it does. */
function usePrimary(): Primary {
	const { t } = useTranslation("devices");
	const { draft, step, expired, create } = useSetup();
	const { status } = create.run;
	if (draft.cancelledAt)
		return {
			label: t("setup.next.startNew", "Start a new setup"),
			icon: Plus,
			trailing: false,
		};
	if (expired)
		return {
			label: t("setup.next.createNew", "Create a new one"),
			icon: Plus,
			trailing: false,
		};
	if (step === WAIT_STEP)
		return {
			label: t("setup.next.openDevice", "Open device"),
			icon: ArrowRight,
			trailing: false,
		};
	if (step === CREATE_STEP && !draft.created) {
		if (status === "failed")
			return {
				label: t("setup.next.retry", "Try again"),
				icon: RefreshCw,
				trailing: false,
			};
		if (status === "running")
			return {
				label: t("setup.next.creatingLabel", "Creating…"),
				trailing: false,
			};
		return {
			label: t("setup.create.action", "Create setup package"),
			icon: Package,
			trailing: false,
		};
	}
	return {
		label:
			step === START_STEP
				? t("setup.next.wait", "Wait for the device")
				: t("setup.next.continue", "Continue"),
		trailing: true,
	};
}

/** The line between Cancel and the buttons: a gate reason, the lock on Back, or where the step sits. */
function FootNote({ backLocked }: Readonly<{ backLocked: boolean }>) {
	const { t } = useTranslation("devices");
	const { draft, step, labels, expired, next, tried, startNew } = useSetup();
	const done = draft.checkedInAt !== undefined;
	const quiet = "text-xs text-muted-foreground";
	if (draft.cancelledAt) return null;
	if (expired)
		return (
			<span className={quiet}>
				<Trans
					t={t}
					i18nKey="setup.next.createNewNote"
					defaults="Starts a new setup named <1/>, with a new package and a new device ID."
					components={{ 1: <Mono>{draft.name}</Mono> }}
				/>
			</span>
		);
	if (step === WAIT_STEP && done)
		return (
			<span className={quiet}>
				{t("setup.next.done", "Done.")}{" "}
				<DvButton variant="link" size="xs" onClick={() => startNew()}>
					{t("setup.next.another", "Set up another device")}
				</DvButton>
			</span>
		);
	if (!next.ok && next.reason && (!next.field || tried))
		return (
			<GateInline
				kind={next.kind ?? "busy"}
				id={NEXT_REASON_ID}
				className="max-w-[52ch]"
			>
				{next.reason}
			</GateInline>
		);
	if (backLocked)
		return (
			<GateInline kind="locked" id={BACK_REASON_ID} className="max-w-[52ch]">
				{draft.created
					? t(
							"setup.back.lockedBuilt",
							"Earlier steps are locked: the package is built.",
						)
					: t(
							"setup.back.lockedRegistered",
							"Earlier steps are locked: the device is registered.",
						)}
			</GateInline>
		);
	const following = labels[step + 1];
	const position = { n: step + 1, count: STEP_COUNT, step: labels[step] };
	const strong = { 1: <b className="font-medium text-ink-2" /> };
	return (
		<output className={`${quiet} @max-[720px]/wfoot:hidden`}>
			{following ? (
				<Trans
					t={t}
					i18nKey="setup.foot.position"
					defaults="<1>Step {{n, number}} of {{count, number}}</1> · {{step}} · next: {{next}}"
					values={{ ...position, next: following }}
					components={strong}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="setup.foot.positionLast"
					defaults="<1>Step {{n, number}} of {{count, number}}</1> · {{step}}"
					values={position}
					components={strong}
				/>
			)}
		</output>
	);
}

function CancelResult(): ReactNode {
	const { t } = useTranslation("devices");
	const { cancelError, dismissCancelError } = useSetup();
	if (!cancelError) return null;
	return (
		<div className="basis-full">
			<InlineResult tone="critical" onDismiss={dismissCancelError}>
				{t(
					"setup.cancel.failed",
					"The setup couldn't be cancelled: {{reason}} Try again.",
					{ reason: cancelError },
				)}
			</InlineResult>
		</div>
	);
}

/**
 * The wizard's sticky foot (SPEC §4.22): Cancel setup… on the left, the reason
 * a gated primary is disabled (R7, at every width), Back, and the one primary.
 * The primary submits the wizard's form, so Enter in a field does the same.
 */
export function SetupFoot() {
	const { t } = useTranslation("devices");
	const { draft, step, nowS, expired, create, next, goTo, cancelSetup } =
		useSetup();
	const primary = usePrimary();
	const { created } = draft;
	const creating = create.run.status === "running";
	const registered = draft.registeredAt !== undefined;
	const done = draft.checkedInAt !== undefined;
	const closed = !!draft.cancelledAt || expired;

	// A package whose agent release ran out can't be started, yet the hub holds its slot until the setup itself ends.
	const heldAtHub = !!created && !registered && created.expiresAt > nowS;
	const canCancel =
		!draft.cancelledAt &&
		(heldAtHub || (creating && !!create.run.registration));
	const showBack = step > 0 && !closed && !done && !creating;
	const floor = created || creating ? lockedBefore(draft) : 0;
	const backOk = step - 1 >= floor;
	const gated = !closed && !next.ok && !next.field;
	const PrimaryIcon = primary.trailing ? undefined : primary.icon;

	return (
		<div
			data-wizard-foot=""
			className="@container/wfoot sticky bottom-2 z-5 flex flex-wrap items-center gap-2 rounded-lg border border-border bg-surface-sunken px-3 py-2.5"
		>
			<CancelResult />
			{canCancel ? (
				<DvButton variant="danger-ghost" onClick={cancelSetup}>
					{t("setup.cancel.button", "Cancel setup…")}
				</DvButton>
			) : null}
			<span className="flex min-w-[16ch] flex-1 items-center @max-[720px]/wfoot:order-first @max-[720px]/wfoot:basis-full">
				<FootNote backLocked={showBack && !backOk} />
			</span>
			{showBack ? (
				<DvButton
					icon={ChevronLeft}
					aria-disabled={backOk ? undefined : true}
					aria-describedby={backOk ? undefined : BACK_REASON_ID}
					onClick={() => {
						if (backOk) goTo((step - 1) as SetupStep);
					}}
				>
					{t("setup.back.label", "Back")}
				</DvButton>
			) : null}
			<DvButton
				type="submit"
				variant="primary"
				icon={PrimaryIcon}
				busy={step === CREATE_STEP && creating}
				aria-disabled={gated ? true : undefined}
				aria-describedby={gated ? NEXT_REASON_ID : undefined}
				data-gated={gated ? (next.kind ?? "busy") : undefined}
				className="@max-[720px]/wfoot:flex-1"
			>
				{primary.label}
				{primary.trailing ? (
					<ChevronRight aria-hidden className="size-4" />
				) : null}
			</DvButton>
		</div>
	);
}

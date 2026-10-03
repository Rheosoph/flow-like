"use client";

import { useTranslation } from "@flow-like/locales";
import { DvButton, type DvButtonSize } from "../primitives/dv-button";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { useInlineResults } from "../workspace";
import type { CloudActionGroup, CloudActions } from "./use-cloud-actions";

interface RevokeProps {
	actions: CloudActions;
	size?: DvButtonSize;
}

/** Only the approver and the device owner can use it; the gate says who. */
export function RevokeApprovalButton({
	actions,
	size = "md",
}: Readonly<RevokeProps>) {
	const { t } = useTranslation("devices");
	return (
		<GatedAction gate={actions.revokeApprovalGate}>
			<DvButton
				size={size}
				variant="danger-ghost"
				data-act="revoke-approval"
				busy={actions.running === "revokeApproval"}
				onClick={() => void actions.revokeApproval()}
			>
				{t("cloud.service.revoke", "Revoke approval…")}
			</DvButton>
		</GatedAction>
	);
}

/** Only the person who pays can use it. */
export function RevokeLimitButton({
	actions,
	size = "md",
}: Readonly<RevokeProps>) {
	const { t } = useTranslation("devices");
	return (
		<GatedAction gate={actions.revokeLimitGate}>
			<DvButton
				size={size}
				variant="danger-ghost"
				data-act="revoke-limit"
				busy={actions.running === "revokeLimit"}
				onClick={() => void actions.revokeLimit()}
			>
				{t("cloud.spend.revoke", "Revoke spending limit…")}
			</DvButton>
		</GatedAction>
	);
}

/**
 * The action layer's sentence while an action runs or when it fails, then the
 * result sentence of the action itself (R9). `about` narrows it to the block
 * that holds the control; without it both groups are shown.
 */
export function useActionResults(
	actions: CloudActions,
	about?: CloudActionGroup,
) {
	const approval = useInlineResults(actions.resultKeys.approval);
	const limit = useInlineResults(actions.resultKeys.limit);
	const running = [
		...(about === "limit" ? [] : approval),
		...(about === "approval" ? [] : limit),
	].filter((result) => result.state !== "done");
	const note =
		actions.note && (!about || actions.note.about === about)
			? actions.note
			: undefined;
	return { running, note, any: running.length > 0 || !!note };
}

export function ActionResults({
	actions,
	about,
}: Readonly<{ actions: CloudActions; about?: CloudActionGroup }>) {
	const { running, note } = useActionResults(actions, about);
	return (
		<>
			{running.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
			{note ? (
				<InlineResult tone={note.tone} onDismiss={actions.dismissNote}>
					{note.text}
				</InlineResult>
			) : null}
		</>
	);
}

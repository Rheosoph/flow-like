"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Trash2 } from "lucide-react";
import { useCallback, useState } from "react";
import type { DevicesT } from "../primitives/area-context";
import type { ConfirmStrength } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import { Mono, keyKindLabel } from "./key-parts";
import { type KeyFileLog, rowScope, useKeyResults } from "./key-store";
import { type KeyRow, type LocalOnlyRow, localRevision } from "./keys-model";
import { LOCAL_ONLY_RESULT } from "./keys-table";
import { useFlowGuard, useKeyActions } from "./use-key-actions";

export type DeleteTarget =
	| { kind: "device"; row: KeyRow }
	| { kind: "local"; row: LocalOnlyRow };

interface DeletePlan {
	name: string;
	deviceId: string;
	sub: string;
	strength: ConfirmStrength;
	typed?: string;
	checkLabel?: string;
	rows: ConsequenceRows;
	/** The sentence shown once the keys are gone. */
	done(at: string): { tone: "good" | "warning"; text: string };
}

function devicePlan(t: DevicesT, row: KeyRow, fileSaved: boolean): DeletePlan {
	const { name, deviceId } = row;
	const hub = row.hubRevision ?? 0;
	const local = localRevision(row);
	const open = row.session?.state === "unlocked";
	const kind = keyKindLabel(t, row.vault?.role ?? "owner");
	const device = <Mono>{name}</Mono>;
	const what = (
		<>
			<Trans
				t={t}
				i18nKey="keys.delete.what"
				defaults="The keys for <1/> are removed from this computer."
				components={{ 1: device }}
			/>
			{open
				? ` ${t(
						"keys.delete.locksFirst",
						"It is unlocked here, so it locks first and its live connection closes.",
					)}`
				: null}
		</>
	);
	const who =
		row.relationship === "owner"
			? t(
					"keys.delete.whoOwner",
					"Nobody else. Other computers keep their keys.",
				)
			: t(
					"keys.delete.whoShared",
					"Nobody else. The owner isn't told; your access stays in the device's rules until it ends or the owner removes it.",
				);
	const when = t("keys.delete.when", "Immediately.");
	const restoredDone = (at: string) => ({
		tone: "good" as const,
		text: t(
			"keys.delete.doneBacked",
			"Keys deleted from this computer at {{at}}. Restore them from your account backup (v{{version}}) with the device password.",
			{ at, version: hub },
		),
	});
	const base = { name, deviceId };
	const behind = row.category === "pending" || row.category === "out_of_date";
	if (behind && hub > 0)
		return {
			...base,
			sub: t(
				"keys.delete.subPending",
				"{{kind}} · your account has v{{hub}}, this computer v{{local}}",
				{ kind, hub, local },
			),
			strength: "check",
			checkLabel: t(
				"keys.delete.checkPending",
				"I understand the newest copy of these keys exists only on this computer",
			),
			rows: {
				what,
				who,
				stays: t(
					"keys.delete.staysPending",
					"The device keeps running. Your account keeps v{{version}}.",
					{ version: hub },
				),
				when,
				undo: {
					reversible: null,
					text: t(
						"keys.delete.undoPartly",
						"Partly. Restoring brings back v{{hub}}, not the newest copy on this computer.",
						{ hub },
					),
				},
				first:
					row.category === "pending"
						? t(
								"keys.delete.firstRetry",
								"Retry the upload first, so your account has the newest copy.",
							)
						: t(
								"keys.delete.firstUpdate",
								"Update the account backup first, so your account has the newest copy.",
							),
			},
			done: restoredDone,
		};
	if (hub > 0 && row.category !== "unchecked")
		return {
			...base,
			sub: t("keys.delete.subBacked", "{{kind}} · backed up (v{{version}})", {
				kind,
				version: hub,
			}),
			strength: "none",
			rows: {
				what,
				who,
				stays: t(
					"keys.delete.staysBacked",
					"The device keeps running. Your account backup (v{{version}}) stays on your account.",
					{ version: hub },
				),
				when,
				undo: {
					reversible: true,
					text:
						row.category === "oldpw"
							? t(
									"keys.delete.undoOldPassword",
									"You can restore them from your account backup (v{{version}}) with the old device password; it still opens with that one.",
									{ version: hub },
								)
							: t(
									"keys.delete.undoBacked",
									"You can restore them from your account backup (v{{version}}) with the device password.",
									{ version: hub },
								),
				},
			},
			done: restoredDone,
		};
	return row.hubRevision === undefined
		? uncheckedPlan(t, row, { what, when, kind })
		: onlyCopyPlan(t, row, fileSaved, { what, when, kind });
}

interface PlanParts {
	what: ConsequenceRows["what"];
	when: string;
	kind: string;
}

/** The account holds no copy: these keys are the only ones, so the device name is typed. */
function onlyCopyPlan(
	t: DevicesT,
	row: KeyRow,
	fileSaved: boolean,
	{ what, when, kind }: PlanParts,
): DeletePlan {
	const { name, deviceId } = row;
	return {
		name,
		deviceId,
		sub: t("keys.delete.subNever", "{{kind}} · not backed up · the only copy", {
			kind,
		}),
		strength: "typed",
		typed: name,
		rows: {
			what,
			who: (
				<Trans
					t={t}
					i18nKey="keys.delete.whoNever"
					defaults="You won't be able to manage <1/> from here, or from anywhere, unless another computer has its keys."
					components={{ 1: <Mono>{name}</Mono> }}
				/>
			),
			stays: t(
				"keys.delete.staysNever",
				"The device keeps running as it is, but nobody can change it.",
			),
			when,
			undo: {
				reversible: false,
				text: fileSaved
					? t(
							"keys.delete.undoFileOnly",
							"There's no account backup. Only a backup file you saved can bring them back.",
						)
					: t(
							"keys.delete.undoNoBackup",
							"No backup exists, on your account or in a file saved here.",
						),
			},
			first:
				row.category === "pending"
					? t(
							"keys.delete.firstRetry",
							"Retry the upload first, so your account has the newest copy.",
						)
					: t(
							"keys.delete.firstBackUp",
							"Back up to your account first: close this and choose Back up to account.",
						),
		},
		done: (at) => ({
			tone: "warning",
			text: t(
				"keys.delete.doneNever",
				"Keys for {{device}} deleted at {{at}}. No copy is left here or on your account; set the device up again to manage it.",
				{ device: name, at },
			),
		}),
	};
}

/** The account's backups could not be compared: nothing is claimed about a copy there, and the name is typed. */
function uncheckedPlan(
	t: DevicesT,
	row: KeyRow,
	{ what, when, kind }: PlanParts,
): DeletePlan {
	const { name, deviceId } = row;
	return {
		name,
		deviceId,
		sub: t(
			"keys.delete.subUnchecked",
			"{{kind}} · account backup not checked",
			{
				kind,
			},
		),
		strength: "typed",
		typed: name,
		rows: {
			what,
			who: (
				<Trans
					t={t}
					i18nKey="keys.delete.whoUnchecked"
					defaults="You won't be able to manage <1/> from here until its keys are back. Whether your account holds a backup hasn't been checked."
					components={{ 1: <Mono>{name}</Mono> }}
				/>
			),
			stays: t(
				"keys.delete.staysUnchecked",
				"The device keeps running as it is.",
			),
			when,
			undo: {
				reversible: null,
				text: t(
					"keys.delete.undoUnchecked",
					"Only if your account holds a backup, or you saved a backup file.",
				),
			},
			first: t(
				"keys.delete.firstCheck",
				"Check backups first, so you know whether your account holds a copy.",
			),
		},
		done: (at) => ({
			tone: "warning",
			text: t(
				"keys.delete.doneUnchecked",
				"Keys for {{device}} deleted at {{at}}. Check backups to see whether your account holds a copy to restore.",
				{ device: name, at },
			),
		}),
	};
}

function localPlan(t: DevicesT, row: LocalOnlyRow): DeletePlan {
	const { name, deviceId } = row;
	const kind = keyKindLabel(t, row.role);
	const device = <Mono>{name}</Mono>;
	const when = t("keys.delete.when", "Immediately.");
	const done = (at: string) => ({
		tone: "good" as const,
		text: t(
			"keys.delete.doneLocal",
			"Keys for {{device}} were deleted from this computer at {{at}}.",
			{ device: name, at },
		),
	});
	if (row.reason === "request")
		return {
			name,
			deviceId,
			sub: t(
				"keys.delete.subRequest",
				"Shared-access request · not approved yet",
			),
			strength: "none",
			rows: {
				what: (
					<Trans
						t={t}
						i18nKey="keys.delete.whatRequest"
						defaults="The keys created for your access request to <1/> are removed from this computer."
						components={{ 1: device }}
					/>
				),
				who: t(
					"keys.delete.whoRequest",
					"The owner isn't told. If the request is approved later, it won't work from here.",
				),
				when,
				undo: {
					reversible: false,
					text: t(
						"keys.delete.undoRequest",
						"Request access again to get new keys.",
					),
				},
			},
			done,
		};
	return {
		name,
		deviceId,
		sub:
			row.reason === "revoked"
				? t("keys.delete.subRevoked", "{{kind}} · device revoked", { kind })
				: t("keys.delete.subUnlisted", "{{kind}} · not in your device list", {
						kind,
					}),
		strength: "none",
		rows: {
			what: (
				<Trans
					t={t}
					i18nKey="keys.delete.what"
					defaults="The keys for <1/> are removed from this computer."
					components={{ 1: device }}
				/>
			),
			who: t(
				"keys.delete.whoOwner",
				"Nobody else. Other computers keep their keys.",
			),
			stays:
				row.reason === "revoked"
					? t(
							"keys.delete.staysRevoked",
							"The device stays revoked; its cloud approvals stay visible.",
						)
					: undefined,
			when,
			undo:
				row.reason === "revoked"
					? {
							reversible: null,
							text: t(
								"keys.delete.undoRevoked",
								"These keys can't be used any more.",
							),
						}
					: {
							reversible: false,
							text: t(
								"keys.delete.undoUnlisted",
								"Only a backup you saved for this device can bring them back.",
							),
						},
		},
		done,
	};
}

/** SPEC §5.9 "Delete keys from this computer…": the backup state decides the confirmation strength. */
export function deletePlan(
	t: DevicesT,
	target: DeleteTarget,
	files: KeyFileLog,
): DeletePlan {
	return target.kind === "device"
		? devicePlan(
				t,
				target.row,
				files[target.row.deviceId]?.savedAt !== undefined,
			)
		: localPlan(t, target.row);
}

/** Runs the delete through the action layer: gate, confirm with the consequence rows, one call. */
export function useDeleteKeys(files: KeyFileLog) {
	const { t } = useTranslation("devices");
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const [busy, setBusy] = useState<string | null>(null);
	const run = useCallback(
		async (target: DeleteTarget) => {
			const plan = deletePlan(t, target, files);
			const flow = guard();
			const scope =
				target.kind === "device" ? rowScope(plan.deviceId) : LOCAL_ONLY_RESULT;
			setBusy(plan.deviceId);
			const outcome = await actions.deleteKeys(
				{ deviceId: plan.deviceId, name: plan.name },
				{
					consequence: plan.rows,
					strength: plan.strength,
					confirm: {
						icon: Trash2,
						title: t(
							"keys.delete.title",
							"Delete the keys for {{device}} from this computer?",
							{ device: plan.name },
						),
						sub: plan.sub,
						tone: "danger",
						...(plan.typed ? { typed: plan.typed } : {}),
						...(plan.checkLabel ? { checkLabel: plan.checkLabel } : {}),
					},
				},
			);
			if (flow.alive()) setBusy(null);
			if (outcome.ok) {
				const done = plan.done(actions.timeNow());
				results.put(scope, done.tone, done.text);
			} else if (outcome.text) results.put(scope, "critical", outcome.text);
			return outcome.ok;
		},
		[t, files, actions, results, guard],
	);
	return { busy, run };
}

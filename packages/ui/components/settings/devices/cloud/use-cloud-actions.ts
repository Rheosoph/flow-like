"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import {
	type ApprovalDraft,
	type SpendingDraft,
	approvalRequest,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	GateContext,
	GateResult,
} from "../../../../lib/device-management/model/types";
import {
	approveDeviceBilling,
	createDeviceResourceGrant,
	revokeDeviceGrant,
} from "../../../../lib/device-resources";
import { useAreaTime } from "../primitives/area-context";
import type { Gate } from "../primitives/gate-notice";
import type { ResultTone } from "../primitives/inline-result";
import {
	type DeviceActionOutcome,
	type GateTarget,
	useAttentionState,
	useDeviceAction,
	useGates,
} from "../workspace";
import type { CloudApproval } from "./cloud-model";
import { money, useGateOf, useMoney, usePersonName } from "./cloud-parts";

/** Which block an action belongs to: the approval or its spending limit. */
export type CloudActionGroup = "approval" | "limit";

interface Sentence {
	tone: ResultTone;
	text: string;
}

export interface ActionNote extends Sentence {
	about: CloudActionGroup;
}

/** What a service needs to get a new approval. */
export interface ServiceIdentity {
	projectId: string;
	deploymentId: string;
	/** null for a local-only app. */
	appId: string | null;
}

export interface CloudSubject {
	deviceId: string;
	deviceName: string;
	serviceId: string;
	/** The approval the controls act on (revoke, replace, limit). */
	approval?: CloudApproval | undefined;
	identity?: ServiceIdentity | undefined;
	/** Offline copies: "model access" wording. */
	modelOnly?: boolean;
	projectRole?: GateContext["projectRole"];
	/** The context line of the confirm sheet ("App Invoice AI · on edge-berlin-01"). */
	sub?: string;
}

export type CloudActionName =
	| "revokeApproval"
	| "revokeLimit"
	| "approve"
	| "addLimit";

export interface CloudActions {
	revokeApprovalGate: Gate | null;
	revokeLimitGate: Gate | null;
	/** New approval, or replacing the current one. */
	approveGate: Gate | null;
	/** New limit, or replacing the current one. */
	limitGate: Gate | null;
	revokeApproval(): Promise<boolean>;
	revokeLimit(): Promise<boolean>;
	approve(
		draft: ApprovalDraft,
		spending: SpendingDraft | null,
	): Promise<boolean>;
	addLimit(spending: SpendingDraft): Promise<boolean>;
	/** The action that runs (from the click to its outcome), for that control's busy state. */
	running: CloudActionName | undefined;
	/** Where the action layer files its running and failure sentences, per group. */
	resultKeys: Record<CloudActionGroup, string>;
	/** The sentence of the last finished action (R9). */
	note: ActionNote | undefined;
	dismissNote(): void;
}

const GROUP: Record<CloudActionName, CloudActionGroup> = {
	revokeApproval: "approval",
	approve: "approval",
	revokeLimit: "limit",
	addLimit: "limit",
};

const cloudResultKey = (
	deviceId: string,
	serviceId: string,
	group: CloudActionGroup,
) => `cloud:${deviceId}/${serviceId}/${group}`;

const firstFailure = (...gates: (GateResult | undefined)[]) =>
	gates.find((gate): gate is Exclude<GateResult, { ok: true }> =>
		gate ? !gate.ok : false,
	);

/** Revoke, approve and limit actions of one service's cloud access (SPEC §6.5), with gates and result sentences. */
export function useCloudActions(subject: CloudSubject): CloudActions {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { workspace } = useAttentionState();
	const actions = useDeviceAction();
	const queryClient = useQueryClient();
	const gateOf = useGateOf();
	const spent = useMoney();
	const { deviceId, deviceName, serviceId, approval, identity, modelOnly } =
		subject;
	const limit = approval?.limit;
	const live = approval?.state === "active";
	const limitLive = limit?.state === "active";
	const projectId =
		identity?.projectId ?? approval?.details?.projectId ?? approval?.appId;
	const appId = identity?.appId ?? approval?.appId ?? null;
	const approverName = usePersonName(approval?.details?.approvedBy);
	const payerName = usePersonName(limit?.payerId);

	const [note, setNote] = useState<ActionNote>();
	const [running, setRunning] = useState<CloudActionName>();
	const current = useRef(workspace);
	current.current = workspace;
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);

	const target = useMemo<GateTarget>(
		() => ({
			placementId: serviceId,
			...(projectId ? { projectId } : {}),
			labels: { service: serviceId },
			...(subject.projectRole ? { projectRole: subject.projectRole } : {}),
			extra: {
				// Replacing revokes the current approval first, so nothing stands in the way of a new one.
				approvalExists: false,
				...(approval ? { delegatorIsMe: approval.approverIsMe } : {}),
				...(approval?.details
					? { approvalHasModels: approval.details.models.length > 0 }
					: {}),
				...(limit ? { payerIsMe: limit.payerIsMe } : {}),
			},
		}),
		[serviceId, projectId, subject.projectRole, approval, limit],
	);
	const gates = useGates(
		[
			"cloud_access_create",
			"cloud_access_revoke",
			"spending_limit_create",
			"spending_limit_revoke",
		],
		deviceId,
		target,
	);

	const onlyApprover = approverName
		? t("cloud.gate.onlyApprover", "Only {{name}} can revoke this approval.", {
				name: approverName,
			})
		: undefined;
	const onlyPayer = payerName
		? t(
				"cloud.gate.onlyPayer",
				"Only {{name}}, who pays, can revoke this limit.",
				{ name: payerName },
			)
		: undefined;
	const reasonFor = (gate: GateResult) =>
		gate.ok
			? undefined
			: gate.copy.code === "delegator_or_owner_only"
				? onlyApprover
				: gate.copy.code === "payer_only"
					? onlyPayer
					: undefined;
	const shown = (gate: GateResult | undefined) =>
		gate ? gateOf(gate, reasonFor(gate)) : null;

	// The hub takes no second approval for a service while one can still be revoked.
	const approveFailure = firstFailure(
		approval?.revocable ? gates.cloud_access_revoke : undefined,
		gates.cloud_access_create,
	);
	const limitFailure = firstFailure(
		limitLive ? gates.spending_limit_revoke : undefined,
		gates.spending_limit_create,
	);
	const limitGate: Gate | null =
		approval && !live
			? {
					kind: "busy",
					reason: modelOnly
						? t(
								"cloud.gate.approveModelFirst",
								"Approve model access again first.",
							)
						: t("cloud.gate.approveFirst", "Approve cloud access again first."),
				}
			: shown(limitFailure);

	const resultKeys = useMemo(
		() => ({
			approval: cloudResultKey(deviceId, serviceId, "approval"),
			limit: cloudResultKey(deviceId, serviceId, "limit"),
		}),
		[deviceId, serviceId],
	);
	const invalidate = useMemo(
		() => [
			deviceKeys.resources(workspace.scopeKey, deviceId),
			deviceKeys.resourceSummary(workspace.scopeKey),
			...(appId ? [deviceKeys.appPlacements(workspace.scopeKey, appId)] : []),
		],
		[workspace.scopeKey, deviceId, appId],
	);

	/** Completions that arrive after the account changed or the control is gone are dropped. */
	const finish = useCallback(
		async <T>(
			name: CloudActionName,
			run: Promise<DeviceActionOutcome<T>>,
			done: (result: T) => Sentence,
			partial?: () => Sentence | undefined,
		): Promise<boolean> => {
			const started = current.current;
			const about = GROUP[name];
			setRunning(name);
			const outcome = await run;
			if (!mounted.current) return false;
			setRunning(undefined);
			if (current.current !== started) return false;
			if (outcome.status === "done") {
				setNote({ ...done(outcome.result), about });
				return true;
			}
			const failed =
				outcome.status === "failed" || outcome.status === "rejected";
			// The hub may have applied it before the answer was lost: show what it has now.
			if (failed || outcome.status === "unknown")
				for (const queryKey of invalidate)
					void queryClient.invalidateQueries({ queryKey });
			const half = failed ? partial?.() : undefined;
			if (half) setNote({ ...half, about });
			return false;
		},
		[invalidate, queryClient],
	);

	const at = () => time.clock(time.nowS, false);
	const sub = subject.sub;
	const confirm = { ...(sub ? { sub } : {}), tone: "danger" as const };
	const api = () => workspace.hub.api;
	const profile = () => workspace.hub.profile;
	const access = modelOnly
		? t("cloud.word.modelAccess", "model access")
		: t("cloud.word.cloudAccess", "cloud access");

	const revokeApproval = () => {
		if (!approval) return Promise.resolve(false);
		setNote(undefined);
		return finish(
			"revokeApproval",
			actions.run({
				action: "cloud_access_revoke",
				deviceId,
				target,
				label: t(
					"cloud.action.revokeApproval",
					"Revoke the {{access}} of {{service}}",
					{ access, service: serviceId },
				),
				consequence: {
					what: t(
						"cloud.conseq.revokeApprovalWhat",
						"{{service}}'s instances lose {{access}} within minutes.",
						{ service: serviceId, access },
					),
					who: modelOnly
						? t("cloud.conseq.revokeModelWho", "Its model calls fail.")
						: t(
								"cloud.conseq.revokeApprovalWho",
								"Model calls and project-file access fail; buffered writes pause and are kept.",
							),
					when: t(
						"cloud.conseq.revokeApprovalWhen",
						"Credentials already issued stay valid for up to 10 minutes. In-flight requests may still complete and bill.",
					),
					undo: {
						reversible: true,
						text: t(
							"cloud.conseq.revokeApprovalUndo",
							"Approve again; the service's settings then need to name the new approval.",
						),
					},
				},
				strength: "none",
				confirm,
				resultKey: resultKeys.approval,
				invalidate,
				call: () =>
					revokeDeviceGrant(
						api(),
						profile(),
						deviceId,
						"resource",
						approval.grantId,
					),
			}),
			() => ({
				tone: "good",
				text: t(
					"cloud.result.approvalRevoked",
					"The {{access}} of {{service}} was revoked at {{time}}. Credentials already issued stay valid for up to 10 minutes.",
					{ access, service: serviceId, time: at() },
				),
			}),
		);
	};

	const revokeLimit = () => {
		if (!limit) return Promise.resolve(false);
		setNote(undefined);
		return finish(
			"revokeLimit",
			actions.run({
				action: "spending_limit_revoke",
				deviceId,
				target,
				label: t(
					"cloud.action.revokeLimit",
					"Revoke the spending limit of {{service}}",
					{ service: serviceId },
				),
				consequence: {
					what: t(
						"cloud.conseq.revokeLimitWhat",
						"No new model requests are charged to you.",
					),
					who: t(
						"cloud.conseq.revokeLimitWho",
						"{{service}}'s model calls fail once its credentials expire.",
						{ service: serviceId },
					),
					when: t(
						"cloud.conseq.revokeLimitWhen",
						"Right away for new requests. Requests already running may still complete and bill.",
					),
					undo: {
						reversible: true,
						text: t("cloud.conseq.revokeLimitUndo", "Add a new limit."),
					},
				},
				strength: "none",
				confirm,
				resultKey: resultKeys.limit,
				invalidate,
				call: () =>
					revokeDeviceGrant(api(), profile(), deviceId, "billing", limit.id),
			}),
			() => ({
				tone: "good",
				text: t(
					"cloud.result.limitRevoked",
					"The spending limit on {{device}} was revoked at {{time}}. You won't be charged for new requests.",
					{ device: deviceName, time: at() },
				),
			}),
		);
	};

	const approve = (draft: ApprovalDraft, spending: SpendingDraft | null) => {
		if (!identity) return Promise.resolve(false);
		setNote(undefined);
		const replaced = approval?.revocable ? approval : undefined;
		let stage: "none" | "revoked" | "approved" = "none";
		const label = replaced
			? t(
					"cloud.action.replaceApproval",
					"Replace the {{access}} of {{service}}",
					{
						access,
						service: serviceId,
					},
				)
			: t("cloud.action.approve", "Approve {{access}} for {{service}}", {
					access,
					service: serviceId,
				});
		return finish(
			"approve",
			actions.run({
				action: "cloud_access_create",
				deviceId,
				target,
				label,
				...(replaced
					? {
							consequence: {
								what: t(
									"cloud.conseq.replaceWhat",
									"The current approval of {{service}} is revoked and a new one is created.",
									{ service: serviceId },
								),
								who: t(
									"cloud.conseq.replaceWho",
									"{{service}} uses the new approval once its settings name it. Until then its model calls and project-file access fail; buffered changes stay with the old approval.",
									{ service: serviceId },
								),
								when: t(
									"cloud.conseq.replaceWhen",
									"Right away. Credentials already issued stay valid for up to 10 minutes.",
								),
								undo: {
									reversible: false,
									text: t(
										"cloud.conseq.replaceUndo",
										"The old approval can't be restored. You can replace the new one again.",
									),
								},
							},
							strength: "none" as const,
							confirm,
						}
					: {}),
				resultKey: resultKeys.approval,
				invalidate,
				call: async () => {
					if (replaced) {
						await revokeDeviceGrant(
							api(),
							profile(),
							deviceId,
							"resource",
							replaced.grantId,
						);
						stage = "revoked";
					}
					const grant = await createDeviceResourceGrant(
						api(),
						profile(),
						deviceId,
						approvalRequest(draft, {
							placementId: serviceId,
							deploymentId: identity.deploymentId,
							projectId: identity.projectId,
							appId: identity.appId,
						}),
					);
					stage = "approved";
					if (spending && draft.models.length)
						await approveDeviceBilling(
							api(),
							profile(),
							deviceId,
							grant.grant_id,
							{
								limit_micros: spending.limitMicros,
								expires_at: Math.min(spending.expiresAt, grant.expires_at),
							},
						);
					return grant;
				},
			}),
			(grant) => ({
				tone: "good",
				text: t(
					"cloud.result.approved",
					"Approved at {{time}}, until {{until}}. {{service}} uses it once its settings name this approval.",
					{
						time: at(),
						until: time.at(grant.expires_at),
						service: serviceId,
					},
				),
			}),
			() =>
				stage === "approved"
					? {
							tone: "warning",
							text: t(
								"cloud.result.approvedNoLimit",
								"The approval was created, but its spending limit wasn't. Hosted models refuse {{service}}'s calls until you add one.",
								{ service: serviceId },
							),
						}
					: stage === "revoked"
						? {
								tone: "warning",
								text: t(
									"cloud.result.replaceHalf",
									"The old approval was revoked, but the new one wasn't created. {{service}} has no {{access}} until you approve again.",
									{ service: serviceId, access },
								),
							}
						: undefined,
		);
	};

	const addLimit = (spending: SpendingDraft) => {
		if (!approval) return Promise.resolve(false);
		setNote(undefined);
		const replaced = limitLive ? limit : undefined;
		const amount = money(spending.limitMicros);
		const until = time.at(Math.min(spending.expiresAt, approval.expiresAt));
		let revoked = false;
		return finish(
			"addLimit",
			actions.run({
				action: "spending_limit_create",
				deviceId,
				target,
				label: replaced
					? t(
							"cloud.action.replaceLimit",
							"Replace the spending limit of {{service}}",
							{ service: serviceId },
						)
					: t("cloud.action.addLimit", "Add a spending limit for {{service}}", {
							service: serviceId,
						}),
				consequence: {
					what: replaced
						? t(
								"cloud.conseq.limitReplaceWhat",
								"Model calls from {{service}} are charged to you, up to {{amount}}. The current limit ({{used}} of {{limit}} used) closes; the new one starts from {{zero}}.",
								{
									service: serviceId,
									amount,
									used: spent(replaced.used),
									limit: money(replaced.limit),
									zero: spent(0),
								},
							)
						: t(
								"cloud.conseq.limitWhat",
								"Model calls from {{service}} are charged to you, up to {{amount}}.",
								{ service: serviceId, amount },
							),
					who: t(
						"cloud.conseq.limitWho",
						"You pay. Your plan's model prices apply.",
					),
					when: t(
						"cloud.conseq.limitWhen",
						"From now until {{until}}. Not recurring: it doesn't renew.",
						{ until },
					),
					undo: {
						reversible: true,
						text: t(
							"cloud.conseq.limitUndo",
							"Revoke the limit at any time; no new requests are charged after that.",
						),
					},
				},
				strength: "review",
				confirm: sub ? { sub } : {},
				resultKey: resultKeys.limit,
				invalidate,
				call: async () => {
					if (replaced) {
						await revokeDeviceGrant(
							api(),
							profile(),
							deviceId,
							"billing",
							replaced.id,
						);
						revoked = true;
					}
					return approveDeviceBilling(
						api(),
						profile(),
						deviceId,
						approval.grantId,
						{
							limit_micros: spending.limitMicros,
							expires_at: Math.min(spending.expiresAt, approval.expiresAt),
						},
					);
				},
			}),
			() => ({
				tone: "good",
				text: replaced
					? t(
							"cloud.result.limitReplaced",
							"A spending limit of {{amount}} replaced the old one at {{time}}. Paid by you until {{until}}.",
							{ amount, time: at(), until },
						)
					: t(
							"cloud.result.limitAdded",
							"A spending limit of {{amount}} was added at {{time}}. Paid by you until {{until}}.",
							{ amount, time: at(), until },
						),
			}),
			() =>
				revoked
					? {
							tone: "warning",
							text: t(
								"cloud.result.limitHalf",
								"The old limit was revoked, but the new one wasn't added. Hosted models refuse {{service}}'s calls until you add a limit.",
								{ service: serviceId },
							),
						}
					: undefined,
		);
	};

	return {
		revokeApprovalGate: shown(gates.cloud_access_revoke),
		revokeLimitGate: shown(gates.spending_limit_revoke),
		approveGate: shown(approveFailure),
		limitGate,
		revokeApproval,
		revokeLimit,
		approve,
		addLimit,
		running,
		resultKeys,
		note,
		dismissNote: () => setNote(undefined),
	};
}

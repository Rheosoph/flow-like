"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleX, LogOut } from "lucide-react";
import { type MouseEvent, useCallback, useMemo, useState } from "react";
import { toHubError } from "../../../../lib/device-management/hub/endpoints";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type {
	DevicesRoute,
	SetupStep,
} from "../../../../lib/device-management/model/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { useAreaTime } from "../primitives/area-context";
import { useConfirm } from "../primitives/confirm-sheet";
import type { ConsequenceRows } from "../primitives/consequence-preview";
import {
	type RouteLinkProps,
	useDevicesRoute,
	useRouteLink,
} from "../routing/use-devices-route";
import {
	hubErrorCopy,
	useDeviceAction,
	useDeviceWorkspace,
} from "../workspace";
import type { SetupLimits } from "./setup-context";
import { type SetupDraft, WAIT_STEP } from "./setup-state";
import {
	type SetupCreate,
	dropSetupActivity,
	setupActivityId,
} from "./use-setup-create";

export interface SetupLeaveInput {
	draft: SetupDraft;
	/** `new`, or the enrollment id in the URL. */
	slot: string;
	step: SetupStep;
	expired: boolean;
	create: SetupCreate;
	limits: SetupLimits;
	update(patch: Partial<SetupDraft>): void;
	/** Forget what the window holds for this setup (password fields). */
	clearSecrets(): void;
}

export interface SetupLeave {
	cancelSetup(): Promise<boolean>;
	cancelError: string | undefined;
	dismissCancelError(): void;
	leave(route: DevicesRoute): Promise<void>;
	leaveLink(route: DevicesRoute): RouteLinkProps;
}

function isPlainClick(event: MouseEvent<HTMLAnchorElement>): boolean {
	return (
		!event.defaultPrevented &&
		event.button === 0 &&
		!(event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
	);
}

/**
 * Cancelling a setup and leaving the wizard (SPEC §6.5): the hub call goes
 * through the action layer with its consequence rows, and leaving asks first
 * when a running creation or the files of this window would be lost.
 */
export function useSetupLeave(input: SetupLeaveInput): SetupLeave {
	const { draft, slot, step, expired, create, limits, update, clearSecrets } =
		input;
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const { scopeKey } = workspace;
	const { navigate } = useDevicesRoute();
	const routeLink = useRouteLink();
	const confirm = useConfirm();
	const actions = useDeviceAction();
	const time = useAreaTime();
	const [cancelError, setCancelError] = useState<string>();

	const { created } = draft;
	const { name } = draft;
	const registration = create.run.registration;
	const savedId = created?.enrollmentId;
	const enrollmentId = savedId ?? registration?.enrollmentId;
	const creating = create.run.status === "running";
	const { abort, settled, registered } = create;

	/** Stops a running creation, then cancels what the hub registered; resolves to the cancelled setup's id. */
	const cancelAtHub = useCallback(
		async (target: DeviceWorkspace) => {
			abort();
			await settled();
			const id = savedId ?? registered()?.enrollmentId;
			if (!id) return undefined;
			try {
				await target.deps.api.del(
					target.deps.profile,
					`devices/enrollments/${encodeURIComponent(id)}`,
				);
			} catch (error) {
				// A run that stops before the package exists releases its own reservation.
				if (toHubError(error).code !== "not_found") throw error;
			}
			return id;
		},
		[abort, settled, registered, savedId],
	);

	const invalidate = useMemo(
		() => [
			deviceKeys.enrollments(scopeKey, "open"),
			deviceKeys.enrollments(scopeKey, "recent"),
			deviceKeys.usage(scopeKey),
			deviceKeys.accountBackups(scopeKey),
		],
		[scopeKey],
	);

	const { reset } = create;
	/** The window lets go of the run: its tray entry, its files and the password. */
	const forgetRun = useCallback(
		(cancelledId: string | undefined) => {
			const itemId = setupActivityId(workspace, cancelledId);
			if (itemId) dropSetupActivity(workspace, itemId);
			reset();
			clearSecrets();
		},
		[workspace, reset, clearSecrets],
	);
	const markCancelled = useCallback(
		(cancelledId: string | undefined) => {
			forgetRun(cancelledId);
			update({ cancelledAt: Math.floor(workspace.clock.now() / 1000) });
		},
		[forgetRun, workspace, update],
	);

	const irreversible = useMemo(
		() => ({
			when: t("setup.cancel.when", "Immediately."),
			undo: {
				reversible: false,
				text: t("setup.cancel.undo", "Create a new package."),
			},
		}),
		[t],
	);

	const cancelSetup = useCallback(async () => {
		setCancelError(undefined);
		const createdAt = created?.createdAt ?? registration?.createdAt;
		const expiresAt = created?.expiresAt ?? registration?.expiresAt;
		const consequence: ConsequenceRows = {
			what: t(
				"setup.cancel.what",
				"The setup package for {{name}} stops working.",
				{ name },
			),
			who:
				limits.maxPending === undefined
					? t(
							"setup.cancel.whoPlain",
							"Frees its unused-package slot. It still counts toward today's packages.",
						)
					: t(
							"setup.cancel.who",
							"Frees 1 of your {{max, number}} unused-package slots. It still counts toward today's packages.",
							{ max: limits.maxPending },
						),
			stays: t(
				"setup.cancel.stays",
				"Nothing was installed yet. The keys made for it stay on this computer until you delete them.",
			),
			...irreversible,
		};
		const outcome = await actions.run({
			action: "cancel_setup",
			label: t("setup.cancel.label", "Cancel setup for {{name}}", { name }),
			resultKey: `setup:cancel:${enrollmentId ?? slot}`,
			consequence,
			confirm: {
				icon: CircleX,
				tone: "danger",
				title: t("setup.cancel.title", "Cancel the setup for {{name}}?", {
					name,
				}),
				...(createdAt !== undefined && expiresAt !== undefined
					? {
							sub: t(
								"setup.cancel.subtitle",
								"Setup package created {{created}} · expires {{expires}}",
								{ created: time.at(createdAt), expires: time.at(expiresAt) },
							),
						}
					: {}),
			},
			call: ({ workspace: target }) => cancelAtHub(target),
			invalidate,
		});
		if (outcome.status === "done") markCancelled(outcome.result);
		else if (outcome.status === "failed")
			setCancelError(hubErrorCopy(t, toHubError(outcome.error).code));
		return outcome.status === "done";
	}, [
		actions,
		t,
		time,
		name,
		created,
		registration,
		enrollmentId,
		slot,
		limits.maxPending,
		irreversible,
		cancelAtHub,
		invalidate,
		markCancelled,
	]);

	const stopCreating = useCallback(async () => {
		const outcome = await actions.run({
			action: "cancel_setup",
			label: t("setup.stop.label", "Stop and cancel setup"),
			resultKey: `setup:stop:${slot}`,
			consequence: {
				what: t(
					"setup.stop.what",
					"Creating stops and the setup is cancelled at the hub, so no half-made package keeps a slot.",
				),
				who: t("setup.stop.who", "Nobody else. The device isn't set up yet."),
				stays: t(
					"setup.stop.stays",
					"Your choices stay in this window if you come back.",
				),
				...irreversible,
			},
			confirm: {
				icon: CircleX,
				tone: "danger",
				title: t(
					"setup.stop.title",
					"Stop creating the package for {{name}}?",
					{ name },
				),
				sub: t(
					"setup.stop.subtitle",
					"Creating is still running in this window",
				),
			},
			call: ({ workspace: target }) => cancelAtHub(target),
			invalidate,
		});
		// No package existed yet, so the draft stays open: the choices are kept for coming back.
		if (outcome.status === "done") forgetRun(outcome.result);
		return outcome.status === "done";
	}, [
		actions,
		t,
		slot,
		name,
		irreversible,
		cancelAtHub,
		invalidate,
		forgetRun,
	]);

	/** Names what leaving loses: the package of this window, unless it was saved (SPEC §5.5). */
	const confirmLeave = useCallback(
		async (expiresAt: number, saved: boolean) => {
			const until = time.at(expiresAt);
			const answer = await confirm({
				icon: LogOut,
				tone: saved ? "default" : "danger",
				strength: "none",
				title: t("setup.leave.title", "Leave the setup for {{name}}?", {
					name,
				}),
				sub: t("setup.leave.subtitle", "Package works until {{until}}", {
					until,
				}),
				confirmLabel: t("setup.leave.confirm", "Leave setup"),
				rows: {
					what: saved
						? t(
								"setup.leave.whatSaved",
								"You leave the setup. You saved the package, so you can still start it on the device.",
							)
						: t(
								"setup.leave.whatLost",
								"You leave the setup. The package for {{name}} is discarded from this window and can't be downloaded again.",
								{ name },
							),
					who: t("setup.leave.who", "Nobody. The device isn't set up yet."),
					stays: t(
						"setup.leave.stays",
						"The setup stays in Pending setups until {{until}} and keeps its unused-package slot. The keys stay on this computer.",
						{ until },
					),
					when: t("setup.cancel.when", "Immediately."),
					undo: saved
						? {
								reversible: true,
								text: t(
									"setup.leave.undoSaved",
									"Open Start instructions from Pending setups to come back.",
								),
							}
						: {
								reversible: false,
								text: t(
									"setup.leave.undoLost",
									"If you didn't save the package, cancel this setup and create a new one.",
								),
							},
					...(saved
						? {}
						: {
								first: t(
									"setup.leave.first",
									"Stay here and download the setup package if you still need it.",
								),
							}),
				},
			});
			return answer.ok;
		},
		[confirm, t, time, name],
	);

	const settledElsewhere =
		!!draft.cancelledAt ||
		expired ||
		draft.registeredAt !== undefined ||
		step === WAIT_STEP;
	const holdsFiles = !!created && !!create.built;
	const expiresAt = created?.expiresAt;
	const saved = draft.packageSaved;

	const leave = useCallback(
		async (to: DevicesRoute) => {
			if (!settledElsewhere) {
				if (creating && !(await stopCreating())) return;
				if (
					!creating &&
					holdsFiles &&
					expiresAt !== undefined &&
					!(await confirmLeave(expiresAt, saved))
				)
					return;
			}
			navigate(to);
		},
		[
			settledElsewhere,
			creating,
			holdsFiles,
			expiresAt,
			saved,
			stopCreating,
			confirmLeave,
			navigate,
		],
	);

	const leaveLink = useCallback(
		(to: DevicesRoute): RouteLinkProps => ({
			href: routeLink(to).href,
			onClick: (event) => {
				if (!isPlainClick(event)) return;
				event.preventDefault();
				void leave(to);
			},
		}),
		[routeLink, leave],
	);

	const dismissCancelError = useCallback(() => setCancelError(undefined), []);

	return { cancelSetup, cancelError, dismissCancelError, leave, leaveLink };
}

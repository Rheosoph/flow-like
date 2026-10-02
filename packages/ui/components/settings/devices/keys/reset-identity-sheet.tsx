"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { RefreshCcwDot } from "lucide-react";
import { type FormEvent, useMemo, useRef, useState } from "react";
import { enumLabel } from "../copy/enum-labels";
import { ConsequencePreview } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { Field, SecretInput, utf8Bytes } from "../primitives/form-fields";
import { useRouteLink } from "../routing/use-devices-route";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { PASSWORD_MAX_BYTES, PASSWORD_MIN_BYTES } from "./key-operations";
import { Mono, useAppNames } from "./key-parts";
import { rowScope, useKeyResults } from "./key-store";
import type { KeyRow } from "./keys-model";
import { useFlowGuard, useKeyActions } from "./use-key-actions";

/** The shared live-metrics groups this computer read on the device, as far as it knows them. */
function useAffectedGroups(deviceId: string): string[] | undefined {
	const workspace = useDeviceWorkspace();
	return useMemo(() => {
		const readers = workspace.facts.get(deviceId)?.metricReaders;
		return readers
			? [...new Set(readers.map((entry) => entry.scope))]
			: undefined;
	}, [workspace, deviceId]);
}

function GroupList({ scopes }: Readonly<{ scopes: readonly string[] }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const appName = useAppNames();
	return (
		<ul
			data-affected-groups=""
			className="mt-1 flex list-none flex-col gap-0.5 p-0"
		>
			{scopes.map((scope) => (
				<li key={scope}>
					{scope === "device" ? (
						enumLabel(t, "scopeKind", "device")
					) : (
						<a
							{...link(
								{ screen: "app-devices", by: "device" },
								{ scope: { kind: "app", appId: scope } },
							)}
							className="underline decoration-border-strong underline-offset-2 hover:decoration-current"
						>
							{enumLabel(t, "scopeKind", "project", {
								name: appName(scope) ?? scope,
							})}
						</a>
					)}
				</li>
			))}
		</ul>
	);
}

/**
 * IA S23 "Reset metric-group identity" (advanced): the consequence rows name
 * every group that needs the owner's approval again, then the device
 * password confirms.
 */
export function ResetIdentitySheet({
	row,
	onClose,
}: Readonly<{ row: KeyRow; onClose(): void }>) {
	const { t } = useTranslation("devices");
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard(row.deviceId);
	const working = useRef(false);
	const groups = useAffectedGroups(row.deviceId);
	const [password, setPassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const device = row.name;
	const short = utf8Bytes(password) < PASSWORD_MIN_BYTES;

	const submit = async (event: FormEvent) => {
		event.preventDefault();
		if (working.current || short) return;
		const secret = password;
		const flow = guard();
		working.current = true;
		setPassword("");
		setError(undefined);
		setBusy(true);
		const outcome = await actions.resetMetricIdentity(
			{ deviceId: row.deviceId, name: device },
			secret,
		);
		if (!flow.alive()) return;
		working.current = false;
		setBusy(false);
		if (!outcome.ok) {
			if (outcome.text) setError(outcome.text);
			return;
		}
		results.put(
			rowScope(row.deviceId),
			"good",
			t(
				"keys.reset.done",
				"This computer has a fresh identity for shared live metrics on {{device}} since {{at}}. The device is locked here; unlock it, then ask each group's owner to approve this computer again.",
				{ device, at: actions.timeNow() },
			),
		);
		onClose();
	};

	const formId = `keys-reset-${row.deviceId}`;
	return (
		<DvSheet
			open
			role="alertdialog"
			closeOnOutside={false}
			onOpenChange={(open) => {
				if (!open && !busy) onClose();
			}}
			icon={RefreshCcwDot}
			eyebrow={t("keys.reset.eyebrow", "Advanced")}
			title={t(
				"keys.reset.title",
				"Reset the metric-group identity for {{device}}?",
				{ device },
			)}
			sub={t(
				"keys.reset.subtitle",
				"Use it when this computer can't read shared live metrics any more, for example after restoring keys.",
			)}
			foot={
				<>
					<DvButton onClick={onClose} aria-disabled={busy || undefined}>
						{t("keys.sheet.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="danger"
						type="submit"
						form={formId}
						busy={busy}
						aria-disabled={short || undefined}
					>
						{t("keys.reset.go", "Reset identity")}
					</DvButton>
				</>
			}
		>
			<form
				id={formId}
				className="flex flex-col gap-3.5"
				onSubmit={(event) => void submit(event)}
			>
				<ConsequencePreview
					rows={{
						what: (
							<Trans
								t={t}
								i18nKey="keys.reset.what"
								defaults="This computer gets a new identity for shared live metrics on <1/>. Its keys and your access stay the same."
								components={{ 1: <Mono>{device}</Mono> }}
							/>
						),
						who: (
							<>
								{groups?.length
									? t("keys.reset.whoGroups", {
											count: groups.length,
											defaultValue_one:
												"{{count, number}} group needs its owner's approval again before this computer can read it:",
											defaultValue_other:
												"{{count, number}} groups need their owner's approval again before this computer can read them:",
										})
									: groups
										? t(
												"keys.reset.whoNone",
												"This computer hasn't joined a shared live-metrics group on this device, so nobody has to approve anything.",
											)
										: t(
												"keys.reset.whoUnknown",
												"Every shared live-metrics group this computer joined on this device needs its owner's approval again. Unlock the device first to see which ones.",
											)}
								{groups?.length ? <GroupList scopes={groups} /> : null}
							</>
						),
						stays: t(
							"keys.reset.stays",
							"Services, access rules, the account backup and retained history are untouched.",
						),
						when: t(
							"keys.reset.when",
							"Immediately. The device locks here, and management reconnects with the new identity at the next unlock.",
						),
						undo: {
							reversible: false,
							text: t(
								"keys.reset.undo",
								"The old identity can't be brought back; the groups have to approve the new one.",
							),
						},
					}}
				/>
				<Field
					id={`${formId}-password`}
					label={t(
						"keys.field.devicePassword",
						"Device password for {{device}}",
						{ device },
					)}
					error={error}
				>
					<SecretInput
						value={password}
						onValueChange={setPassword}
						minBytes={PASSWORD_MIN_BYTES}
						maxBytes={PASSWORD_MAX_BYTES}
						disabled={busy}
					/>
				</Field>
			</form>
		</DvSheet>
	);
}

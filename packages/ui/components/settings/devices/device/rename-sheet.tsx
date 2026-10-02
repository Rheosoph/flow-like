"use client";

import { useTranslation } from "@flow-like/locales";
import { PencilLine } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import {
	isMissingOnHub,
	toHubError,
} from "../../../../lib/device-management/hub/endpoints";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import { MAX_DISPLAY_NAME_LENGTH, renameDevice } from "../../../../lib/devices";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { DvInput, Field } from "../primitives/form-fields";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import {
	actionResultKey,
	useDeviceAction,
	useDeviceWorkspace,
	useGate,
	useInlineResults,
} from "../workspace";
import { type DevicePage, type GateView, gateView } from "./use-device-page";

const CONTROL = /\p{Cc}/u;

/** The hub trims and NFC-normalizes; an empty name clears the display name. */
function nameError(t: DevicesT, value: string): string | undefined {
	const name = value.normalize("NFC").trim();
	if ([...name].length > MAX_DISPLAY_NAME_LENGTH)
		return t(
			"devices:device.rename.tooLong",
			"Use at most {{max, number}} characters.",
			{ max: MAX_DISPLAY_NAME_LENGTH },
		);
	if (CONTROL.test(name))
		return t(
			"devices:device.rename.control",
			"Remove line breaks and other control characters.",
		);
	return undefined;
}

export interface RenameSheetProps {
	page: DevicePage;
	open: boolean;
	onOpenChange(open: boolean): void;
	/** The hub answered 404/405 to the rename: it is older than this feature. */
	onUnsupported(): void;
}

/** BG5: a display name shown instead of the name from setup; the device keeps its setup name. */
export function RenameSheet({
	page,
	open,
	onOpenChange,
	onUnsupported,
}: Readonly<RenameSheetProps>) {
	const { t } = useTranslation("devices");
	const id = useId();
	const actions = useDeviceAction();
	const workspace = useDeviceWorkspace();
	const { row } = page.view;
	const [value, setValue] = useState(row.display_name ?? "");
	const [failure, setFailure] = useState<string>();
	const resultKey = actionResultKey("rename_device", page.deviceId);
	const error = nameError(t, value);
	const next = value.normalize("NFC").trim();
	const unchanged = next === (row.display_name ?? "");
	const save = async (event?: FormEvent) => {
		event?.preventDefault();
		if (error || unchanged) return;
		setFailure(undefined);
		const outcome = await actions.run({
			action: "rename_device",
			deviceId: page.deviceId,
			label: next
				? t("device.rename.label", "Rename {{device}} to {{name}}", {
						device: row.name,
						name: next,
					})
				: t("device.rename.labelClear", "Use the setup name for {{device}}", {
						device: row.name,
					}),
			resultKey,
			call: ({ workspace }) =>
				renameDevice(
					workspace.hub.api,
					workspace.hub.profile,
					page.deviceId,
					next || null,
				),
			invalidate: [deviceKeys.list(workspace.scopeKey)],
		});
		if (outcome.status === "done") {
			onOpenChange(false);
			return;
		}
		if (outcome.status !== "failed") return;
		if (isMissingOnHub(outcome.error, "device")) {
			onUnsupported();
			onOpenChange(false);
			return;
		}
		setFailure(toHubError(outcome.error).message);
	};
	return (
		<DvSheet
			open={open}
			onOpenChange={onOpenChange}
			icon={PencilLine}
			title={t("device.rename.title", "Rename {{device}}", {
				device: page.name,
			})}
			sub={t(
				"device.rename.subtitle",
				"Shown everywhere in Flow-Like instead of the name from setup",
			)}
			foot={
				<>
					<DvButton onClick={() => onOpenChange(false)}>
						{t("device.rename.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={actions.pending(resultKey)}
						aria-disabled={error || unchanged ? true : undefined}
						onClick={() => void save()}
					>
						{t("device.rename.save", "Save name")}
					</DvButton>
				</>
			}
		>
			<form
				onSubmit={(event) => void save(event)}
				className="flex flex-col gap-3"
			>
				<Field
					id={`${id}-name`}
					label={t("device.rename.field", "Display name")}
					error={error}
					hint={t(
						"device.rename.hint",
						"Leave it empty to show the setup name, {{name}}. The device itself, its setup package and its logs keep that name.",
						{ name: row.name },
					)}
				>
					<DvInput
						value={value}
						maxLength={MAX_DISPLAY_NAME_LENGTH * 2}
						autoComplete="off"
						spellCheck={false}
						onChange={(event) => setValue(event.target.value)}
					/>
				</Field>
				{failure ? (
					<InlineResult tone="critical" onDismiss={() => setFailure(undefined)}>
						{failure}
					</InlineResult>
				) : null}
			</form>
		</DvSheet>
	);
}

/** Identity layer: the name, Rename… and what happened to the last rename. */
export function RenameControl({
	page,
	open,
	onOpenChange,
}: Readonly<{
	page: DevicePage;
	open: boolean;
	onOpenChange(open: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const result = useGate("rename_device", page.deviceId);
	const results = useInlineResults(
		actionResultKey("rename_device", page.deviceId),
	);
	const [unsupported, setUnsupported] = useState(false);
	const olderHub = page.view.row.display_name === undefined || unsupported;
	const view: GateView | null = olderHub
		? {
				gate: {
					kind: "unsupported",
					reason: t(
						"device.rename.unsupported",
						"Set at setup and can't be changed. This hub doesn't support display names yet.",
					),
				},
			}
		: gateView(t, time, result);
	const button = (
		<DvButton size="sm" icon={PencilLine} onClick={() => onOpenChange(true)}>
			{t("device.rename.open", "Rename…")}
		</DvButton>
	);
	return (
		<div className="flex flex-col items-start gap-2">
			{view ? <GatedAction gate={view.gate}>{button}</GatedAction> : button}
			{results.map((entry) => (
				<InlineResult
					key={entry.id}
					tone={entry.tone}
					onDismiss={entry.dismiss}
				>
					{entry.text}
				</InlineResult>
			))}
			{open && !view ? (
				<RenameSheet
					page={page}
					open
					onOpenChange={onOpenChange}
					onUnsupported={() => setUnsupported(true)}
				/>
			) : null}
		</div>
	);
}

"use client";

import { useTranslation } from "@flow-like/locales";
import { Plus, X } from "lucide-react";
import { useId, useRef, useState } from "react";
import {
	type TunnelService,
	tunnelServicesSchema,
} from "../../../../lib/device-management/tunnel-services";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvInput, DvSelect, Field } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { DEFAULT_APPLY } from "./config-model";
import type { Note } from "./config-parts";
import { type SettingsEditor, outcomeNote } from "./settings-sheets";

export function TunnelListeners({
	editor,
}: Readonly<{ editor: SettingsEditor }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const initial = tunnelServicesSchema.parse(
		editor.configuration.config.tunnel_services ?? [],
	);
	const nextKey = useRef(initial.length);
	const [rows, setRows] = useState(() =>
		initial.map((value, key) => ({ value, key })),
	);
	const [note, setNote] = useState<Note | null>(null);
	const [busy, setBusy] = useState(false);
	const [safe, setSafe] = useState(!editor.safeUnavailable && editor.running);
	const unavailable =
		editor.editGate?.text ??
		(safe ? editor.safeUnavailable : editor.quickUnavailable);
	const values = rows.map((row) => row.value);
	const changed = JSON.stringify(initial) !== JSON.stringify(values);
	const pending = busy || editor.apply.pending;
	const update = (index: number, patch: Partial<TunnelService>) =>
		setRows((current) =>
			current.map((row, at) =>
				at === index ? { ...row, value: { ...row.value, ...patch } } : row,
			),
		);
	const save = async () => {
		if (pending || unavailable) return;
		const parsed = tunnelServicesSchema.safeParse(values);
		if (!parsed.success) {
			setNote({
				tone: "critical",
				text: parsed.error.issues.map((issue) => issue.message).join(" "),
			});
			return;
		}
		setBusy(true);
		setNote(null);
		try {
			const outcome = await editor.apply.apply({
				existing: editor.configuration,
				config: {
					...editor.configuration.config,
					tunnel_services: parsed.data,
				},
				how: { ...DEFAULT_APPLY, mode: safe ? "safe" : "quick" },
			});
			setNote(
				outcome.status === "done"
					? {
							tone: "good",
							text: t(
								"serviceConfig.tunnels.saved",
								"Service listeners saved.",
							),
						}
					: outcomeNote(t, time, outcome, {
							device: editor.read.deviceLabel,
							service: editor.serviceId,
						}),
			);
		} finally {
			setBusy(false);
		}
	};
	return (
		<Block
			title={t(
				"serviceConfig.tunnels.listeners",
				"Additional service listeners",
			)}
		>
			<p className="text-xs text-muted-foreground">
				{t(
					"serviceConfig.tunnels.listenersHint",
					"Name an existing listener on the device's loopback interface. People with permission to connect to this deployment can reach these services. Saving does not start the service itself. Hosting is configured above.",
				)}
			</p>
			{rows.map(({ value: row, key }, index) => (
				<fieldset
					key={key}
					disabled={pending || !!editor.editGate}
					className="grid gap-3 rounded-lg border p-3 sm:grid-cols-2"
				>
					<legend className="px-1 text-xs">
						{t("serviceConfig.tunnels.listenerNumber", "Listener {{number}}", {
							number: index + 1,
						})}
					</legend>
					<Field
						id={`${id}-${index}-id`}
						label={t("serviceConfig.tunnels.listenerId", "Service ID")}
					>
						<DvInput
							mono
							value={row.id}
							maxLength={128}
							onChange={(event) => update(index, { id: event.target.value })}
							placeholder="database"
						/>
					</Field>
					<Field
						id={`${id}-${index}-protocol`}
						label={t("serviceConfig.tunnels.protocol", "Protocol")}
					>
						<DvSelect
							value={row.protocol}
							options={[
								{ value: "tcp", label: "TCP" },
								{ value: "http", label: "HTTP" },
								{ value: "https", label: "HTTPS" },
							]}
							onValueChange={(protocol: TunnelService["protocol"]) =>
								update(index, {
									protocol,
									tls_server_name: undefined,
									tls_sha256_fingerprint: undefined,
								})
							}
						/>
					</Field>
					<Field
						id={`${id}-${index}-host`}
						label={t(
							"serviceConfig.tunnels.deviceHost",
							"Device loopback address",
						)}
					>
						<DvInput
							mono
							value={row.host}
							onChange={(event) => update(index, { host: event.target.value })}
							placeholder="127.0.0.1"
						/>
					</Field>
					<Field
						id={`${id}-${index}-port`}
						label={t("serviceConfig.tunnels.devicePort", "Device port")}
					>
						<DvInput
							type="number"
							min={1}
							max={65535}
							value={row.port || ""}
							onChange={(event) =>
								update(index, { port: Number(event.target.value) })
							}
						/>
					</Field>
					{row.protocol === "https" ? (
						<>
							<Field
								id={`${id}-${index}-tls-name`}
								label={t("serviceConfig.tunnels.tlsName", "TLS server name")}
							>
								<DvInput
									mono
									value={row.tls_server_name ?? ""}
									onChange={(event) =>
										update(index, { tls_server_name: event.target.value })
									}
									placeholder="api.example.com"
								/>
							</Field>
							<Field
								id={`${id}-${index}-tls-pin`}
								label={t(
									"serviceConfig.tunnels.tlsPin",
									"Certificate SHA-256 fingerprint",
								)}
								hint={t(
									"serviceConfig.tunnels.tlsPinHint",
									"64 hexadecimal characters from the server certificate. Update this value when that certificate changes.",
								)}
							>
								<DvInput
									mono
									value={row.tls_sha256_fingerprint ?? ""}
									maxLength={64}
									onChange={(event) =>
										update(index, {
											tls_sha256_fingerprint: event.target.value.toLowerCase(),
										})
									}
								/>
							</Field>
						</>
					) : null}
					<DvButton
						className="w-fit"
						icon={X}
						onClick={() =>
							setRows((current) => current.filter((_, at) => at !== index))
						}
					>
						{t("serviceConfig.tunnels.remove", "Remove listener")}
					</DvButton>
				</fieldset>
			))}
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					icon={Plus}
					disabled={pending || !!editor.editGate || rows.length >= 16}
					onClick={() =>
						setRows((current) => [
							...current,
							{
								key: nextKey.current++,
								value: { id: "", host: "127.0.0.1", port: 0, protocol: "tcp" },
							},
						])
					}
				>
					{t("serviceConfig.tunnels.add", "Add listener")}
				</DvButton>
				{editor.running ? (
					<label className="flex items-center gap-2 text-xs">
						<input
							type="checkbox"
							checked={safe}
							disabled={pending || (!safe && !!editor.safeUnavailable)}
							onChange={(event) => setSafe(event.target.checked)}
						/>
						{t(
							"serviceConfig.tunnels.healthChecks",
							"Apply with health checks",
						)}
					</label>
				) : null}
				<DvButton
					busy={pending}
					disabled={!changed || !!unavailable}
					onClick={() => void save()}
				>
					{editor.running
						? safe
							? t("serviceConfig.tunnels.saveChecks", "Save with health checks")
							: t(
									"serviceConfig.tunnels.saveRestart",
									"Save and restart service",
								)
						: t("serviceConfig.tunnels.save", "Save listeners")}
				</DvButton>
			</div>
			{unavailable ? (
				<p className="text-xs text-muted-foreground">{unavailable}</p>
			) : null}
			{note ? <InlineResult tone={note.tone}>{note.text}</InlineResult> : null}
		</Block>
	);
}

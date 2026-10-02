"use client";

import { useTranslation } from "@flow-like/locales";
import { RefreshCw, ShieldCheck, UserMinus } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import {
	type GrantRow,
	type RenewalOption,
	renewalOptions,
	rulesExpiryAfterSave,
} from "../../../../lib/device-management/sharing";
import type { ManagementGrant } from "../../../../lib/device-management/types";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { Field, SecretInput } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import {
	ConnectionFileSheet,
	type GrantHandlers,
	appliesSentence,
	permissionSummary,
	useScopeNames,
} from "./access-parts";
import { AccessWizard, type AccessWizardStart } from "./add-people-sheet";
import type { RequestFileRow } from "./import-file-sheet";
import { SelectControl } from "./permission-picker";
import {
	type DeviceAccess,
	type PersonNames,
	type SaveAccessOutcome,
	saveErrorText,
	useSaveAccessRules,
} from "./use-access";

/** Change permissions = steps 3–6 of Add people for one person, with the diff in the review. */
export function ChangePermissionsSheet({
	start,
	devices,
	onClose,
}: Readonly<{
	start: AccessWizardStart | null;
	devices: readonly DeviceAccess[];
	onClose(): void;
}>) {
	return <AccessWizard start={start} devices={devices} onClose={onClose} />;
}

/** The device password, asked in place when this unlock does not hold the owner key. */
function SigningPassword({
	device,
	value,
	onChange,
}: Readonly<{
	device: Pick<DeviceAccess, "deviceId" | "name">;
	value: string;
	onChange(value: string): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<Field
			id={`access-signing-password-${device.deviceId}`}
			label={t("access.signing.password", "Device password for {{device}}", {
				device: device.name,
			})}
			hint={t(
				"access.signing.passwordHint",
				"The owner key isn't held by this unlock, so it's opened with the password for this one change.",
			)}
		>
			<SecretInput
				autoComplete="current-password"
				value={value}
				onValueChange={onChange}
			/>
		</Field>
	);
}

/** A password field that lives inside the action layer's confirm sheet; the save reads it when it signs. */
function ConfirmPassword({
	device,
	target,
}: Readonly<{
	device: Pick<DeviceAccess, "deviceId" | "name">;
	target: { current: string };
}>) {
	const [value, setValue] = useState("");
	return (
		<SigningPassword
			device={device}
			value={value}
			onChange={(next) => {
				target.current = next;
				setValue(next);
			}}
		/>
	);
}

interface RenewTarget {
	row: GrantRow;
}

function renewalLabel(
	t: DevicesT,
	time: AreaTime,
	option: RenewalOption,
): string {
	const when = time.at(option.until);
	if (option.addS === undefined)
		return t(
			"devices:access.renew.untilRules",
			"Until the access rules expire · until {{when}}",
			{ when },
		);
	return t("devices:access.renew.moreDays", {
		count: Math.round(option.addS / 86_400),
		when,
		defaultValue_one: "{{count, number}} more day · until {{when}}",
		defaultValue_other: "{{count, number}} more days · until {{when}}",
	});
}

function renewalChoices(
	t: DevicesT,
	time: AreaTime,
	options: readonly RenewalOption[],
) {
	const choices: { value: string; label: string }[] = [];
	for (const option of options)
		choices.push({ value: option.id, label: renewalLabel(t, time, option) });
	return choices;
}

function RenewAccessSheet({
	device,
	target,
	names,
	onClose,
	onSaved,
}: Readonly<{
	device: DeviceAccess;
	target: RenewTarget;
	names: PersonNames;
	onClose(): void;
	onSaved(outcome: SaveAccessOutcome, grant: ManagementGrant): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const scopes = useScopeNames();
	const save = useSaveAccessRules();
	const { grant } = target.row;
	const person = names(grant.user_id);
	const now = Math.floor(time.nowS);
	// The offered ends stay put while the sheet is open; the save caps them again.
	const [openedAt] = useState(now);
	const options = useMemo(
		() => renewalOptions(grant.expires_at, openedAt),
		[grant.expires_at, openedAt],
	);
	const [choice, setChoice] = useState(options[0]?.id ?? "rules");
	const [password, setPassword] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [askPassword, setAskPassword] = useState(false);
	const selected = options.find((option) => option.id === choice) ?? options[0];
	const until = selected?.until ?? rulesExpiryAfterSave(openedAt);
	const needsPassword = askPassword || !device.keys.canSign;
	const version = (device.rules?.saved ?? 0) + 1;
	const rows: ConsequenceRows = {
		what: t(
			"access.renew.what",
			"{{name}} keeps the same {{permissions}} on {{scope}}, now until {{until}}. Nothing else changes.",
			{
				name: person.first,
				permissions: permissionSummary(t, grant.capabilities),
				scope: scopes.phrase(grant.scope),
				until: time.at(until),
			},
		),
		who: t("access.renew.who", "{{name}} only. Nobody else's access changes.", {
			name: person.name,
		}),
		when: t(
			"access.renew.when",
			"{{applies}} Until then it still ends at {{end}}.",
			{
				applies: appliesSentence(
					t,
					device.name,
					version,
					device.presence,
					time,
				),
				end: time.at(grant.expires_at),
			},
		),
		undo: {
			reversible: true,
			text: t(
				"access.renew.undo",
				"Change the end date again, or remove access.",
			),
		},
	};
	const submit = async () => {
		if (busy) return;
		setBusy(true);
		setError(undefined);
		const next = { ...grant, expires_at: until };
		const outcome = await save({
			device,
			label: t("access.renew.label", "Renew {{name}}'s access", {
				name: person.first,
			}),
			upserts: [next],
			...(needsPassword ? { password } : {}),
		});
		setBusy(false);
		setPassword("");
		if (outcome.status === "done") {
			onSaved(outcome, next);
			return;
		}
		if (outcome.status === "password_required") setAskPassword(true);
		setError(saveErrorText(t, outcome, device.name));
	};
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			role="alertdialog"
			closeOnOutside={false}
			icon={RefreshCw}
			eyebrow={t("access.renew.eyebrow", "Before this runs")}
			title={t("access.renew.title", "Renew {{name}}'s access to {{device}}?", {
				name: person.name,
				device: device.name,
			})}
			sub={t(
				"access.renew.subtitle",
				"{{scope}} · {{permissions}} · ends {{when}}",
				{
					scope: scopes.label(grant.scope),
					permissions: permissionSummary(t, grant.capabilities),
					when: time.at(grant.expires_at),
				},
			)}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("access.renew.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={busy}
						aria-disabled={needsPassword && !password ? true : undefined}
						onClick={() => void submit()}
					>
						{t("access.renew.label", "Renew {{name}}'s access", {
							name: person.first,
						})}
					</DvButton>
				</>
			}
		>
			<Field
				id="access-renew-extend"
				label={t("access.renew.extendBy", "Extend by")}
				hint={t(
					"access.renew.extendHint",
					"Access can't outlast the access rules. Saving re-signs them until {{date}}.",
					{ date: time.at(rulesExpiryAfterSave(openedAt)) },
				)}
			>
				<SelectControl
					value={choice}
					onValueChange={setChoice}
					options={renewalChoices(t, time, options)}
				/>
			</Field>
			<ConsequencePreview rows={rows} />
			{needsPassword ? (
				<SigningPassword
					device={device}
					value={password}
					onChange={setPassword}
				/>
			) : null}
			{error ? <InlineResult tone="critical">{error}</InlineResult> : null}
		</DvSheet>
	);
}

interface RemoveFacts {
	name: string;
	first: string;
	permissions: string;
	device: string;
	scope: string;
	/** When the device picks the change up. */
	applies: string;
}

/** SPEC §6.5 Remove access. */
function removeRows(t: DevicesT, facts: RemoveFacts): ConsequenceRows {
	const { name, first, permissions, device, scope, applies } = facts;
	return {
		what: t(
			"devices:access.remove.what",
			"{{name}} loses {{permissions}} on {{device}}, {{scope}}.",
			{ name, permissions, device, scope },
		),
		who: t(
			"devices:access.remove.who",
			"{{name}} stops being a history reader for {{scope}}. Their live connections to {{device}} close once the device applies the change.",
			{ name: first, scope, device },
		),
		stays: t(
			"devices:access.remove.stays",
			"Data they already downloaded stays readable to them. Their services keep running; cloud approvals they created stay.",
		),
		when: t(
			"devices:access.remove.when",
			"{{applies}} Until then {{name}} can still connect.",
			{ applies, name: first },
		),
		undo: {
			reversible: true,
			text: t(
				"devices:access.remove.undo",
				"Add {{name}} again from a new access request.",
				{ name: first },
			),
		},
	};
}

interface FlowResult {
	tone: "good" | "critical";
	text: string;
	/** A saved version the device still has to apply. */
	version?: number;
}

export interface GrantFlows {
	handlers: GrantHandlers;
	renewRules(): void;
	downloadConnection(): void;
	/** Opens Add people with this device preselected. */
	addPeople(files?: readonly RequestFileRow[]): void;
	/** The sheets these actions open; render once. */
	sheets: ReactNode;
	/** The outcome next to the controls (R9); `null` when there is none. */
	result: ReactNode;
}

let wizardRuns = 0;
export const nextWizardId = () => `access-wizard-${++wizardRuns}`;

/**
 * Every action on one device's access: renew, change permissions, remove
 * access, renew the rules, download the connection file, add people. All
 * saves go through the action layer; the outcome stays next to the controls.
 */
export function useGrantFlows(
	device: DeviceAccess | undefined,
	devices: readonly DeviceAccess[],
	names: PersonNames,
): GrantFlows {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const scopes = useScopeNames();
	const save = useSaveAccessRules();
	const [wizard, setWizard] = useState<AccessWizardStart | null>(null);
	const [renew, setRenew] = useState<RenewTarget | null>(null);
	const [connection, setConnection] = useState(false);
	const [result, setResult] = useState<FlowResult | null>(null);
	const password = useRef("");
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const latest = useRef(device);
	latest.current = device;

	const report = useCallback(
		(
			outcome: SaveAccessOutcome,
			done: (version: number, at: string) => string,
		) => {
			// A password typed into the confirm sheet is not kept past the save.
			password.current = "";
			const current = latest.current;
			if (!mounted.current || !current) return;
			if (outcome.status === "cancelled") return;
			if (outcome.status === "done") {
				setResult({
					tone: "good",
					text: done(
						outcome.result.version,
						time.clock(outcome.result.savedAt),
					),
					version: outcome.result.version,
				});
				return;
			}
			const text = saveErrorText(t, outcome, current.name);
			if (text) setResult({ tone: "critical", text });
		},
		[t, time],
	);

	const signingExtra = useCallback((target: DeviceAccess) => {
		password.current = "";
		return target.keys.canSign ? undefined : (
			<ConfirmPassword device={target} target={password} />
		);
	}, []);

	const handlers = useMemo<GrantHandlers>(
		() => ({
			onRenew(row) {
				const current = latest.current;
				if (!current) return;
				if (row.status === "expired")
					setWizard({
						id: nextWizardId(),
						mode: "change",
						change: { deviceId: current.deviceId, grant: row.grant },
					});
				else setRenew({ row });
			},
			onChange(row) {
				const current = latest.current;
				if (!current) return;
				setWizard({
					id: nextWizardId(),
					mode: "change",
					change: { deviceId: current.deviceId, grant: row.grant },
				});
			},
			onRemove(row) {
				const current = latest.current;
				if (!current) return;
				const person = names(row.grant.user_id);
				const version = (current.rules?.saved ?? 0) + 1;
				const permissions = permissionSummary(t, row.grant.capabilities);
				const extra = signingExtra(current);
				void save({
					device: current,
					label: t("access.remove.label", "Remove {{name}}", {
						name: person.name,
					}),
					removeIds: [row.grant.grant_id],
					password: () => password.current,
					consequence: removeRows(t, {
						name: person.name,
						first: person.first,
						permissions,
						device: current.name,
						scope: scopes.phrase(row.grant.scope),
						applies: appliesSentence(
							t,
							current.name,
							version,
							current.presence,
							time,
						),
					}),
					confirm: {
						icon: UserMinus,
						title: t("access.remove.title", "Remove {{name}}'s access?", {
							name: person.name,
						}),
						sub: t(
							"access.remove.subtitle",
							"{{device}} · {{scope}} · {{permissions}}",
							{
								device: current.name,
								scope: scopes.label(row.grant.scope),
								permissions,
							},
						),
						tone: "danger",
						whoLabel: "loses",
						...(extra ? { extra } : {}),
					},
				}).then((outcome) =>
					report(outcome, (saved, at) =>
						t(
							"access.remove.done",
							"Saved access rules v{{n}} without {{name}} at {{time}}. Until {{device}} applies it, {{first}} can still connect.",
							{
								n: saved,
								name: person.name,
								time: at,
								device: current.name,
								first: person.first,
							},
						),
					),
				);
			},
		}),
		[names, save, scopes, signingExtra, report, t, time],
	);

	const renewRules = useCallback(() => {
		const current = latest.current;
		if (!current?.rules) return;
		const { rules } = current;
		const until = rulesExpiryAfterSave(Math.floor(time.nowS));
		const extra = signingExtra(current);
		void save({
			device: current,
			label: t("access.renewRules.label", "Renew access rules"),
			password: () => password.current,
			consequence: {
				what: t(
					"access.renewRules.what",
					"The same people keep the same permissions and end dates. Only the rules' own expiry moves to {{until}}.",
					{ until: time.at(until) },
				),
				who: t(
					"access.renewRules.who",
					"Nobody loses access. Retained history keeps recording.",
				),
				when: appliesSentence(
					t,
					current.name,
					rules.saved + 1,
					current.presence,
					time,
				),
				undo: {
					reversible: null,
					text: t(
						"access.renewRules.undo",
						"Not needed: it changes nothing but the date.",
					),
				},
			},
			confirm: {
				icon: ShieldCheck,
				title: t(
					"access.renewRules.title",
					"Renew the access rules for {{device}}?",
					{ device: current.name },
				),
				sub:
					rules.expiresAt === undefined
						? t("access.renewRules.subVersion", "Access rules v{{n}}", {
								n: rules.saved,
							})
						: t(
								"access.renewRules.subtitle",
								"Access rules v{{n}} · expire {{when}}",
								{ n: rules.saved, when: time.at(rules.expiresAt) },
							),
				...(extra ? { extra } : {}),
			},
		}).then((outcome) =>
			report(outcome, (saved, at) =>
				t(
					"access.renewRules.done",
					"Access rules renewed as v{{n}} at {{time}}. They now expire {{until}}.",
					{ n: saved, time: at, until: time.at(until) },
				),
			),
		);
	}, [save, signingExtra, report, t, time]);

	const addPeople = useCallback((files?: readonly RequestFileRow[]) => {
		const current = latest.current;
		if (!current) return;
		const unlocked = current.keys.state === "unlocked";
		setWizard({
			id: nextWizardId(),
			mode: "add",
			deviceIds: unlocked ? [current.deviceId] : [],
			...(files?.length ? { files, step: "files" as const } : {}),
			...(unlocked ? {} : { noteDeviceId: current.deviceId }),
		});
	}, []);

	const applied = device?.rules?.applied ?? 0;
	const waiting = result?.version !== undefined && applied < result.version;
	const resultNode = result ? (
		<InlineResult
			tone={result.tone === "critical" ? "critical" : waiting ? "info" : "good"}
			onDismiss={() => setResult(null)}
		>
			{result.text}{" "}
			{result.version === undefined || !device
				? null
				: waiting
					? t("access.result.waiting", "Waiting for {{device}} to apply it.", {
							device: device.name,
						})
					: t("access.result.applied", "{{device}} applied it.", {
							device: device.name,
						})}
		</InlineResult>
	) : null;

	const sheets = device ? (
		<>
			<AccessWizard
				start={wizard}
				devices={devices}
				onClose={() => setWizard(null)}
			/>
			{renew ? (
				<RenewAccessSheet
					device={device}
					target={renew}
					names={names}
					onClose={() => setRenew(null)}
					onSaved={(outcome, grant) => {
						setRenew(null);
						const person = names(grant.user_id);
						report(outcome, (saved, at) =>
							t(
								"access.renew.done",
								"{{name}}'s access renewed until {{until}}. Saved as access rules v{{n}} at {{time}}.",
								{
									name: person.name,
									until: time.at(grant.expires_at),
									n: saved,
									time: at,
								},
							),
						);
					}}
				/>
			) : null}
			<ConnectionFileSheet
				device={device}
				open={connection}
				onOpenChange={setConnection}
			/>
		</>
	) : null;

	return {
		handlers,
		renewRules,
		downloadConnection: useCallback(() => setConnection(true), []),
		addPeople,
		sheets,
		result: resultNode,
	};
}

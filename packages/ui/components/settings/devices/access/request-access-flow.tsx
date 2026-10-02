"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ChevronLeft,
	Download,
	FileText,
	Laptop,
	Share2,
	ShieldAlert,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { withPassword } from "../../../../lib/device-management/crypto";
import {
	type ConnectionFile,
	accessRequestFileName,
	accessRequestFileText,
	parseConnectionFile,
	requestKeysState,
} from "../../../../lib/device-management/sharing";
import {
	addDeviceVault,
	deviceApiBase,
	pinDeviceIdentity,
	readDeviceVault,
	requestPersistentDeviceStorage,
} from "../../../../lib/device-management/storage";
import type {
	Ed25519PublicKey,
	OnboardingManifest,
} from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { humanFileSize } from "../../../../lib/utils";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Checklist } from "../primitives/checklist";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	CheckField,
	DropZone,
	Field,
	SecretInput,
	utf8Bytes,
} from "../primitives/form-fields";
import { IdRef } from "../primitives/id-ref";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { StatusChip } from "../primitives/status-chip";
import { WizardStepper } from "../primitives/wizard";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useAttentionState } from "../workspace/use-attention";
import { useDeviceAction } from "../workspace/use-device-action";
import { useLocalSummary } from "../workspace/use-keys";
import {
	AccessPerson,
	FingerprintReadout,
	KeyFingerprint,
	WizardPosition,
	hostOf,
	saveTextFile,
} from "./access-parts";
import { type PersonNames, useGuardedRead, usePersonNames } from "./use-access";

const PASSWORD_MIN_BYTES = 12;
const PASSWORD_MAX_BYTES = 4096;

type FileProblem =
	| "too_large"
	| "not_connection_file"
	| "bad_signature"
	| "other_hub"
	| "already_requested"
	| "other_keys"
	| "already_have";

interface FileFacts {
	fileName: string;
	bytes: number;
}

type CheckedFile =
	| (FileFacts & {
			ok: true;
			file: ConnectionFile;
			manifest: OnboardingManifest;
	  })
	| (FileFacts & {
			ok: false;
			problem: FileProblem;
			manifest?: OnboardingManifest;
			hub?: string;
			/** Keys made earlier for the same file: their request can be downloaded again. */
			reusable?: { controllerKey: Ed25519PublicKey; grantId: string };
	  });

/** Reads and checks a connection file on this computer: shape, the device's signature, the hub and keys already here. */
async function checkConnectionFile(
	file: File,
	workspace: DeviceWorkspace,
	listed: (deviceId: string) => boolean,
): Promise<CheckedFile> {
	const facts = { fileName: file.name, bytes: file.size };
	const text = await file.text().catch(() => "");
	const parsed = parseConnectionFile(text, file.size);
	if (!parsed.ok)
		return {
			...facts,
			ok: false,
			problem:
				parsed.error === "too_large" ? "too_large" : "not_connection_file",
		};
	const { receipt, owner_controller_key } = parsed.file;
	let manifest: OnboardingManifest;
	try {
		const crypto = await workspace.deps.crypto();
		manifest = crypto.verifyDeviceReceipt(
			receipt,
			receipt.manifest_jws,
			owner_controller_key,
		);
	} catch {
		return { ...facts, ok: false, problem: "bad_signature" };
	}
	const { scope } = workspace.deps;
	if (manifest.api_base_url !== deviceApiBase(scope))
		return {
			...facts,
			ok: false,
			problem: "other_hub",
			manifest,
			hub: hostOf(manifest.api_base_url),
		};
	const existing = await readDeviceVault(scope, manifest.device_id);
	const keys = requestKeysState(existing, parsed.file);
	if (keys === "conflict")
		return { ...facts, ok: false, problem: "other_keys", manifest };
	if (keys === "reusable" && existing)
		return {
			...facts,
			ok: false,
			problem: listed(manifest.device_id)
				? "already_have"
				: "already_requested",
			manifest,
			reusable: {
				controllerKey: existing.controllerPublic.controller_key,
				grantId: existing.grantId,
			},
		};
	if (listed(manifest.device_id))
		return { ...facts, ok: false, problem: "already_have", manifest };
	return { ...facts, ok: true, file: parsed.file, manifest };
}

interface ProblemFacts {
	file: string;
	size: string;
	device: string;
	hub: string;
	mine: string;
	owner: string;
}

const PROBLEM_COPY: Record<
	FileProblem,
	(t: DevicesT, facts: ProblemFacts) => string
> = {
	too_large: (t, { file, size }) =>
		t(
			"devices:access.request.problem.tooLarge",
			"{{file}} is {{size}}. Connection files are at most 128 KiB. Ask the owner for the file their Access page downloads.",
			{ file, size },
		),
	not_connection_file: (t) =>
		t(
			"devices:access.request.problem.notConnection",
			"This isn't a connection file. Ask the owner for the file their Access page downloads, named flow-like-connection-….json.",
		),
	bad_signature: (t) =>
		t(
			"devices:access.request.problem.badSignature",
			"The device's registration in this file doesn't check out with the owner key in it. The file was changed or is damaged; ask the owner for a new one.",
		),
	other_hub: (t, { hub, mine }) =>
		t(
			"devices:access.request.problem.otherHub",
			"This device belongs to another hub ({{hub}}). You're signed in to {{mine}}. Sign in to that hub, or ask for a device on this one.",
			{ hub, mine },
		),
	already_requested: (t, { device, owner }) =>
		t(
			"devices:access.request.problem.alreadyRequested",
			"This computer already has a request for {{device}}. Download it again instead of making a new one; new keys would replace the ones {{owner}} may be about to approve.",
			{ device, owner },
		),
	other_keys: (t, { device }) =>
		t(
			"devices:access.request.problem.otherKeys",
			"This computer already holds other keys for {{device}}. Use the device with them instead of requesting new access.",
			{ device },
		),
	already_have: (t, { device }) =>
		t(
			"devices:access.request.problem.alreadyHave",
			"You already have access to {{device}}. Ask the owner to renew it instead.",
			{ device },
		),
};

function problemText(
	t: DevicesT,
	checked: Extract<CheckedFile, { ok: false }>,
	myHub: string,
	owner: string,
): string {
	return PROBLEM_COPY[checked.problem](t, {
		file: checked.fileName,
		size: humanFileSize(checked.bytes),
		device: checked.manifest?.name ?? "",
		hub: checked.hub ?? "",
		mine: myHub,
		owner,
	});
}

interface Created {
	grantId: string;
	controllerKey: Ed25519PublicKey;
	/** Unix seconds. */
	at: number;
}

function FileStep({
	checked,
	reading,
	names,
	myHub,
	onFile,
	onDownloadAgain,
	downloadedAgain,
}: Readonly<{
	checked: CheckedFile | null;
	reading: boolean;
	names: PersonNames;
	myHub: string;
	onFile(file: File): void;
	onDownloadAgain(): void;
	downloadedAgain: string | null;
}>) {
	const { t } = useTranslation("devices");
	return (
		<>
			<p className="text-sm/5">
				{t(
					"access.request.file.intro",
					"Ask the owner for their device's connection file. They download it from their Access page. It names the device and the owner's key, and holds no secrets.",
				)}
			</p>
			<DropZone
				id="access-request-file"
				accept=".json,application/json"
				title={
					<>
						<b className="font-semibold">
							{t("access.request.file.drop", "Drop the connection file")}
						</b>{" "}
						{t("access.request.file.orChoose", "or choose it")}
					</>
				}
				hint={t("access.request.file.hint", ".json · up to 128 KiB")}
				onFiles={(files) => {
					const [file] = files;
					if (file) onFile(file);
				}}
				files={
					checked?.ok ? (
						<StatusChip tone="outline" icon={FileText}>
							{t("access.request.file.chip", "{{file}} · {{size}}", {
								file: checked.fileName,
								size: humanFileSize(checked.bytes),
							})}
						</StatusChip>
					) : undefined
				}
			/>
			{reading ? (
				<InlineResult tone="info">
					{t("access.request.file.reading", "Checking the file…")}
				</InlineResult>
			) : null}
			{checked?.ok ? (
				<p className="flex items-center gap-1.5 text-xs text-muted-foreground">
					<Laptop aria-hidden className="size-3.25" />
					{t(
						"access.request.file.local",
						"Read on this computer. Nothing was uploaded.",
					)}
				</p>
			) : null}
			{checked && !checked.ok ? (
				<div
					data-file-problem={checked.problem}
					className="flex flex-col gap-2"
				>
					<InlineResult tone="critical">
						{problemText(
							t,
							checked,
							myHub,
							checked.manifest ? names(checked.manifest.owner_id).first : "",
						)}
					</InlineResult>
					{checked.reusable ? (
						<DvButton
							size="sm"
							icon={Download}
							className="self-start"
							onClick={onDownloadAgain}
						>
							{t("access.request.again", "Download request again")}
						</DvButton>
					) : null}
					{downloadedAgain ? (
						<InlineResult tone="good">{downloadedAgain}</InlineResult>
					) : null}
					<p className="text-xs text-muted-foreground">
						{t(
							"access.request.file.nothingSaved",
							"Nothing was saved. Choose another file.",
						)}
					</p>
				</div>
			) : null}
		</>
	);
}

function CheckStep({
	checked,
	names,
	myHub,
	matches,
	onMatches,
}: Readonly<{
	checked: Extract<CheckedFile, { ok: true }>;
	names: PersonNames;
	myHub: string;
	matches: boolean;
	onMatches(matches: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { manifest, file } = checked;
	const owner = names(manifest.owner_id);
	const lead = { 1: <b className="font-semibold" /> };
	return (
		<>
			<Checklist
				label={t("access.request.check.label", "Checks on the connection file")}
				items={[
					{
						id: "size",
						state: "pass",
						source: "local",
						label: (
							<Trans
								t={t}
								i18nKey="access.request.check.size"
								defaults="<1>Connection file</1> · {{size}}, under the 128 KiB limit"
								values={{ size: humanFileSize(checked.bytes) }}
								components={lead}
							/>
						),
					},
					{
						id: "signature",
						state: "pass",
						source: "local",
						label: (
							<Trans
								t={t}
								i18nKey="access.request.check.signature"
								defaults="<1>Signed by {{device}}</1> · its registration signature is valid"
								values={{ device: manifest.name }}
								components={lead}
							/>
						),
					},
					{
						id: "hub",
						state: "pass",
						source: "local",
						label: (
							<Trans
								t={t}
								i18nKey="access.request.check.hub"
								defaults="<1>Same hub</1> · {{hub}}"
								values={{ hub: myHub }}
								components={lead}
							/>
						),
					},
					{
						id: "keys",
						state: "pass",
						source: "local",
						label: (
							<Trans
								t={t}
								i18nKey="access.request.check.keys"
								defaults="<1>No keys for {{device}} on this computer yet</1>"
								values={{ device: manifest.name }}
								components={lead}
							/>
						),
					},
				]}
			/>
			<KeyValueList>
				<KvRow label={t("access.request.check.device", "Device")}>
					<span className="font-mono">{manifest.name}</span>{" "}
					<IdRef
						id={manifest.device_id}
						copyLabel={t("access.request.check.copyDevice", "Copy device ID")}
					/>
				</KvRow>
				<KvRow label={t("access.request.check.owner", "Owner")}>
					<AccessPerson userId={manifest.owner_id} />
				</KvRow>
				<KvRow label={t("access.request.check.registered", "Registered")}>
					<span className="tabular-nums">
						{time.at(file.receipt.registered_at)}
					</span>
				</KvRow>
			</KeyValueList>
			<FingerprintReadout
				label={t(
					"access.request.check.fingerprint",
					"{{name}}'s owner key fingerprint",
					{ name: owner.name },
				)}
				value={file.owner_controller_key.x}
				hint={t(
					"access.request.check.fingerprintHint",
					"Ask {{name}} to read out the fingerprint on their Access page and compare every block. A mismatch means the file was changed on the way.",
					{ name: owner.first },
				)}
			>
				<CheckField
					id="access-request-fingerprint"
					checked={matches}
					onCheckedChange={onMatches}
				>
					{t(
						"access.request.check.matches",
						"{{name}} read out the same fingerprint.",
						{ name: owner.first },
					)}
				</CheckField>
			</FingerprintReadout>
			<Banner
				tone="warning"
				icon={ShieldAlert}
				title={t(
					"access.request.check.warnTitle",
					"Know what {{name}} can give you",
					{ name: owner.first },
				)}
			>
				{t(
					"access.request.check.warnText",
					"{{name}} chooses your permissions when approving. If they include Deploy & configure, Start, Restart or Change instance count on a device without a required sandbox, your services run with the agent's full access to {{device}}. You'll see your permissions here once they approve.",
					{ name: owner.first, device: manifest.name },
				)}
			</Banner>
		</>
	);
}

function PasswordStep({
	device,
	password,
	repeat,
	onPassword,
	onRepeat,
	error,
}: Readonly<{
	device: string;
	password: string;
	repeat: string;
	onPassword(value: string): void;
	onRepeat(value: string): void;
	error?: string;
}>) {
	const { t } = useTranslation("devices");
	const local = useLocalSummary();
	const { input } = useAttentionState();
	const matches = repeat.length > 0 && repeat === password;
	return (
		<>
			<p className="text-sm/5">
				{t(
					"access.request.password.intro",
					"Set a device password for {{device}}. It protects the new keys the app makes for you on this computer.",
					{ device },
				)}
			</p>
			<div className="grid gap-4 min-[720px]:grid-cols-2">
				<Field
					id="access-request-password"
					label={t(
						"access.request.password.label",
						"Device password for {{device}}",
						{ device },
					)}
				>
					<SecretInput
						autoComplete="new-password"
						value={password}
						onValueChange={onPassword}
						minBytes={PASSWORD_MIN_BYTES}
						maxBytes={PASSWORD_MAX_BYTES}
					/>
				</Field>
				<Field
					id="access-request-repeat"
					label={t(
						"access.request.password.repeatLabel",
						"Repeat the password",
					)}
					hint={
						!repeat
							? t(
									"access.request.password.repeatHint",
									"Type it again to check it.",
								)
							: matches
								? t("access.request.password.bothMatch", "Both match.")
								: t(
										"access.request.password.mismatch",
										"The two passwords don't match yet.",
									)
					}
				>
					<SecretInput
						autoComplete="new-password"
						value={repeat}
						onValueChange={onRepeat}
					/>
				</Field>
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"access.request.password.hint",
					"One password per device on this computer. It isn't your account password. If you forget it, nobody can recover it, and you request access again. The password never leaves this computer and the hub never sees it.",
				)}
			</p>
			{local.persistence !== "persisted" && local.platform === "web" ? (
				<InlineResult tone="warning">
					{t(
						"access.request.password.storage",
						"This browser may delete the new keys when space runs low. Keep keys safely under Keys & recovery after creating the request.",
					)}
				</InlineResult>
			) : null}
			<KeyValueList>
				<KvRow label={t("access.request.password.keptIn", "Keys are kept in")}>
					{local.platform === "desktop"
						? t(
								"access.request.password.desktop",
								"Desktop app · this computer",
							)
						: t(
								"access.request.password.browser",
								"This browser · this computer",
							)}
				</KvRow>
				<KvRow label={t("access.request.password.for", "For")}>
					<AccessPerson userId={input.me} showId={false} />
				</KvRow>
			</KeyValueList>
			{error ? <InlineResult tone="critical">{error}</InlineResult> : null}
		</>
	);
}

function DoneStep({
	checked,
	created,
	names,
	downloadedAt,
}: Readonly<{
	checked: Extract<CheckedFile, { ok: true }>;
	created: Created;
	names: PersonNames;
	downloadedAt: number | null;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { manifest } = checked;
	const owner = names(manifest.owner_id);
	return (
		<>
			<InlineResult tone="good">
				{t(
					"access.request.done.created",
					"Keys created at {{time}} and stored on this computer.",
					{ time: time.clock(created.at) },
				)}
			</InlineResult>
			<KeyValueList>
				<KvRow label={t("access.request.done.file", "Request file")}>
					<span className="font-mono wrap-anywhere">
						{accessRequestFileName(manifest.device_id)}
					</span>
				</KvRow>
				<KvRow label={t("access.request.done.contains", "Contains")}>
					{t(
						"access.request.done.containsText",
						"Your account ID, your new public key and a request ID. No secrets.",
					)}
				</KvRow>
				<KvRow label={t("access.request.done.requestId", "Request ID")}>
					<IdRef
						id={created.grantId}
						copyLabel={t("access.request.done.copyRequest", "Copy request ID")}
					/>
				</KvRow>
				<KvRow
					label={t("access.request.done.fingerprint", "Your key fingerprint")}
				>
					<KeyFingerprint
						value={created.controllerKey.x}
						copyLabel={t(
							"access.request.done.copyFingerprint",
							"Copy key fingerprint",
						)}
					/>
					<span className="mt-0.5 block text-xs text-muted-foreground">
						{t(
							"access.request.done.fingerprintHint",
							"{{name}} compares this with what you read out.",
							{ name: owner.first },
						)}
					</span>
				</KvRow>
			</KeyValueList>
			{downloadedAt === null ? null : (
				<InlineResult tone="good">
					{t(
						"access.request.done.downloaded",
						"Downloaded at {{time}}. Send the file to {{name}}.",
						{ time: time.clock(downloadedAt), name: owner.name },
					)}
				</InlineResult>
			)}
			<ol className="flex flex-col gap-2.5 text-ui">
				{[
					{
						id: "import",
						title: t("access.request.done.step1", "{{name}} imports it", {
							name: owner.first,
						}),
						hint: t(
							"access.request.done.step1Hint",
							"Under Access › People, and chooses your permissions.",
						),
					},
					{
						id: "apply",
						title: t(
							"access.request.done.step2",
							"{{device}} applies the new access rules",
							{ device: manifest.name },
						),
						hint: t(
							"access.request.done.step2Hint",
							"Usually within 5 minutes while it's online.",
						),
					},
					{
						id: "list",
						title: t(
							"access.request.done.step3",
							"It appears in your device list",
						),
						hint: t(
							"access.request.done.step3Hint",
							"Until then it's listed under Shared with me as Waiting for approval, on this computer only.",
						),
					},
				].map((step, index) => (
					<li key={step.id} className="flex items-start gap-3">
						<span className="inline-flex size-6 shrink-0 items-center justify-center rounded-full border border-border text-xs font-medium tabular-nums">
							{index + 1}
						</span>
						<span className="min-w-0">
							<b className="font-semibold">{step.title}</b>
							<span className="block text-xs text-muted-foreground">
								{step.hint}
							</span>
						</span>
					</li>
				))}
			</ol>
		</>
	);
}

function FlowBody({ onClose }: Readonly<{ onClose(): void }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const { input } = useAttentionState();
	const actions = useDeviceAction();
	const guarded = useGuardedRead();
	const [step, setStep] = useState(0);
	const [checked, setChecked] = useState<CheckedFile | null>(null);
	const [reading, setReading] = useState(false);
	const [matches, setMatches] = useState(false);
	const [password, setPassword] = useState("");
	const [repeat, setRepeat] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [created, setCreated] = useState<Created | null>(null);
	const [downloadedAt, setDownloadedAt] = useState<number | null>(null);
	const [downloadedAgain, setDownloadedAgain] = useState<string | null>(null);
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	const listedIds = useRef(new Set<string>());
	listedIds.current = new Set(input.devices.map((row) => row.device_id));
	const ownerIds = checked?.manifest ? [checked.manifest.owner_id] : [];
	const names = usePersonNames(t, ownerIds);
	const myHub = hostOf(deviceApiBase(workspace.deps.scope));
	const nowS = () => Math.floor(workspace.clock.now() / 1000);

	const onFile = useCallback(
		(file: File) => {
			setReading(true);
			setChecked(null);
			setMatches(false);
			setDownloadedAgain(null);
			void guarded(
				() =>
					checkConnectionFile(file, workspace, (id) =>
						listedIds.current.has(id),
					),
				(result) => {
					setChecked(result);
					setReading(false);
				},
			);
		},
		[guarded, workspace],
	);

	const account = workspace.deps.scope.account;
	const download = (deviceId: string, key: Ed25519PublicKey, grantId: string) =>
		saveTextFile(
			accessRequestFileName(deviceId),
			accessRequestFileText(account, key, grantId),
		);

	const create = async () => {
		if (!checked?.ok || busy) return;
		const { file, manifest } = checked;
		setBusy(true);
		setError(undefined);
		const secret = password;
		setPassword("");
		setRepeat("");
		const outcome = await actions.run<Created>({
			action: "request_access",
			label: t("access.request.createLabel", "Create keys and request"),
			resultKey: `access-request:${manifest.device_id}`,
			call: async ({ workspace: current }) => {
				const { scope } = current.deps;
				const crypto = await current.deps.crypto();
				const made = await withPassword(secret, (bytes) =>
					crypto.createControllerVault(manifest.device_id, bytes),
				);
				const grantId = globalThis.crypto.randomUUID();
				await requestPersistentDeviceStorage();
				await addDeviceVault(scope, {
					deviceId: manifest.device_id,
					controllerPublic: made.public_bundle,
					controllerVault: Uint8Array.from(made.vault),
					manifestJws: file.receipt.manifest_jws,
					ownerControllerKey: file.owner_controller_key,
					grantId,
				});
				await pinDeviceIdentity(scope, manifest.device_id, file.receipt);
				await current.local.reload();
				current.facts.recordAccessRequest({
					deviceId: manifest.device_id,
					deviceName: manifest.name,
					ownerId: manifest.owner_id,
				});
				return {
					grantId,
					controllerKey: made.public_bundle.controller_key,
					at: Math.floor(current.clock.now() / 1000),
				};
			},
		});
		if (!mounted.current) return;
		setBusy(false);
		if (outcome.status === "done") {
			setCreated(outcome.result);
			setStep(3);
			return;
		}
		if (outcome.status !== "cancelled")
			setError(
				t(
					"access.request.createFailed",
					"The keys couldn't be created on this computer, so no request was made. Try again.",
				),
			);
	};

	const labels = [
		t("access.request.step.file", "Connection file"),
		t("access.request.step.check", "Check"),
		t("access.request.step.password", "Device password"),
		t("access.request.step.request", "Access request"),
	];
	const passwordBytes = utf8Bytes(password);
	const passwordOk =
		passwordBytes >= PASSWORD_MIN_BYTES &&
		passwordBytes <= PASSWORD_MAX_BYTES &&
		password === repeat;
	const blocker =
		step === 0
			? !checked
				? t("access.request.block.file", "Import the connection file.")
				: checked.ok
					? null
					: t("access.request.block.unusable", "This file can't be used.")
			: step === 1 && !matches
				? t(
						"access.request.block.fingerprint",
						"Confirm the fingerprint matches.",
					)
				: step === 2 && !passwordOk
					? t("access.request.block.password", "Set the device password twice.")
					: null;

	const next = () => {
		if (blocker || busy) return;
		if (step === 2) {
			void create();
			return;
		}
		if (step === 3) {
			if (checked?.ok && created) {
				download(
					checked.manifest.device_id,
					created.controllerKey,
					created.grantId,
				);
				setDownloadedAt(nowS());
			}
			return;
		}
		setStep(step + 1);
	};

	const ok = checked?.ok ? checked : null;
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			wide
			closeOnOutside={false}
			icon={Share2}
			title={t("access.request.title", "Request access to someone's device")}
			sub={t(
				"access.request.subtitle",
				"Your keys stay on this computer. The owner decides what you can do.",
			)}
			footNote={
				step === 3 ? (
					t(
						"access.request.footDone",
						"Keys created. The request is ready to send.",
					)
				) : (
					<WizardPosition
						position={t(
							"access.request.position",
							"Step {{n, number}} of {{count, number}} · {{step}}",
							{ n: step + 1, count: labels.length, step: labels[step] ?? "" },
						)}
						blocker={blocker}
					/>
				)
			}
			foot={
				<>
					{step > 0 && step < 3 ? (
						<DvButton
							icon={ChevronLeft}
							aria-disabled={busy || undefined}
							onClick={() => setStep(step - 1)}
						>
							{t("access.request.back", "Back")}
						</DvButton>
					) : null}
					<DvButton onClick={onClose}>
						{step === 3
							? t("access.request.finish", "Done")
							: t("access.request.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						icon={step === 3 ? Download : undefined}
						busy={busy}
						aria-disabled={blocker ? true : undefined}
						onClick={next}
					>
						{step === 2
							? t("access.request.create", "Create keys and request")
							: step === 3
								? t("access.request.download", "Download access request")
								: t("access.request.continue", "Continue")}
					</DvButton>
				</>
			}
		>
			<WizardStepper
				steps={labels}
				current={step}
				label={t("access.request.steps", "Steps")}
			/>
			{step === 0 ? (
				<FileStep
					checked={checked}
					reading={reading}
					names={names}
					myHub={myHub}
					onFile={onFile}
					downloadedAgain={downloadedAgain}
					onDownloadAgain={() => {
						if (
							!checked ||
							checked.ok ||
							!checked.reusable ||
							!checked.manifest
						)
							return;
						download(
							checked.manifest.device_id,
							checked.reusable.controllerKey,
							checked.reusable.grantId,
						);
						setDownloadedAgain(
							t(
								"access.request.againDone",
								"Downloaded again at {{time}}. It's the same request; the owner can import it any time.",
								{ time: time.clock(nowS()) },
							),
						);
					}}
				/>
			) : null}
			{step === 1 && ok ? (
				<CheckStep
					checked={ok}
					names={names}
					myHub={myHub}
					matches={matches}
					onMatches={setMatches}
				/>
			) : null}
			{step === 2 && ok ? (
				<PasswordStep
					device={ok.manifest.name}
					password={password}
					repeat={repeat}
					onPassword={setPassword}
					onRepeat={setRepeat}
					error={error}
				/>
			) : null}
			{step === 3 && ok && created ? (
				<DoneStep
					checked={ok}
					created={created}
					names={names}
					downloadedAt={downloadedAt}
				/>
			) : null}
		</DvSheet>
	);
}

/**
 * The recipient's side (SPEC §5.7 `#request`): import the owner's connection
 * file, compare the owner key fingerprint, set a device password, download
 * the access request. A pending row then shows under Shared with me.
 */
export function RequestAccessFlow({
	open,
	onClose,
}: Readonly<{ open: boolean; onClose(): void }>) {
	return open ? <FlowBody onClose={onClose} /> : null;
}

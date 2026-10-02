"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Download, ShieldCheck } from "lucide-react";
import {
	type MutableRefObject,
	type ReactNode,
	useEffect,
	useId,
	useRef,
	useState,
} from "react";
import {
	type CertificateAuthorityEnvelope,
	createLocalCertificateAuthority,
} from "../../../../lib/device-management/certificate-authority";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { Block } from "../primitives/block";
import { Checklist } from "../primitives/checklist";
import { DvButton } from "../primitives/dv-button";
import {
	DvInput,
	Field,
	InputWithUnit,
	ListEditor,
	SecretInput,
	utf8Bytes,
} from "../primitives/form-fields";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { IdRef } from "../primitives/id-ref";
import { InlineConfirm } from "../primitives/inline-confirm";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import {
	WizardFoot,
	WizardLayout,
	WizardStepHeader,
	WizardStepper,
} from "../primitives/wizard";
import {
	type AuthorityHome,
	type AuthorityNote,
	BackupSaveStep,
	MAX_PASSWORD_BYTES,
	MIN_PASSWORD,
	backupFileName,
	homeText,
} from "./authority-sheets";
import {
	type AddressProblem,
	type AuthorityDraft,
	MAX_AUTHORITY_NAMES,
	type SuffixProblem,
	addressProblem,
	authorityDates,
	cleanAddresses,
	cleanSuffixes,
	nameCount,
	suffixProblem,
	validYears,
} from "./certificates-model";
import { dayText, shortId } from "./parts";
import {
	type AuthorityStore,
	localAuthority,
	saveFile,
	useStillHere,
} from "./use-certificates";

const MAX_LABEL_BYTES = 64;
const NOTE_BOX =
	"rounded-lg border border-border bg-surface-sunken px-3.5 py-3 text-ui text-ink-2";

type StepId = "names" | "lifetime" | "password" | "backup" | "done";
const STEPS: readonly StepId[] = [
	"names",
	"lifetime",
	"password",
	"backup",
	"done",
];

function stepLabels(t: DevicesT): Record<StepId, string> {
	return {
		names: t("devices:certificates.wizard.step.names", "Names"),
		lifetime: t("devices:certificates.wizard.step.lifetime", "Lifetime"),
		password: t("devices:certificates.wizard.step.password", "Password"),
		backup: t("devices:certificates.wizard.step.backup", "Back up"),
		done: t("devices:certificates.wizard.step.done", "Done"),
	};
}

function suffixProblemText(t: DevicesT, problem: SuffixProblem): string {
	const texts: Record<SuffixProblem, string> = {
		wildcard: t(
			"devices:certificates.wizard.problem.wildcard",
			"Wildcards aren't allowed. A suffix already covers its subdomains.",
		),
		port_or_path: t(
			"devices:certificates.wizard.problem.portOrPath",
			"Leave out ports, paths and https://.",
		),
		trailing_dot: t(
			"devices:certificates.wizard.problem.trailingDot",
			"Remove the trailing dot.",
		),
		characters: t(
			"devices:certificates.wizard.problem.characters",
			"Use letters, digits, hyphens and dots only.",
		),
		numeric_end: t(
			"devices:certificates.wizard.problem.numericEnd",
			"The last part can't be only digits. Put IP addresses in the next list.",
		),
		duplicate: t(
			"devices:certificates.wizard.problem.duplicateSuffix",
			"This suffix is listed twice.",
		),
	};
	return texts[problem];
}

function addressProblemText(t: DevicesT, problem: AddressProblem): string {
	return problem === "duplicate"
		? t(
				"devices:certificates.wizard.problem.duplicateAddress",
				"This address is listed twice.",
			)
		: t(
				"devices:certificates.wizard.problem.notAnAddress",
				"Enter one IP address, like 10.0.0.20. Ranges aren't allowed.",
			);
}

interface ItemProblem {
	index: number;
	text: string;
}

interface NameErrors {
	label?: string;
	suffixes: ItemProblem[];
	addresses: ItemProblem[];
	names?: string;
}

/** One problem per list row; a value counts as seen from its first valid mention. */
function listProblems<P>(
	values: readonly string[],
	normalize: (value: string) => string,
	problemOf: (value: string, seen: ReadonlySet<string>) => P | null,
	text: (problem: P) => string,
): ItemProblem[] {
	const seen = new Set<string>();
	const problems: ItemProblem[] = [];
	values.forEach((value, index) => {
		const problem = problemOf(value, seen);
		if (problem) problems.push({ index, text: text(problem) });
		const clean = normalize(value);
		if (clean) seen.add(clean);
	});
	return problems;
}

function nameErrors(t: DevicesT, draft: AuthorityDraft): NameErrors {
	const bytes = utf8Bytes(draft.label.trim());
	const count = nameCount(draft);
	return {
		...(bytes === 0
			? {
					label: t(
						"devices:certificates.wizard.labelMissing",
						"Give the authority a name, for example Rheosoph Lab.",
					),
				}
			: bytes > MAX_LABEL_BYTES
				? {
						label: t(
							"devices:certificates.wizard.labelTooLong",
							"The name is {{count, number}} bytes. Use at most {{max, number}}.",
							{ count: bytes, max: MAX_LABEL_BYTES },
						),
					}
				: {}),
		suffixes: listProblems(
			draft.suffixes,
			(value) => value.trim().toLowerCase(),
			suffixProblem,
			(problem) => suffixProblemText(t, problem),
		),
		addresses: listProblems(
			draft.addresses,
			(value) => value.trim(),
			addressProblem,
			(problem) => addressProblemText(t, problem),
		),
		...(count === 0
			? {
					names: t(
						"devices:certificates.wizard.namesMissing",
						"Add at least one DNS suffix or IP address.",
					),
				}
			: count > MAX_AUTHORITY_NAMES
				? {
						names: t(
							"devices:certificates.wizard.namesTooMany",
							"That's {{count, number}} names. An authority allows at most {{max, number}}.",
							{ count, max: MAX_AUTHORITY_NAMES },
						),
					}
				: {}),
	};
}

const hasNameErrors = (errors: NameErrors) =>
	!!errors.label ||
	!!errors.names ||
	errors.suffixes.length > 0 ||
	errors.addresses.length > 0;

function ItemProblems({
	problems,
	itemLabel,
}: Readonly<{
	problems: readonly ItemProblem[];
	itemLabel(index: number): string;
}>) {
	if (!problems.length) return null;
	return (
		<ul role="alert" className="flex flex-col gap-0.5 text-xs text-critical">
			{problems.map((problem) => (
				<li key={problem.index}>
					{itemLabel(problem.index)}: {problem.text}
				</li>
			))}
		</ul>
	);
}

function NamesStep({
	draft,
	errors,
	onChange,
}: Readonly<{
	draft: AuthorityDraft;
	errors: NameErrors | null;
	onChange(patch: Partial<AuthorityDraft>): void;
}>) {
	const { t } = useTranslation("devices");
	const labelId = useId();
	const listId = useId();
	const name =
		draft.label.trim() || t("certificates.wizard.labelFallback", "Name");
	const suffixLabel = (index: number) =>
		t("certificates.wizard.suffixItem", "DNS suffix {{n, number}}", {
			n: index + 1,
		});
	const addressLabel = (index: number) =>
		t("certificates.wizard.addressItem", "IP address {{n, number}}", {
			n: index + 1,
		});
	const kept = (values: string[]) => (values.length ? values : [""]);
	return (
		<>
			<Field
				id={labelId}
				label={t("certificates.wizard.label", "Name")}
				error={errors?.label}
				hint={t(
					"certificates.wizard.labelHint",
					'{{count, number}} of {{max, number}} bytes · certificates show it as "{{name}} root" and "{{name}} service issuer"',
					{
						count: utf8Bytes(draft.label.trim()),
						max: MAX_LABEL_BYTES,
						name,
					},
				)}
			>
				<DvInput
					value={draft.label}
					onChange={(event) => onChange({ label: event.target.value })}
					placeholder={t(
						"certificates.wizard.labelPlaceholder",
						"Rheosoph Lab",
					)}
					autoComplete="off"
					className="max-w-105"
				/>
			</Field>
			<fieldset className="m-0 flex min-w-0 flex-col gap-1.5 border-0 p-0">
				<legend className="mb-1.5 p-0 text-[13px]/[18px] font-medium">
					{t("certificates.wizard.suffixes", "DNS suffixes it may sign for")}
				</legend>
				<div className="max-w-105">
					<ListEditor
						id={`${listId}-suffixes`}
						values={draft.suffixes}
						onChange={(values) => onChange({ suffixes: kept(values) })}
						addLabel={t("certificates.wizard.addSuffix", "Add suffix")}
						placeholder="lab.example.com"
						itemLabel={suffixLabel}
					/>
				</div>
				<ItemProblems
					problems={errors?.suffixes ?? []}
					itemLabel={suffixLabel}
				/>
				<p className="text-xs text-muted-foreground">
					{t(
						"certificates.wizard.suffixHint",
						"Each suffix covers its subdomains: lab.internal covers mqtt.lab.internal. No wildcards or ports.",
					)}
				</p>
			</fieldset>
			<fieldset className="m-0 flex min-w-0 flex-col gap-1.5 border-0 p-0">
				<legend className="mb-1.5 p-0 text-[13px]/[18px] font-medium">
					{t("certificates.wizard.addresses", "IP addresses it may sign for")}
				</legend>
				<div className="max-w-105">
					<ListEditor
						id={`${listId}-addresses`}
						values={draft.addresses}
						onChange={(values) => onChange({ addresses: kept(values) })}
						addLabel={t("certificates.wizard.addAddress", "Add address")}
						placeholder="10.0.4.20"
						itemLabel={addressLabel}
					/>
				</div>
				<ItemProblems
					problems={errors?.addresses ?? []}
					itemLabel={addressLabel}
				/>
				<p className="text-xs text-muted-foreground">
					{t(
						"certificates.wizard.addressHint",
						"Each address matches exactly. Leave this empty if you only use names.",
					)}
				</p>
			</fieldset>
			<p className="text-xs text-muted-foreground tabular-nums">
				{t(
					"certificates.wizard.namesTotal",
					"{{count, number}} of {{max, number}} names in total. These limits are written into the root, so it can never sign anything else.",
					{ count: nameCount(draft), max: MAX_AUTHORITY_NAMES },
				)}
			</p>
			{errors?.names ? (
				<p role="alert" className="text-xs text-critical">
					{errors.names}
				</p>
			) : null}
		</>
	);
}

function LifetimeStep({
	draft,
	error,
	onChange,
}: Readonly<{
	draft: AuthorityDraft;
	error?: string;
	onChange(patch: Partial<AuthorityDraft>): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const dates = authorityDates(validYears(draft.years) ?? 3, time.nowS);
	return (
		<>
			<Field
				id={id}
				label={t("certificates.wizard.years", "Root valid for")}
				error={error}
				hint={t(
					"certificates.wizard.yearsHint",
					"1 to 10 years. Longer means fewer replacements on clients, but a lost root stays dangerous for longer.",
				)}
			>
				<InputWithUnit
					unit={t("certificates.wizard.yearsUnit", "years")}
					type="number"
					inputMode="numeric"
					min={1}
					max={10}
					step={1}
					numeric
					value={draft.years}
					onChange={(event) => onChange({ years: event.target.value })}
					className="max-w-45"
				/>
			</Field>
			<KeyValueList>
				<KvRow
					label={t("certificates.wizard.rootExpires", "Root expires")}
					provenance={t(
						"certificates.wizard.rootExpiresNote",
						"clients stop trusting it then",
					)}
				>
					{dayText(time, dates.root)}
				</KvRow>
				<KvRow
					label={t("certificates.wizard.signingExpires", "Signing key expires")}
					provenance={t(
						"certificates.wizard.signingExpiresNote",
						"renew it every year from the backup",
					)}
				>
					{dayText(time, dates.signing)}
				</KvRow>
				<KvRow label={t("certificates.wizard.signs", "Certificates it signs")}>
					{t(
						"certificates.wizard.signsValue",
						"up to 397 days each, never past the signing key",
					)}
				</KvRow>
			</KeyValueList>
			<div className={NOTE_BOX}>
				<b className="font-semibold text-foreground">
					{t("certificates.wizard.keysLead", "Where the keys will live.")}
				</b>{" "}
				{t(
					"certificates.wizard.keysText",
					"The signing key is saved on this computer, encrypted with the password you choose next. The root key goes only into the backup file you download in step 4: it isn't kept here and never reaches Flow-Like.",
				)}
			</div>
		</>
	);
}

interface Passwords {
	first: string;
	again: string;
}

const NO_PASSWORDS: Passwords = { first: "", again: "" };

function passwordErrors(t: DevicesT, passwords: Passwords): Partial<Passwords> {
	if (passwords.first.length < MIN_PASSWORD)
		return {
			first: t(
				"devices:certificates.password.tooShort",
				"The password has {{count, number}} characters. Use at least {{min, number}}.",
				{ count: passwords.first.length, min: MIN_PASSWORD },
			),
		};
	if (utf8Bytes(passwords.first) > MAX_PASSWORD_BYTES)
		return {
			first: t(
				"devices:certificates.password.tooLong",
				"Use at most {{max, number}} bytes.",
				{ max: MAX_PASSWORD_BYTES },
			),
		};
	return passwords.again === passwords.first
		? {}
		: {
				again: t(
					"devices:certificates.password.mismatch",
					"The two passwords don't match. Type the password again in both fields.",
				),
			};
}

function PasswordStep({
	passwords,
	errors,
	failure,
	onChange,
}: Readonly<{
	passwords: Passwords;
	errors: Partial<Passwords>;
	failure: string | null;
	onChange(patch: Partial<Passwords>): void;
}>) {
	const { t } = useTranslation("devices");
	const firstId = useId();
	const againId = useId();
	return (
		<>
			{failure ? <InlineResult tone="critical">{failure}</InlineResult> : null}
			<Field
				id={firstId}
				label={t("certificates.wizard.password", "Authority password")}
				error={errors.first}
				className="max-w-105"
			>
				<SecretInput
					value={passwords.first}
					onValueChange={(first) => onChange({ first })}
					autoComplete="new-password"
					minBytes={MIN_PASSWORD}
					maxBytes={MAX_PASSWORD_BYTES}
				/>
			</Field>
			<Field
				id={againId}
				label={t(
					"certificates.wizard.passwordAgain",
					"Type the password again",
				)}
				error={errors.again}
				className="max-w-105"
			>
				<SecretInput
					value={passwords.again}
					onValueChange={(again) => onChange({ again })}
					autoComplete="new-password"
				/>
			</Field>
			<div className={NOTE_BOX}>
				<b className="font-semibold text-foreground">
					{t(
						"certificates.wizard.passwordLead",
						"This is a different secret from device passwords.",
					)}
				</b>{" "}
				{t(
					"certificates.wizard.passwordText",
					"It encrypts the signing key on this computer and both keys in the backup file. Nobody can recover it: without it, the backup is useless and you create a new authority.",
				)}
			</div>
		</>
	);
}

function NextStep({
	title,
	text,
	children,
}: Readonly<{ title: string; text: string; children?: ReactNode }>) {
	return (
		<li className="flex flex-col gap-1">
			<b className="font-semibold">{title}</b>
			<span className="text-xs text-muted-foreground">{text}</span>
			{children}
		</li>
	);
}

function RootDownload({
	envelope,
}: Readonly<{ envelope: CertificateAuthorityEnvelope }>) {
	const { t } = useTranslation("devices");
	const [downloaded, setDownloaded] = useState<string | null>(null);
	const bundle = envelope.public_bundle;
	const download = () => {
		const file = `${shortId(bundle.authority_id)}-root.crt`;
		saveFile(file, bundle.root_certificate_pem);
		setDownloaded(file);
	};
	return (
		<div className="mt-1 flex flex-col items-start gap-1.5">
			<DvButton size="sm" icon={Download} onClick={download}>
				{t("certificates.authority.downloadRoot", "Download root certificate")}
			</DvButton>
			{downloaded ? (
				<InlineResult tone="good" onDismiss={() => setDownloaded(null)}>
					{t(
						"certificates.wizard.rootDownloaded",
						"{{file}} was handed to your browser.",
						{ file: downloaded },
					)}
				</InlineResult>
			) : null}
		</div>
	);
}

function DoneStep({
	label,
	envelope,
	firstName,
}: Readonly<{
	label: string;
	envelope: CertificateAuthorityEnvelope;
	firstName: string | undefined;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<>
			<InlineResult tone="good">
				{t(
					"certificates.wizard.ready",
					"{{label}} is ready on this computer. Its signing key is saved here, encrypted with your password; the root key is only in your backup.",
					{ label },
				)}
			</InlineResult>
			<ol className="flex list-decimal flex-col gap-3 pl-5 text-ui">
				<NextStep
					title={t(
						"certificates.wizard.nextInstall",
						"Install the root on clients.",
					)}
					text={t(
						"certificates.wizard.nextInstallText",
						"Download the root certificate and add it to the trust store of every client that should trust services signed by {{label}}.",
						{ label },
					)}
				>
					<RootDownload envelope={envelope} />
				</NextStep>
				<NextStep
					title={t(
						"certificates.wizard.nextSign",
						"Sign requests from devices.",
					)}
					text={
						firstName
							? t(
									"certificates.wizard.nextSignText",
									"On a device's Certificates tab, create a signing request for a name under {{name}}, then choose Sign with organisation authority.",
									{ name: firstName },
								)
							: t(
									"certificates.wizard.nextSignTextPlain",
									"On a device's Certificates tab, create a signing request for one of its allowed names, then choose Sign with organisation authority.",
								)
					}
				/>
				<NextStep
					title={t(
						"certificates.wizard.nextRenew",
						"Renew the signing key before {{date}}.",
						{ date: dayText(time, envelope.public_bundle.issuer_not_after) },
					)}
					text={t(
						"certificates.wizard.nextRenewText",
						"You need the backup file and the password for that.",
					)}
				/>
			</ol>
		</>
	);
}

export interface CreateAuthorityResult {
	authorityId?: string;
	note?: AuthorityNote;
}

export interface CreateAuthorityWizardProps {
	store: AuthorityStore;
	home: AuthorityHome;
	onClose(result?: CreateAuthorityResult): void;
	/** Lets the screen ask before leaving while the new root key exists only in memory. */
	leaveGuard: MutableRefObject<(() => boolean) | null>;
}

const EMPTY_DRAFT: AuthorityDraft = {
	label: "",
	suffixes: [""],
	addresses: [""],
	years: "3",
};

/**
 * SPEC §5.8 Create authority: names → lifetime → password → back up → done.
 * The signing key is stored only after the backup was downloaded and the
 * reader ticked that it is saved; the root key never stays on this computer.
 */
export function CreateAuthorityWizard({
	store,
	home,
	onClose,
	leaveGuard,
}: Readonly<CreateAuthorityWizardProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const stillHere = useStillHere();
	const heading = useRef<HTMLHeadingElement>(null);
	const [step, setStep] = useState(0);
	const [draft, setDraft] = useState<AuthorityDraft>(EMPTY_DRAFT);
	const [passwords, setPasswords] = useState<Passwords>(NO_PASSWORDS);
	const [tried, setTried] = useState<ReadonlySet<number>>(new Set());
	const [busy, setBusy] = useState(false);
	const [failure, setFailure] = useState<string | null>(null);
	const [envelope, setEnvelope] = useState<CertificateAuthorityEnvelope | null>(
		null,
	);
	const [downloaded, setDownloaded] = useState(false);
	const [saved, setSaved] = useState(false);
	const [leaving, setLeaving] = useState(false);
	const labels = stepLabels(t);
	const stepId = STEPS[step] ?? "names";
	const label = draft.label.trim();
	const unsaved = stepId === "backup" && envelope !== null;

	useEffect(() => {
		leaveGuard.current = () => {
			if (!unsaved) return false;
			setLeaving(true);
			return true;
		};
		return () => {
			leaveGuard.current = null;
		};
	}, [leaveGuard, unsaved]);

	const go = (next: number) => {
		setStep(next);
		requestAnimationFrame(() => heading.current?.focus());
	};
	const wasTried = tried.has(step);
	const names = nameErrors(t, draft);
	const yearsError =
		validYears(draft.years) === null
			? t(
					"certificates.wizard.yearsInvalid",
					"Choose a whole number of years from 1 to 10.",
				)
			: undefined;
	const secretErrors = passwordErrors(t, passwords);

	const create = async () => {
		const secret = passwords.first;
		const years = validYears(draft.years);
		if (years === null) return;
		const here = stillHere();
		setPasswords(NO_PASSWORDS);
		setTried(new Set());
		setFailure(null);
		setBusy(true);
		try {
			const crypto = await store.crypto();
			const created = await createLocalCertificateAuthority(
				store.scope,
				{
					label,
					dns_suffixes: cleanSuffixes(draft),
					ip_addresses: cleanAddresses(draft),
					validity_days: years * 365,
				},
				secret,
				crypto,
			);
			if (!here()) return;
			setEnvelope(created);
			go(3);
		} catch {
			if (!here()) return;
			setFailure(
				t(
					"certificates.wizard.createFailed",
					"The authority couldn't be created. Check its name, DNS suffixes, IP addresses and validity, then type the password again.",
				),
			);
		} finally {
			if (here()) setBusy(false);
		}
	};

	const next = () => {
		setTried(new Set(tried).add(step));
		if (stepId === "names" && !hasNameErrors(names)) go(1);
		else if (stepId === "lifetime" && !yearsError) go(2);
		else if (stepId === "password" && !Object.keys(secretErrors).length)
			void create();
	};

	const use = async () => {
		if (!envelope || !saved) return;
		const here = stillHere();
		setBusy(true);
		setFailure(null);
		const stored = await store.add(
			localAuthority(envelope),
			t("certificates.wizard.storeLabel", "Save the signing key of {{label}}", {
				label,
			}),
		);
		if (!here()) return;
		setBusy(false);
		if (stored) go(4);
		else
			setFailure(
				t(
					"certificates.wizard.storeFailed",
					"This computer couldn't save the signing key. Nothing was saved; try again.",
				),
			);
	};

	const discard = () =>
		onClose({
			note: {
				tone: "good",
				text: t(
					"certificates.wizard.discarded",
					"Discarded {{label}} at {{time}}. Nothing was saved on this computer.",
					{ label, time: time.clock(time.nowS) },
				),
			},
		});
	const finish = () =>
		onClose(
			envelope
				? {
						authorityId: envelope.public_bundle.authority_id,
						note: {
							tone: "good",
							text: t(
								"certificates.wizard.created",
								"{{label}} was created at {{time}} and is ready to sign. Keep its backup file offline: it's the only copy of the root key.",
								{ label, time: time.clock(time.nowS) },
							),
						},
					}
				: undefined,
		);

	const cancel =
		stepId === "backup" ? (
			<DvButton variant="danger-ghost" onClick={() => setLeaving(true)}>
				{t("certificates.wizard.discard", "Discard authority…")}
			</DvButton>
		) : stepId === "done" ? undefined : (
			<DvButton variant="ghost" disabled={busy} onClick={() => onClose()}>
				{t("certificates.sheet.cancel", "Cancel")}
			</DvButton>
		);

	let body: ReactNode;
	if (stepId === "names")
		body = (
			<NamesStep
				draft={draft}
				errors={wasTried ? names : null}
				onChange={(patch) => setDraft({ ...draft, ...patch })}
			/>
		);
	else if (stepId === "lifetime")
		body = (
			<LifetimeStep
				draft={draft}
				error={wasTried ? yearsError : undefined}
				onChange={(patch) => setDraft({ ...draft, ...patch })}
			/>
		);
	else if (stepId === "password")
		body = busy ? (
			<Checklist
				label={t("certificates.wizard.creating", "Creating the authority")}
				items={[
					{
						id: "root",
						state: "active",
						label: t(
							"certificates.wizard.creatingRoot",
							"Creating the root key",
						),
						source: "local",
					},
					{
						id: "signing",
						state: "pending",
						label: t(
							"certificates.wizard.creatingSigning",
							"Creating the signing key, signed by the root",
						),
						source: "local",
					},
					{
						id: "seal",
						state: "pending",
						label: t(
							"certificates.wizard.creatingSeal",
							"Encrypting both with your password",
						),
						source: "local",
					},
				]}
			/>
		) : (
			<PasswordStep
				passwords={passwords}
				errors={wasTried ? secretErrors : {}}
				failure={failure}
				onChange={(patch) => setPasswords({ ...passwords, ...patch })}
			/>
		);
	else if (stepId === "backup" && envelope)
		body = (
			<>
				{leaving ? (
					<InlineConfirm
						label={t(
							"certificates.wizard.discardLabel",
							"Discard the new authority?",
						)}
						title={t("certificates.wizard.discardTitle", "Discard {{label}}?", {
							label,
						})}
						sub={t(
							"certificates.wizard.discardWarning",
							"The root key exists only in this backup. If you close now, it's lost.",
						)}
						rows={{
							what: t(
								"certificates.wizard.discardWhat",
								"The new root key and signing key are deleted from memory. Nothing was saved on this computer.",
							),
							who: t(
								"certificates.wizard.discardWho",
								"Nobody. Nothing was signed with them yet.",
							),
							when: t("certificates.wizard.discardWhen", "Immediately."),
							undo: {
								reversible: false,
								text: t(
									"certificates.wizard.discardUndo",
									"Create a new authority instead; it gets a different root and fingerprint.",
								),
							},
						}}
						confirmLabel={t(
							"certificates.wizard.discardConfirm",
							"Discard authority",
						)}
						onConfirm={discard}
						onCancel={() => setLeaving(false)}
					/>
				) : null}
				<Banner
					tone="warning"
					title={t(
						"certificates.wizard.backupTitle",
						"This backup is the only copy of the root key.",
					)}
				>
					{t(
						"certificates.wizard.backupText",
						"Download it and store it offline together with its password, then continue. Nothing is saved on this computer until you use the authority.",
					)}
				</Banner>
				<KeyValueList>
					<KvRow label={t("certificates.backup.file", "File")}>
						<span className="font-mono text-xs">
							{backupFileName(envelope.public_bundle.authority_id)}
						</span>
					</KvRow>
					<KvRow label={t("certificates.wizard.contains", "Contains")}>
						{t(
							"certificates.wizard.containsValue",
							"the root key and the signing key, both encrypted with the authority password",
						)}
					</KvRow>
					<KvRow label={t("certificates.wizard.opensFor", "Opens for")}>
						{t(
							"certificates.wizard.opensForValue",
							"{{home}}, with the password",
							{
								home: homeText(t, home),
							},
						)}
					</KvRow>
					<KvRow
						label={t(
							"certificates.authority.rootFingerprint",
							"Root fingerprint",
						)}
					>
						<IdRef
							id={envelope.public_bundle.sha256_fingerprint}
							group4
							copyLabel={t(
								"certificates.authority.copyFingerprint",
								"Copy root fingerprint",
							)}
						/>
					</KvRow>
				</KeyValueList>
				<BackupSaveStep
					envelope={envelope}
					downloaded={downloaded}
					onDownloaded={() => setDownloaded(true)}
					saved={saved}
					onSaved={(next) => {
						setSaved(next);
						if (next) setLeaving(false);
					}}
					downloadLabel={t(
						"certificates.wizard.download",
						"Download encrypted authority backup",
					)}
					savedLabel={t(
						"certificates.wizard.saved",
						"I saved the backup and its password",
					)}
					downloadFirst={t(
						"certificates.wizard.downloadFirst",
						"Download the backup first.",
					)}
				/>
				{failure ? (
					<InlineResult tone="critical">{failure}</InlineResult>
				) : null}
			</>
		);
	else if (envelope)
		body = (
			<DoneStep
				label={label}
				envelope={envelope}
				firstName={cleanSuffixes(draft)[0] ?? cleanAddresses(draft)[0]}
			/>
		);

	const headings: Record<StepId, string> = {
		names: t(
			"certificates.wizard.heading.names",
			"What may this authority sign for?",
		),
		lifetime: t(
			"certificates.wizard.heading.lifetime",
			"How long should its root be valid?",
		),
		password: t(
			"certificates.wizard.heading.password",
			"Choose the authority password",
		),
		backup: t(
			"certificates.wizard.heading.backup",
			"Save the backup before you use it",
		),
		done: t("certificates.wizard.heading.done", "The authority is ready"),
	};
	const nextStep = STEPS[step + 1];

	return (
		<Block
			id="certificates-wizard"
			icon={ShieldCheck}
			title={t("certificates.wizard.title", "Create an organisation authority")}
			stamp={
				<FreshnessStamp
					source="local"
					age="current"
					text={t(
						"certificates.wizard.createdHere",
						"created on this computer",
					)}
				/>
			}
		>
			<WizardLayout
				stepper={
					<WizardStepper steps={STEPS.map((id) => labels[id])} current={step} />
				}
				foot={
					<WizardFoot
						step={step + 1}
						total={STEPS.length}
						stepLabel={labels[stepId]}
						nextStepLabel={nextStep ? labels[nextStep] : undefined}
						cancel={cancel}
						onBack={
							step > 0 && step < 3 && !busy ? () => go(step - 1) : undefined
						}
						onNext={
							stepId === "backup"
								? () => void use()
								: stepId === "done"
									? undefined
									: next
						}
						nextLabel={
							stepId === "password"
								? t("certificates.wizard.create", "Create authority")
								: stepId === "backup"
									? t("certificates.wizard.use", "Use this authority")
									: undefined
						}
						nextIcon={
							stepId === "password"
								? ShieldCheck
								: stepId === "backup"
									? Check
									: undefined
						}
						nextBusy={busy}
						nextGate={
							stepId === "backup" && !saved
								? {
										kind: "busy",
										reason: downloaded
											? t(
													"certificates.wizard.tickFirst",
													"Tick the box once the backup and password are stored.",
												)
											: t(
													"certificates.wizard.downloadAndTick",
													"Download the backup and tick the box first.",
												),
									}
								: null
						}
					>
						{stepId === "done" ? (
							<DvButton variant="primary" onClick={finish}>
								{t("certificates.wizard.back", "Back to authorities")}
							</DvButton>
						) : undefined}
					</WizardFoot>
				}
			>
				<WizardStepHeader
					headingRef={heading}
					title={headings[stepId]}
					step={step + 1}
					total={STEPS.length}
				/>
				<div className="flex min-w-0 flex-col gap-4">{body}</div>
			</WizardLayout>
		</Block>
	);
}

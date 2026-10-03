"use client";

import { useTranslation } from "@flow-like/locales";
import { FileKey, FileText, FileUp, X } from "lucide-react";
import { useId, useState } from "react";
import {
	MAX_CERTIFICATE_PEM_BYTES,
	putCertificate,
} from "../../../../../lib/device-management/certificates";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { ConsequencePreview } from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { CheckField, DropZone, Field } from "../../primitives/form-fields";
import { useDeviceAction } from "../../workspace";
import { listResultKey } from "./certificate-list";
import {
	ActionResults,
	LabelField,
	SheetBlocker,
	TickFirst,
	certificateResultKey,
	chainProblem,
	formatKiB,
	gateView,
	labelProblem,
	useAlive,
	withDeviceReason,
} from "./certificate-parts";
import {
	type CertificateRow,
	type DeviceCertificates,
	MAX_CERTIFICATE_SLOTS,
	certificateLabel,
} from "./use-device-certificates";

const PRIVATE_KEY =
	/^\s*-----BEGIN (PRIVATE KEY|RSA PRIVATE KEY|EC PRIVATE KEY)-----[\s\S]+-----END \1-----\s*$/u;
/** Enough of a PEM file to read its first marker, never its content. */
const HEADER_BYTES = 96;

type Role = "chain" | "key";
type FileProblem =
	| "unknown_file"
	| "unreadable"
	| "missing"
	| "too_large"
	| "encrypted_key"
	| "not_a_chain"
	| "not_a_key";

function problemText(t: DevicesT, problem: FileProblem): string {
	const texts = {
		unknown_file: t(
			"devices:deviceCertificates.import.unknownFile",
			"One of the files is neither a PEM certificate chain nor a PEM private key.",
		),
		unreadable: t(
			"devices:deviceCertificates.import.unreadable",
			"The files couldn't be read. Choose them again.",
		),
		missing: t(
			"devices:deviceCertificates.import.missing",
			"Choose both the certificate chain and its private key.",
		),
		too_large: t(
			"devices:deviceCertificates.import.tooLarge",
			"The chain and the key are larger than 12 KiB together.",
		),
		encrypted_key: t(
			"devices:deviceCertificates.import.encryptedKey",
			"This private key is encrypted. Use the unencrypted PEM key: it travels over the encrypted connection to the device.",
		),
		not_a_chain: t(
			"devices:deviceCertificates.import.notAChain",
			"The chain file must hold PEM certificates only, the service certificate first.",
		),
		not_a_key: t(
			"devices:deviceCertificates.import.notAKey",
			"The key file must hold one unencrypted PEM private key.",
		),
	} satisfies Record<FileProblem, string>;
	return texts[problem];
}

async function roleOf(file: File): Promise<Role | "encrypted" | null> {
	const header = await file.slice(0, HEADER_BYTES).text();
	if (/ENCRYPTED/u.test(header)) return "encrypted";
	if (/PRIVATE KEY/u.test(header)) return "key";
	return /BEGIN CERTIFICATE/u.test(header) ? "chain" : null;
}

/** Why the file pair can't go to the device as it is; the texts themselves are never kept. */
function pairProblem(chain: string, key: string): FileProblem | null {
	const chainIssue = chainProblem(chain);
	if (chainIssue === "too_large") return "too_large";
	if (chainIssue) return "not_a_chain";
	if (/ENCRYPTED/u.test(key)) return "encrypted_key";
	return PRIVATE_KEY.test(key) ? null : "not_a_key";
}

function FileChip({
	file,
	holds,
	onRemove,
}: Readonly<{ file: File; holds: Role; onRemove(): void }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const Icon = holds === "key" ? FileKey : FileText;
	return (
		<span
			data-file={holds}
			className="inline-flex max-w-full items-center gap-1.5 rounded-sm border border-hairline bg-card py-0.5 pr-0.5 pl-1.5 text-xs"
		>
			<Icon aria-hidden className="size-3.5 shrink-0 text-ink-2" />
			<span className="min-w-0 truncate font-mono">{file.name}</span>
			<span className="whitespace-nowrap text-muted-foreground">
				{holds === "key"
					? t(
							"deviceCertificates.import.keyFile",
							"private key · {{size}} KiB",
							{
								size: formatKiB(time.locale, file.size),
							},
						)
					: t(
							"deviceCertificates.import.chainFile",
							"certificate chain · {{size}} KiB",
							{ size: formatKiB(time.locale, file.size) },
						)}
			</span>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={X}
				aria-label={t("deviceCertificates.import.remove", "Remove {{file}}", {
					file: file.name,
				})}
				onClick={onRemove}
			/>
		</span>
	);
}

/**
 * S32: import a certificate from PEM files, or replace one under the same ID.
 * The chosen files stay selected when validation or the device refuses them.
 */
export function CertificateImportSheet({
	certs,
	row,
	onClose,
}: Readonly<{
	certs: DeviceCertificates;
	row?: CertificateRow;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const alive = useAlive();
	const actions = useDeviceAction();
	const current = row ? certificateLabel(row) : "";
	const [label, setLabel] = useState(current);
	const [files, setFiles] = useState<Partial<Record<Role, File>>>({});
	const [problem, setProblem] = useState<FileProblem | null>(null);
	const [acknowledged, setAcknowledged] = useState(false);
	const [touched, setTouched] = useState(false);
	const [reading, setReading] = useState(false);
	const [ran, setRan] = useState(false);
	const resultKey = row
		? certificateResultKey(certs.deviceId, row.id)
		: listResultKey(certs.deviceId);
	const automatic = row?.mode === "acme" || row?.mode === "delegated";
	const users = row?.uses ?? 0;
	const full = !row && (certs.slots?.used ?? 0) >= MAX_CERTIFICATE_SLOTS;
	const view = gateView(t, time, certs.gate("certificate_import"));
	const labelError = labelProblem(label);
	const blocked = !!view || full || (automatic && !acknowledged);

	const pick = async (chosen: File[]) => {
		const next = { ...files };
		let found: FileProblem | null = null;
		for (const file of chosen) {
			let role: Awaited<ReturnType<typeof roleOf>>;
			try {
				role = await roleOf(file);
			} catch {
				found = "unreadable";
				continue;
			}
			if (role === "encrypted") found = "encrypted_key";
			else if (role) next[role] = file;
			else found = "unknown_file";
		}
		if (!alive()) return;
		setFiles(next);
		setProblem(found);
	};
	const remove = (role: Role) => {
		setFiles(({ [role]: _removed, ...rest }) => rest);
		setProblem(null);
	};

	const submit = async () => {
		setTouched(true);
		const { chain, key } = files;
		if (labelError || blocked) return;
		if (!chain || !key) {
			setProblem("missing");
			return;
		}
		if (chain.size + key.size > MAX_CERTIFICATE_PEM_BYTES) {
			setProblem("too_large");
			return;
		}
		setReading(true);
		let chainText = "";
		let keyText = "";
		try {
			[chainText, keyText] = await Promise.all([chain.text(), key.text()]);
		} catch {
			if (alive()) {
				setProblem("unreadable");
				setReading(false);
			}
			return;
		}
		if (!alive()) return;
		const invalid = pairProblem(chainText, keyText);
		setProblem(invalid);
		setReading(false);
		if (invalid) return;
		setRan(true);
		const name = label.trim();
		const outcome = await actions.run({
			action: "certificate_import",
			deviceId: certs.deviceId,
			label: row
				? t("deviceCertificates.import.replaceLabel", "Replace {{label}}", {
						label: name,
					})
				: t("deviceCertificates.import.label", "Import certificate {{label}}", {
						label: name,
					}),
			resultKey,
			call: (context) =>
				withDeviceReason(context, async (call) => {
					await putCertificate(call, {
						certificateId: row?.id ?? crypto.randomUUID(),
						label,
						expectedRevision: row?.detail ? row.revision : 0,
						chain: chainText,
						key: keyText,
					});
				}),
		});
		// Drop the references promptly; JavaScript strings can't be wiped.
		chainText = "";
		keyText = "";
		if (outcome.status !== "done") return;
		await certs.reload();
		if (alive()) onClose();
	};

	return (
		<DvSheet
			open
			onOpenChange={(next) => {
				if (!next) onClose();
			}}
			icon={FileUp}
			title={
				row
					? t(
							"deviceCertificates.import.replaceTitle",
							"Replace {{label}} with a file",
							{ label: current },
						)
					: t("deviceCertificates.import.title", "Import a certificate")
			}
			sub={certs.name}
			footNote={
				row ? (
					automatic && !acknowledged && !view ? (
						<TickFirst />
					) : undefined
				) : (
					t(
						"deviceCertificates.request.slot",
						"Uses 1 certificate slot: {{used, number}} of {{max, number}} after this.",
						{
							used: (certs.slots?.used ?? 0) + 1,
							max: MAX_CERTIFICATE_SLOTS,
						},
					)
				)
			}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("deviceCertificates.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={reading || actions.pending(resultKey)}
						aria-disabled={blocked || undefined}
						onClick={() => void submit()}
					>
						{row
							? t(
									"deviceCertificates.import.replaceSubmit",
									"Replace certificate",
								)
							: t("deviceCertificates.import.submit", "Import certificate")}
					</DvButton>
				</>
			}
		>
			<SheetBlocker view={view} full={full} />
			<LabelField
				id={`${id}-label`}
				value={label}
				onChange={setLabel}
				showProblem={touched}
			/>
			<Field
				id={`${id}-files`}
				label={t(
					"deviceCertificates.import.files",
					"Certificate chain and private key (PEM, up to 12 KiB together)",
				)}
				hint={t(
					"deviceCertificates.import.filesHint",
					"The chain starts with the service certificate, followed by its intermediates. The key is unencrypted and travels over the unlocked, encrypted connection; it can't be downloaded from the device again.",
				)}
				error={problem ? problemText(t, problem) : undefined}
			>
				<DropZone
					id={`${id}-files`}
					accept=".pem,.crt,.cer,.key"
					multiple
					title={t(
						"deviceCertificates.import.drop",
						"Drop both files here or choose files",
					)}
					files={
						files.chain || files.key ? (
							<>
								{files.chain ? (
									<FileChip
										file={files.chain}
										holds="chain"
										onRemove={() => remove("chain")}
									/>
								) : null}
								{files.key ? (
									<FileChip
										file={files.key}
										holds="key"
										onRemove={() => remove("key")}
									/>
								) : null}
							</>
						) : undefined
					}
					onFiles={(chosen) => void pick(chosen)}
				/>
			</Field>
			<ConsequencePreview
				rows={
					row
						? {
								what: t(
									"deviceCertificates.import.replaceWhat",
									"The device checks that the chain and the key match, then swaps {{label}} for the new certificate under the same ID.",
									{ label: current },
								),
								who: users
									? t(
											"deviceCertificates.import.replaceWho",
											"The services that use {{label}} pick up the new certificate on their next connection. No restart.",
											{ label: current },
										)
									: t(
											"deviceCertificates.import.whoNone",
											"Nobody: nothing uses it today.",
										),
								stays: t(
									"deviceCertificates.import.replaceStays",
									"The certificate ID and the services that use it. Names come from the new certificate.",
								),
								when: automatic
									? t(
											"deviceCertificates.import.replaceWhenAutomatic",
											"Immediately after the check. Replacing this certificate stops its automatic renewal.",
										)
									: t(
											"deviceCertificates.import.when",
											"Immediately after the check.",
										),
								undo: {
									reversible: false,
									text: automatic
										? t(
												"deviceCertificates.import.replaceUndoAutomatic",
												"Replace it again with another file. Automatic renewal has to be set up again.",
											)
										: t(
												"deviceCertificates.import.replaceUndo",
												"Replace it again with another file.",
											),
								},
							}
						: {
								what: t(
									"deviceCertificates.import.what",
									"The device checks that the chain and the key match, then keeps the certificate under a new ID.",
								),
								who: t(
									"deviceCertificates.import.who",
									"Nobody: nothing uses it until you pick it for a service's web endpoint.",
								),
								when: t(
									"deviceCertificates.import.when",
									"Immediately after the check.",
								),
								undo: {
									reversible: true,
									text: t(
										"deviceCertificates.import.undo",
										"Delete the certificate while no service uses it.",
									),
								},
							}
				}
			/>
			{automatic ? (
				<CheckField
					id={`${id}-ack`}
					checked={acknowledged}
					onCheckedChange={setAcknowledged}
				>
					{t(
						"deviceCertificates.install.stopsRenewal",
						"Automatic renewal of {{label}} stops. I'll renew it myself.",
						{ label: current },
					)}
				</CheckField>
			) : null}
			{ran ? <ActionResults scopeKey={resultKey} /> : null}
		</DvSheet>
	);
}

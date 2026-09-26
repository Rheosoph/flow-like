"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	certificateAuthorityBackup,
	createLocalCertificateAuthority,
	renewLocalCertificateAuthority,
	restoreCertificateAuthority,
	type CertificateAuthorityEnvelope,
	type LocalCertificateAuthority,
} from "../../../lib/device-management/certificate-authority";
import { certificateNames } from "../../../lib/device-management/certificate-issuance";
import { loadDeviceCrypto } from "../../../lib/device-management/crypto";
import {
	addCertificateAuthority,
	readCertificateAuthorities,
	removeCertificateAuthority,
	replaceCertificateAuthority,
	type DeviceAccountScope,
} from "../../../lib/device-management/storage";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export function downloadCertificateFile(name: string, content: Blob | string) {
	const url = URL.createObjectURL(
		typeof content === "string"
			? new Blob([content], { type: "application/x-pem-file" })
			: content,
	);
	const link = document.createElement("a");
	link.href = url;
	link.download = name;
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function DeviceCertificateAuthorities({
	scope,
	authorities,
	onChanged,
	disabled = false,
}: {
	scope: DeviceAccountScope;
	authorities: LocalCertificateAuthority[];
	onChanged: (authorities: LocalCertificateAuthority[]) => void;
	disabled?: boolean;
}) {
	const id = useId();
	const alive = useRef(true);
	const working = useRef(false);
	const importInput = useRef<HTMLInputElement>(null);
	const rootInput = useRef<HTMLInputElement>(null);
	const [label, setLabel] = useState("");
	const [dns, setDns] = useState("");
	const [ips, setIps] = useState("");
	const [years, setYears] = useState("3");
	const [password, setPassword] = useState("");
	const [confirmation, setConfirmation] = useState("");
	const [importPassword, setImportPassword] = useState("");
	const [renewId, setRenewId] = useState("");
	const [renewPassword, setRenewPassword] = useState("");
	const [pending, setPending] = useState<{
		envelope: CertificateAuthorityEnvelope;
		previous?: LocalCertificateAuthority;
	}>();
	const [saved, setSaved] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState("");
	const [message, setMessage] = useState("");
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	async function execute(operation: () => Promise<void>) {
		if (working.current || disabled) return;
		working.current = true;
		setBusy(true);
		setError("");
		setMessage("");
		try {
			await operation();
		} catch (failure) {
			if (alive.current)
				setError(
					failure instanceof Error
						? failure.message
						: "The authority operation failed.",
				);
		} finally {
			working.current = false;
			if (alive.current) setBusy(false);
		}
	}
	async function refresh() {
		const values = await readCertificateAuthorities(scope);
		if (alive.current) onChanged(values);
	}
	const blocked = disabled || busy || Boolean(pending);
	return (
		<details className="space-y-3 rounded border p-3">
			<summary className="cursor-pointer font-medium">
				Organisation certificate authorities
			</summary>
			<p className="text-sm text-muted-foreground">
				Create a private certificate authority for your organisation's service
				names. Your password encrypts its signing keys on this app. The
				encrypted root backup is downloaded separately and is never saved in
				browser storage or uploaded to Flow-Like. Install only the public root
				certificate on clients that should trust these services.
			</p>
			{authorities.map((authority) => (
				<div
					key={authority.public_bundle.authority_id}
					className="space-y-2 rounded border p-3 text-sm"
				>
					<strong>{authority.public_bundle.label}</strong>
					<p>
						Allowed DNS suffixes:{" "}
						{authority.public_bundle.dns_suffixes.join(", ") || "None"}. IP
						addresses:{" "}
						{authority.public_bundle.ip_addresses.join(", ") || "None"}.
					</p>
					<p>
						Issuing authority expires:{" "}
						{new Date(
							authority.public_bundle.issuer_not_after * 1000,
						).toLocaleString()}
						. Root expires:{" "}
						{new Date(
							authority.public_bundle.not_after * 1000,
						).toLocaleString()}
						.
					</p>
					<p className="break-all font-mono text-xs">
						Root SHA-256: {authority.public_bundle.sha256_fingerprint}
					</p>
					<div className="flex flex-wrap gap-2">
						<Button
							type="button"
							variant="outline"
							disabled={disabled || busy}
							onClick={() =>
								downloadCertificateFile(
									`${authority.public_bundle.authority_id}-root.crt`,
									authority.public_bundle.root_certificate_pem,
								)
							}
						>
							Download public root for {authority.public_bundle.label}
						</Button>
						<Button
							type="button"
							variant="outline"
							disabled={blocked}
							onClick={() => setRenewId(authority.public_bundle.authority_id)}
						>
							Renew issuing authority
						</Button>
						<Button
							type="button"
							variant="outline"
							disabled={blocked}
							onClick={() =>
								void execute(async () => {
									await removeCertificateAuthority(
										scope,
										authority.public_bundle.authority_id,
									);
									await refresh();
									if (alive.current)
										setMessage(
											"The encrypted signing key was removed from this app. Installed certificates and saved backups still exist.",
										);
								})
							}
						>
							Remove local signing key for {authority.public_bundle.label}
						</Button>
					</div>
				</div>
			))}
			{pending && (
				<div
					className="space-y-3 rounded border border-primary p-3"
					role="status"
				>
					<p>
						Save the encrypted root backup before enabling this authority. Keep
						it offline with its password. It is required to renew the issuing
						authority and recover access after local storage is lost.
					</p>
					<Button
						type="button"
						disabled={busy || disabled}
						onClick={() =>
							downloadCertificateFile(
								`${pending.envelope.public_bundle.authority_id}-authority.json`,
								certificateAuthorityBackup(pending.envelope),
							)
						}
					>
						Download encrypted authority backup
					</Button>
					<label className="flex items-center gap-2 text-sm">
						<input
							type="checkbox"
							checked={saved}
							disabled={busy || disabled}
							onChange={(event) => setSaved(event.target.checked)}
						/>
						I saved the encrypted root backup and its password.
					</label>
					<Button
						type="button"
						disabled={busy || disabled || !saved}
						onClick={() =>
							void execute(async () => {
								const local = {
									public_bundle: pending.envelope.public_bundle,
									vault: Uint8Array.from(pending.envelope.vault),
								};
								if (pending.previous)
									await replaceCertificateAuthority(
										scope,
										pending.previous,
										local,
									);
								else await addCertificateAuthority(scope, local);
								await refresh();
								if (alive.current) {
									setPending(undefined);
									setSaved(false);
									setMessage(
										"The encrypted issuing key is saved locally. The root key remains only in your downloaded encrypted backup.",
									);
								}
							})
						}
					>
						Use this authority
					</Button>
					<Button
						type="button"
						variant="outline"
						disabled={busy || disabled}
						onClick={() => {
							setPending(undefined);
							setSaved(false);
						}}
					>
						Discard unsaved authority
					</Button>
				</div>
			)}
			{renewId && (
				<form
					className="space-y-2 rounded border p-3"
					onSubmit={(event) => {
						event.preventDefault();
						const secret = renewPassword;
						setRenewPassword("");
						const file = rootInput.current?.files?.[0];
						if (rootInput.current) rootInput.current.value = "";
						void execute(async () => {
							const authority = authorities.find(
								(value) => value.public_bundle.authority_id === renewId,
							);
							if (!authority || !file || file.size > 1024 * 1024)
								throw new Error(
									"Choose this authority's encrypted root backup of at most 1 MiB.",
								);
							const text = await file.text();
							const crypto = await loadDeviceCrypto();
							if (!alive.current) return;
							const envelope = await renewLocalCertificateAuthority(
								scope,
								authority,
								text,
								secret,
								crypto,
							);
							if (alive.current) {
								setPending({ envelope, previous: authority });
								setSaved(false);
								setRenewId("");
							}
						});
					}}
				>
					<label htmlFor={`${id}-root-backup`} className="block text-sm">
						Encrypted root backup
						<Input
							id={`${id}-root-backup`}
							ref={rootInput}
							type="file"
							accept=".json"
							required
							disabled={blocked}
						/>
					</label>
					<label htmlFor={`${id}-renew-password`} className="block text-sm">
						Authority password
						<Input
							id={`${id}-renew-password`}
							type="password"
							autoComplete="off"
							value={renewPassword}
							onChange={(event) => setRenewPassword(event.target.value)}
							required
							disabled={blocked}
						/>
					</label>
					<Button type="submit" disabled={blocked || !renewPassword}>
						Renew with root backup
					</Button>
					<Button
						type="button"
						variant="outline"
						disabled={blocked}
						onClick={() => {
							setRenewId("");
							setRenewPassword("");
						}}
					>
						Cancel renewal
					</Button>
				</form>
			)}
			<form
				className="space-y-2 border-t pt-3"
				onSubmit={(event) => {
					event.preventDefault();
					const secret = password;
					const matches = password === confirmation;
					setPassword("");
					setConfirmation("");
					void execute(async () => {
						if (!matches)
							throw new Error("The authority passwords do not match.");
						const crypto = await loadDeviceCrypto();
						if (!alive.current) return;
						const envelope = await createLocalCertificateAuthority(
							scope,
							{
								label: label.trim(),
								dns_suffixes: certificateNames(dns),
								ip_addresses: certificateNames(ips),
								validity_days: Number(years) * 365,
							},
							secret,
							crypto,
						);
						if (alive.current) {
							setPending({ envelope });
							setSaved(false);
						}
					});
				}}
			>
				<h4 className="font-medium">Create organisation authority</h4>
				<label htmlFor={`${id}-label`} className="block text-sm">
					Authority label
					<Input
						id={`${id}-label`}
						value={label}
						onChange={(event) => setLabel(event.target.value)}
						required
						maxLength={64}
						disabled={blocked}
					/>
				</label>
				<label htmlFor={`${id}-dns`} className="block text-sm">
					Permitted DNS suffixes
					<Input
						id={`${id}-dns`}
						placeholder="internal.example.com"
						value={dns}
						onChange={(event) => setDns(event.target.value)}
						disabled={blocked}
					/>
				</label>
				<label htmlFor={`${id}-ips`} className="block text-sm">
					Permitted IP addresses
					<Input
						id={`${id}-ips`}
						placeholder="10.0.0.20"
						value={ips}
						onChange={(event) => setIps(event.target.value)}
						disabled={blocked}
					/>
				</label>
				<label htmlFor={`${id}-years`} className="block text-sm">
					Root validity in years
					<Input
						id={`${id}-years`}
						type="number"
						min={1}
						max={10}
						value={years}
						onChange={(event) => setYears(event.target.value)}
						required
						disabled={blocked}
					/>
				</label>
				<p className="text-xs text-muted-foreground">
					Use up to 32 DNS suffixes and IP addresses in total. DNS suffixes
					include their subdomains. IP addresses are exact. The issuing
					authority lasts up to one year; service certificates can never outlive
					their signing chain.
				</p>
				<label htmlFor={`${id}-password`} className="block text-sm">
					New authority password
					<Input
						id={`${id}-password`}
						type="password"
						autoComplete="new-password"
						minLength={12}
						value={password}
						onChange={(event) => setPassword(event.target.value)}
						required
						disabled={blocked}
					/>
				</label>
				<label htmlFor={`${id}-confirm`} className="block text-sm">
					Repeat authority password
					<Input
						id={`${id}-confirm`}
						type="password"
						autoComplete="new-password"
						value={confirmation}
						onChange={(event) => setConfirmation(event.target.value)}
						required
						disabled={blocked}
					/>
				</label>
				<Button
					type="submit"
					disabled={
						blocked ||
						!label.trim() ||
						(!dns.trim() && !ips.trim()) ||
						password.length < 12 ||
						password !== confirmation
					}
				>
					Create certificate authority
				</Button>
			</form>
			<form
				className="space-y-2 border-t pt-3"
				onSubmit={(event) => {
					event.preventDefault();
					const file = importInput.current?.files?.[0];
					const secret = importPassword;
					setImportPassword("");
					if (importInput.current) importInput.current.value = "";
					void execute(async () => {
						if (!file || file.size > 1024 * 1024)
							throw new Error(
								"Choose an encrypted authority backup of at most 1 MiB.",
							);
						const text = await file.text();
						const crypto = await loadDeviceCrypto();
						if (!alive.current) return;
						const authority = await restoreCertificateAuthority(
							scope,
							text,
							secret,
							crypto,
						);
						if (!alive.current) return;
						await addCertificateAuthority(scope, authority);
						await refresh();
						if (alive.current)
							setMessage(
								"The authority's encrypted issuing key is saved in this app. Keep the original root backup offline.",
							);
					});
				}}
			>
				<h4 className="font-medium">Restore an authority backup</h4>
				<label htmlFor={`${id}-backup`} className="block text-sm">
					Encrypted authority backup
					<Input
						id={`${id}-backup`}
						ref={importInput}
						type="file"
						accept=".json"
						required
						disabled={blocked}
					/>
				</label>
				<label htmlFor={`${id}-import-password`} className="block text-sm">
					Backup password
					<Input
						id={`${id}-import-password`}
						type="password"
						autoComplete="off"
						value={importPassword}
						onChange={(event) => setImportPassword(event.target.value)}
						required
						disabled={blocked}
					/>
				</label>
				<Button type="submit" disabled={blocked || !importPassword}>
					Restore certificate authority
				</Button>
			</form>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			{message && <output className="block text-sm">{message}</output>}
		</details>
	);
}

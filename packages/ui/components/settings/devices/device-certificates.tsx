"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	MAX_CERTIFICATE_PEM_BYTES,
	certificateStatus,
	certificateWarnings,
	deleteCertificate,
	putCertificate,
	readCertificates,
	type DeviceCertificate,
} from "../../../lib/device-management/certificates";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import type { DeviceAccountScope } from "../../../lib/device-management/storage";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { DeviceCertificateIssuance } from "./device-certificate-issuance";
import { DeviceCertificateAcme } from "./device-certificate-acme";

const statusLabels = {
	valid: "Valid",
	expiring: "Expires within 7 days",
	expired: "Expired",
	not_yet_valid: "Not valid yet",
};

export function CertificateWarnings({
	certificates,
}: { certificates: { not_after: number }[] }) {
	const { expired, expiring } = certificateWarnings(certificates);
	if (!expired && !expiring) return null;
	return (
		<p
			role="alert"
			className="rounded border border-destructive/40 bg-destructive/5 p-3 text-sm font-medium text-destructive"
		>
			{[
				expired
					? `${expired} expired service certificate${expired === 1 ? "" : "s"}`
					: "",
				expiring
					? `${expiring} service certificate${expiring === 1 ? "" : "s"} expiring within 7 days`
					: "",
			]
				.filter(Boolean)
				.join(". ")}
			. Replace affected certificates to keep services reachable.
		</p>
	);
}

export function DeviceCertificates({
	run,
	disabled = false,
	canManage = false,
	onChanged,
	scope,
	issuance = false,
	acme = false,
	canDelegate = false,
}: {
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
	disabled?: boolean;
	canManage?: boolean;
	onChanged?: (certificates: DeviceCertificate[]) => void;
	scope?: DeviceAccountScope;
	issuance?: boolean;
	acme?: boolean;
	canDelegate?: boolean;
}) {
	const id = useId();
	const [certificates, setCertificates] = useState<DeviceCertificate[]>();
	const [target, setTarget] = useState("");
	const [label, setLabel] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState("");
	const [message, setMessage] = useState("");
	const [deleteTarget, setDeleteTarget] = useState("");
	const [filesReady, setFilesReady] = useState(false);
	const chainInput = useRef<HTMLInputElement>(null);
	const labelInput = useRef<HTMLInputElement>(null);
	const keyInput = useRef<HTMLInputElement>(null);
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	function accept(values: DeviceCertificate[]) {
		if (!alive.current) return;
		setCertificates(values);
		onChanged?.(values);
	}
	async function execute(operation: () => Promise<void>) {
		if (busy || disabled) return;
		setBusy(true);
		setError("");
		setMessage("");
		try {
			await operation();
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error
						? error.message
						: "Certificate operation failed.",
				);
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function refresh() {
		await execute(async () =>
			accept((await run(readCertificates)).certificates),
		);
	}
	async function save() {
		if (!canManage) return;
		const chainFile = chainInput.current?.files?.[0];
		const keyFile = keyInput.current?.files?.[0];
		if (chainInput.current) chainInput.current.value = "";
		if (keyInput.current) keyInput.current.value = "";
		setFilesReady(false);
		await execute(async () => {
			if (!chainFile || !keyFile)
				throw new Error(
					"Choose both the certificate chain and its private key.",
				);
			if (chainFile.size + keyFile.size > MAX_CERTIFICATE_PEM_BYTES)
				throw new Error(
					"The certificate chain and private key must total at most 12 KiB.",
				);
			const previous = certificates?.find(
				(value) => value.certificate_id === target,
			);
			if (target && !previous)
				throw new Error(
					"Refresh certificates before replacing this certificate.",
				);
			let chain = "";
			let key = "";
			try {
				try {
					[chain, key] = await Promise.all([chainFile.text(), keyFile.text()]);
				} catch {
					throw new Error("The selected certificate files could not be read.");
				}
				if (!alive.current) return;
				await run(async (call) => {
					await putCertificate(call, {
						certificateId: previous?.certificate_id ?? crypto.randomUUID(),
						label,
						expectedRevision: previous?.revision ?? 0,
						chain,
						key,
					});
					if (alive.current)
						setMessage(
							previous
								? "Certificate replaced. New TLS connections pick up the replacement automatically. Existing connections stay open."
								: "Certificate imported. Assign it to a placement when deploying or updating its services.",
						);
					accept((await readCertificates(call)).certificates);
				});
				if (alive.current) {
					setTarget("");
					setLabel("");
				}
			} finally {
				// Drop references promptly. JavaScript strings cannot be reliably zeroized.
				chain = "";
				key = "";
			}
		});
	}
	const blocked = busy || disabled;
	return (
		<section
			className="space-y-3 rounded border p-3"
			aria-labelledby={`${id}-title`}
		>
			<div className="flex flex-wrap items-center justify-between gap-2">
				<h3 id={`${id}-title`} className="font-semibold">
					Service certificates
				</h3>
				<Button
					type="button"
					variant="outline"
					size="sm"
					disabled={blocked}
					onClick={() => void refresh()}
				>
					Refresh certificates
				</Button>
			</div>
			<p className="text-sm text-muted-foreground">
				Store multiple certificates on this device and choose one per placement.
				Certificate chains and private keys are sent through this unlocked
				encrypted connection. Private keys cannot be downloaded.
			</p>
			{certificates && <CertificateWarnings certificates={certificates} />}
			{certificates?.length === 0 && (
				<p className="text-sm">No service certificates are installed.</p>
			)}
			{certificates && (
				<ul className="space-y-3">
					{certificates.map((certificate) => {
						const status = certificateStatus(certificate);
						const bindingCount =
							certificate.binding_count ?? certificate.bindings.length;
						return (
							<li
								key={certificate.certificate_id}
								className="space-y-2 rounded border p-3 text-sm"
							>
								<div className="flex flex-wrap justify-between gap-2">
									<strong>{certificate.label}</strong>
									<span
										className={
											status === "valid"
												? "text-muted-foreground"
												: "font-semibold text-destructive"
										}
									>
										{statusLabels[status]}
									</span>
								</div>
								<dl className="grid gap-2 sm:grid-cols-2">
									<div>
										<dt className="text-muted-foreground">
											Expires (including chain)
										</dt>
										<dd className="font-medium">
											{new Date(certificate.not_after * 1000).toLocaleString()}
										</dd>
									</div>
									<div>
										<dt className="text-muted-foreground">Valid from</dt>
										<dd>
											{new Date(certificate.not_before * 1000).toLocaleString()}
										</dd>
									</div>
									<div>
										<dt className="text-muted-foreground">
											Domains and IP addresses
										</dt>
										<dd className="break-all">
											{[
												...certificate.dns_names,
												...certificate.ip_addresses,
											].join(", ") || "None"}
										</dd>
									</div>
									<div>
										<dt className="text-muted-foreground">Issuer</dt>
										<dd className="break-all">{certificate.issuer}</dd>
									</div>
									<div>
										<dt className="text-muted-foreground">Subject</dt>
										<dd className="break-all">{certificate.subject}</dd>
									</div>
									<div>
										<dt className="text-muted-foreground">
											Certificate revision
										</dt>
										<dd>{certificate.revision}</dd>
									</div>
									<div className="sm:col-span-2">
										<dt className="text-muted-foreground">
											SHA-256 fingerprint
										</dt>
										<dd className="break-all font-mono text-xs">
											{certificate.sha256_fingerprint}
										</dd>
									</div>
								</dl>
								<p>
									Used by:{" "}
									{certificate.bindings.length
										? certificate.bindings
												.map(
													(binding) =>
														`${binding.project_id} / ${binding.placement_id} (${binding.service})`,
												)
												.join(", ")
										: bindingCount
											? `${bindingCount} service assignments`
											: "No services"}
								</p>
								{bindingCount > certificate.bindings.length && (
									<p className="text-xs text-muted-foreground">
										Showing {certificate.bindings.length} of {bindingCount}{" "}
										service assignments.
									</p>
								)}
								<div className="flex flex-wrap gap-2">
									<Button
										type="button"
										variant="outline"
										size="sm"
										disabled={blocked || !canManage}
										onClick={() => {
											setTarget(certificate.certificate_id);
											setLabel(certificate.label);
											labelInput.current?.focus();
										}}
									>
										Replace {certificate.label}
									</Button>
									<Button
										type="button"
										variant="outline"
										size="sm"
										disabled={blocked || !canManage || bindingCount > 0}
										onClick={() => setDeleteTarget(certificate.certificate_id)}
									>
										Delete {certificate.label}
									</Button>
								</div>
								{deleteTarget === certificate.certificate_id && (
									<div className="space-y-2 rounded border border-destructive/40 p-2">
										<p>
											Delete {certificate.label} and its private key from this
											device?
										</p>
										<Button
											type="button"
											variant="destructive"
											disabled={blocked || !canManage}
											onClick={() =>
												void execute(() =>
													run(async (call) => {
														await deleteCertificate(call, certificate);
														if (alive.current) {
															setDeleteTarget("");
															setMessage("Certificate deleted.");
														}
														accept((await readCertificates(call)).certificates);
													}),
												)
											}
										>
											Confirm certificate deletion
										</Button>
										<Button
											type="button"
											variant="outline"
											disabled={blocked}
											onClick={() => setDeleteTarget("")}
										>
											Cancel deletion
										</Button>
									</div>
								)}
							</li>
						);
					})}
				</ul>
			)}
			{canManage ? (
				<form
					className="space-y-3 border-t pt-3"
					onSubmit={(event) => {
						event.preventDefault();
						void save();
					}}
				>
					<h4 className="font-medium">
						{target ? "Replace certificate" : "Import certificate"}
					</h4>
					{target && (
						<p className="text-sm text-muted-foreground">
							Replacement keeps existing service assignments. The device checks
							the key, validity and current revision before replacing it.
						</p>
					)}
					<label htmlFor={`${id}-label`} className="block text-sm">
						Certificate label
						<Input
							id={`${id}-label`}
							ref={labelInput}
							value={label}
							maxLength={128}
							required
							disabled={blocked}
							onChange={(event) => setLabel(event.target.value)}
						/>
					</label>
					<label htmlFor={`${id}-chain`} className="block text-sm">
						PEM certificate chain
						<Input
							id={`${id}-chain`}
							ref={chainInput}
							type="file"
							accept=".pem,.crt,.cer"
							required
							disabled={blocked}
							onChange={() =>
								setFilesReady(
									Boolean(
										chainInput.current?.files?.length &&
											keyInput.current?.files?.length,
									),
								)
							}
						/>
					</label>
					<label htmlFor={`${id}-key`} className="block text-sm">
						PEM private key
						<Input
							id={`${id}-key`}
							ref={keyInput}
							type="file"
							accept=".pem,.key"
							required
							disabled={blocked}
							onChange={() =>
								setFilesReady(
									Boolean(
										chainInput.current?.files?.length &&
											keyInput.current?.files?.length,
									),
								)
							}
						/>
					</label>
					<p className="text-xs text-muted-foreground">
						Use an unencrypted key and a chain starting with the service
						certificate, followed by intermediates. Combined limit: 12 KiB.
					</p>
					<Button
						type="submit"
						disabled={blocked || !label.trim() || !filesReady}
					>
						{target
							? "Send replacement to device"
							: "Send certificate to device"}
					</Button>
					{target && (
						<Button
							type="button"
							variant="outline"
							disabled={blocked}
							onClick={() => {
								setTarget("");
								setLabel("");
							}}
						>
							Cancel replacement
						</Button>
					)}
				</form>
			) : (
				<p className="text-sm text-muted-foreground">
					You can inspect service certificates. Device certificate management
					permission is required to import, replace, delete or assign them.
				</p>
			)}
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			{canManage && issuance && scope && (
				<DeviceCertificateIssuance
					scope={scope}
					run={run}
					certificates={certificates ?? []}
					onChanged={accept}
					disabled={blocked}
					canDelegate={canDelegate}
				/>
			)}
			{canManage && acme && canDelegate && (
				<DeviceCertificateAcme
					run={run}
					certificates={certificates ?? []}
					onChanged={accept}
					disabled={blocked}
				/>
			)}
			{canManage && !issuance && (
				<p className="text-xs text-muted-foreground">
					Update this device's standalone binary to generate certificate
					requests and configure renewal.
				</p>
			)}
			{message && <output className="block text-sm">{message}</output>}
		</section>
	);
}

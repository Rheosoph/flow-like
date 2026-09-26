"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	signCertificateRequest,
	type LocalCertificateAuthority,
} from "../../../lib/device-management/certificate-authority";
import {
	certificateNames,
	createCertificateIssuerRequest,
	createCertificateRequest,
	deleteCertificateIssuer,
	deleteCertificateRequest,
	installCertificateIssuer,
	installCertificateRequest,
	readCertificateIssuers,
	readCertificateRequests,
	type CertificateIssuer,
	type CertificateRequest,
} from "../../../lib/device-management/certificate-issuance";
import {
	MAX_CERTIFICATE_PEM_BYTES,
	readCertificates,
	type DeviceCertificate,
} from "../../../lib/device-management/certificates";
import { loadDeviceCrypto } from "../../../lib/device-management/crypto";
import {
	readCertificateAuthorities,
	type DeviceAccountScope,
} from "../../../lib/device-management/storage";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import {
	DeviceCertificateAuthorities,
	downloadCertificateFile,
} from "./device-certificate-authorities";

export function DeviceCertificateIssuance({
	scope,
	run,
	certificates,
	onChanged,
	disabled = false,
	canDelegate = false,
}: {
	scope: DeviceAccountScope;
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
	certificates: DeviceCertificate[];
	onChanged: (certificates: DeviceCertificate[]) => void;
	disabled?: boolean;
	canDelegate?: boolean;
}) {
	const id = useId();
	const alive = useRef(true);
	const working = useRef(false);
	const [requests, setRequests] = useState<CertificateRequest[]>();
	const [issuers, setIssuers] = useState<CertificateIssuer[]>();
	const [authorities, setAuthorities] = useState<LocalCertificateAuthority[]>(
		[],
	);
	const [target, setTarget] = useState("");
	const [label, setLabel] = useState("");
	const [dns, setDns] = useState("");
	const [ips, setIps] = useState("");
	const [leafDays, setLeafDays] = useState("30");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState("");
	const [message, setMessage] = useState("");
	useEffect(() => {
		alive.current = true;
		void readCertificateAuthorities(scope)
			.then((values) => {
				if (alive.current) setAuthorities(values);
			})
			.catch(() => {
				if (alive.current)
					setError(
						"Local certificate authorities could not be loaded. Check this app's encrypted storage.",
					);
			});
		return () => {
			alive.current = false;
		};
	}, [scope]);
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
						: "Certificate issuance failed.",
				);
		} finally {
			working.current = false;
			if (alive.current) setBusy(false);
		}
	}
	async function refresh(call: ManagementCall) {
		const pending = await readCertificateRequests(call);
		const installed = await readCertificates(call);
		const policies = canDelegate
			? await readCertificateIssuers(call)
			: undefined;
		if (alive.current) {
			setRequests(pending);
			onChanged(installed.certificates);
			setIssuers(policies);
		}
	}
	const blocked = busy || disabled;
	return (
		<section
			className="space-y-3 border-t pt-3"
			aria-labelledby={`${id}-title`}
		>
			<div className="flex flex-wrap items-center justify-between gap-2">
				<h4 id={`${id}-title`} className="font-medium">
					Create certificates on this device
				</h4>
				<Button
					type="button"
					variant="outline"
					disabled={blocked}
					onClick={() => void execute(() => run(refresh))}
				>
					Refresh requests and renewal
				</Button>
			</div>
			<p className="text-sm text-muted-foreground">
				The device generates and keeps the private key. Download its certificate
				signing request for your enterprise CA, or sign it here with a locally
				stored organisation authority. Only the signed certificate chain returns
				to the device.
			</p>
			<form
				className="space-y-2"
				onSubmit={(event) => {
					event.preventDefault();
					void execute(() =>
						run(async (call) => {
							const previous = certificates.find(
								(value) => value.certificate_id === target,
							);
							if (target && !previous)
								throw new Error(
									"Refresh certificates before creating a replacement request.",
								);
							await createCertificateRequest(call, {
								certificateId: previous?.certificate_id ?? crypto.randomUUID(),
								label,
								expectedRevision: previous?.revision ?? 0,
								dnsNames: certificateNames(dns),
								ipAddresses: certificateNames(ips),
							});
							await refresh(call);
							if (alive.current)
								setMessage(
									"The device created a private key and signing request. Download the request or sign it below.",
								);
						}),
					);
				}}
			>
				<label htmlFor={`${id}-target`} className="block text-sm">
					Certificate to issue
					<select
						id={`${id}-target`}
						className="mt-1 block w-full rounded border bg-background p-2"
						value={target}
						disabled={blocked}
						onChange={(event) => {
							const value = certificates.find(
								(entry) => entry.certificate_id === event.target.value,
							);
							setTarget(event.target.value);
							setLabel(value?.label ?? "");
							setDns(value?.dns_names.join(", ") ?? "");
							setIps(value?.ip_addresses.join(", ") ?? "");
						}}
					>
						<option value="">New certificate</option>
						{certificates.map((value) => (
							<option key={value.certificate_id} value={value.certificate_id}>
								Replace {value.label}
							</option>
						))}
					</select>
				</label>
				<label htmlFor={`${id}-label`} className="block text-sm">
					Request label
					<Input
						id={`${id}-label`}
						value={label}
						required
						maxLength={128}
						disabled={blocked}
						onChange={(event) => setLabel(event.target.value)}
					/>
				</label>
				<label htmlFor={`${id}-dns`} className="block text-sm">
					Service DNS names
					<Input
						id={`${id}-dns`}
						value={dns}
						placeholder="api.internal.example.com"
						disabled={blocked}
						onChange={(event) => setDns(event.target.value)}
					/>
				</label>
				<label htmlFor={`${id}-ips`} className="block text-sm">
					Service IP addresses
					<Input
						id={`${id}-ips`}
						value={ips}
						placeholder="10.0.0.20"
						disabled={blocked}
						onChange={(event) => setIps(event.target.value)}
					/>
				</label>
				<Button
					type="submit"
					disabled={blocked || !label.trim() || (!dns.trim() && !ips.trim())}
				>
					Generate device key and CSR
				</Button>
			</form>
			{requests?.length === 0 && (
				<p className="text-sm">No pending certificate requests.</p>
			)}
			{requests?.map((request) => (
				<CertificateRequestCard
					key={request.request_id}
					request={request}
					scope={scope}
					authorities={authorities}
					disabled={blocked || (request.purpose === "issuer" && !canDelegate)}
					onDelete={() =>
						execute(() =>
							run(async (call) => {
								await deleteCertificateRequest(call, request);
								await refresh(call);
							}),
						)
					}
					onInstall={(chain) =>
						execute(() =>
							run(async (call) => {
								if (request.purpose === "issuer") {
									if (!canDelegate)
										throw new Error(
											"Only the device owner can delegate renewal.",
										);
									await installCertificateIssuer(call, request, chain);
								} else await installCertificateRequest(call, request, chain);
								await refresh(call);
								if (alive.current)
									setMessage(
										request.purpose === "issuer"
											? "Renewal is delegated for the approved names until the issuing certificate expires. The device can renew during a backend outage."
											: "The signed certificate was installed. Existing assignments use it for new TLS connections.",
									);
							}),
						)
					}
				/>
			))}
			{canDelegate ? (
				<div className="space-y-3 rounded border p-3">
					<h4 className="font-medium">Owner-approved automatic renewal</h4>
					<p className="text-sm text-muted-foreground">
						Delegate an issuing key to this device for one certificate's exact
						names. The management agent keeps the key and renews locally until
						the delegation expires. Project workloads do not receive the issuing
						key. Removing the policy stops future renewals; certificates already
						issued retain their validity.
					</p>
					<label htmlFor={`${id}-leaf-days`} className="block text-sm">
						Renewed service certificate lifetime (days)
						<Input
							id={`${id}-leaf-days`}
							type="number"
							min={1}
							max={397}
							value={leafDays}
							disabled={blocked}
							onChange={(event) => setLeafDays(event.target.value)}
						/>
					</label>
					{certificates.map((certificate) => {
						const issuer = issuers?.find(
							(value) => value.certificate_id === certificate.certificate_id,
						);
						return (
							<div
								key={certificate.certificate_id}
								className="space-y-2 rounded border p-2 text-sm"
							>
								<strong>{certificate.label}</strong>
								<p>
									Approved names:{" "}
									{[...certificate.dns_names, ...certificate.ip_addresses].join(
										", ",
									)}
								</p>
								{issuer && (
									<>
										<p>
											Delegation expires:{" "}
											{new Date(issuer.not_after * 1000).toLocaleString()}. Leaf
											lifetime: {issuer.leaf_lifetime_days} days. Next renewal:{" "}
											{new Date(issuer.next_renewal_at * 1000).toLocaleString()}
											.
										</p>
										{issuer.last_error && (
											<p role="alert" className="text-destructive">
												{issuer.last_error}
											</p>
										)}
										<Button
											type="button"
											variant="outline"
											disabled={blocked}
											onClick={() =>
												void execute(() =>
													run(async (call) => {
														await deleteCertificateIssuer(call, issuer);
														await refresh(call);
														if (alive.current)
															setMessage(
																"Automatic renewal was stopped. The current service certificate remains installed.",
															);
													}),
												)
											}
										>
											Stop renewal for {certificate.label}
										</Button>
									</>
								)}
								<Button
									type="button"
									variant="outline"
									disabled={
										blocked ||
										!issuers ||
										!Number.isInteger(Number(leafDays)) ||
										Number(leafDays) < 1 ||
										Number(leafDays) > 397
									}
									onClick={() =>
										void execute(() =>
											run(async (call) => {
												await createCertificateIssuerRequest(call, {
													certificateId: certificate.certificate_id,
													expectedRevision: certificate.revision,
													dnsNames: certificate.dns_names,
													ipAddresses: certificate.ip_addresses,
													leafLifetimeDays: Number(leafDays),
												});
												await refresh(call);
												if (alive.current)
													setMessage(
														"The device created a constrained issuing request. Sign and approve it above to enable automatic renewal.",
													);
											}),
										)
									}
								>
									Prepare renewal for {certificate.label}
								</Button>
							</div>
						);
					})}
				</div>
			) : (
				<p className="text-xs text-muted-foreground">
					Only the device owner can authorise unattended certificate renewal.
				</p>
			)}
			<DeviceCertificateAuthorities
				scope={scope}
				authorities={authorities}
				onChanged={setAuthorities}
				disabled={blocked}
			/>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			{message && <output className="block text-sm">{message}</output>}
		</section>
	);
}

function CertificateRequestCard({
	request,
	scope,
	authorities,
	disabled,
	onInstall,
	onDelete,
}: {
	request: CertificateRequest;
	scope: DeviceAccountScope;
	authorities: LocalCertificateAuthority[];
	disabled: boolean;
	onInstall: (chain: string) => Promise<void>;
	onDelete: () => Promise<void>;
}) {
	const id = useId();
	const alive = useRef(true);
	const working = useRef(false);
	const chainInput = useRef<HTMLInputElement>(null);
	const [authorityId, setAuthorityId] = useState("");
	const [password, setPassword] = useState("");
	const [days, setDays] = useState(request.purpose === "issuer" ? "180" : "90");
	const [approved, setApproved] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState("");
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	async function execute(operation: () => Promise<void>) {
		if (disabled || working.current) return;
		working.current = true;
		setBusy(true);
		setError("");
		try {
			await operation();
		} catch (failure) {
			if (alive.current)
				setError(
					failure instanceof Error
						? failure.message
						: "The certificate request could not be completed.",
				);
		} finally {
			working.current = false;
			if (alive.current) setBusy(false);
		}
	}
	const blocked =
		disabled || busy || (request.purpose === "issuer" && !approved);
	return (
		<div className="space-y-3 rounded border p-3 text-sm">
			<strong>
				{request.label}:{" "}
				{request.purpose === "issuer"
					? "renewal delegation request"
					: "service certificate request"}
			</strong>
			<p>
				Exact names:{" "}
				{[...request.dns_names, ...request.ip_addresses].join(", ")}
			</p>
			<p>
				Request expires: {new Date(request.expires_at * 1000).toLocaleString()}.
			</p>
			<div className="flex flex-wrap gap-2">
				<Button
					type="button"
					variant="outline"
					disabled={disabled || busy}
					onClick={() =>
						downloadCertificateFile(
							`${request.request_id}.csr`,
							request.csr_pem,
						)
					}
				>
					Download CSR for {request.label}
				</Button>
				<Button
					type="button"
					variant="outline"
					disabled={disabled || busy}
					onClick={() => void execute(onDelete)}
				>
					Discard request for {request.label}
				</Button>
			</div>
			{request.purpose === "issuer" && (
				<label className="flex items-start gap-2">
					<input
						type="checkbox"
						checked={approved}
						disabled={disabled || busy}
						onChange={(event) => setApproved(event.target.checked)}
					/>
					<span>
						I authorise this device to issue service certificates for exactly
						the names above, with a maximum lifetime of{" "}
						{request.leaf_lifetime_days} days, until the signed issuing
						certificate expires.
					</span>
				</label>
			)}
			<form
				className="space-y-2"
				onSubmit={(event) => {
					event.preventDefault();
					const file = chainInput.current?.files?.[0];
					if (chainInput.current) chainInput.current.value = "";
					void execute(async () => {
						if (request.purpose === "issuer" && !approved) return;
						if (!file || file.size > MAX_CERTIFICATE_PEM_BYTES)
							throw new Error(
								"Choose a signed certificate chain of at most 12 KiB.",
							);
						const chain = await file.text();
						if (alive.current) await onInstall(chain);
					});
				}}
			>
				<label htmlFor={`${id}-chain`} className="block">
					{request.purpose === "issuer"
						? "Flow-Like constrained issuing chain"
						: "Enterprise-signed PEM chain"}
					<Input
						id={`${id}-chain`}
						ref={chainInput}
						type="file"
						accept=".pem,.crt,.cer"
						required
						disabled={blocked}
					/>
				</label>
				<Button type="submit" disabled={blocked}>
					Install signed {request.purpose === "issuer" ? "issuing" : "service"}{" "}
					certificate
				</Button>
			</form>
			{authorities.length > 0 && (
				<form
					className="space-y-2 border-t pt-2"
					onSubmit={(event) => {
						event.preventDefault();
						const secret = password;
						setPassword("");
						void execute(async () => {
							if (request.purpose === "issuer" && !approved) return;
							const authority = authorities.find(
								(value) => value.public_bundle.authority_id === authorityId,
							);
							if (!authority)
								throw new Error("Choose a local certificate authority.");
							const crypto = await loadDeviceCrypto();
							if (!alive.current) return;
							const result = await signCertificateRequest(
								scope,
								authority,
								secret,
								{
									csr_pem: request.csr_pem,
									dns_names: request.dns_names,
									ip_addresses: request.ip_addresses,
									validity_days: Number(days),
								},
								crypto,
								request.purpose,
							);
							if (alive.current) await onInstall(result.certificate_chain_pem);
						});
					}}
				>
					<label htmlFor={`${id}-authority`} className="block">
						Sign with organisation authority
						<select
							id={`${id}-authority`}
							className="mt-1 block w-full rounded border bg-background p-2"
							required
							disabled={blocked}
							value={authorityId}
							onChange={(event) => setAuthorityId(event.target.value)}
						>
							<option value="">Choose authority</option>
							{authorities.map((value) => (
								<option
									key={value.public_bundle.authority_id}
									value={value.public_bundle.authority_id}
								>
									{value.public_bundle.label}
								</option>
							))}
						</select>
					</label>
					<label htmlFor={`${id}-days`} className="block">
						Certificate validity (days)
						<Input
							id={`${id}-days`}
							type="number"
							min={1}
							max={request.purpose === "issuer" ? 365 : 397}
							value={days}
							onChange={(event) => setDays(event.target.value)}
							required
							disabled={blocked}
						/>
					</label>
					<label htmlFor={`${id}-password`} className="block">
						Authority password
						<Input
							id={`${id}-password`}
							type="password"
							autoComplete="off"
							value={password}
							onChange={(event) => setPassword(event.target.value)}
							required
							disabled={blocked}
						/>
					</label>
					<Button type="submit" disabled={blocked || !authorityId || !password}>
						Sign and install{" "}
						{request.purpose === "issuer"
							? "renewal delegation"
							: "service certificate"}
					</Button>
				</form>
			)}
			{error && (
				<p role="alert" className="text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}

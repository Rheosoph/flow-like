"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	configureAcmeCertificate,
	deleteAcmeCertificate,
	readAcmeCertificates,
	type AcmeCertificate,
} from "../../../lib/device-management/certificate-acme";
import { certificateNames } from "../../../lib/device-management/certificate-issuance";
import {
	readCertificates,
	type DeviceCertificate,
} from "../../../lib/device-management/certificates";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export function DeviceCertificateAcme({
	run,
	certificates,
	onChanged,
	disabled = false,
}: {
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
	certificates: DeviceCertificate[];
	onChanged: (certificates: DeviceCertificate[]) => void;
	disabled?: boolean;
}) {
	const id = useId();
	const alive = useRef(true);
	const working = useRef(false);
	const [values, setValues] = useState<AcmeCertificate[]>();
	const [target, setTarget] = useState("");
	const [label, setLabel] = useState("");
	const [dns, setDns] = useState("");
	const [environment, setEnvironment] = useState<
		AcmeCertificate["environment"]
	>("lets_encrypt_staging");
	const [bind, setBind] = useState("0.0.0.0:80");
	const [agreed, setAgreed] = useState(false);
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
						: "Public certificate operation failed.",
				);
		} finally {
			working.current = false;
			if (alive.current) setBusy(false);
		}
	}
	async function refresh(call: ManagementCall) {
		const policies = await readAcmeCertificates(call);
		const installed = await readCertificates(call);
		if (alive.current) {
			setValues(policies);
			onChanged(installed.certificates);
		}
	}
	const blocked = busy || disabled;
	const options = new Map([
		...certificates.map(
			(value) => [value.certificate_id, value.label] as const,
		),
		...(values?.map((value) => [value.certificate_id, value.label] as const) ??
			[]),
	]);
	return (
		<section
			className="space-y-3 border-t pt-3"
			aria-labelledby={`${id}-title`}
		>
			<div className="flex flex-wrap items-center justify-between gap-2">
				<h4 id={`${id}-title`} className="font-medium">
					Automatic public certificates
				</h4>
				<Button
					type="button"
					variant="outline"
					disabled={blocked}
					onClick={() => void execute(() => run(refresh))}
				>
					Refresh public certificate settings
				</Button>
			</div>
			<p className="text-sm text-muted-foreground">
				Let’s Encrypt can issue and renew certificates for public domain names
				without installing a private root. Public DNS must point to this device
				or its proxy. Inbound port 80 must reach the challenge listener, or your
				reverse proxy must forward <code>/.well-known/acme-challenge/</code> to
				the listener address below. The device needs outbound internet access
				for issuance and renewal.
			</p>
			{values?.map((value) => (
				<div
					key={value.certificate_id}
					className="space-y-2 rounded border p-3 text-sm"
				>
					<strong>{value.label}</strong>
					<p>
						{value.dns_names.join(", ")} ·{" "}
						{value.environment === "lets_encrypt_staging"
							? "Staging: certificates are not browser-trusted"
							: "Production"}
					</p>
					<p>
						Challenge listener: {value.http_bind}. Next attempt:{" "}
						{new Date(value.next_attempt_at * 1000).toLocaleString()}.
					</p>
					{value.last_renewed_at && (
						<p>
							Last issued:{" "}
							{new Date(value.last_renewed_at * 1000).toLocaleString()}
						</p>
					)}
					{value.last_error && (
						<p role="alert" className="text-destructive">
							{value.last_error}
						</p>
					)}
					<Button
						type="button"
						variant="outline"
						disabled={blocked}
						onClick={() =>
							void execute(() =>
								run(async (call) => {
									await deleteAcmeCertificate(call, value);
									await refresh(call);
									if (alive.current)
										setMessage(
											"Public certificate renewal stopped. The installed service certificate remains valid until its expiry.",
										);
								}),
							)
						}
					>
						Stop public renewal for {value.label}
					</Button>
				</div>
			))}
			<form
				className="space-y-2"
				onSubmit={(event) => {
					event.preventDefault();
					void execute(() =>
						run(async (call) => {
							const previous = certificates.find(
								(value) => value.certificate_id === target,
							);
							const policy = values?.find(
								(value) => value.certificate_id === target,
							);
							if (target && !previous && !policy)
								throw new Error(
									"Refresh public certificate settings before changing this certificate.",
								);
							await configureAcmeCertificate(call, {
								certificateId: target || crypto.randomUUID(),
								label,
								expectedRevision: policy?.revision ?? 0,
								expectedCertificateRevision: previous?.revision ?? 0,
								dnsNames: certificateNames(dns),
								environment,
								httpBind: bind,
								termsAgreed: agreed,
							});
							await refresh(call);
							if (alive.current) {
								setAgreed(false);
								setMessage(
									"Public certificate issuance is scheduled. Refresh to see validation results and renewal status.",
								);
							}
						}),
					);
				}}
			>
				<label htmlFor={`${id}-target`} className="block text-sm">
					Public certificate
					<select
						id={`${id}-target`}
						className="mt-1 block w-full rounded border bg-background p-2"
						value={target}
						disabled={blocked}
						onChange={(event) => {
							setTarget(event.target.value);
							const policy = values?.find(
								(value) => value.certificate_id === event.target.value,
							);
							const installed = certificates.find(
								(value) => value.certificate_id === event.target.value,
							);
							setLabel(policy?.label ?? installed?.label ?? "");
							setDns(
								(policy?.dns_names ?? installed?.dns_names ?? []).join(", "),
							);
							setEnvironment(policy?.environment ?? "lets_encrypt_staging");
							setBind(policy?.http_bind ?? "0.0.0.0:80");
							setAgreed(false);
						}}
					>
						<option value="">New public certificate</option>
						{[...options].map(([key, label]) => (
							<option key={key} value={key}>
								{label}
							</option>
						))}
					</select>
				</label>
				<label htmlFor={`${id}-label`} className="block text-sm">
					Public certificate label
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
					Public DNS names
					<Input
						id={`${id}-dns`}
						value={dns}
						required
						disabled={blocked}
						placeholder="api.example.com"
						onChange={(event) => setDns(event.target.value)}
					/>
				</label>
				<label htmlFor={`${id}-environment`} className="block text-sm">
					Certificate authority environment
					<select
						id={`${id}-environment`}
						className="mt-1 block w-full rounded border bg-background p-2"
						value={environment}
						disabled={blocked}
						onChange={(event) => {
							setEnvironment(
								event.target.value as AcmeCertificate["environment"],
							);
							setAgreed(false);
						}}
					>
						<option value="lets_encrypt_staging">
							Let’s Encrypt staging (test certificates)
						</option>
						<option value="lets_encrypt_production">
							Let’s Encrypt production
						</option>
					</select>
				</label>
				<label htmlFor={`${id}-bind`} className="block text-sm">
					Device challenge listener
					<Input
						id={`${id}-bind`}
						value={bind}
						required
						disabled={blocked}
						onChange={(event) => setBind(event.target.value)}
					/>
				</label>
				<label className="flex items-start gap-2 text-sm">
					<input
						type="checkbox"
						checked={agreed}
						disabled={blocked}
						onChange={(event) => setAgreed(event.target.checked)}
					/>
					<span>
						I accept the{" "}
						<a
							href="https://letsencrypt.org/repository/"
							target="_blank"
							rel="noreferrer"
							className="underline"
						>
							Let’s Encrypt subscriber agreement
						</a>{" "}
						and authorise this device to request and renew certificates for
						these names. Production certificates publish the names in public
						certificate transparency logs.
					</span>
				</label>
				<Button
					type="submit"
					disabled={
						blocked ||
						!values ||
						!agreed ||
						!label.trim() ||
						!dns.trim() ||
						!bind.trim()
					}
				>
					Enable public certificate issuance
				</Button>
			</form>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			{message && <output className="block text-sm">{message}</output>}
		</section>
	);
}

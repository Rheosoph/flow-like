"use client";

import { useTranslation } from "@flow-like/locales";
import { ShieldCheck } from "lucide-react";
import { useId, useState } from "react";
import { signCertificateRequest } from "../../../../../lib/device-management/certificate-authority";
import {
	type CertificateRequest,
	installCertificateIssuer,
	installCertificateRequest,
} from "../../../../../lib/device-management/certificate-issuance";
import { readCertificateAuthorities } from "../../../../../lib/device-management/storage";
import { enumLabel } from "../../copy/enum-labels";
import { useAreaTime } from "../../primitives/area-context";
import { ConsequencePreview } from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import {
	CheckField,
	ChoiceCards,
	Field,
	InputWithUnit,
	SecretInput,
} from "../../primitives/form-fields";
import { GateNotice } from "../../primitives/gate-notice";
import { InlineResult } from "../../primitives/inline-result";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { useRouteLink } from "../../routing/use-devices-route";
import {
	useDeviceAction,
	useDeviceWorkspace,
	useLocalSummary,
} from "../../workspace";
import {
	ActionResults,
	LINK,
	Names,
	TickFirst,
	gateView,
	listOf,
	useAlive,
	useDay,
	withDeviceReason,
} from "./certificate-parts";
import { requestsResultKey } from "./signing-requests";
import {
	type DeviceCertificates,
	certificateNames,
} from "./use-device-certificates";

const LIMITS = {
	service: { max: 397, initial: 90 },
	issuer: { max: 365, initial: 180 },
} as const;

/**
 * S33/S34: sign a device's request with an organisation authority kept on this
 * computer and install the result. Only the signed chain goes to the device;
 * the authority password is dropped as soon as it was used.
 */
export function SignWithAuthoritySheet({
	certs,
	request,
	onClose,
}: Readonly<{
	certs: DeviceCertificates;
	request: CertificateRequest;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const day = useDay();
	const alive = useAlive();
	const link = useRouteLink();
	const workspace = useDeviceWorkspace();
	const actions = useDeviceAction();
	const authorities = useLocalSummary().authorities;
	const issuer = request.purpose === "issuer";
	const limits = LIMITS[request.purpose];
	const usable = authorities.filter(
		(authority) => authority.issuingNotAfter > certs.now,
	);
	const [authorityId, setAuthorityId] = useState<string | undefined>(
		usable.length === 1 ? usable[0]?.authorityId : undefined,
	);
	const [days, setDays] = useState(String(limits.initial));
	const [password, setPassword] = useState("");
	const [approved, setApproved] = useState(false);
	const [signing, setSigning] = useState(false);
	const [signFailed, setSignFailed] = useState(false);
	const [touched, setTouched] = useState(false);
	const [ran, setRan] = useState(false);
	const resultKey = requestsResultKey(certs.deviceId);
	const names = certificateNames(request);
	const action = issuer ? "renewal_delegation" : "csr_install";
	const view =
		gateView(t, time, certs.gate(action)) ??
		gateView(
			t,
			time,
			certs.gate("sign_with_org_ca", {
				authorityPresent: authorities.length > 0,
			}),
		);
	const validity = Number(days);
	const daysError =
		!Number.isInteger(validity) || validity < 1 || validity > limits.max;
	const target = certs.rows.find((row) => row.id === request.certificate_id);
	const users = target?.uses ?? 0;
	const blocked = !!view || (issuer && !approved);

	const submit = async () => {
		setTouched(true);
		const secret = password;
		if (blocked || daysError || !authorityId || !secret) return;
		setPassword("");
		setSignFailed(false);
		setSigning(true);
		let chain = "";
		try {
			const { scope } = workspace.deps;
			const stored = await readCertificateAuthorities(scope);
			const authority = stored.find(
				(entry) => entry.public_bundle.authority_id === authorityId,
			);
			if (!authority) throw new Error("authority");
			const signed = await signCertificateRequest(
				scope,
				authority,
				secret,
				{
					csr_pem: request.csr_pem,
					dns_names: request.dns_names,
					ip_addresses: request.ip_addresses,
					validity_days: validity,
				},
				await workspace.deps.crypto(),
				request.purpose,
			);
			chain = signed.certificate_chain_pem;
		} catch {
			// The cause may quote key material: one fixed sentence instead.
			if (alive()) {
				setSignFailed(true);
				setSigning(false);
			}
			return;
		}
		if (!alive()) return;
		setSigning(false);
		setRan(true);
		const outcome = await actions.run({
			action,
			deviceId: certs.deviceId,
			label: issuer
				? t(
						"deviceCertificates.sign.labelIssuer",
						"Sign and install the renewal authority for {{label}}",
						{ label: request.label },
					)
				: t(
						"deviceCertificates.sign.label",
						"Sign and install the certificate for {{label}}",
						{ label: request.label },
					),
			resultKey,
			activity: {
				kind: "signing_request",
				deviceName: certs.name,
				href: {
					screen: "device",
					deviceId: certs.deviceId,
					tab: "certificates",
				},
			},
			call: (context) =>
				withDeviceReason(context, async (call) => {
					if (issuer) await installCertificateIssuer(call, request, chain);
					else await installCertificateRequest(call, request, chain);
				}),
		});
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
			icon={ShieldCheck}
			title={
				issuer
					? t(
							"deviceCertificates.sign.titleIssuer",
							"Sign the renewal authority for {{label}}",
							{ label: request.label },
						)
					: t(
							"deviceCertificates.sign.title",
							"Sign the certificate for {{label}}",
							{ label: request.label },
						)
			}
			sub={t(
				"deviceCertificates.sign.subtitle",
				"{{device}} · with an organisation authority on this computer",
				{ device: certs.name },
			)}
			footNote={!view && issuer && !approved ? <TickFirst /> : undefined}
			foot={
				<>
					<DvButton onClick={onClose}>
						{t("deviceCertificates.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={signing || actions.pending(resultKey)}
						aria-disabled={blocked || undefined}
						onClick={() => void submit()}
					>
						{t("deviceCertificates.sign.submit", "Sign and install")}
					</DvButton>
				</>
			}
		>
			{view ? (
				<GateNotice
					kind={view.gate.kind}
					title={view.gate.reason}
					actions={
						authorities.length ? undefined : (
							<a
								className={LINK}
								{...link({
									screen: "certificates",
									tab: "authorities",
									action: "create-authority",
								})}
							>
								{t(
									"deviceCertificates.sign.createAuthority",
									"Create an organisation authority",
								)}
							</a>
						)
					}
				/>
			) : null}
			<KeyValueList>
				<KvRow label={t("deviceCertificates.install.request", "Request")}>
					<b className="font-semibold">{request.label}</b>
					{" · "}
					{enumLabel(t, "csrPurpose", request.purpose)}
				</KvRow>
				<KvRow label={t("deviceCertificates.install.names", "Exact names")}>
					<Names names={names} />
				</KvRow>
			</KeyValueList>
			{authorities.length ? (
				<ChoiceCards
					id={`${id}-authority`}
					legend={t(
						"deviceCertificates.sign.authority",
						"Organisation authority",
					)}
					value={authorityId}
					onValueChange={setAuthorityId}
					options={authorities.map((authority) => {
						const expired = authority.issuingNotAfter <= certs.now;
						return {
							value: authority.authorityId,
							title: authority.label,
							disabled: expired,
							hint: expired
								? t(
										"deviceCertificates.sign.authorityExpired",
										"Its signing key expired {{date}}. Renew it under Certificates › Organisation authorities.",
										{ date: day(authority.issuingNotAfter) },
									)
								: t(
										"deviceCertificates.sign.authorityValid",
										"Signs until {{date}}",
										{ date: day(authority.issuingNotAfter) },
									),
						};
					})}
				/>
			) : null}
			{touched && authorities.length > 0 && !authorityId ? (
				<p className="text-xs text-critical">
					{t("deviceCertificates.sign.pickAuthority", "Pick an authority.")}
				</p>
			) : null}
			<Field
				id={`${id}-days`}
				label={
					issuer
						? t(
								"deviceCertificates.sign.daysIssuer",
								"Renewal authority valid for",
							)
						: t("deviceCertificates.sign.days", "Certificate valid for")
				}
				hint={t(
					"deviceCertificates.sign.daysHint",
					"1 to {{max, number}} days, and never longer than the authority's own signing key.",
					{ max: limits.max },
				)}
				error={
					touched && daysError
						? t(
								"deviceCertificates.sign.daysError",
								"Enter a whole number from 1 to {{max, number}}.",
								{ max: limits.max },
							)
						: undefined
				}
			>
				<InputWithUnit
					className="max-w-40"
					numeric
					inputMode="numeric"
					value={days}
					unit={t("deviceCertificates.days", "days")}
					onChange={(event) => setDays(event.target.value)}
				/>
			</Field>
			<Field
				id={`${id}-password`}
				label={t("deviceCertificates.sign.password", "Authority password")}
				error={
					touched && !password && !signFailed
						? t(
								"deviceCertificates.sign.passwordEmpty",
								"Enter the authority's password.",
							)
						: undefined
				}
			>
				<SecretInput
					value={password}
					onValueChange={setPassword}
					autoComplete="off"
				/>
			</Field>
			{signFailed ? (
				<InlineResult tone="critical" onDismiss={() => setSignFailed(false)}>
					{t(
						"deviceCertificates.sign.failed",
						"The request couldn't be signed. Check the authority password, the names this authority may sign, its expiry and the validity you asked for. Nothing was sent to the device.",
					)}
				</InlineResult>
			) : null}
			<ConsequencePreview
				rows={
					issuer
						? {
								what: t(
									"deviceCertificates.sign.issuerWhat",
									"Your authority signs a renewal authority for exactly {{names}}. The device keeps it and renews {{label}} by itself, {{count, number}} days at a time.",
									{
										names: listOf(time.locale, names),
										label: request.label,
										count: request.leaf_lifetime_days ?? 0,
									},
								),
								who: t(
									"deviceCertificates.install.issuerWho",
									"Nobody notices. {{label}} keeps working while it renews.",
									{ label: request.label },
								),
								stays: t(
									"deviceCertificates.sign.issuerStays",
									"The certificate ID, its names and the services that use it. The authority's own key stays on this computer.",
								),
								when: t(
									"deviceCertificates.install.issuerWhen",
									"Immediately after the check. Renewal works even while the hub is unreachable, until the authority expires.",
								),
								undo: {
									reversible: true,
									text: t(
										"deviceCertificates.install.issuerUndo",
										"Stop renewal at any time. The certificate then expires on its date.",
									),
								},
							}
						: {
								what: t(
									"deviceCertificates.sign.what",
									"Your authority signs a certificate for exactly {{names}} and the device installs it as {{label}}. Only the signed chain is sent.",
									{ names: listOf(time.locale, names), label: request.label },
								),
								who: users
									? t(
											"deviceCertificates.install.whoUsed",
											"The services that use {{label}} pick up the new certificate on their next connection. No restart.",
											{ label: request.label },
										)
									: t(
											"deviceCertificates.install.who",
											"Nobody: nothing uses it until you pick it for a service's web endpoint.",
										),
								when: t(
									"deviceCertificates.install.when",
									"Immediately after the check.",
								),
								undo: target
									? {
											reversible: false,
											text: t(
												"deviceCertificates.install.replaceUndo",
												"Replace it again. The request is used up.",
											),
										}
									: {
											reversible: true,
											text: t(
												"deviceCertificates.install.undo",
												"Delete the certificate. The request is used up.",
											),
										},
							}
				}
			/>
			{issuer ? (
				<CheckField
					id={`${id}-approve`}
					checked={approved}
					onCheckedChange={setApproved}
				>
					{t(
						"deviceCertificates.install.approve",
						"I authorise this device to issue certificates for exactly the names above, valid for at most {{count, number}} days each, until the signed authority expires.",
						{ count: request.leaf_lifetime_days ?? 0 },
					)}
				</CheckField>
			) : null}
			{ran ? <ActionResults scopeKey={resultKey} /> : null}
		</DvSheet>
	);
}

"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import { apiErrorMessage } from "../../../lib/api-error";
import { getApiOrigin } from "../../../lib/api-url";
import {
	type BillingGrant,
	type CreateResourceGrant,
	type DeviceResources,
	type OfflinePlacementIdentity,
	type ResourceGrant,
	approveDeviceBilling,
	createDeviceResourceGrant,
	createResourceRequest,
	eurosToMicros,
	formatEuroMicros,
	importResourcePlacement,
	isActiveGrant,
	loadDeviceResources,
	localExpiryValue,
	parseExpiry,
	publicResourceBinding,
	revokeDeviceGrant,
} from "../../../lib/device-resources";
import type { DeviceStatus } from "../../../lib/devices";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";

type Action =
	| { kind: "create"; request: CreateResourceGrant }
	| {
			kind: "billing";
			grantId: string;
			request: { limit_micros: number; expires_at: number };
	  }
	| { kind: "resource" | "revoke-billing"; id: string };
type Revocation = { kind: "resource" | "revoke-billing"; id: string };

export function DeviceResourcesDialog({
	device,
	deviceConfirmed,
	profile,
	account,
	issuer,
	onClose,
}: {
	device: DeviceStatus;
	deviceConfirmed: boolean;
	profile: IProfile;
	account: string;
	issuer: string;
	onClose: () => void;
}) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const client = useQueryClient();
	const [creating, setCreating] = useState(false);
	const [billingFor, setBillingFor] = useState<string | null>(null);
	const [revocation, setRevocation] = useState<Revocation | null>(null);
	const [recoveredRevocation, setRecoveredRevocation] = useState(false);
	const [now, setNow] = useState(() => Date.now() / 1000);
	const submitting = useRef(false);
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		const timer = setInterval(() => setNow(Date.now() / 1000), 1000);
		return () => {
			mounted.current = false;
			clearInterval(timer);
		};
	}, []);
	const queryKey = [
		"device-resources",
		getApiOrigin(profile),
		issuer,
		account,
		profile.id,
		device.device_id,
	];
	const resources = useQuery({
		queryKey,
		queryFn: () =>
			loadDeviceResources(backend.apiState, profile, device.device_id),
		gcTime: 0,
		staleTime: 0,
		retry: false,
		meta: { persist: false },
		refetchInterval: (query) => (query.state.error ? false : 30_000),
		refetchIntervalInBackground: false,
	});
	const mutation = useMutation({
		mutationFn: async (action: Action) => {
			if (!mounted.current) throw new Error("The selected account changed.");
			if (action.kind === "create")
				return createDeviceResourceGrant(
					backend.apiState,
					profile,
					device.device_id,
					action.request,
				);
			if (action.kind === "billing")
				return approveDeviceBilling(
					backend.apiState,
					profile,
					device.device_id,
					action.grantId,
					action.request,
				);
			await revokeDeviceGrant(
				backend.apiState,
				profile,
				device.device_id,
				action.kind === "resource" ? "resource" : "billing",
				action.id,
			);
			return null;
		},
		onSuccess: async (result, action) => {
			if (!mounted.current) return;
			await client.cancelQueries({ queryKey, exact: true });
			if (!mounted.current) return;
			client.setQueryData<DeviceResources>(queryKey, (previous) => {
				if (!previous) return previous;
				if (action.kind === "create")
					return {
						...previous,
						grants: [
							result as ResourceGrant,
							...previous.grants.filter(
								(row) => row.grant_id !== (result as ResourceGrant).grant_id,
							),
						],
					};
				if (action.kind === "billing")
					return {
						...previous,
						billing: [
							result as BillingGrant,
							...previous.billing.filter(
								(row) =>
									row.billing_grant_id !==
									(result as BillingGrant).billing_grant_id,
							),
						],
					};
				if (action.kind === "resource")
					return {
						...previous,
						grants: previous.grants.map((row) =>
							row.grant_id === action.id ? { ...row, status: "revoked" } : row,
						),
					};
				return {
					...previous,
					billing: previous.billing.map((row) =>
						row.billing_grant_id === action.id
							? { ...row, status: "revoked" }
							: row,
					),
				};
			});
			if (mounted.current) {
				setCreating(false);
				setBillingFor(null);
				setRevocation(null);
			}
			await client.invalidateQueries({ queryKey, exact: true });
		},
		onError: async (_error, action) => {
			// A response can be lost after consent commits. Re-read before retrying.
			if (!mounted.current) return;
			const refreshed = await resources.refetch();
			if (refreshed.isError || !mounted.current) return;
			const revoked =
				action.kind === "resource"
					? refreshed.data?.grants.some(
							(row) => row.grant_id === action.id && row.status === "revoked",
						)
					: action.kind === "revoke-billing" &&
						refreshed.data?.billing.some(
							(row) =>
								row.billing_grant_id === action.id && row.status === "revoked",
						);
			if (revoked) {
				setRevocation(null);
				setRecoveredRevocation(true);
			}
		},
		onSettled: () => {
			submitting.current = false;
		},
	});
	const fresh = !!resources.data && !resources.isError && !resources.isFetching;
	const busy = mutation.isPending || !fresh;
	const canApprove = device.status === "active" && deviceConfirmed && !busy;
	const submit = (action: Action) => {
		if (
			submitting.current ||
			busy ||
			((action.kind === "create" || action.kind === "billing") && !canApprove)
		)
			return;
		submitting.current = true;
		setRecoveredRevocation(false);
		mutation.mutate(action);
	};
	const data = resources.data;
	const validations =
		data?.instances.filter(
			(instance) => instance.purpose === "rollout_validation",
		) ?? [];
	const date = (seconds: number) => new Date(seconds * 1000).toLocaleString();

	return (
		<Dialog
			open
			onOpenChange={(open) => {
				if (!open && !mutation.isPending) onClose();
			}}
		>
			<DialogContent
				className="sm:max-w-3xl"
				showCloseButton={!mutation.isPending}
			>
				<DialogHeader>
					<DialogTitle>
						{t("deviceResourcesTitle", "Hosted resources")} · {device.name}
					</DialogTitle>
					<DialogDescription>
						{t(
							"deviceResourcesDescription",
							"Approve model access for an existing offline placement, then approve a personal spending allowance. Apply the resulting public binding on the device yourself.",
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="flex flex-wrap gap-2">
					<Button
						variant="outline"
						disabled={resources.isFetching || mutation.isPending}
						onClick={() => void resources.refetch()}
					>
						{t("refresh", "Refresh")}
					</Button>
					<Button
						disabled={!canApprove}
						onClick={() => {
							mutation.reset();
							setCreating(true);
						}}
					>
						{t("deviceResourcesCreate", "Authorize a placement")}
					</Button>
				</div>
				{device.status !== "active" && (
					<output className="text-sm">
						{t(
							"deviceResourcesRevoked",
							"This device is revoked. New approvals and configuration exports are disabled. Existing approvals can still be revoked.",
						)}
					</output>
				)}
				{!deviceConfirmed && (
					<output className="text-sm">
						{t(
							"deviceResourcesUnconfirmed",
							"Device registration is being checked or could not be refreshed. New approvals and configuration exports are disabled.",
						)}
					</output>
				)}
				{resources.isPending && (
					<output>
						{t("deviceResourcesLoading", "Loading resource approvals…")}
					</output>
				)}
				{resources.isError && (
					<p role="alert" className="text-destructive">
						{apiErrorMessage(
							resources.error,
							t(
								"deviceResourcesLoadError",
								"Resource approvals could not be refreshed. Retry before approving or exporting configuration.",
							),
						)}
					</p>
				)}
				{mutation.isError && !recoveredRevocation && (
					<div className="space-y-1">
						<p role="alert" className="text-destructive">
							{apiErrorMessage(
								mutation.error,
								t(
									"deviceResourcesMutationError",
									"The request was not confirmed.",
								),
							)}
						</p>
						<p className="text-sm">
							{t(
								"deviceResourcesRecovery",
								"The request may already have taken effect. Review the refreshed approvals below before retrying. Configuration exports use the recorded approvals shown below.",
							)}
						</p>
					</div>
				)}
				{recoveredRevocation && (
					<output>
						{t(
							"deviceResourcesRevocationRecovered",
							"The refreshed hub record confirms that this approval is revoked.",
						)}
					</output>
				)}
				{creating && (
					<ResourceForm
						disabled={!canApprove}
						grants={data?.grants ?? []}
						onCancel={() => setCreating(false)}
						onSubmit={(request) => submit({ kind: "create", request })}
					/>
				)}
				{data && (
					<>
						{data.grants.length === 0 && (
							<p>
								{t(
									"deviceResourcesEmpty",
									"No resource approvals for this device.",
								)}
							</p>
						)}
						{[
							data.grants.length,
							data.billing.length,
							data.instances.length,
						].some((count) => count === 1000) && (
							<output>
								{t(
									"deviceResourcesTruncated",
									"The hub returned its limit of 1,000 records. This snapshot may omit older records.",
								)}
							</output>
						)}
						<ul className="space-y-4">
							{data.grants.map((grant) => {
								const billing = data.billing.filter(
									(row) => row.grant_id === grant.grant_id,
								);
								const currentBilling = billing.find((row) =>
									isActiveGrant(row, now),
								);
								let binding: string | undefined;
								if (canApprove && (currentBilling || grant.online_access)) {
									try {
										binding = publicResourceBinding(
											grant,
											currentBilling,
											device.device_id,
											account,
											now,
											device.owner_id,
										);
									} catch {}
								}
								return (
									<li
										key={grant.grant_id}
										className="space-y-4 rounded-lg border p-4"
									>
										<div className="flex flex-wrap items-center justify-between gap-2">
											<h3 className="break-all font-semibold">
												{grant.placement_id}
											</h3>
											<Badge variant="outline">
												{grant.status === "revoked"
													? t(
															"deviceResourceRevoked",
															"Resource access revoked",
														)
													: isActiveGrant(grant, now)
														? t(
																"deviceResourceApproved",
																"Resource access approved",
															)
														: t(
																"deviceResourceExpired",
																"Resource approval expired",
															)}
											</Badge>
										</div>
										<dl className="grid gap-2 text-sm sm:grid-cols-2">
											<div>
												<dt className="text-muted-foreground">
													{t("deviceResourceProject", "Project ID")}
												</dt>
												<dd className="break-all font-mono">
													{grant.project_id}
												</dd>
											</div>
											<div>
												<dt className="text-muted-foreground">
													{t("deviceResourceDeployment", "Deployment ID")}
												</dt>
												<dd className="break-all font-mono">
													{grant.deployment_id}
												</dd>
											</div>
											<div>
												<dt className="text-muted-foreground">
													{t("deviceResourceCap", "Instance limit")}
												</dt>
												<dd>{grant.max_instances}</dd>
											</div>
											<div>
												<dt className="text-muted-foreground">
													{t("deviceResourceExpiry", "Resource access expires")}
												</dt>
												<dd>{date(grant.expires_at)}</dd>
											</div>
										</dl>
										<p className="break-all text-sm">
											{t("deviceResourceModels", "Allowed model Bit IDs")}:{" "}
											<span className="font-mono">
												{grant.model_ids.join(", ") || "No hosted models"}
												{grant.online_access && (
													<p>
														Online files:{" "}
														{grant.online_access === "read_write"
															? "read and write"
															: "read only"}
													</p>
												)}
											</span>
										</p>
										{grant.status === "active" && (
											<Button
												variant="outline"
												disabled={busy}
												onClick={() => {
													mutation.reset();
													setRevocation({
														kind: "resource",
														id: grant.grant_id,
													});
												}}
											>
												{t("deviceResourceRevoke", "Revoke resource access")}
											</Button>
										)}
										{billing.map((row) => (
											<div
												key={row.billing_grant_id}
												className="space-y-2 rounded-md bg-muted/40 p-3 text-sm"
											>
												<p className="font-medium">
													{t(
														"deviceBillingPersonal",
														"Personal billing allowance",
													)}{" "}
													·{" "}
													{row.status === "revoked"
														? t("deviceBillingRevoked", "Revoked")
														: isActiveGrant(row, now)
															? t("deviceBillingApproved", "Approved")
															: t("deviceBillingExpired", "Expired")}
												</p>
												<p>
													{t("deviceBillingPayer", "Payer")}:{" "}
													<span className="break-all font-mono">
														{row.payer_id}
													</span>
												</p>
												<dl className="grid grid-cols-3 gap-3">
													<div>
														<dt>{t("deviceBillingLimit", "Allowance")}</dt>
														<dd>{formatEuroMicros(row.limit_micros)}</dd>
													</div>
													<div>
														<dt>{t("deviceBillingUsed", "Used")}</dt>
														<dd>{formatEuroMicros(row.used_micros)}</dd>
													</div>
													<div>
														<dt>{t("deviceBillingReserved", "Reserved")}</dt>
														<dd>{formatEuroMicros(row.reserved_micros)}</dd>
													</div>
												</dl>
												<p>
													{t("deviceBillingExpiry", "Billing expires")}:{" "}
													{date(row.expires_at)}
												</p>
												{row.status === "active" &&
													row.payer_id === account && (
														<Button
															size="sm"
															variant="outline"
															disabled={busy}
															onClick={() => {
																mutation.reset();
																setRevocation({
																	kind: "revoke-billing",
																	id: row.billing_grant_id,
																});
															}}
														>
															{t(
																"deviceBillingRevoke",
																"Revoke billing approval",
															)}
														</Button>
													)}
											</div>
										))}
										{!currentBilling &&
											grant.model_ids.length > 0 &&
											isActiveGrant(grant, now) && (
												<>
													<p className="text-sm">
														{t(
															"deviceBillingMissing",
															"Hosted requests require a separate personal billing approval.",
														)}
													</p>
													{billingFor === grant.grant_id ? (
														<BillingForm
															grant={grant}
															account={account}
															disabled={!canApprove}
															onCancel={() => setBillingFor(null)}
															onSubmit={(request) =>
																submit({
																	kind: "billing",
																	grantId: grant.grant_id,
																	request,
																})
															}
														/>
													) : (
														<Button
															disabled={!canApprove}
															onClick={() => {
																mutation.reset();
																setBillingFor(grant.grant_id);
															}}
														>
															{t(
																"deviceBillingReview",
																"Review personal billing",
															)}
														</Button>
													)}
												</>
											)}
										{binding && <PublicBinding binding={binding} />}
									</li>
								);
							})}
						</ul>
						<section className="space-y-2 border-t pt-4">
							<h3 className="font-semibold">
								{t("deviceLeasesTitle", "Registered resource leases")}
							</h3>
							<p className="text-sm text-muted-foreground">
								{t(
									"deviceLeasesExplanation",
									"This hub snapshot lists registered instances with unexpired resource leases. It does not confirm that a service is running or healthy. Revoked approvals can prevent access before a lease expires.",
								)}
							</p>
							<p className="text-sm">
								{t("deviceWorkloadLeases", "Service workloads")}:{" "}
								{data.instances.length - validations.length} ·{" "}
								{t("deviceValidationLeases", "Metadata validations")}:{" "}
								{validations.length}
							</p>
							{validations.length > 0 && (
								<p className="text-sm text-muted-foreground">
									{t(
										"deviceValidationLeasesExplanation",
										"Metadata validation checks a staged update before services start. These temporary leases do not count toward the placement's service instance limit.",
									)}
								</p>
							)}
							{data.instances.length === 0 ? (
								<p className="text-sm">
									{t(
										"deviceLeasesEmpty",
										"No resource leases returned by the hub.",
									)}
								</p>
							) : (
								<ul className="space-y-2">
									{data.instances.map((instance) => (
										<li
											key={instance.instance_id}
											className="rounded border p-3 text-sm"
										>
											<p className="break-all font-mono">
												{instance.instance_id}
											</p>
											<Badge variant="outline">
												{instance.purpose === "rollout_validation"
													? t("deviceValidationLease", "Metadata validation")
													: t("deviceWorkloadLease", "Service workload")}
											</Badge>
											<p>
												{data.grants.find(
													(grant) => grant.grant_id === instance.grant_id,
												)?.placement_id ?? instance.grant_id}
											</p>
											<p>
												{instance.lease_expires_at <= now
													? t("deviceLeaseExpired", "Lease may have expired")
													: t("deviceLeaseRecorded", "Recorded lease expires")}
												: {date(instance.lease_expires_at)}
											</p>
										</li>
									))}
								</ul>
							)}
						</section>
					</>
				)}
				<AlertDialog
					open={revocation !== null}
					onOpenChange={(open) => {
						if (!open && !mutation.isPending) setRevocation(null);
					}}
				>
					<AlertDialogContent>
						<AlertDialogHeader>
							<AlertDialogTitle>
								{t("deviceGrantRevokeTitle", "Revoke this approval?")}
							</AlertDialogTitle>
							<AlertDialogDescription>
								{revocation?.kind === "resource"
									? t(
											"deviceResourceRevokeEffect",
											"New hosted model requests using this resource grant will be denied. Local services keep running, and requests already dispatched may complete and be billed. The personal billing approval remains recorded separately.",
										)
									: t(
											"deviceBillingRevokeEffect",
											"New hosted model requests using this billing approval will be denied. Charges for requests already dispatched can still settle. Resource access approval and local services remain unchanged.",
										)}
							</AlertDialogDescription>
						</AlertDialogHeader>
						{mutation.isError && (
							<p role="alert">
								{apiErrorMessage(
									mutation.error,
									t(
										"deviceGrantRevokeError",
										"Revocation was not confirmed. Refresh and review the approval.",
									),
								)}
							</p>
						)}
						<AlertDialogFooter>
							<AlertDialogCancel disabled={mutation.isPending}>
								{t("cancel", "Cancel")}
							</AlertDialogCancel>
							<AlertDialogAction
								disabled={busy || !revocation}
								onClick={(event) => {
									event.preventDefault();
									if (revocation) submit(revocation);
								}}
							>
								{mutation.isPending
									? t("deviceGrantRevoking", "Revoking…")
									: t("deviceGrantRevokeConfirm", "Revoke approval")}
							</AlertDialogAction>
						</AlertDialogFooter>
					</AlertDialogContent>
				</AlertDialog>
			</DialogContent>
		</Dialog>
	);
}

function ResourceForm({
	disabled,
	grants,
	onSubmit,
	onCancel,
}: {
	disabled: boolean;
	grants: ResourceGrant[];
	onSubmit: (request: CreateResourceGrant) => void;
	onCancel: () => void;
}) {
	const { t } = useTranslation("settings");
	const formId = useId();
	const [placement, setPlacement] = useState<OfflinePlacementIdentity | null>(
		null,
	);
	const [models, setModels] = useState("");
	const [onlineAccess, setOnlineAccess] = useState<
		"none" | "read_only" | "read_write"
	>("none");
	const [ownerConsent, setOwnerConsent] = useState(false);
	const [cap, setCap] = useState("1");
	const [expiry, setExpiry] = useState(() =>
		localExpiryValue(Date.now() / 1000 + 30 * 86400),
	);
	const [error, setError] = useState<string | null>(null);
	const importSequence = useRef(0);
	useEffect(
		() => () => {
			importSequence.current += 1;
		},
		[],
	);
	const exists = grants.some(
		(grant) =>
			grant.placement_id === placement?.placement_id && isActiveGrant(grant),
	);
	return (
		<form
			className="space-y-3 rounded-lg border p-4"
			onSubmit={(event) => {
				event.preventDefault();
				if (disabled || !placement || exists) return;
				try {
					if (onlineAccess !== "none" && !ownerConsent)
						throw new Error(
							"Approve online project access explicitly before continuing.",
						);
					const request = createResourceRequest(
						placement,
						models,
						cap,
						expiry,
						undefined,
						onlineAccess === "none" ? undefined : onlineAccess,
					);
					setError(null);
					onSubmit(request);
				} catch (error) {
					setError(
						error instanceof Error
							? error.message
							: "Invalid resource approval.",
					);
				}
			}}
		>
			<h3 className="font-semibold">
				{t("deviceResourceSetupTitle", "Authorize a project placement")}
			</h3>
			<label
				htmlFor={`${formId}-placement`}
				className="block space-y-1 text-sm"
			>
				<span>{t("deviceResourceFile", "Existing placement JSON file")}</span>
				<Input
					id={`${formId}-placement`}
					type="file"
					accept=".json,application/json"
					disabled={disabled}
					onChange={async (event) => {
						const sequence = ++importSequence.current;
						const file = event.target.files?.[0];
						event.target.value = "";
						setPlacement(null);
						setOnlineAccess("none");
						setOwnerConsent(false);
						setError(null);
						if (!file) return;
						try {
							if (file.size > 1024 * 1024)
								throw new Error("Placement files must be at most 1 MiB.");
							const identity = importResourcePlacement(await file.text());
							if (sequence === importSequence.current) setPlacement(identity);
						} catch {
							if (sequence === importSequence.current)
								setError(
									t(
										"deviceResourceImportError",
										"Choose a valid placement JSON file under 1 MiB with placement, project, and deployment IDs.",
									),
								);
						}
					}}
				/>
			</label>
			{placement && (
				<dl className="space-y-1 break-all text-sm">
					<div>
						<dt>{t("deviceResourcePlacement", "Placement ID")}</dt>
						<dd className="font-mono">{placement.placement_id}</dd>
					</div>
					<div>
						<dt>{t("deviceResourceProject", "Project ID")}</dt>
						<dd className="font-mono">{placement.project_id}</dd>
					</div>
					<div>
						<dt>{t("deviceResourceDeployment", "Deployment ID")}</dt>
						<dd className="font-mono">{placement.deployment_id}</dd>
					</div>
				</dl>
			)}
			{placement?.app_id && (
				<div className="space-y-2 rounded border p-3">
					<label className="block text-sm">
						Online project access
						<select
							className="ml-2 rounded border bg-background p-2"
							value={onlineAccess}
							onChange={(event) => {
								setOnlineAccess(event.target.value as typeof onlineAccess);
								setOwnerConsent(false);
							}}
							disabled={disabled}
						>
							<option value="none">No online storage access</option>
							<option value="read_only">Read project files</option>
							<option value="read_write">Read and write project files</option>
						</select>
					</label>
					{onlineAccess !== "none" && (
						<label className="flex items-start gap-2 text-sm">
							<input
								type="checkbox"
								checked={ownerConsent}
								onChange={(event) => setOwnerConsent(event.target.checked)}
								disabled={disabled}
							/>
							I own this online project and approve this device to{" "}
							{onlineAccess === "read_write" ? "read and write" : "read"} its
							files.
						</label>
					)}
					<p className="text-xs text-muted-foreground">
						The hub checks current project ownership. Leave model IDs empty for
						storage-only access.
					</p>
				</div>
			)}
			<label htmlFor={`${formId}-models`} className="block space-y-1 text-sm">
				<span>
					{t(
						"deviceResourceExactModels",
						"Exact model Bit IDs, separated by commas or new lines",
					)}
				</span>
				<Textarea
					id={`${formId}-models`}
					value={models}
					maxLength={8256}
					disabled={disabled}
					onChange={(event) => setModels(event.target.value)}
					required={onlineAccess === "none"}
				/>
			</label>
			<div className="grid gap-3 sm:grid-cols-2">
				<label htmlFor={`${formId}-cap`} className="block space-y-1 text-sm">
					<span>{t("deviceResourceCap", "Instance limit")}</span>
					<Input
						id={`${formId}-cap`}
						inputMode="numeric"
						value={cap}
						maxLength={3}
						disabled={disabled}
						onChange={(event) => setCap(event.target.value)}
						required
					/>
				</label>
				<label htmlFor={`${formId}-expiry`} className="block space-y-1 text-sm">
					<span>{t("deviceResourceExpiry", "Resource access expires")}</span>
					<Input
						id={`${formId}-expiry`}
						type="datetime-local"
						value={expiry}
						disabled={disabled}
						onChange={(event) => setExpiry(event.target.value)}
						required
					/>
				</label>
			</div>
			<p className="text-sm text-muted-foreground">
				{t(
					"deviceResourceConsent",
					"Authorize only these model IDs for this placement, for up to one year. Personal billing approval is a separate step.",
				)}
			</p>
			{exists && (
				<output>
					{t(
						"deviceResourceExists",
						"This placement already has an active resource approval. Review it below before creating another.",
					)}
				</output>
			)}
			{error && (
				<p role="alert" className="text-destructive">
					{error}
				</p>
			)}
			<div className="flex gap-2">
				<Button
					type="button"
					variant="outline"
					disabled={disabled}
					onClick={onCancel}
				>
					{t("cancel", "Cancel")}
				</Button>
				<Button type="submit" disabled={disabled || !placement || exists}>
					{t("deviceResourceApprove", "Approve model access")}
				</Button>
			</div>
		</form>
	);
}

function BillingForm({
	grant,
	account,
	disabled,
	onSubmit,
	onCancel,
}: {
	grant: ResourceGrant;
	account: string;
	disabled: boolean;
	onSubmit: (request: { limit_micros: number; expires_at: number }) => void;
	onCancel: () => void;
}) {
	const { t } = useTranslation("settings");
	const formId = useId();
	const [amount, setAmount] = useState("");
	const [expiry, setExpiry] = useState(() =>
		localExpiryValue(grant.expires_at),
	);
	const [error, setError] = useState<string | null>(null);
	return (
		<form
			className="space-y-3 rounded-md border p-3"
			onSubmit={(event) => {
				event.preventDefault();
				if (disabled) return;
				try {
					const request = {
						limit_micros: eurosToMicros(amount),
						expires_at: parseExpiry(
							expiry,
							Math.floor(Date.now() / 1000),
							grant.expires_at,
						),
					};
					setError(null);
					onSubmit(request);
				} catch (error) {
					setError(
						error instanceof Error
							? error.message
							: "Invalid billing approval.",
					);
				}
			}}
		>
			<p className="text-sm">
				{t(
					"deviceBillingConsent",
					"Charge my personal account for this placement, up to this fixed allowance shared across all its instances. This is not a recurring allowance.",
				)}
			</p>
			<p className="break-all font-mono text-xs">{account}</p>
			<label htmlFor={`${formId}-amount`} className="block space-y-1 text-sm">
				<span>{t("deviceBillingAmount", "Personal allowance (EUR)")}</span>
				<Input
					id={`${formId}-amount`}
					inputMode="decimal"
					value={amount}
					maxLength={32}
					disabled={disabled}
					onChange={(event) => setAmount(event.target.value)}
					placeholder="10.00"
					required
				/>
			</label>
			<label
				htmlFor={`${formId}-billing-expiry`}
				className="block space-y-1 text-sm"
			>
				<span>{t("deviceBillingExpiry", "Billing expires")}</span>
				<Input
					id={`${formId}-billing-expiry`}
					type="datetime-local"
					value={expiry}
					max={localExpiryValue(grant.expires_at)}
					disabled={disabled}
					onChange={(event) => setExpiry(event.target.value)}
					required
				/>
			</label>
			{error && (
				<p role="alert" className="text-destructive">
					{error}
				</p>
			)}
			<div className="flex gap-2">
				<Button
					type="button"
					variant="outline"
					disabled={disabled}
					onClick={onCancel}
				>
					{t("cancel", "Cancel")}
				</Button>
				<Button type="submit" disabled={disabled}>
					{t("deviceBillingApprove", "Approve personal billing")}
				</Button>
			</div>
		</form>
	);
}

function PublicBinding({ binding }: { binding: string }) {
	const { t } = useTranslation("settings");
	const [message, setMessage] = useState<string | null>(null);
	return (
		<div className="space-y-2 border-t pt-3">
			<p className="text-sm">
				{t(
					"deviceResourceBindingHelp",
					"This public binding uses the approvals displayed above. Add its resource_grant field to the same placement file before applying it on the device.",
				)}
			</p>
			<pre className="overflow-x-auto rounded bg-muted p-3 text-xs">
				{binding}
			</pre>
			<div className="flex gap-2">
				<Button
					size="sm"
					variant="outline"
					onClick={async () => {
						try {
							await navigator.clipboard.writeText(binding);
							setMessage(t("deviceResourceCopied", "Public binding copied."));
						} catch {
							setMessage(
								t(
									"deviceResourceCopyError",
									"Copy failed. Select the public binding above or download it.",
								),
							);
						}
					}}
				>
					{t("deviceResourceCopy", "Copy public binding")}
				</Button>
				<Button
					size="sm"
					variant="outline"
					onClick={() => {
						try {
							const url = URL.createObjectURL(
								new Blob([binding], { type: "application/json" }),
							);
							const link = document.createElement("a");
							link.href = url;
							link.download = "resource-grant.json";
							link.click();
							setTimeout(() => URL.revokeObjectURL(url), 0);
						} catch {
							setMessage(
								t(
									"deviceResourceDownloadError",
									"Download failed. Copy the public binding above.",
								),
							);
						}
					}}
				>
					{t("deviceResourceDownload", "Download public binding")}
				</Button>
			</div>
			{message && <output className="text-sm">{message}</output>}
		</div>
	);
}

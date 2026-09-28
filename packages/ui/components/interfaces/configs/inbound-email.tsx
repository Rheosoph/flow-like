"use client";

import { useTranslation } from "@flow-like/locales";
import { Copy, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { apiErrorMessage } from "../../../lib/api-error";
import {
	normalizeInboundEmailAlias,
	validateInboundEmailAlias,
} from "../../../lib/inbound-email";
import { useBackend } from "../../../state/backend-state";
import type { IInboundEmailAddress } from "../../../state/backend-state/event-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import type { IConfigInterfaceProps } from "../interfaces";

async function unavailableAddress(
	_appId: string,
	_eventId: string,
): Promise<IInboundEmailAddress> {
	throw new Error("Inbound email is not supported by this backend");
}

/** The server's own message (409 taken, 422, 429 rate limit) without its `[CODE]` prefix. */
function errorText(error: unknown): string {
	return apiErrorMessage(
		error,
		error instanceof Error ? error.message : String(error),
	);
}

export function InboundEmailConfig({
	appId,
	eventId,
	isEditing,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const offline = useInvoke(backend.isOffline, backend, [appId], !!appId);
	const getAddress = backend.eventState.getInboundEmailAddress;
	const address = useInvoke(
		getAddress ?? unavailableAddress,
		backend.eventState,
		[appId, eventId ?? ""],
		!!eventId && offline.data === false && !!getAddress,
	);
	const [alias, setAlias] = useState("");
	const [saving, setSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const scope = `${appId}/${eventId ?? ""}`;
	const scopeRef = useRef(scope);

	// Switching events discards unsaved alias edits even when the saved aliases match.
	useEffect(() => {
		scopeRef.current = scope;
		setAlias(address.data?.alias ?? "");
		setSaveError(null);
		setSaving(false);
	}, [scope, address.data?.alias]);

	const aliasIssue = validateInboundEmailAlias(alias);
	const validationError =
		aliasIssue === "format"
			? t(
					"inboundEmailAliasFormat",
					"Use 3 to 64 lowercase letters, numbers or hyphens. Start and end with a letter or number.",
				)
			: aliasIssue === "reserved"
				? t(
						"inboundEmailAliasReserved",
						"This alias is reserved. Choose another name.",
					)
				: undefined;
	const normalizedAlias = normalizeInboundEmailAlias(alias);
	const aliasAddress =
		address.data?.alias && address.data.domain
			? `${address.data.alias}@${address.data.domain}`
			: null;

	async function copy(value: string) {
		try {
			await navigator.clipboard.writeText(value);
			toast.success(t("inboundEmailAddressCopied", "Email address copied"));
		} catch {
			toast.error(
				t(
					"inboundEmailCouldNotCopyTheAddress",
					"Could not copy the address. Select and copy it manually.",
				),
			);
		}
	}

	async function saveAlias(value: string | null) {
		if (!eventId || !backend.eventState.updateInboundEmailAlias) return;
		const started = scope;
		const current = () => scopeRef.current === started;
		setSaving(true);
		setSaveError(null);
		try {
			const saved = await backend.eventState.updateInboundEmailAlias(
				appId,
				eventId,
				value,
			);
			if (!current()) return;
			setAlias(saved.alias ?? "");
			toast.success(
				value
					? t("inboundEmailAliasSaved", "Email alias saved")
					: t("inboundEmailAliasRemoved", "Email alias removed"),
			);
		} catch (error) {
			if (current()) setSaveError(errorText(error));
		} finally {
			await address.refetch();
			if (current()) setSaving(false);
		}
	}

	return (
		<div className="w-full max-w-2xl space-y-5">
			<p className="text-sm text-muted-foreground">
				{t(
					"inboundEmailRunsThisEventOnTheServer",
					"Incoming email runs this event on the server. Your desktop app can be closed.",
				)}
			</p>
			{offline.isPending ? (
				<p aria-live="polite" className="text-sm">
					{t("checkingAppConnection", "Checking app connection…")}
				</p>
			) : offline.error ? (
				<p role="alert" className="text-sm text-destructive">
					{t(
						"couldNotCheckTheAppConnection",
						"Could not check the app connection: {{message}}",
						{ message: offline.error.message },
					)}
				</p>
			) : offline.data !== false ? (
				<p aria-live="polite" className="text-sm">
					{t(
						"inboundEmailSyncThisApp",
						"Sync this app to an online profile to receive email on the server.",
					)}
				</p>
			) : !eventId ? (
				<p aria-live="polite" className="text-sm">
					{t(
						"inboundEmailSaveTheEventToReceiveItsAddress",
						"Save the event to receive its generated email address.",
					)}
				</p>
			) : !getAddress ? (
				<p aria-live="polite" className="text-sm">
					{t(
						"inboundEmailNotSupportedByThisConnection",
						"This connection does not support inbound email.",
					)}
				</p>
			) : address.isPending ? (
				<p aria-live="polite" className="flex items-center gap-2 text-sm">
					<Loader2 className="h-4 w-4 animate-spin" />{" "}
					{t("inboundEmailLoadingAddress", "Loading email address…")}
				</p>
			) : address.error ? (
				<div className="space-y-2">
					<p role="alert" className="text-sm text-destructive">
						{errorText(address.error)}
					</p>
					<Button
						type="button"
						variant="outline"
						disabled={address.isFetching}
						onClick={() => address.refetch()}
					>
						<RefreshCw className="h-4 w-4" /> {t("retry", "Retry")}
					</Button>
				</div>
			) : !address.data?.configured ? (
				<div className="space-y-2">
					<p aria-live="polite" className="text-sm">
						{t(
							"inboundEmailNotEnabledOnThisServer",
							"Inbound email is not enabled on this server. Ask your platform administrator to enable it.",
						)}
					</p>
					<RefreshAddressButton
						pending={address.isFetching}
						onClick={() => address.refetch()}
					/>
				</div>
			) : !address.data.address ? (
				<div className="space-y-2">
					<p aria-live="polite" className="text-sm">
						{t(
							"inboundEmailAddressIsPending",
							"The email address is pending. Save the event, then refresh its address.",
						)}
					</p>
					<RefreshAddressButton
						pending={address.isFetching}
						onClick={() => address.refetch()}
					/>
				</div>
			) : (
				<>
					<div className="space-y-2">
						<Label htmlFor="inbound-email-address">
							{t("inboundEmailGeneratedAddress", "Generated email address")}
						</Label>
						<div className="flex gap-2">
							<Input
								id="inbound-email-address"
								readOnly
								value={address.data.address}
								className="font-mono"
							/>
							<Button
								type="button"
								variant="outline"
								aria-label={t(
									"inboundEmailCopyGeneratedAddress",
									"Copy generated email address",
								)}
								onClick={() => {
									if (address.data?.address) void copy(address.data.address);
								}}
							>
								<Copy className="h-4 w-4" />
							</Button>
						</div>
						<p className="text-xs text-muted-foreground">
							{t(
								"inboundEmailAddressStaysTheSame",
								"This address stays the same when you change the alias.",
							)}
						</p>
					</div>
					{!address.data.active && (
						<p aria-live="polite" className="text-sm">
							{t(
								"inboundEmailEventIsInactive",
								"This event is inactive. Activate it to process incoming email.",
							)}
						</p>
					)}
					<div className="space-y-2">
						<Label htmlFor="inbound-email-alias">
							{t("inboundEmailAliasOptional", "Email alias (optional)")}
						</Label>
						<div className="flex items-center gap-2">
							<Input
								id="inbound-email-alias"
								value={alias}
								disabled={!isEditing || saving}
								placeholder="invoices"
								aria-describedby={
									validationError
										? "inbound-email-alias-help inbound-email-alias-error"
										: "inbound-email-alias-help"
								}
								aria-invalid={!!validationError}
								onChange={(e) => {
									setAlias(e.target.value);
									setSaveError(null);
								}}
							/>
							<span className="text-sm text-muted-foreground">
								@{address.data.domain}
							</span>
						</div>
						<p
							id="inbound-email-alias-help"
							className="text-xs text-muted-foreground"
						>
							{t(
								"inboundEmailAliasHelp",
								"Choose the part before @. Alias changes take effect when saved here.",
							)}
						</p>
						{validationError && (
							<p
								id="inbound-email-alias-error"
								role="alert"
								className="text-sm text-destructive"
							>
								{validationError}
							</p>
						)}
						{saveError && (
							<p role="alert" className="text-sm text-destructive">
								{saveError}
							</p>
						)}
						{aliasAddress && (
							<div className="flex items-center gap-2">
								<code className="break-all text-sm">{aliasAddress}</code>
								<Button
									type="button"
									size="sm"
									variant="ghost"
									aria-label={t("inboundEmailCopyAlias", "Copy email alias")}
									onClick={() => copy(aliasAddress)}
								>
									<Copy className="h-4 w-4" />
								</Button>
							</div>
						)}
						{isEditing && (
							<div className="flex flex-wrap gap-2">
								<Button
									type="button"
									disabled={
										saving ||
										!!validationError ||
										normalizedAlias === address.data.alias ||
										!backend.eventState.updateInboundEmailAlias
									}
									onClick={() => saveAlias(normalizedAlias)}
								>
									{saving && <Loader2 className="h-4 w-4 animate-spin" />}
									{t("inboundEmailSaveAlias", "Save alias")}
								</Button>
								{address.data.alias && (
									<Button
										type="button"
										variant="outline"
										disabled={
											saving || !backend.eventState.updateInboundEmailAlias
										}
										onClick={() => saveAlias(null)}
									>
										{t("inboundEmailRemoveAlias", "Remove alias")}
									</Button>
								)}
							</div>
						)}
					</div>
				</>
			)}
		</div>
	);
}

function RefreshAddressButton({
	pending,
	onClick,
}: Readonly<{ pending: boolean; onClick: () => void }>) {
	const { t } = useTranslation("interfaces");
	return (
		<Button
			type="button"
			variant="outline"
			disabled={pending}
			onClick={onClick}
		>
			<RefreshCw className="h-4 w-4" />{" "}
			{t("inboundEmailRefreshAddress", "Refresh address")}
		</Button>
	);
}

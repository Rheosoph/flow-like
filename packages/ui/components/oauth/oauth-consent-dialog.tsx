"use client";

import {
	AlertTriangle,
	Check,
	ExternalLink,
	PlayCircle,
	Shield,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { IOAuthProvider } from "../../lib/oauth/types";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Checkbox } from "../ui/checkbox";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { ScrollArea } from "../ui/scroll-area";

interface OAuthConsentDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	providers: IOAuthProvider[];
	/** Called when user clicks Authorize for a provider that needs OAuth */
	onAuthorize: (providerId: string) => Promise<void>;
	/** Called when user confirms all and wants to proceed - receives remember preference */
	onConfirmAll: (rememberConsent: boolean) => void;
	onCancel: () => void;
	authorizedProviders?: Set<string>;
	/** Providers that already have valid tokens but need consent for this specific app */
	preAuthorizedProviders?: Set<string>;
	missingScopesByProvider?: Record<string, string[]>;
	/** Lets the person go on without connecting: Cancel reads `label`, and `description` says what that does. */
	skip?: { label: string; description: string };
}

function formatScope(scope: string): string {
	if (scope.startsWith("https://")) {
		const parts = scope.split("/");
		return parts[parts.length - 1];
	}
	return scope;
}

export function OAuthConsentDialog({
	open,
	onOpenChange,
	providers,
	onAuthorize,
	onConfirmAll,
	onCancel,
	authorizedProviders: externalAuthorized,
	preAuthorizedProviders = new Set(),
	missingScopesByProvider = {},
	skip,
}: OAuthConsentDialogProps) {
	const [authorizing, setAuthorizing] = useState<string | null>(null);
	const [acknowledged, setAcknowledged] = useState(false);
	const [rememberConsent, setRememberConsent] = useState(true);
	const [internalAuthorized, setInternalAuthorized] = useState<Set<string>>(
		new Set(),
	);

	const authorizedProviders = externalAuthorized ?? internalAuthorized;

	// Check if all providers are ready (either authorized or pre-authorized)
	const allReady = useMemo(() => {
		return providers.every(
			(p) => authorizedProviders.has(p.id) || preAuthorizedProviders.has(p.id),
		);
	}, [providers, authorizedProviders, preAuthorizedProviders]);

	// Count providers needing authorization
	const pendingCount = useMemo(() => {
		return providers.filter(
			(p) =>
				!authorizedProviders.has(p.id) && !preAuthorizedProviders.has(p.id),
		).length;
	}, [providers, authorizedProviders, preAuthorizedProviders]);

	useEffect(() => {
		if (open) {
			setInternalAuthorized(new Set());
			setAcknowledged(false);
		}
	}, [open]);

	const handleAuthorize = async (provider: IOAuthProvider) => {
		setAuthorizing(provider.id);
		try {
			await onAuthorize(provider.id);
		} finally {
			setAuthorizing(null);
		}
	};

	const handleConfirm = () => {
		onConfirmAll(rememberConsent);
	};

	const handleCancel = () => {
		setAcknowledged(false);
		onCancel();
	};

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Shield className="h-5 w-5 text-blue-500" />
						Third-Party Authorization Required
					</DialogTitle>
					<DialogDescription>
						This workflow requires access to external services. Please review
						and authorize the required connections below.
					</DialogDescription>
					{skip && (
						<p className="text-sm text-muted-foreground">{skip.description}</p>
					)}
				</DialogHeader>

				<ScrollArea className="max-h-[300px] pr-4">
					<div className="space-y-4">
						{providers.map((provider) => {
							const isAuthorized = authorizedProviders.has(provider.id);
							const isPreAuthorized = preAuthorizedProviders.has(provider.id);
							const isReady = isAuthorized || isPreAuthorized;
							const missingScopes = missingScopesByProvider[provider.id] ?? [];
							return (
								<div
									key={provider.id}
									className={`rounded-lg border p-4 space-y-3 ${isReady ? "border-green-500 bg-green-50 dark:bg-green-950/20" : ""}`}
								>
									<div className="flex items-center justify-between">
										<div className="font-medium flex items-center gap-2">
											{provider.name}
											{isReady && <Check className="h-4 w-4 text-green-600" />}
										</div>
										{isReady ? (
											<Badge
												variant="outline"
												className="text-green-600 border-green-600"
											>
												{isPreAuthorized && !isAuthorized
													? "Connected"
													: "Authorized"}
											</Badge>
										) : (
											<Button
												size="sm"
												onClick={() => handleAuthorize(provider)}
												disabled={!acknowledged || authorizing !== null}
											>
												{authorizing === provider.id ? (
													"Authorizing..."
												) : (
													<>
														<ExternalLink className="h-4 w-4 mr-1" />
														Authorize
													</>
												)}
											</Button>
										)}
									</div>
									<div className="text-sm text-muted-foreground">
										<div className="font-medium mb-1">
											Requested permissions:
										</div>
										<div className="flex flex-wrap gap-1">
											{(provider.merged_scopes ?? provider.scopes).map(
												(scope) => (
													<Badge
														key={scope}
														variant="secondary"
														className="text-xs"
													>
														{formatScope(scope)}
													</Badge>
												),
											)}
										</div>
									</div>

									{missingScopes.length > 0 && (
										<p role="alert" className="text-sm text-destructive">
											{provider.name} did not grant all required permissions.
											Authorize again and allow:{" "}
											{missingScopes.map(formatScope).join(", ")}.
										</p>
									)}

									{provider.pkce_required && (
										<div className="flex items-center gap-1 text-xs text-green-600">
											<Shield className="h-3 w-3" />
											Using secure PKCE authentication
										</div>
									)}
								</div>
							);
						})}
					</div>
				</ScrollArea>

				<div className="flex items-start gap-2 p-3 bg-amber-50 dark:bg-amber-950/30 rounded-lg border border-amber-200 dark:border-amber-800">
					<AlertTriangle className="h-4 w-4 text-amber-600 mt-0.5 shrink-0" />
					<div className="space-y-2">
						<p className="text-sm text-amber-800 dark:text-amber-200">
							You are granting this workflow access to your data on these
							services. Tokens will be stored locally and used only for this
							workflow.
						</p>
						<div className="flex items-center gap-2">
							<Checkbox
								id="acknowledge"
								checked={acknowledged}
								onCheckedChange={(checked) => setAcknowledged(checked === true)}
							/>
							<label
								htmlFor="acknowledge"
								className="text-sm text-amber-800 dark:text-amber-200 cursor-pointer"
							>
								I understand and want to proceed
							</label>
						</div>
						<div className="flex items-center gap-2">
							<Checkbox
								id="remember-consent"
								checked={rememberConsent}
								onCheckedChange={(checked) =>
									setRememberConsent(checked === true)
								}
							/>
							<label
								htmlFor="remember-consent"
								className="text-sm text-amber-800 dark:text-amber-200 cursor-pointer"
							>
								Don&apos;t ask me again for these services
							</label>
						</div>
					</div>
				</div>

				<DialogFooter className="flex-col sm:flex-row gap-2">
					<Button variant="outline" onClick={handleCancel}>
						{skip?.label ?? "Cancel"}
					</Button>
					<Button onClick={handleConfirm} disabled={!acknowledged || !allReady}>
						<PlayCircle className="h-4 w-4 mr-1" />
						{allReady
							? "Continue"
							: `Waiting for ${pendingCount} service${pendingCount > 1 ? "s" : ""}`}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

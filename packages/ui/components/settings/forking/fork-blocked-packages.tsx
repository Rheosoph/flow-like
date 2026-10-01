"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	CheckCircle2Icon,
	ClockIcon,
	KeyRoundIcon,
	Loader2Icon,
	LockIcon,
	type LucideIcon,
	PackageIcon,
	PackageXIcon,
	ShoppingCartIcon,
} from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { apiErrorMessage } from "../../../lib/api-error";
import { openExternalUrl } from "../../../lib/open-external";
import type {
	IBlockedPackage,
	IPackageBlock,
} from "../../../lib/schema/app/fork";
import type { RegistryEntry } from "../../../lib/schema/wasm";
import { useBackend, useSignedIn } from "../../../state/backend-state";
import { MarketplaceCheckoutDialog } from "../../payments/checkout-dialog";
import { paymentMoney } from "../../payments/types";
import {
	usePaymentDistribution,
	usePayments,
} from "../../payments/use-payments";
import {
	useCheckoutPolling,
	viewerHasPackageAccess,
} from "../../store/use-package-store-data";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";

/** Rows the forker can act on come first. */
const BLOCK_ORDER: Record<IPackageBlock, number> = {
	paid: 0,
	request_access: 1,
	revoked: 2,
	private: 3,
	unavailable: 4,
	missing: 5,
};

interface PurchaseOptions {
	signedIn: boolean;
	purchasingAllowed: boolean;
	marketplaceEnabled: boolean;
}

interface BlockedPackageProps {
	pkg: IBlockedPackage;
	onAccessChanged: () => void;
	disabled: boolean;
}

/**
 * Packages the source app pins that the forker doesn't hold. Each row offers
 * the way to keep it — buying or requesting access — or says it is left out.
 * `onAccessChanged` reloads the preview, which removes rows that resolved.
 * `onCheckoutPendingChange` reports packages whose payment is still open, so
 * the dialog can warn before a fork leaves them out.
 */
export function ForkBlockedPackages({
	packages,
	onAccessChanged,
	onCheckoutPendingChange,
	disabled = false,
}: Readonly<{
	packages: readonly IBlockedPackage[];
	onAccessChanged: () => void;
	onCheckoutPendingChange?: (packageId: string, pending: boolean) => void;
	disabled?: boolean;
}>) {
	const { t } = useTranslation("settings");
	const signedIn = useSignedIn();
	const purchasingAllowed = usePaymentDistribution();
	const marketplaceEnabled = usePayments().config?.marketplace_enabled === true;
	const purchase: PurchaseOptions = {
		signedIn,
		purchasingAllowed,
		marketplaceEnabled,
	};
	const sorted = useMemo(
		() =>
			[...packages].sort(
				(a, b) =>
					BLOCK_ORDER[a.block] - BLOCK_ORDER[b.block] ||
					a.name.localeCompare(b.name),
			),
		[packages],
	);
	const obtainable = packages.some(
		(pkg) =>
			pkg.block === "request_access" ||
			(pkg.block === "paid" && purchasingAllowed),
	);

	return (
		<section className="@container/packages rounded-md border">
			<header className="space-y-0.5 border-b px-4 py-3">
				<h3 className="text-sm font-medium">
					{t("forkBlockedPackagesTitle", {
						defaultValue_one: "{{count}} package won't come with your copy",
						defaultValue_other: "{{count}} packages won't come with your copy",
						count: packages.length,
					})}
				</h3>
				<p className="text-xs text-muted-foreground">
					{t(
						"forkBlockedPackagesDescription",
						"Flows and page widgets that use them need the package to work.",
					)}
				</p>
			</header>
			<ul className="divide-y">
				{sorted.map((pkg) => {
					const props = { pkg, onAccessChanged, disabled };
					switch (pkg.block) {
						case "paid":
							return (
								<PaidPackage
									key={pkg.package_id}
									{...props}
									purchase={purchase}
									onCheckoutPendingChange={onCheckoutPendingChange}
								/>
							);
						case "request_access":
							return (
								<RequestAccessPackage
									key={pkg.package_id}
									{...props}
									signedIn={signedIn}
								/>
							);
						default:
							return <LeftOutPackage key={pkg.package_id} pkg={pkg} />;
					}
				})}
			</ul>
			{!signedIn && obtainable && (
				<p className="border-t px-4 py-2 text-xs text-muted-foreground">
					{t(
						"forkBlockedPackagesSignIn",
						"Sign in to get these packages for your fork.",
					)}
				</p>
			)}
		</section>
	);
}

function PackageRow({
	icon: Icon,
	name,
	description,
	action,
}: Readonly<{
	icon: LucideIcon;
	name: string;
	description: string;
	action: ReactNode;
}>) {
	return (
		<li className="flex items-start gap-3 px-4 py-3">
			<span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
				<Icon className="size-4" />
			</span>
			<div className="flex min-w-0 flex-1 flex-col gap-2 @sm/packages:flex-row @sm/packages:items-center @sm/packages:gap-3">
				<div className="min-w-0 flex-1 space-y-0.5">
					<p className="truncate text-sm font-medium" title={name}>
						{name}
					</p>
					<p className="text-xs text-muted-foreground wrap-break-word">
						{description}
					</p>
				</div>
				<div className="shrink-0">{action}</div>
			</div>
		</li>
	);
}

function LeftOutBadge() {
	const { t } = useTranslation("settings");
	return (
		<Badge variant="outline" className="text-muted-foreground">
			{t("forkPackageLeftOutBadge", "Left out")}
		</Badge>
	);
}

function PaidPackage({
	pkg,
	onAccessChanged,
	onCheckoutPendingChange,
	disabled,
	purchase,
}: Readonly<
	BlockedPackageProps & {
		purchase: PurchaseOptions;
		onCheckoutPendingChange?: (packageId: string, pending: boolean) => void;
	}
>) {
	const { t, i18n } = useTranslation("settings");
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const hubProfile = profile.data?.hub_profile;
	const [starting, setStarting] = useState(false);
	const [awaitingCheckout, setAwaitingCheckout] = useState(false);
	const [purchased, setPurchased] = useState(false);
	const [checkoutOpen, setCheckoutOpen] = useState(false);
	const settled = useRef(false);

	useEffect(() => {
		if (!awaitingCheckout) return;
		onCheckoutPendingChange?.(pkg.package_id, true);
		return () => onCheckoutPendingChange?.(pkg.package_id, false);
	}, [awaitingCheckout, onCheckoutPendingChange, pkg.package_id]);

	const markPurchased = useCallback(() => {
		if (settled.current) return;
		settled.current = true;
		setCheckoutOpen(false);
		setAwaitingCheckout(false);
		setPurchased(true);
		toast.success(
			t("forkPackagePurchased", "{{name}} is yours and comes with your fork.", {
				name: pkg.name,
			}),
		);
		onAccessChanged();
	}, [t, pkg.name, onAccessChanged]);

	const checkAccess = useCallback(async () => {
		if (!hubProfile) return;
		const entry = await backend.apiState
			.get<RegistryEntry>(
				hubProfile,
				`registry/package/${encodeURIComponent(pkg.package_id)}`,
			)
			.catch(() => null);
		if (viewerHasPackageAccess(entry)) markPurchased();
	}, [backend.apiState, hubProfile, pkg.package_id, markPurchased]);
	const pollAccess = useCallback(() => void checkAccess(), [checkAccess]);
	const stopAwaiting = useCallback(() => setAwaitingCheckout(false), []);
	useCheckoutPolling(awaitingCheckout, pollAccess, stopAwaiting);

	const buy = useCallback(async () => {
		if (purchase.marketplaceEnabled) {
			setCheckoutOpen(true);
			return;
		}
		const failed = t(
			"forkPackageCheckoutFailed",
			"Couldn't start checkout. Please try again.",
		);
		setStarting(true);
		try {
			const result = await backend.registryState.purchasePackage(
				pkg.package_id,
			);
			if (result.alreadyHasAccess) {
				markPurchased();
				return;
			}
			if (!result.checkoutUrl) {
				toast.error(failed);
				return;
			}
			await openExternalUrl(result.checkoutUrl);
			toast.info(
				t("forkPackageCheckoutOpened", "Checkout opened in your browser."),
			);
			setAwaitingCheckout(true);
		} catch (error) {
			toast.error(apiErrorMessage(error, failed));
		} finally {
			setStarting(false);
		}
	}, [
		purchase.marketplaceEnabled,
		backend.registryState,
		pkg.package_id,
		markPurchased,
		t,
	]);

	if (purchased) {
		return (
			<PackageRow
				icon={PackageIcon}
				name={pkg.name}
				description={t(
					"forkPackagePurchasedDescription",
					"Payment confirmed. The package comes with your fork.",
				)}
				action={
					<Badge variant="secondary">
						<CheckCircle2Icon />
						{t("forkPackagePurchasedBadge", "Purchased")}
					</Badge>
				}
			/>
		);
	}

	if (!purchase.purchasingAllowed) {
		return (
			<PackageRow
				icon={PackageIcon}
				name={pkg.name}
				description={t(
					"forkPackagePaidUnavailable",
					"You don't own this package, and purchasing isn't available in this app.",
				)}
				action={<LeftOutBadge />}
			/>
		);
	}

	const price = paymentMoney(pkg.price, "eur", i18n.language);
	return (
		<>
			<PackageRow
				icon={PackageIcon}
				name={pkg.name}
				description={
					awaitingCheckout
						? t(
								"forkPackageAwaitingPayment",
								"Finish checkout in your browser. Once the payment is confirmed, the package comes with your fork. If you fork before that, it's left out.",
							)
						: t(
								"forkPackagePaid",
								"You don't own this package. Buy it before forking to keep it in your copy.",
							)
				}
				action={
					awaitingCheckout ? (
						<Button size="sm" disabled>
							<Loader2Icon className="animate-spin" />
							{t("forkPackageWaitingForPayment", "Waiting for payment…")}
						</Button>
					) : (
						<Button
							size="sm"
							onClick={() => void buy()}
							disabled={disabled || starting || !purchase.signedIn}
						>
							{starting ? (
								<Loader2Icon className="animate-spin" />
							) : (
								<ShoppingCartIcon />
							)}
							{t("forkPackageBuy", "Buy · {{price}}", { price })}
						</Button>
					)
				}
			/>
			{purchase.marketplaceEnabled && (
				<MarketplaceCheckoutDialog
					itemKind="PACKAGE"
					itemId={pkg.package_id}
					itemName={pkg.name}
					amount={pkg.price}
					open={checkoutOpen}
					onOpenChange={setCheckoutOpen}
					onPurchased={markPurchased}
				/>
			)}
		</>
	);
}

function RequestAccessPackage({
	pkg,
	onAccessChanged,
	disabled,
	signedIn,
}: Readonly<BlockedPackageProps & { signedIn: boolean }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [requesting, setRequesting] = useState(false);
	const [requested, setRequested] = useState(pkg.request_pending);

	const request = useCallback(async () => {
		setRequesting(true);
		try {
			const result = await backend.registryState.requestAccess(pkg.package_id);
			if (result.granted) {
				toast.success(
					t(
						"forkPackageAccessGranted",
						"Access granted. {{name}} comes with your fork.",
						{ name: pkg.name },
					),
				);
				onAccessChanged();
				return;
			}
			if (result.queued) {
				setRequested(true);
				toast.success(
					t("forkPackageAccessRequested", "Access request sent to the author."),
				);
				return;
			}
			onAccessChanged();
		} catch (error) {
			toast.error(
				apiErrorMessage(
					error,
					t(
						"forkPackageAccessRequestFailed",
						"Couldn't request access. Please try again.",
					),
				),
			);
		} finally {
			setRequesting(false);
		}
	}, [backend.registryState, pkg.package_id, pkg.name, onAccessChanged, t]);

	return (
		<PackageRow
			icon={PackageIcon}
			name={pkg.name}
			description={
				requested
					? t(
							"forkPackageRequestPending",
							"Request sent. Until the author approves it, the package is left out of your fork. You can add it later.",
						)
					: t(
							"forkPackageRequestAccess",
							"Its author decides who can use it. Request access before forking to keep it in your copy.",
						)
			}
			action={
				requested ? (
					<Badge variant="secondary">
						<ClockIcon />
						{t("forkPackageRequested", "Requested")}
					</Badge>
				) : (
					<Button
						size="sm"
						variant="outline"
						onClick={() => void request()}
						disabled={disabled || requesting || !signedIn}
					>
						{requesting ? (
							<Loader2Icon className="animate-spin" />
						) : (
							<KeyRoundIcon />
						)}
						{t("requestAccess", "Request access")}
					</Button>
				)
			}
		/>
	);
}

function LeftOutPackage({ pkg }: Readonly<{ pkg: IBlockedPackage }>) {
	const { t } = useTranslation("settings");
	let icon: LucideIcon = LockIcon;
	let description = t(
		"forkPackagePrivate",
		"This package is private and you don't have access.",
	);
	if (pkg.block === "missing") {
		icon = PackageXIcon;
		description = t(
			"forkPackageMissing",
			"This package no longer exists in the registry.",
		);
	} else if (pkg.block === "unavailable") {
		icon = PackageXIcon;
		description = t(
			"forkPackageUnavailable",
			"This package isn't available in the registry right now.",
		);
	} else if (pkg.block === "revoked") {
		icon = BanIcon;
		description = t(
			"forkPackageRevoked",
			"Your access to this package was revoked. Only its author can restore it.",
		);
	}
	return (
		<PackageRow
			icon={icon}
			name={pkg.name}
			description={description}
			action={<LeftOutBadge />}
		/>
	);
}

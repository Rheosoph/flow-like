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
import {
	ApiResponseError,
	apiErrorMessage,
	isMissingResourceError,
} from "../../../lib/api-error";
import { openExternalUrl } from "../../../lib/open-external";
import type {
	IBlockedPackage,
	IPackageBlock,
	IRepinnedPackage,
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
	checkoutWindowLeft,
	useCheckoutPolling,
	viewerHasPackageAccess,
} from "../../store/use-package-store-data";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	usePaidDescription,
	useRequestDescription,
} from "./fork-package-texts";

/** Rows the forker can act on come first. */
const BLOCK_ORDER: Record<IPackageBlock, number> = {
	paid: 0,
	request_access: 1,
	revoked: 2,
	private: 3,
	unavailable: 4,
	missing: 5,
};

export const CONFIG_RETRY_MS = 10_000;

interface PurchaseOptions {
	signedIn: boolean;
	purchasingAllowed: boolean;
	marketplaceEnabled: boolean;
	/** The hub's payment config is known, so the right checkout can be picked. */
	ready: boolean;
}

/**
 * Reports a package whose payment opened or is no longer open.
 * `browserCheckout` marks one that runs in the browser, as opposed to an order
 * the checkout dialog follows.
 */
type CheckoutPendingChange = (
	packageId: string,
	pending: boolean,
	browserCheckout?: boolean,
) => void;

/** A purchase a host still remembers. */
interface PendingCheckout {
	/**
	 * When a browser checkout started. Nothing says when one was abandoned, so
	 * it is followed for a limited time. An order ends when the hub says so.
	 */
	since?: number;
}

/** The purchases that are still open, by package id. */
type PendingCheckouts = ReadonlyMap<string, PendingCheckout>;

const NO_CHECKOUTS: PendingCheckouts = new Map();

/**
 * The purchases that are still open, for a host to keep: the rows are
 * remounted whenever their list reloads, and a payment that is still open must
 * survive that. A remounted row follows a browser checkout for what is left of
 * its time, so one that was abandoned is not followed afresh on every remount.
 * Hand `pendingCheckouts` and `handleCheckoutPendingChange` to
 * {@link ForkBlockedPackages}.
 */
export function usePendingCheckouts() {
	const [pendingCheckouts, setPendingCheckouts] = useState(NO_CHECKOUTS);
	const handleCheckoutPendingChange: CheckoutPendingChange = useCallback(
		(packageId, pending, browserCheckout) => {
			const checkout = { since: browserCheckout ? Date.now() : undefined };
			setPendingCheckouts((current) => {
				if (current.has(packageId) === pending) return current;
				const next = new Map(current);
				if (pending) next.set(packageId, checkout);
				else next.delete(packageId);
				return next;
			});
		},
		[],
	);
	const forgetPendingCheckouts = useCallback(
		() => setPendingCheckouts(NO_CHECKOUTS),
		[],
	);
	return {
		pendingCheckouts,
		handleCheckoutPendingChange,
		forgetPendingCheckouts,
	};
}

interface BlockedPackageProps {
	pkg: IBlockedPackage;
	onAccessChanged: () => void;
	disabled: boolean;
	/** The copy already exists, so getting a package adds it to that copy. */
	inCopy: boolean;
}

/**
 * Packages the source app pins that the fork leaves out. Each row offers the
 * way to keep it — buying or requesting access — or says it is left out.
 * `onAccessChanged` reloads the preview, which removes rows that resolved.
 * `onCheckoutPendingChange` reports packages whose payment is still open, so
 * the host can warn before a fork leaves them out. A host that keeps them
 * ({@link usePendingCheckouts}) hands them back as `pendingCheckouts`: rows are
 * remounted whenever the preview reloads, and a payment that is still open
 * must survive that.
 * `disabled` (a fork is being created) only locks the row actions: an open
 * payment is still followed, and one confirmed meanwhile makes no promise
 * about that fork.
 * `context` is `copy` where the copy was made earlier (a course lesson) and
 * the host adds a package to it once the viewer holds it.
 * `onRequestSent` tells a host that keeps the list around that an access
 * request was queued, so it can reload a list that still says otherwise.
 */
export function ForkBlockedPackages({
	packages,
	onAccessChanged,
	onRequestSent,
	onCheckoutPendingChange,
	pendingCheckouts,
	context = "fork",
	disabled = false,
}: Readonly<{
	packages: readonly IBlockedPackage[];
	onAccessChanged: () => void;
	onRequestSent?: () => void;
	onCheckoutPendingChange?: CheckoutPendingChange;
	pendingCheckouts?: PendingCheckouts;
	context?: "fork" | "copy";
	disabled?: boolean;
}>) {
	const { t } = useTranslation("settings");
	const inCopy = context === "copy";
	const signedIn = useSignedIn();
	const purchasingAllowed = usePaymentDistribution();
	const { config, configLoaded, refetchConfig } = usePayments();
	const purchase: PurchaseOptions = {
		signedIn,
		purchasingAllowed,
		marketplaceEnabled: config?.marketplace_enabled === true,
		ready: configLoaded,
	};

	// Buy stays locked until the hub's payment config is known, so a request
	// that failed is repeated instead of locking it for good.
	useEffect(() => {
		if (configLoaded) return;
		const retry = window.setInterval(
			() => void refetchConfig(),
			CONFIG_RETRY_MS,
		);
		return () => window.clearInterval(retry);
	}, [configLoaded, refetchConfig]);
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

	const title = inCopy
		? t("forkCopyBlockedPackagesTitle", {
				defaultValue_one: "{{count}} package is missing from your copy",
				defaultValue_other: "{{count}} packages are missing from your copy",
				count: packages.length,
			})
		: t("forkBlockedPackagesTitle", {
				defaultValue_one: "{{count}} package won't come with your copy",
				defaultValue_other: "{{count}} packages won't come with your copy",
				count: packages.length,
			});

	return (
		<PackageSection
			title={title}
			description={t(
				"forkBlockedPackagesDescription",
				"Flows and page widgets that use them need the package to work.",
			)}
			footer={
				!signedIn && obtainable
					? t("forkBlockedPackagesSignIn", "Sign in to get these packages.")
					: undefined
			}
		>
			{sorted.map((pkg) => {
				const props = { pkg, onAccessChanged, disabled, inCopy };
				switch (pkg.block) {
					case "paid":
						return (
							<PaidPackage
								key={pkg.package_id}
								{...props}
								purchase={purchase}
								onCheckoutPendingChange={onCheckoutPendingChange}
								remembered={pendingCheckouts?.get(pkg.package_id)}
							/>
						);
					case "request_access":
						return (
							<RequestAccessPackage
								key={pkg.package_id}
								{...props}
								onRequestSent={onRequestSent}
								signedIn={signedIn}
							/>
						);
					default:
						return <LeftOutPackage key={pkg.package_id} pkg={pkg} />;
				}
			})}
		</PackageSection>
	);
}

/**
 * Packages the fork carries in another version than the source app, because
 * the version it pins was never published. Nothing to act on: the fork works,
 * but flows built for the pinned version may need their nodes updated.
 */
export function ForkRepinnedPackages({
	packages,
}: Readonly<{ packages: readonly IRepinnedPackage[] }>) {
	const { t } = useTranslation("settings");
	const sorted = useMemo(
		() => [...packages].sort((a, b) => a.name.localeCompare(b.name)),
		[packages],
	);

	return (
		<PackageSection
			title={t("forkRepinnedPackagesTitle", {
				defaultValue_one: "{{count}} package comes in another version",
				defaultValue_other: "{{count}} packages come in another version",
				count: packages.length,
			})}
			description={t(
				"forkRepinnedPackagesDescription",
				"The version this app uses isn't published. Flows built for it may need their nodes updated.",
			)}
		>
			{sorted.map((pkg) => (
				<PackageRow
					key={pkg.package_id}
					icon={PackageIcon}
					name={pkg.name}
					description={t(
						"forkPackageRepinned",
						"Your copy uses {{version}} instead of {{pinnedVersion}}.",
						{ version: pkg.version, pinnedVersion: pkg.pinned_version },
					)}
					action={<Badge variant="outline">{pkg.version}</Badge>}
				/>
			))}
		</PackageSection>
	);
}

function PackageSection({
	title,
	description,
	footer,
	children,
}: Readonly<{
	title: string;
	description: string;
	footer?: string;
	children: ReactNode;
}>) {
	return (
		<section className="@container/packages rounded-md border">
			<header className="space-y-0.5 border-b px-4 py-3">
				<h3 className="text-sm font-medium">{title}</h3>
				<p className="text-xs text-muted-foreground">{description}</p>
			</header>
			<ul className="divide-y">{children}</ul>
			{footer && (
				<p className="border-t px-4 py-2 text-xs text-muted-foreground">
					{footer}
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
	remembered,
	disabled,
	inCopy,
	purchase,
}: Readonly<
	BlockedPackageProps & {
		purchase: PurchaseOptions;
		onCheckoutPendingChange?: CheckoutPendingChange;
		/** What the host remembers of this package's payment, while it is open. */
		remembered?: PendingCheckout;
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
	// A payment whose outcome is polled from the registry: a browser checkout, or
	// one the host remembers from before a remount. A remembered browser
	// checkout is resumed only while its time lasts.
	const [awaitingCheckout, setAwaitingCheckout] = useState(
		() => remembered !== undefined && checkoutWindowLeft(remembered.since) > 0,
	);
	// A marketplace order the checkout dialog keeps following.
	const [orderOpen, setOrderOpen] = useState(false);
	const [purchased, setPurchased] = useState(false);
	const [checkoutOpen, setCheckoutOpen] = useState(false);
	const [checkoutKey, setCheckoutKey] = useState(0);
	const settled = useRef(false);
	const checkoutUsed = useRef(false);
	const hadOrder = useRef(false);
	const pending = awaitingCheckout || orderOpen;
	const description = usePaidDescription(pending, inCopy);

	// Not undone on unmount: the host keeps the id while the preview reloads.
	// A payment that opens without an order is a browser checkout.
	useEffect(() => {
		onCheckoutPendingChange?.(pkg.package_id, pending, !orderOpen);
	}, [pending, orderOpen, onCheckoutPendingChange, pkg.package_id]);

	// A checkout that is closed with no order in progress is replaced, so a
	// cancelled, expired or failed order can be retried. Replacing it here and
	// not when it opens gives the new dialog time to load the hub's payment
	// config before anyone sees it.
	useEffect(() => {
		if (!checkoutUsed.current || checkoutOpen || orderOpen) return;
		checkoutUsed.current = false;
		setCheckoutKey((key) => key + 1);
	}, [checkoutOpen, orderOpen]);

	const handleOrderOpenChange = useCallback((open: boolean) => {
		setOrderOpen(open);
		if (open) {
			hadOrder.current = true;
		} else if (hadOrder.current) {
			// The order ended, so no payment is being waited for any more.
			hadOrder.current = false;
			setAwaitingCheckout(false);
		}
	}, []);

	const markPurchased = useCallback(() => {
		if (settled.current) return;
		settled.current = true;
		setCheckoutOpen(false);
		setAwaitingCheckout(false);
		setOrderOpen(false);
		setPurchased(true);
		let message = t(
			"forkPackagePurchased",
			"{{name}} is yours and comes with your fork.",
			{ name: pkg.name },
		);
		if (inCopy) {
			message = t(
				"forkCopyPackagePurchased",
				"{{name}} is yours and is being added to your copy.",
				{ name: pkg.name },
			);
		} else if (disabled) {
			// A fork that is already being created decided earlier what it carries.
			message = t(
				"forkPackagePurchasedLate",
				"{{name}} is yours. If your new copy was created without it, add it there.",
				{ name: pkg.name },
			);
		}
		toast.success(message);
		onAccessChanged();
	}, [t, pkg.name, onAccessChanged, disabled, inCopy]);

	/** Settles the row when the registry says the forker holds the package. */
	const checkAccess = useCallback(async () => {
		if (!hubProfile) return false;
		const entry = await backend.apiState
			.get<RegistryEntry>(
				hubProfile,
				`registry/package/${encodeURIComponent(pkg.package_id)}`,
			)
			.catch(() => null);
		if (!viewerHasPackageAccess(entry)) return false;
		markPurchased();
		return true;
	}, [backend.apiState, hubProfile, pkg.package_id, markPurchased]);
	const pollAccess = useCallback(() => void checkAccess(), [checkAccess]);
	const stopAwaiting = useCallback(() => setAwaitingCheckout(false), []);
	useCheckoutPolling(
		awaitingCheckout,
		pollAccess,
		stopAwaiting,
		remembered?.since,
	);

	const openCheckout = useCallback(() => {
		checkoutUsed.current = true;
		setCheckoutOpen(true);
		// Bought elsewhere in the meantime: the row settles and closes the dialog.
		void checkAccess();
	}, [checkAccess]);

	// Some refusals mean the preview that offered this purchase is out of date.
	const handleCheckoutFailed = useCallback(
		async (error: unknown) => {
			if (!(error instanceof ApiResponseError)) return;
			if (error.code === "ALREADY_OWNED") {
				// The buyer has the package or is barred from it. The registry can
				// lag behind a fresh purchase, so when it says no the preview, which
				// reads the hub's own records, decides.
				if (!(await checkAccess())) onAccessChanged();
				return;
			}
			// Disabled, withdrawn or deleted after the preview loaded.
			if (error.code === "LISTING_UNAVAILABLE" || isMissingResourceError(error))
				onAccessChanged();
		},
		[checkAccess, onAccessChanged],
	);

	const buy = useCallback(async () => {
		if (purchase.marketplaceEnabled) {
			openCheckout();
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
		openCheckout,
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
					"Payment confirmed. You own this package.",
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
	// Until the hub's payment config is known the right checkout can't be picked.
	const canAct = !disabled && purchase.signedIn && purchase.ready;
	// Only the marketplace checkout opens an order, so a remembered one shows
	// its action even before the hub's payment config is back after a remount.
	const viaMarketplace =
		purchase.marketplaceEnabled ||
		(remembered !== undefined && remembered.since === undefined);
	let action: ReactNode;
	if (pending && viaMarketplace) {
		// Also after a remount lost the dialog that held the order: the hub
		// hands the same open order back when checkout is prepared again.
		action = (
			<Button
				size="sm"
				variant="outline"
				onClick={openCheckout}
				disabled={!canAct}
			>
				<Loader2Icon className="animate-spin" />
				{t("forkPackageContinueCheckout", "Continue checkout")}
			</Button>
		);
	} else if (pending) {
		action = (
			<Button size="sm" disabled>
				<Loader2Icon className="animate-spin" />
				{t("forkPackageWaitingForPayment", "Waiting for payment…")}
			</Button>
		);
	} else {
		action = (
			<Button
				size="sm"
				onClick={() => void buy()}
				disabled={!canAct || starting}
			>
				{starting ? (
					<Loader2Icon className="animate-spin" />
				) : (
					<ShoppingCartIcon />
				)}
				{t("forkPackageBuy", "Buy · {{price}}", { price })}
			</Button>
		);
	}
	return (
		<>
			<PackageRow
				icon={PackageIcon}
				name={pkg.name}
				description={description}
				action={action}
			/>
			{purchase.marketplaceEnabled && (
				<MarketplaceCheckoutDialog
					key={checkoutKey}
					itemKind="PACKAGE"
					itemId={pkg.package_id}
					itemName={pkg.name}
					amount={pkg.price}
					open={checkoutOpen}
					onOpenChange={setCheckoutOpen}
					onPurchased={markPurchased}
					pollWhileClosed
					onAwaitingPaymentChange={handleOrderOpenChange}
					marketplaceEnabled
					onCheckoutFailed={handleCheckoutFailed}
				/>
			)}
		</>
	);
}

function RequestAccessPackage({
	pkg,
	onAccessChanged,
	onRequestSent,
	disabled,
	inCopy,
	signedIn,
}: Readonly<
	BlockedPackageProps & { onRequestSent?: () => void; signedIn: boolean }
>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [requesting, setRequesting] = useState(false);
	const [sent, setSent] = useState(false);
	// The list can be newer than this row, so it is not read at mount alone.
	const requested = sent || pkg.request_pending;
	const description = useRequestDescription(requested, inCopy);
	const grantedMessage = inCopy
		? t(
				"forkCopyPackageAccessGranted",
				"Access granted. {{name}} is being added to your copy.",
				{ name: pkg.name },
			)
		: t(
				"forkPackageAccessGranted",
				"Access granted. {{name}} comes with your fork.",
				{ name: pkg.name },
			);

	const request = useCallback(async () => {
		setRequesting(true);
		try {
			const result = await backend.registryState.requestAccess(pkg.package_id);
			if (result.granted) {
				toast.success(grantedMessage);
				onAccessChanged();
				return;
			}
			if (result.queued) {
				setSent(true);
				toast.success(
					t("forkPackageAccessRequested", "Access request sent to the author."),
				);
				onRequestSent?.();
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
	}, [
		backend.registryState,
		pkg.package_id,
		grantedMessage,
		onAccessChanged,
		onRequestSent,
		t,
	]);

	return (
		<PackageRow
			icon={PackageIcon}
			name={pkg.name}
			description={description}
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

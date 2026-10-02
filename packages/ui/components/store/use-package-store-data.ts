"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { openExternalUrl } from "../../lib/open-external";
import type { RegistryEntry } from "../../lib/schema/wasm";
import type {
	RequestAccessResponse,
	WasmPurchaseResponse,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import type { GenericFetcher } from "../pages/store/store-package-detail";
import { usePaymentDistribution, usePayments } from "../payments/use-payments";

export const CHECKOUT_POLL_MS = 4000;
const CHECKOUT_POLL_LIMIT_MS = 10 * 60_000;

/**
 * Whether the viewer may install the package. Free public packages download
 * without an access row; everything else needs one (`currentUserPermission`).
 * `undefined` while the package is unknown.
 */
export function viewerHasPackageAccess(
	pkg:
		| Pick<RegistryEntry, "price" | "visibility" | "currentUserPermission">
		| null
		| undefined,
): boolean | undefined {
	if (!pkg) return undefined;
	if (pkg.visibility === "local") return true;
	if ((pkg.currentUserPermission ?? 0) !== 0) return true;
	return pkg.visibility === "public" && (pkg.price ?? 0) <= 0;
}

/**
 * How long a checkout is still followed: the whole window for one that starts
 * now, what is left of it for one that started at `since`.
 */
export function checkoutWindowLeft(since?: number): number {
	if (since === undefined) return CHECKOUT_POLL_LIMIT_MS;
	return Math.max(0, since + CHECKOUT_POLL_LIMIT_MS - Date.now());
}

/**
 * While a checkout runs in the browser, re-checks access until it lands or the
 * window closes. `since` is when a checkout that is being resumed started.
 */
export function useCheckoutPolling(
	awaiting: boolean,
	onPoll: (() => void) | undefined,
	onGiveUp: () => void,
	since?: number,
) {
	useEffect(() => {
		if (!awaiting) return;
		const poll = window.setInterval(() => onPoll?.(), CHECKOUT_POLL_MS);
		const stop = window.setTimeout(onGiveUp, checkoutWindowLeft(since));
		return () => {
			window.clearInterval(poll);
			window.clearTimeout(stop);
		};
	}, [awaiting, onPoll, onGiveUp, since]);
}

export function usePackageStoreData(
	packageId: string | undefined,
	pkg: RegistryEntry | null | undefined,
	fetcher: GenericFetcher,
	auth?: unknown,
	onAccessChanged?: () => void,
) {
	const backend = useBackend();
	const purchasingAllowed = usePaymentDistribution();
	const marketplaceEnabled = usePayments().config?.marketplace_enabled === true;
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [isPurchasing, setIsPurchasing] = useState(false);
	const [isRequesting, setIsRequesting] = useState(false);
	const [grantedAccess, setGrantedAccess] = useState(false);
	const [awaitingCheckout, setAwaitingCheckout] = useState(false);
	const [checkoutOpen, setCheckoutOpen] = useState(false);

	const hasAccess = useMemo(
		() => (grantedAccess ? true : viewerHasPackageAccess(pkg)),
		[grantedAccess, pkg],
	);

	const stopAwaitingCheckout = useCallback(
		() => setAwaitingCheckout(false),
		[],
	);
	useCheckoutPolling(
		awaitingCheckout && !hasAccess,
		onAccessChanged,
		stopAwaitingCheckout,
	);

	const formatPrice = useCallback((price?: number | null) => {
		if (!price || price <= 0) return "Free";
		return `€${(price / 100).toFixed(2)}`;
	}, []);

	const priceLabel = formatPrice(pkg?.price ?? null);

	const onBuy = useCallback(async () => {
		if (!packageId || !profile.data || isPurchasing || !purchasingAllowed)
			return;

		if (marketplaceEnabled) {
			setCheckoutOpen(true);
			return;
		}

		setIsPurchasing(true);
		try {
			const result = await fetcher<WasmPurchaseResponse>(
				profile.data.hub_profile,
				`registry/package/${packageId}/purchase`,
				{
					method: "POST",
					body: JSON.stringify({}),
					headers: { "Content-Type": "application/json" },
				},
				auth,
			);

			if (result.alreadyHasAccess) {
				toast.info("You already have access to this package!");
				setGrantedAccess(true);
				onAccessChanged?.();
				return;
			}

			if (result.checkoutUrl) {
				await openExternalUrl(result.checkoutUrl, "checkout");
				setAwaitingCheckout(true);
			} else {
				toast.error("Unable to start purchase. Please try again.");
			}
		} catch (e) {
			console.error("Purchase error:", e);
			toast.error("Failed to start purchase. Please try again later.");
		} finally {
			setIsPurchasing(false);
		}
	}, [
		packageId,
		profile.data,
		isPurchasing,
		purchasingAllowed,
		marketplaceEnabled,
		fetcher,
		auth,
		onAccessChanged,
	]);

	const onRequestAccess = useCallback(async () => {
		if (!packageId || !profile.data || isRequesting) return;

		setIsRequesting(true);
		try {
			const result = await fetcher<RequestAccessResponse>(
				profile.data.hub_profile,
				`registry/package/${packageId}/access`,
				{
					method: "PUT",
					body: JSON.stringify({}),
					headers: { "Content-Type": "application/json" },
				},
				auth,
			);

			if (result.granted) {
				toast.success("Access granted! You can now use this package.");
				setGrantedAccess(true);
				onAccessChanged?.();
				return;
			}

			if (result.requiresPurchase) {
				toast.info("This package requires a purchase.");
				await onBuy();
				return;
			}

			if (result.queued) {
				toast.success(
					"Access request sent! The author will review your request.",
				);
				return;
			}
		} catch (e) {
			console.error("Access request error:", e);
			toast.error("Failed to request access. Please try again later.");
		} finally {
			setIsRequesting(false);
		}
	}, [
		packageId,
		profile.data,
		isRequesting,
		fetcher,
		auth,
		onAccessChanged,
		onBuy,
	]);

	const onGetOrBuy = useCallback(async () => {
		if (!pkg || !packageId) return;

		if (pkg.price > 0) {
			await onBuy();
			return;
		}

		if (
			pkg.visibility === "public_request_access" ||
			pkg.visibility === "public"
		) {
			await onRequestAccess();
			return;
		}

		toast.error("You don't have access to this package.");
	}, [pkg, packageId, onBuy, onRequestAccess]);

	return {
		isPurchasing,
		isRequesting,
		awaitingCheckout,
		checkoutOpen,
		setCheckoutOpen,
		priceLabel,
		hasAccess,
		onBuy,
		onRequestAccess,
		onGetOrBuy,
		formatPrice,
	} as const;
}

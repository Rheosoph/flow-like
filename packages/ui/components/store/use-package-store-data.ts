"use client";

import { useCallback, useState } from "react";
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

export function usePackageStoreData(
	packageId: string | undefined,
	pkg: RegistryEntry | null | undefined,
	fetcher: GenericFetcher,
	auth?: unknown,
	onAccessChanged?: () => void,
) {
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [isPurchasing, setIsPurchasing] = useState(false);
	const [isRequesting, setIsRequesting] = useState(false);
	const [hasAccess, setHasAccess] = useState<boolean | undefined>(undefined);

	const formatPrice = useCallback((price?: number | null) => {
		if (!price || price <= 0) return "Free";
		return `€${(price / 100).toFixed(2)}`;
	}, []);

	const priceLabel = formatPrice(pkg?.price ?? null);

	const onBuy = useCallback(async () => {
		if (!packageId || !profile.data || isPurchasing) return;

		setIsPurchasing(true);
		try {
			const result = await fetcher<WasmPurchaseResponse>(
				profile.data.hub_profile,
				`registry/package/${packageId}/purchase`,
				{ method: "POST" },
				auth,
			);

			if (result.alreadyHasAccess) {
				toast.info("You already have access to this package!");
				setHasAccess(true);
				onAccessChanged?.();
				return;
			}

			if (result.checkoutUrl) {
				await openExternalUrl(result.checkoutUrl, "checkout");
			} else {
				toast.error("Unable to start purchase. Please try again.");
			}
		} catch (e) {
			console.error("Purchase error:", e);
			toast.error("Failed to start purchase. Please try again later.");
		} finally {
			setIsPurchasing(false);
		}
	}, [packageId, profile.data, isPurchasing, fetcher, auth, onAccessChanged]);

	const onRequestAccess = useCallback(async () => {
		if (!packageId || !profile.data || isRequesting) return;

		setIsRequesting(true);
		try {
			const result = await fetcher<RequestAccessResponse>(
				profile.data.hub_profile,
				`registry/package/${packageId}/access`,
				{ method: "PUT" },
				auth,
			);

			if (result.granted) {
				toast.success("Access granted! You can now use this package.");
				setHasAccess(true);
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
		priceLabel,
		hasAccess,
		onBuy,
		onRequestAccess,
		onGetOrBuy,
		formatPrice,
	} as const;
}

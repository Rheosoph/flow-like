"use client";

import type { AppRouterInstance } from "next/dist/shared/lib/app-router-context.shared-runtime";
import { useCallback, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import type { IApp } from "../../lib/schema/app/app";
import { IAppVisibility } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";
import { useBackend } from "../../state/backend-state";
import type { IEventMapping } from "../interfaces/interfaces";

async function openCheckoutUrl(url: string) {
	if (typeof window !== "undefined" && "__TAURI__" in window) {
		const { openUrl } = await import("@tauri-apps/plugin-opener");
		await openUrl(url);
		toast.info("Opening checkout in your browser...");
	} else {
		window.open(url, "_blank");
		toast.info("Opening checkout in a new tab...");
	}
}

export function useStoreData(
	id: string | undefined,
	router: AppRouterInstance,
	eventConfig: IEventMapping,
) {
	const backend = useBackend();
	const [isPurchasing, setIsPurchasing] = useState(false);

	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const app = useInvoke<IApp, [appId: string]>(
		backend.appState.getApp,
		backend.appState,
		[id!],
		!!id,
	);
	const meta = useInvoke<
		IMetadata,
		[appId: string, language?: string | undefined]
	>(backend.appState.getAppMeta, backend.appState, [id!], !!id);

	const appData = app.data ?? null;
	const metaData = meta.data ?? null;

	const isMember = useMemo(
		() => !!(id && apps.data?.some(([a]) => a.id === id)),
		[apps.data, id],
	);
	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[id!],
		!!id && isMember,
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[id!],
		!!id && isMember,
	);
	const usableEvents = useMemo(() => {
		const set = new Set<string>();
		Object.values(eventConfig).forEach((config) => {
			const usable = Object.keys(config.useInterfaces);
			for (const eventType of usable) {
				if (config.eventTypes.includes(eventType)) set.add(eventType);
			}
		});
		return set;
	}, [eventConfig]);
	const useAppHref = useMemo(() => {
		if (!id || !isMember) return null;

		const activeEvents = (events.data ?? []).filter((event) => event.active);
		const activeEventsById = new Map(
			activeEvents.map((event) => [event.id, event] as const),
		);

		const hasUsableRoute = (routes.data ?? []).some((route) => {
			const routeEvent = activeEventsById.get(route.eventId);
			if (!routeEvent) return false;
			return (
				!!routeEvent.default_page_id || usableEvents.has(routeEvent.event_type)
			);
		});
		if (hasUsableRoute) {
			return `/use?id=${id}`;
		}

		const fallbackEvent = activeEvents.find((event) =>
			usableEvents.has(event.event_type),
		);
		if (!fallbackEvent) return null;

		return `/use?id=${id}&eventId=${fallbackEvent.id}`;
	}, [id, isMember, events.data, routes.data, usableEvents]);
	const canUseApp = !!useAppHref;

	const formatPrice = useCallback((price?: number | null) => {
		if (!price || price <= 0) return "Free";
		return `€${(price / 100).toFixed(2)}`;
	}, []);

	const onUse = useCallback(() => {
		if (!useAppHref) return;
		router.push(useAppHref);
	}, [router, useAppHref]);

	const onSettings = useCallback(() => {
		if (!id) return;
		router.push(`/library/config?id=${id}`);
	}, [id, router]);

	const onBuy = useCallback(async () => {
		if (!id || isPurchasing) return;

		setIsPurchasing(true);
		try {
			const result = await backend.appState.purchaseApp(id);

			if (result.alreadyMember) {
				toast.info("You already own this app!");
				await apps.refetch?.();
				router.push(`/use?id=${id}`);
				return;
			}

			if (result.checkoutUrl) {
				await openCheckoutUrl(result.checkoutUrl);
			} else {
				toast.error("Unable to start purchase. Please try again.");
			}
		} catch (e) {
			console.error("Purchase error:", e);
			toast.error("Failed to start purchase. Please try again later.");
		} finally {
			setIsPurchasing(false);
		}
	}, [id, isPurchasing, backend.appState, apps, router]);

	const onJoinOrRequest = useCallback(async () => {
		if (!appData || !id) return;
		try {
			if (appData.price && appData.price > 0) {
				await onBuy();
				return;
			}

			if (appData.visibility === IAppVisibility.PublicRequestAccess) {
				await backend.appState.requestJoinApp(
					appData.id,
					"Interested in trying out your app!",
				);
				toast.success(
					"Request to join app sent! The author will review your request.",
				);
				await apps.refetch?.();
				return;
			}

			if (appData.visibility !== IAppVisibility.Public) {
				toast.error(
					"You don't have access to this app. Please request access from the author.",
				);
				return;
			}

			await backend.appState.requestJoinApp(
				appData.id,
				"Interested in trying out your app!",
			);
			toast.success("Joined app! You can now access it.");
			await apps.refetch?.();
			await router.push(`/use?id=${appData.id}`);
		} catch (e) {
			toast.error("Failed to request to join app. Please try again later.");
		}
	}, [appData, id, backend.appState, apps, router, onBuy]);

	const hasThumbnail = !!metaData?.thumbnail;
	const coverUrl = metaData?.thumbnail || "/placeholder-thumbnail.webp";
	const iconUrl = metaData?.icon || "/app-logo.webp";
	const appName = metaData?.name || appData?.id || "App";
	const priceLabel = formatPrice(appData?.price ?? null);
	const isLoading = app.isLoading || meta.isLoading;
	const isError = app.isError || meta.isError;
	const notFound = !isLoading && !isError && !appData;
	const refetchAppData = useCallback(async () => {
		await Promise.all([app.refetch?.(), meta.refetch?.(), apps.refetch?.()]);
	}, [app, meta, apps]);

	return {
		apps,
		app,
		meta,
		appData,
		metaData,
		isMember,
		isPurchasing,
		isLoading,
		isError,
		notFound,
		hasThumbnail,
		coverUrl,
		iconUrl,
		appName,
		priceLabel,
		canUseApp,
		onUse,
		onSettings,
		onBuy,
		onJoinOrRequest,
		refetchAppData,
	} as const;
}

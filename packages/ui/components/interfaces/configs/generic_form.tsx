"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import type { IRouteMapping } from "../../../state/backend-state/route-state";
import { Checkbox, Label, ScrollArea } from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

export function GenericFormConfig({
	isEditing,
	appId,
	config,
	onConfigUpdate,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const routesQuery = useInvoke<IRouteMapping[], [appId: string]>(
		backend.routeState.getRoutes,
		backend.routeState,
		[appId],
		!!appId,
		[appId],
	);

	const routes = useMemo(() => {
		const list = routesQuery.data ?? [];
		return list.slice().sort((a, b) => a.path.localeCompare(b.path));
	}, [routesQuery.data]);

	const setValue = (key: string, value: any, deleteKeys: string[] = []) => {
		if (!onConfigUpdate) return;
		const next = { ...(config as any) };
		for (const k of deleteKeys) {
			delete next[k];
		}
		next[key] = value;
		onConfigUpdate(next);
	};

	const normalizeRoute = (value: string): string => {
		const trimmed = value.trim();
		if (!trimmed) return "";
		return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
	};

	const selectedRoutes = useMemo(() => {
		const rawArray = (config as any)?.navigate_to_routes;
		const raw: string[] = Array.isArray(rawArray) ? rawArray : [];
		const normalized = raw
			.map((r) => normalizeRoute(String(r)))
			.filter((r) => !!r);
		return Array.from(new Set(normalized));
	}, [config]);

	return (
		<div className="w-full space-y-6">
			<div className="space-y-3">
				<Label>{t('navigateTo', 'Navigate To')}</Label>
				{isEditing ? (
					<div className="rounded-md border border-input bg-background">
						<ScrollArea className="max-h-48">
							<div className="p-2 space-y-1">
								{routesQuery.isLoading ? (
									<div className="px-2 py-2 text-sm text-muted-foreground">
										{t('loadingRoutes', 'Loading routes…')}
									</div>
								) : routes.length === 0 ? (
									<div className="px-2 py-2 text-sm text-muted-foreground">
										{t('noRoutesConfigured', 'No routes configured.')}
									</div>
								) : (
									routes
										.filter(
											(r) => typeof r.path === "string" && r.path.length > 0,
										)
										.map((route) => {
											const checked = selectedRoutes.includes(route.path);
											return (
												<label
													key={route.path}
													className="flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-muted cursor-pointer"
												>
													<Checkbox
														checked={checked}
														onCheckedChange={(nextChecked) => {
															const normalized = normalizeRoute(route.path);
															const next = new Set(selectedRoutes);
															if (nextChecked) {
																next.add(normalized);
															} else {
																next.delete(normalized);
															}
															const nextArr = Array.from(next).filter(Boolean);
															setValue(
																"navigate_to_routes",
																nextArr.length > 0 ? nextArr : null,
															);
														}}
													/>
													<span className="text-sm">{route.path}</span>
												</label>
											);
										})
								)}
							</div>
						</ScrollArea>
						<div className="border-t border-input p-2">
							<button
								type="button"
								className="text-xs text-muted-foreground hover:text-foreground"
								onClick={() => setValue("navigate_to_routes", null)}
							>
								{t('clearSelection', 'Clear selection')}
							</button>
						</div>
					</div>
				) : (
					<div className="flex min-h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
						{selectedRoutes.length > 0
							? selectedRoutes.join(", ")
							: t('noDestinations', 'No destinations')}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					{t('optionalRouteDestinationsThisFormCanNavigateTo', 'Optional route destinations this form can navigate to.')}
				</p>
			</div>
		</div>
	);
}

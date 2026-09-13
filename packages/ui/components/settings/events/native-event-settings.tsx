"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Smartphone } from "lucide-react";
import { useId } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../../../lib/api-url";
import {
	NATIVE_SURFACES,
	type NativeEventSettings,
	nativeEventActionKind,
	nativeEventSettings,
} from "../../../lib/native-event";
import type { IEvent } from "../../../lib/schema/flow/event";
import { useBackend } from "../../../state/backend-state";
import { describeMcpAppInterface } from "../../global-chat/app-mcp-interface";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import { Switch } from "../../ui/switch";

export function NativeEventSettingsCard({
	appId,
	event,
	disabled,
	onChange,
}: {
	appId: string;
	event: IEvent;
	disabled: boolean;
	onChange: (settings: NativeEventSettings) => void;
}) {
	const { t } = useTranslation("common");
	const enableId = useId();
	const favoriteId = useId();
	const settings = nativeEventSettings(event);
	const backend = useBackend();
	const auth = useAuth();
	const mcp = event.event_type === "mcp" && event.execution_mode === "Remote";
	const supported = nativeEventActionKind(event) !== undefined || mcp;
	const tools = useQuery({
		queryKey: [
			"native-mcp-tools",
			getApiOrigin(backend.profile),
			backend.profile?.id,
			auth.user?.profile.sub,
			appId,
			event.id,
		],
		queryFn: () => describeMcpAppInterface(backend.eventState, appId, event.id),
		enabled: mcp && settings.enabled && event.active,
	});
	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<Smartphone className="size-5" />
					{t("nativeIntegrations", "Native integrations")}
				</CardTitle>
				<CardDescription>
					{supported
						? t(
								"nativeEventDescription",
								"Make this Event available in Siri, Shortcuts and native widgets. Access and current Event settings are checked when it opens or runs.",
							)
						: t(
								"nativeEventUnsupported",
								"Native entry points support pages, chat, Quick Actions, forms, API Events and selected hosted MCP tools. Other trigger types must expose one of these interfaces first.",
							)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				<label
					htmlFor={enableId}
					className="flex items-center justify-between gap-4 text-sm"
				>
					<span>{t("enableNativeEvent", "Enable native entry points")}</span>
					<Switch
						id={enableId}
						disabled={disabled || !supported}
						checked={settings.enabled && supported}
						onCheckedChange={(enabled) =>
							onChange({
								...settings,
								enabled,
								surfaces:
									enabled && !settings.surfaces.length
										? [...NATIVE_SURFACES]
										: settings.surfaces,
							})
						}
					/>
				</label>
				{settings.enabled && supported && (
					<>
						{mcp && (
							<div className="space-y-2">
								<label className="flex flex-col gap-2 text-sm">
									{t("nativeMcpTool", "MCP tool")}
									<select
										className="h-9 rounded-md border bg-background px-3"
										disabled={disabled || tools.isFetching}
										value={settings.operation || ""}
										onChange={(event) =>
											onChange({
												...settings,
												operation: event.target.value || undefined,
											})
										}
									>
										<option value="">
											{t("nativeMcpSelect", "Select a registered tool")}
										</option>
										{(tools.data?.tools ?? [])
											.filter(
												(tool): tool is { name: string } =>
													!!tool &&
													typeof tool === "object" &&
													typeof (tool as { name?: unknown }).name === "string",
											)
											.map((tool) => (
												<option key={tool.name} value={tool.name}>
													{tool.name}
												</option>
											))}
									</select>
								</label>
								<p className="text-xs text-muted-foreground">
									{tools.isError
										? t(
												"nativeMcpUnavailable",
												"Tool discovery failed. Save and activate the hosted Event, then try again.",
											)
										: t(
												"nativeMcpArguments",
												"Only this tool is exposed. Its arguments are entered in the app before it runs.",
											)}
								</p>
							</div>
						)}
						<div className="flex flex-wrap gap-4">
							{NATIVE_SURFACES.filter((surface) => surface !== "shortcuts").map(
								(surface) => (
									<label
										key={surface}
										className="flex items-center gap-2 text-sm"
									>
										<input
											type="checkbox"
											disabled={disabled}
											checked={
												settings.surfaces.includes(surface) ||
												(surface === "siri" &&
													settings.surfaces.includes("shortcuts"))
											}
											onChange={(event) =>
												onChange({
													...settings,
													surfaces: event.target.checked
														? [
																...new Set([
																	...settings.surfaces,
																	surface,
																	...(surface === "siri"
																		? ["shortcuts" as const]
																		: []),
																]),
															]
														: settings.surfaces.filter(
																(item) =>
																	item !== surface &&
																	!(surface === "siri" && item === "shortcuts"),
															),
												})
											}
										/>
										<span>
											{
												{
													siri: "Siri and Shortcuts",
													shortcuts: "Shortcuts",
													widget: "Widgets",
													spotlight: "Spotlight",
												}[surface]
											}
										</span>
									</label>
								),
							)}
						</div>
						<label
							htmlFor={favoriteId}
							className="flex items-center justify-between gap-4 text-sm"
						>
							<span>
								{t("nativeEventFavorite", "Show in Favorite Events widget")}
							</span>
							<Switch
								id={favoriteId}
								disabled={disabled || !settings.surfaces.includes("widget")}
								checked={settings.favorite}
								onCheckedChange={(favorite) =>
									onChange({ ...settings, favorite })
								}
							/>
						</label>
						<p className="text-xs text-muted-foreground">
							{t(
								"nativeEventCacheDescription",
								"The device shares the Event name, app name and app icon with these system surfaces. Shortcuts can receive the Event response. Supply required inputs in the Shortcut or open the app to complete its form.",
							)}
						</p>
					</>
				)}
			</CardContent>
		</Card>
	);
}

"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Check, Copy, ExternalLink } from "lucide-react";
import { useState } from "react";
import { Button, Input, Label } from "../../ui";
import type { IConfigInterfaceProps } from "../interfaces";

type ConfigPayload = Parameters<IConfigInterfaceProps["onConfigUpdate"]>[0];

export function DeeplinkConfig({
	isEditing,
	appId,
	config,
	onConfigUpdate,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const [copied, setCopied] = useState(false);

	const setRoute = (value: string) => {
		if (!onConfigUpdate) {
			return;
		}

		const baseConfig = { ...(config as Record<string, unknown>) };
		delete baseConfig.route;
		delete baseConfig.path;

		onConfigUpdate({
			...baseConfig,
			route: value,
		} as ConfigPayload);
	};

	const route = config?.route ?? config?.path ?? "";
	const deeplinkUrl = `flow-like://trigger/${appId}/${route}`;
	const exampleWithParams = `${deeplinkUrl}?param1=value1&param2=value2`;

	const copyToClipboard = (text: string) => {
		navigator.clipboard.writeText(text);
		setCopied(true);
		setTimeout(() => setCopied(false), 2000);
	};

	const openDeeplink = () => {
		window.location.href = deeplinkUrl;
	};

	return (
		<div className="w-full space-y-6">
			<div className="space-y-3">
				<Label htmlFor="route">{t("deepLinkRoute", "Deep Link Route")}</Label>
				{isEditing ? (
					<Input
						value={route}
						onChange={(e) => setRoute(e.target.value)}
						id="route"
						placeholder="my-trigger"
					/>
				) : (
					<div className="flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm">
						{route}
					</div>
				)}
				<p className="text-sm text-muted-foreground">
					{t(
						"theRouteSegmentThatIdentifiesThisTrigger",
						"The route segment that identifies this trigger",
					)}
				</p>
			</div>

			<div className="space-y-3">
				<Label>{t("deepLinkUrl", "Deep Link URL")}</Label>
				<div className="flex items-center gap-2">
					<div className="flex-1 flex h-10 w-full rounded-md border border-input bg-muted px-3 py-2 text-sm font-mono overflow-x-auto">
						{deeplinkUrl}
					</div>
					<Button
						variant="outline"
						size="icon"
						onClick={() => copyToClipboard(deeplinkUrl)}
						title={t("copyToClipboard", "Copy to clipboard")}
					>
						{copied ? (
							<Check className="h-4 w-4 text-green-500" />
						) : (
							<Copy className="h-4 w-4" />
						)}
					</Button>
					<Button
						variant="outline"
						size="icon"
						onClick={openDeeplink}
						title={t("testDeepLink", "Test deep link")}
					>
						<ExternalLink className="h-4 w-4" />
					</Button>
				</div>
				<p className="text-sm text-muted-foreground">
					{t(
						"clickTheLinkIconToTestTheDeepLinkTrigger",
						"Click the link icon to test the deep link trigger",
					)}
				</p>
			</div>

			<div className="space-y-3">
				<Label>{t("usageExamples", "Usage Examples")}</Label>
				<div className="space-y-4">
					<div>
						<p className="text-sm font-medium mb-2">
							{t("basicTrigger", "Basic Trigger")}
						</p>
						<code className="block p-3 bg-muted rounded-md text-sm font-mono break-all">
							{deeplinkUrl}
						</code>
					</div>

					<div>
						<p className="text-sm font-medium mb-2">
							{t("withQueryParameters", "With Query Parameters")}
						</p>
						<code className="block p-3 bg-muted rounded-md text-sm font-mono break-all">
							{exampleWithParams}
						</code>
						<p className="text-xs text-muted-foreground mt-2">
							{t(
								"queryParametersArePassedAsTheEventPayload",
								"Query parameters are passed as the event payload",
							)}
						</p>
					</div>

					<div>
						<p className="text-sm font-medium mb-2">
							{t("iosShortcuts", "iOS Shortcuts")}
						</p>
						<div className="p-3 bg-muted rounded-md text-sm space-y-2">
							<p className="text-muted-foreground">
								{t(
									"1AddAnOpenUrlsActionInShortcuts",
									'1. Add an "Open URLs" action in Shortcuts',
								)}
							</p>
							<p className="text-muted-foreground">
								{t("2SetTheUrlTo", "2. Set the URL to:")}{" "}
								<code className="font-mono">{deeplinkUrl}</code>
							</p>
							<p className="text-muted-foreground">
								{t(
									"3AddQueryParametersDynamicallyUsingShortcutVariables",
									"3. Add query parameters dynamically using Shortcut variables",
								)}
							</p>
						</div>
					</div>

					<div>
						<p className="text-sm font-medium mb-2">macOS/Linux/Windows</p>
						<div className="p-3 bg-muted rounded-md text-sm space-y-2">
							<p className="text-muted-foreground">
								{t("fromCommandLine", "From command line:")}
							</p>
							<code className="block mt-1 font-mono text-xs">
								<Trans i18nKey="macosBrOpenDeeplinkurlBrBrLinuxBrXdgopenDeeplinkurl2DeeplinkurlBrBrWindowsBrStartDeeplinkurl3Deeplinkurl">
									# macOS
									<br />
									open "{{ deeplinkUrl }}"<br />
									<br /># Linux
									<br />
									xdg-open "{{ deeplinkUrl2: deeplinkUrl }}"<br />
									<br /># Windows
									<br />
									start "{{ deeplinkUrl3: deeplinkUrl }}"
								</Trans>
							</code>
						</div>
					</div>
				</div>
			</div>

			<div className="rounded-lg border border-blue-200 bg-blue-50 dark:border-blue-900 dark:bg-blue-950 p-4">
				<h4 className="text-sm font-semibold text-blue-900 dark:text-blue-100 mb-2">
					{t("useCases", "💡 Use Cases")}
				</h4>
				<ul className="text-sm text-blue-800 dark:text-blue-200 space-y-1 list-disc list-inside">
					<li>{`Trigger flows from iOS/Android Shortcuts`}</li>
					<li>
						{t(
							"createLauncherIconsForQuickActions",
							"Create launcher icons for quick actions",
						)}
					</li>
					<li>
						{t(
							"integrateWithAutomationToolsAlfredRaycastEtc",
							"Integrate with automation tools (Alfred, Raycast, etc.)",
						)}
					</li>
					<li>
						{t(
							"buildCustomUrlSchemesForYourWorkflows",
							"Build custom URL schemes for your workflows",
						)}
					</li>
					<li>
						{t(
							"passDynamicDataThroughQueryParameters",
							"Pass dynamic data through query parameters",
						)}
					</li>
				</ul>
			</div>
		</div>
	);
}

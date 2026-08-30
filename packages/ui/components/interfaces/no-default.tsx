"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertTriangle, HelpCircle, Settings } from "lucide-react";
import Link from "next/link";
import { Alert, AlertDescription, Button, Card, CardContent } from "../ui";

export function NoDefaultInterface({
	appId,
	eventId,
}: Readonly<{ appId: string; eventId?: string }>) {
	const { t } = useTranslation("interfaces");
	return (
		<div className="flex flex-col h-full bg-muted/20 grow">
			<div className="flex-1 flex items-center justify-center p-8">
				<Card className="w-full max-w-md">
					<CardContent className="pt-6">
						<div className="flex flex-col items-center text-center space-y-6">
							{/* Icon */}
							<div className="relative">
								<div className="w-16 h-16 bg-muted rounded-full flex items-center justify-center">
									<Settings className="w-8 h-8 text-muted-foreground" />
								</div>
								<div className="absolute -top-1 -right-1 w-6 h-6 bg-yellow-100 rounded-full flex items-center justify-center">
									<AlertTriangle className="w-4 h-4 text-yellow-600" />
								</div>
							</div>

							{/* Heading */}
							<div className="space-y-2">
								<h3 className="text-lg font-semibold">
									{t("interfaceNotAvailable", "Interface Not Available")}
								</h3>
								<p className="text-sm text-muted-foreground">
									{t(
										"thisEventTypeDoesNotSupportACustomInterface",
										"This event type does not support a custom interface",
									)}
								</p>
							</div>

							{/* Alert */}
							<Alert className="w-full">
								<HelpCircle className="h-4 w-4" />
								<AlertDescription>
									{t(
										"eventTypesWithoutInterfaceSupportHandleDataAutomaticallyWithoutRequiringUserInteraction",
										"Event types without interface support handle data automatically without requiring user interaction.",
									)}
								</AlertDescription>
							</Alert>

							{/* Action Button */}
							<Link
								href={`/library/config/events?id=${appId}${eventId ? `&eventId=${eventId}` : ""}`}
								className="w-full"
							>
								<Button variant="outline" className="w-full">
									<Settings className="w-4 h-4 mr-2" />
									{t("configureEventSettings", "Configure Event Settings")}
								</Button>
							</Link>
						</div>
					</CardContent>
				</Card>
			</div>
		</div>
	);
}

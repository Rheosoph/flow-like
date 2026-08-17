"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertTriangle, Loader2, RefreshCw, WifiOff } from "lucide-react";
import { Alert, AlertDescription, Button, Card, CardContent } from "../ui";

export function InterfaceLoadError({
	message,
	offline = false,
	retrying,
	onRetry,
}: Readonly<{
	message?: string | null;
	offline?: boolean;
	retrying: boolean;
	onRetry: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	const Icon = offline ? WifiOff : AlertTriangle;

	return (
		<div className="flex flex-col h-full bg-muted/20 grow">
			<div className="flex-1 flex items-center justify-center p-8">
				<Card className="w-full max-w-md">
					<CardContent className="pt-6">
						<div className="flex flex-col items-center text-center space-y-6">
							<div className="w-16 h-16 bg-muted rounded-full flex items-center justify-center">
								<Icon className="w-8 h-8 text-muted-foreground" />
							</div>

							<div className="space-y-2">
								<h3 className="text-lg font-semibold">
									{offline
										? t('notAvailableOffline', 'Not Available Offline')
										: t('interfaceCouldNotBeLoaded', 'Interface Could Not Be Loaded')}
								</h3>
								<p className="text-sm text-muted-foreground">
									{offline
										? t('thisInterfaceHasNotBeenDownloadedToThisDeviceYetItOpensAutomaticallyOnceYouAreBackOnline', 'This interface has not been downloaded to this device yet. It opens automatically once you are back online.')
										: retrying
											? t('reconnectingToLoadThisInterface', 'Reconnecting to load this interface…')
											: t('thisInterfaceExistsButItsContentCouldNotBeFetchedOnThisDevice', 'This interface exists, but its content could not be fetched on this device.')}
								</p>
							</div>

							{message && !offline ? (
								<Alert className="w-full text-left">
									<AlertDescription className="wrap-break-word">
										{message}
									</AlertDescription>
								</Alert>
							) : null}

							<Button
								variant="outline"
								className="w-full"
								onClick={onRetry}
								disabled={retrying}
							>
								{retrying ? (
									<Loader2 className="w-4 h-4 mr-2 animate-spin" />
								) : (
									<RefreshCw className="w-4 h-4 mr-2" />
								)}
								{retrying ? "Retrying…" : "Try Again"}
							</Button>
						</div>
					</CardContent>
				</Card>
			</div>
		</div>
	);
}

"use client";

import {
	type IApiKeyState,
	type IApiState,
	type IAppRouteState,
	type IAppState,
	type IBackendState,
	type IBitState,
	type IBoardState,
	type ICapabilities,
	type IDatabaseState,
	type IEventState,
	type IGenericCommand,
	type IGraphState,
	type IHelperState,
	type IPageState,
	type IProfile,
	type IQueryState,
	type IRegistryState,
	type IRoleState,
	type ISalesState,
	type ISinkState,
	type IStorageState,
	type ITeamState,
	type ITemplateState,
	type IUsageState,
	type IUserState,
	type IWidgetState,
	LoadingScreen,
	type QueryClient,
	isAzureBlobStorageUrl,
	useBackendStore,
	useQueryClient,
} from "@flow-like/flow-like-ui";
import { setWidgetQueryResponder } from "@flow-like/flow-like-ui";
import type { ICommandSync } from "@flow-like/flow-like-ui/lib";
import type { IAIState } from "@flow-like/flow-like-ui/state/backend-state/ai-state";
import type { IAnalyticsState } from "@flow-like/flow-like-ui/state/backend-state/analytics-state";
import { i18n as i18next } from "@flow-like/locales";
import { useEffect } from "react";
import type { AuthContextProps } from "react-oidc-context";

import {
	WebAIState,
	WebApiKeyState,
	WebApiState,
	WebAppState,
	WebBitState,
	WebBoardState,
	WebDatabaseState,
	WebEventState,
	WebGraphState,
	WebHelperState,
	WebPageState,
	WebQueryState,
	WebRegistryState,
	WebRoleState,
	WebRouteState,
	WebSinkState,
	WebStorageState,
	WebTeamState,
	WebTemplateState,
	WebUsageState,
	WebUserState,
	WebWidgetState,
} from "@/lib/web-states";
import { WebAnalyticsState } from "@/lib/web-states/analytics-state";
import type { WebBackendRef } from "@/lib/web-states/api-utils";
import { WebSalesState } from "@/lib/web-states/sales-state";

export class WebBackend implements IBackendState {
	appState: IAppState;
	apiState: IApiState;
	apiKeyState: IApiKeyState;
	bitState: IBitState;
	boardState: IBoardState;
	eventState: IEventState;
	helperState: IHelperState;
	roleState: IRoleState;
	routeState: IAppRouteState;
	storageState: IStorageState;
	teamState: ITeamState;
	templateState: ITemplateState;
	userState: IUserState;
	aiState: IAIState;
	dbState: IDatabaseState;
	graphState: IGraphState;
	queryState: IQueryState;
	widgetState: IWidgetState;
	pageState: IPageState;
	registryState: IRegistryState;
	sinkState: ISinkState;
	salesState: ISalesState;
	usageState: IUsageState;
	analyticsState: IAnalyticsState;

	private backendRef: WebBackendRef;

	constructor(
		public readonly backgroundTaskHandler: (task: Promise<any>) => void,
		public queryClient?: QueryClient,
		public auth?: AuthContextProps,
		public profile?: IProfile,
	) {
		this.backendRef = { profile, auth, queryClient };

		this.apiState = new WebApiState(this.backendRef);
		this.apiKeyState = new WebApiKeyState(this.backendRef);
		this.appState = new WebAppState(this.backendRef);
		this.bitState = new WebBitState(this.backendRef);
		this.boardState = new WebBoardState(this.backendRef);
		this.eventState = new WebEventState(this.backendRef);
		this.helperState = new WebHelperState(this.backendRef);
		this.roleState = new WebRoleState(this.backendRef);
		this.routeState = new WebRouteState(this.backendRef);
		this.storageState = new WebStorageState(this.backendRef);
		this.teamState = new WebTeamState(this.backendRef);
		this.templateState = new WebTemplateState(this.backendRef);
		this.userState = new WebUserState(this.backendRef);
		this.aiState = new WebAIState(this.backendRef);
		this.dbState = new WebDatabaseState(this.backendRef);
		this.graphState = new WebGraphState(this.backendRef);
		this.queryState = new WebQueryState(this.backendRef);
		this.widgetState = new WebWidgetState(this.backendRef);
		this.pageState = new WebPageState(this.backendRef);
		this.registryState = new WebRegistryState(this.backendRef);
		this.sinkState = new WebSinkState(this.backendRef);
		this.salesState = new WebSalesState(this.backendRef);
		this.usageState = new WebUsageState(this.backendRef);
		this.analyticsState = new WebAnalyticsState(this.backendRef);
	}

	capabilities(): ICapabilities {
		return {
			needsSignIn: true,
			canHostLlamaCPP: false,
			canHostMLX: false,
			canHostEmbeddings: false,
			canExecuteLocally: false,
		};
	}

	pushProfile(profile: IProfile) {
		this.profile = profile;
		this.backendRef.profile = profile;
	}

	pushAuthContext(auth: AuthContextProps) {
		this.auth = auth;
		this.backendRef.auth = auth;
	}

	pushQueryClient(queryClient: QueryClient) {
		this.queryClient = queryClient;
		this.backendRef.queryClient = queryClient;
	}

	async isOffline(appId: string): Promise<boolean> {
		// For web, apps are always online
		return false;
	}

	async pushOfflineSyncCommand(
		appId: string,
		boardId: string,
		commands: IGenericCommand[],
	) {
		// Web apps are always online - no offline sync needed
	}

	async getOfflineSyncCommands(
		appId: string,
		boardId: string,
	): Promise<ICommandSync[]> {
		// Web apps are always online - no offline commands
		return [];
	}

	async clearOfflineSyncCommands(
		commandId: string,
		appId: string,
		boardId: string,
	): Promise<void> {
		// Web apps are always online - no offline commands to clear
	}

	async uploadSignedUrl(
		signedUrl: string,
		file: File,
		completedFiles: number,
		totalFiles: number,
		onProgress?: (progress: number) => void,
	): Promise<void> {
		await new Promise<void>((resolve, reject) => {
			const xhr = new XMLHttpRequest();

			xhr.upload.addEventListener("progress", (event) => {
				if (event.lengthComputable) {
					const fileProgress = event.loaded / event.total;
					const totalProgress =
						((completedFiles + fileProgress) / totalFiles) * 100;
					onProgress?.(totalProgress);
				}
			});

			xhr.addEventListener("load", () => {
				if (xhr.status >= 200 && xhr.status < 300) {
					resolve();
				} else {
					reject(
						new Error(
							i18next.t(
								"uploadFailedWithStatusStatusStatustext",
								"Upload failed with status {{status}}: {{statusText}}",
								{ status: xhr.status, statusText: xhr.statusText },
							),
						),
					);
				}
			});

			xhr.addEventListener("error", () => {
				reject(new Error("Upload failed: Network error (possible CORS issue)"));
			});

			xhr.open("PUT", signedUrl);
			xhr.setRequestHeader(
				"Content-Type",
				file.type || "application/octet-stream",
			);

			// Azure Blob Storage requires x-ms-blob-type header
			if (isAzureBlobStorageUrl(signedUrl)) {
				xhr.setRequestHeader("x-ms-blob-type", "BlockBlob");
			}

			xhr.send(file);
		});

		onProgress?.((completedFiles / totalFiles) * 100);
	}
}

export function WebProvider({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const queryClient = useQueryClient();
	const { backend, setBackend } = useBackendStore();

	useEffect(() => {
		if (backend && queryClient && backend instanceof WebBackend) {
			backend.pushQueryClient(queryClient);
		}
	}, [backend, queryClient]);

	useEffect(() => {
		const backend = new WebBackend((promise) => {
			promise
				.then((result) => {
					console.log("Background task completed:", result);
				})
				.catch((error) => {
					console.error("Background task failed:", error);
				});
		}, queryClient);

		setWidgetQueryResponder((requestId, response) =>
			backend.boardState.respondWidgetQuery
				? backend.boardState.respondWidgetQuery(requestId, response)
				: Promise.resolve(false),
		);
		setBackend(backend);
		return () => setWidgetQueryResponder(null);
	}, [queryClient, setBackend]);

	if (!backend) {
		return <LoadingScreen progress={50} />;
	}

	return <>{children}</>;
}

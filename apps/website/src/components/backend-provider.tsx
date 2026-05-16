import {
	EmptyAIState,
	EmptyApiKeyState,
	EmptyApiState,
	EmptyAppState,
	EmptyBitState,
	EmptyBoardState,
	EmptyDatabaseState,
	EmptyEventState,
	EmptyGraphState,
	EmptyHelperState,
	EmptyRoleState,
	EmptyRouteState,
	EmptyStorageState,
	EmptyTeamState,
	EmptyTemplateState,
	EmptyUserState,
	type IAIState,
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
	type IGraphState,
	type IHelperState,
	type IPageState,
	type IRegistryState,
	type IRoleState,
	type IStorageState,
	type ITeamState,
	type ITemplateState,
	type IUserState,
	type IWidgetState,
	LoadingScreen,
	ThemeProvider,
	useBackendStore,
} from "@tm9657/flow-like-ui";
import { Suspense, lazy, useEffect, useState } from "react";

const BoardWrapper = lazy(() => import("./board-wrapper"));

function unavailableState<T>(name: string): T {
	return new Proxy(
		{},
		{
			get: () => {
				throw new Error(`${name} is not available in EmptyBackendProvider`);
			},
		},
	) as T;
}

export class EmptyBackend implements IBackendState {
	aiState: IAIState = new EmptyAIState();
	apiState: IApiState = new EmptyApiState();
	apiKeyState: IApiKeyState = new EmptyApiKeyState();
	appState: IAppState = new EmptyAppState();
	bitState: IBitState = new EmptyBitState();
	boardState: IBoardState = new EmptyBoardState();
	eventState: IEventState = new EmptyEventState();
	helperState: IHelperState = new EmptyHelperState();
	roleState: IRoleState = new EmptyRoleState();
	storageState: IStorageState = new EmptyStorageState();
	teamState: ITeamState = new EmptyTeamState();
	templateState: ITemplateState = new EmptyTemplateState();
	userState: IUserState = new EmptyUserState();
	dbState: IDatabaseState = new EmptyDatabaseState();
	graphState: IGraphState = new EmptyGraphState();
	widgetState: IWidgetState = unavailableState<IWidgetState>("WidgetState");
	pageState: IPageState = unavailableState<IPageState>("PageState");
	routeState: IAppRouteState = new EmptyRouteState();
	registryState: IRegistryState =
		unavailableState<IRegistryState>("RegistryState");

	capabilities(): ICapabilities {
		return {
			needsSignIn: false,
			canHostLlamaCPP: false,
			canHostEmbeddings: false,
			canExecuteLocally: false,
		};
	}

	async isOffline(_appId: string): Promise<boolean> {
		return false;
	}
}

export function EmptyBackendProvider({ data }: Readonly<{ data: string }>) {
	const [nodes, setNodes] = useState<any[]>([]);
	const [edges, setEdges] = useState<any[]>([]);
	const [loaded, setLoaded] = useState(false);
	const { setBackend } = useBackendStore();

	useEffect(() => {
		let cancelled = false;

		(async () => {
			try {
				const response = await fetch(data);
				if (!response.ok) {
					throw new Error(`Failed to load board data: ${response.status}`);
				}
				const json = await response.json();
				const nextNodes = Array.isArray(json?.nodes) ? json.nodes : [];
				const nextEdges = Array.isArray(json?.edges) ? json.edges : [];

				if (cancelled) return;

				setNodes(nextNodes);
				setEdges(nextEdges);
				setBackend(new EmptyBackend());
				setLoaded(true);
			} catch (error) {
				console.error("Failed to initialize EmptyBackendProvider", error);
				if (cancelled) return;

				setNodes([]);
				setEdges([]);
				setBackend(new EmptyBackend());
				setLoaded(true);
			}
		})();

		return () => {
			cancelled = true;
		};
	}, [data, setBackend]);

	if (!loaded) {
		return (
			<ThemeProvider
				attribute="class"
				defaultTheme="dark"
				enableSystem
				disableTransitionOnChange
			>
				<LoadingScreen className="absolute top-0 left-0 right-0 bottom-0" />
			</ThemeProvider>
		);
	}

	return (
		<ThemeProvider
			attribute="class"
			defaultTheme="dark"
			enableSystem
			disableTransitionOnChange
		>
			<Suspense
				fallback={
					<LoadingScreen className="absolute top-0 left-0 right-0 bottom-0" />
				}
			>
				<BoardWrapper nodes={nodes} edges={edges} />
			</Suspense>
		</ThemeProvider>
	);
}

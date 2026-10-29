import {
	configureOAuthSession,
	oauthConsentStore,
	oauthTokenStore,
} from "@flow-like/flow-like-ui/db/oauth-db";
import { getApiOrigin } from "@flow-like/flow-like-ui/lib/api-url";
import { checkOAuthTokensFromPrerun } from "@flow-like/flow-like-ui/lib/oauth/helpers";
import { oauthSessionKey } from "@flow-like/flow-like-ui/lib/oauth/scope";
import {
	type OAuthService,
	createOAuthService,
} from "@flow-like/flow-like-ui/lib/oauth/service";
import type {
	IOAuthProvider,
	IOAuthRuntime,
	IOAuthToken,
} from "@flow-like/flow-like-ui/lib/oauth/types";
import {
	HUB_REFRESH_TIMEOUT_MS,
	withRequestDeadline,
} from "@flow-like/flow-like-ui/lib/request-deadline";
import type { IHub } from "@flow-like/flow-like-ui/lib/schema/hub/hub";
import type { IOAuthRequirement } from "@flow-like/flow-like-ui/state/backend-state/types";
import type { AuthContextProps } from "react-oidc-context";
import {
	type OAuthAccount,
	createWebOAuthRuntime,
	oauthAccountSession,
	webOAuthPlatform,
} from "./web-oauth-runtime";

export type HostedOAuthDecision = "connect" | "skip";

/** A connect prompt waiting for the visitor. */
export interface HostedOAuthRequest {
	readonly id: number;
	readonly providers: IOAuthProvider[];
	/** Providers connected in this browser already, waiting only for the visitor's go-ahead. */
	readonly connected: Set<string>;
	answer(decision: HostedOAuthDecision, remember?: boolean): void;
}

/** The Hub's provider settings turn a run's requirements into providers to connect. */
async function fetchHubConfig(): Promise<IHub> {
	const url = `${getApiOrigin()}/api/v1`;
	return withRequestDeadline(
		url,
		async ({ signal }) => {
			const response = await fetch(url, { signal, credentials: "omit" });
			if (!response.ok)
				throw new Error(
					`Loading the Hub configuration from ${url} failed (${response.status}).`,
				);
			return (await response.json()) as IHub;
		},
		{ timeoutMs: HUB_REFRESH_TIMEOUT_MS },
	);
}

/**
 * A signed-in visitor's own OAuth sign-ins for the runs of one hosted interface. A run carries
 * them once every provider it needs is connected and approved. A skipped prompt, or a Hub that
 * offers none of the providers, leaves the run on the Event's saved sign-ins.
 */
export class HostedOAuth {
	readonly runtime: IOAuthRuntime;
	readonly service: OAuthService;
	private readonly approved = new Set<string>();
	private readonly skipped = new Set<string>();
	private readonly listeners = new Set<() => void>();
	private request?: HostedOAuthRequest;
	private asked = 0;
	private queue: Promise<unknown> = Promise.resolve();
	private hub?: Promise<IHub | undefined>;

	constructor(
		readonly appId: string,
		private readonly auth: () => AuthContextProps | undefined,
	) {
		this.runtime = createWebOAuthRuntime(() => this.account());
		this.service = createOAuthService({
			runtime: this.runtime,
			tokenStore: oauthTokenStore,
			getApiBaseUrl: async () => getApiOrigin(),
			platform: webOAuthPlatform(),
		});
	}

	private account(): OAuthAccount {
		return { hub: getApiOrigin(), auth: this.auth() };
	}

	sessionKey(): string | undefined {
		const session = oauthAccountSession(this.account());
		return session && oauthSessionKey(session);
	}

	/** Scopes the browser's OAuth store to the visitor until released. */
	open(): () => void {
		return configureOAuthSession(() => oauthAccountSession(this.account()));
	}

	/** The visitor's tokens for a run, asking first when it must. Runs ask one at a time. */
	readonly tokensFor = (
		requirements: IOAuthRequirement[],
	): Promise<Record<string, IOAuthToken> | undefined> => {
		const tokens = this.queue.then(() => this.resolve(requirements));
		this.queue = tokens.catch(() => undefined);
		return tokens;
	};

	readonly subscribe = (listener: () => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	readonly pending = (): HostedOAuthRequest | undefined => this.request;

	private async resolve(requirements: IOAuthRequirement[]) {
		if (requirements.length === 0 || !this.sessionKey()) return undefined;
		const hub = await this.hubConfig();
		for (;;) {
			const { tokens, waiting } = await this.collect(requirements, hub);
			if (waiting.length === 0)
				return Object.keys(tokens).length > 0 ? tokens : undefined;
			if (waiting.every((provider) => this.skipped.has(provider.id)))
				return undefined;
			if ((await this.ask(waiting, tokens)) === "skip") return undefined;
		}
	}

	/** Tokens in this browser, and the providers still waiting for a sign-in or the visitor's go-ahead. */
	private async collect(requirements: IOAuthRequirement[], hub?: IHub) {
		const { tokens, requiredProviders } = await checkOAuthTokensFromPrerun(
			requirements,
			oauthTokenStore,
			hub,
			{ refreshToken: this.service.refreshToken.bind(this.service) },
		);
		const consented = await oauthConsentStore.getConsentedProviderIds(
			this.appId,
		);
		const ready = (provider: IOAuthProvider) =>
			tokens[provider.id] !== undefined &&
			(consented.has(provider.id) || this.approved.has(provider.id));
		return {
			tokens,
			waiting: requiredProviders.filter((provider) => !ready(provider)),
		};
	}

	private async ask(
		providers: IOAuthProvider[],
		tokens: Record<string, IOAuthToken>,
	): Promise<HostedOAuthDecision> {
		const { decision, remember } = await new Promise<{
			decision: HostedOAuthDecision;
			remember: boolean;
		}>((settle) => {
			const request: HostedOAuthRequest = {
				id: ++this.asked,
				providers,
				connected: new Set(
					providers
						.filter((provider) => tokens[provider.id] !== undefined)
						.map((provider) => provider.id),
				),
				answer: (decision, remember = false) => {
					if (this.request !== request) return;
					this.show(undefined);
					settle({ decision, remember });
				},
			};
			this.show(request);
		});
		const [chosen, dropped] =
			decision === "skip"
				? [this.skipped, this.approved]
				: [this.approved, this.skipped];
		for (const provider of providers) {
			chosen.add(provider.id);
			dropped.delete(provider.id);
		}
		if (decision === "connect" && remember) await this.remember(providers);
		return decision;
	}

	private show(request: HostedOAuthRequest | undefined) {
		this.request = request;
		for (const listener of this.listeners) listener();
	}

	/** "Don't ask again" keeps the go-ahead for this app in the browser, as the app itself does. */
	private async remember(providers: IOAuthProvider[]) {
		for (const provider of providers) {
			try {
				await oauthConsentStore.setConsent(
					this.appId,
					provider.id,
					provider.merged_scopes ?? provider.scopes,
				);
			} catch (error) {
				console.warn(
					`[Hosted OAuth] The go-ahead for ${provider.id} in app ${this.appId} holds for this visit only: it could not be saved.`,
					error,
				);
			}
		}
	}

	private hubConfig(): Promise<IHub | undefined> {
		this.hub ??= fetchHubConfig().catch((error: unknown) => {
			this.hub = undefined;
			console.warn(
				"[Hosted OAuth] The Hub's OAuth providers could not be loaded, so this run uses the app's saved sign-ins.",
				error,
			);
			return undefined;
		});
		return this.hub;
	}
}

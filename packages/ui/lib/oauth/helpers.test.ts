import { describe, expect, it } from "vitest";
import type { IBoard, IHub, ILayer, INode } from "../schema";
import {
	extractOAuthProvidersFromBoard,
	extractOAuthRequirementsFromBoard,
} from "./helpers";

const serviceScopes = [
	"drive.file",
	"gmail.send",
	"gmail.labels",
	"calendar.readonly",
	"calendar.events",
	"documents",
	"documents.readonly",
	"spreadsheets",
	"spreadsheets.readonly",
	"presentations",
	"presentations.readonly",
	"forms.body",
	"forms.body.readonly",
	"forms.responses.readonly",
	"youtube",
	"youtube.readonly",
].map((scope) => `https://www.googleapis.com/auth/${scope}`);

const oauthProviders: IHub["oauth_providers"] = {
	google: {
		name: "Google",
		auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
		token_url: "https://oauth2.googleapis.com/token",
		scopes: ["openid", "email", "profile"],
	},
};
const hub = { oauth_providers: oauthProviders } as IHub;

function node(id: string, fields: Partial<INode> = {}): INode {
	return { id, ...fields } as INode;
}

function board(nodes: INode[], layers: ILayer[] = []): IBoard {
	return {
		nodes: Object.fromEntries(nodes.map((value) => [value.id, value])),
		layers: Object.fromEntries(layers.map((value) => [value.id, value])),
		execution_mode: "Remote",
	} as IBoard;
}

function layer(id: string, nodes: INode[]): ILayer {
	return {
		id,
		type: "Function",
		nodes: Object.fromEntries(nodes.map((value) => [value.id, value])),
	} as ILayer;
}

const provider = node("provider", { oauth_providers: ["google"] });
const serviceNodes = serviceScopes.map((scope, i) =>
	node(`service-${i}`, {
		layer: "verification-function",
		required_oauth_scopes: { google: [scope] },
	}),
);

function expectAllScopes(value: IBoard) {
	const requirements = extractOAuthRequirementsFromBoard(value);
	expect(requirements.oauth_requirements).toHaveLength(1);
	expect(requirements.oauth_requirements[0].provider_id).toBe("google");
	expect(requirements.oauth_requirements[0].scopes.toSorted()).toEqual(
		serviceScopes.toSorted(),
	);
	const providers = extractOAuthProvidersFromBoard(value, hub);
	expect(providers).toHaveLength(1);
	expect(providers[0].merged_scopes?.toSorted()).toEqual(
		["openid", "email", "profile", ...serviceScopes].toSorted(),
	);
}

describe("OAuth requirement collection", () => {
	it.each([false, true])(
		"finds every scope with provider first = %s, including flattened function nodes",
		(providerFirst) => {
			expectAllScopes(
				board(
					providerFirst
						? [provider, ...serviceNodes]
						: [...serviceNodes, provider],
				),
			);
		},
	);

	it("collects root and function scopes when the provider is in a later layer", () => {
		expectAllScopes(
			board(serviceNodes.slice(0, 8), [
				layer("verification", serviceNodes.slice(8)),
				layer("connection", [provider]),
			]),
		);
	});

	it("does not require OAuth for service nodes using other credentials", () => {
		const value = board(serviceNodes);
		expect(extractOAuthRequirementsFromBoard(value).oauth_requirements).toEqual(
			[],
		);
		expect(extractOAuthProvidersFromBoard(value, hub)).toEqual([]);
	});

	it("merges protobuf scopes and deduplicates without registering unrelated providers", () => {
		const value = board([
			node("service", {
				required_oauth_scopes: {
					google: { values: [serviceScopes[0], serviceScopes[0]] },
					github: ["repo"],
				},
			}),
			provider,
		]);
		expect(extractOAuthRequirementsFromBoard(value).oauth_requirements).toEqual(
			[{ provider_id: "google", scopes: [serviceScopes[0]] }],
		);
		expect(extractOAuthProvidersFromBoard(value, hub)[0].merged_scopes).toEqual(
			["openid", "email", "profile", serviceScopes[0]],
		);
	});

	it("keeps a provider with no extra scopes and detects offline nodes in layers", () => {
		const value = board(
			[provider],
			[layer("local", [node("offline", { only_offline: true })])],
		);
		expect(extractOAuthRequirementsFromBoard(value)).toEqual({
			oauth_requirements: [{ provider_id: "google", scopes: [] }],
			requires_local_execution: true,
			execution_mode: "Remote",
			can_execute_locally: true,
		});
	});
});

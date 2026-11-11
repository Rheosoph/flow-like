"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import {
	useLookupDisplayName,
	userLookupQueryOptions,
} from "../../hooks/use-user-lookup";
import {
	SCOPE_ALL,
	SCOPE_DOCUMENTS_EXPENSIVE,
} from "../../lib/tool-grant-scopes";
import { useBackend } from "../../state/backend-state";

const PERSON_PREFIX = "person:";

function toolName(tool: string): string {
	return tool.replace(/_/g, " ");
}

function after(scope: string, prefix: string): string {
	return scope.slice(prefix.length);
}

/** The people `person:<id>` scopes name, by id, as far as the directory has answered. */
function usePersonNames(
	scopes: readonly string[],
): ReadonlyMap<string, string> {
	const source = useBackend().userState;
	const displayName = useLookupDisplayName();
	const key = scopes.join("\u0000");
	const ids = useMemo(
		() => [
			...new Set(
				key
					.split("\u0000")
					.filter((scope) => scope.startsWith(PERSON_PREFIX))
					.map((scope) => after(scope, PERSON_PREFIX)),
			),
		],
		[key],
	);
	const results = useQueries({
		queries: ids.map((id) => userLookupQueryOptions(source, id)),
	});
	const names = new Map<string, string>();
	ids.forEach((id, index) => {
		const user = results[index]?.data;
		if (user) names.set(id, displayName(user, id));
	});
	return names;
}

/**
 * A grant or approval scope in plain words, e.g. `command:git` as "Run `git` on the command line".
 * Pass the scopes you will label so the people they name (`person:<id>`) can be looked up.
 */
export function useGrantLabel(scopes: readonly string[] = []) {
	const { t } = useTranslation("bots");
	const people = usePersonNames(scopes);
	return useCallback(
		(tool: string, scope: string): string => {
			if (scope.startsWith(PERSON_PREFIX)) {
				const name = people.get(after(scope, PERSON_PREFIX));
				return name
					? t("grants.person", "Message {{name}}", { name })
					: t("grants.personUnknown", "Message a person you chat with");
			}
			if (scope === SCOPE_ALL) {
				return t("grants.tool", "Use {{tool}}", { tool: toolName(tool) });
			}
			if (scope === SCOPE_DOCUMENTS_EXPENSIVE) {
				return t("grants.documents", "Process large documents");
			}
			if (scope === "computer:session") {
				return t("grants.computer", "Use the computer's screen and mouse");
			}
			if (scope.startsWith("command:")) {
				return t("grants.command", "Run `{{program}}` on the command line", {
					program: after(scope, "command:"),
				});
			}
			if (scope.startsWith("email:")) {
				return t("grants.email", "Send email to {{domain}}", {
					domain: after(scope, "email:"),
				});
			}
			if (scope.startsWith("owner_browser:")) {
				return t(
					"grants.ownerBrowser",
					"Open sites on {{site}} in your own browser",
					{ site: after(scope, "owner_browser:") },
				);
			}
			if (scope.startsWith("browser:")) {
				return t("grants.browser", "Open sites on {{site}}", {
					site: after(scope, "browser:"),
				});
			}
			if (scope.startsWith("host:")) {
				return t("grants.host", "Run code that reaches {{host}}", {
					host: after(scope, "host:"),
				});
			}
			if (scope.startsWith("app:")) {
				return t("grants.app", "Use {{tool}} in app {{app}}", {
					tool: toolName(tool),
					app: after(scope, "app:"),
				});
			}
			if (scope.startsWith("table:")) {
				return t("grants.table", "Use {{tool}} on table {{table}}", {
					tool: toolName(tool),
					table: after(scope, "table:"),
				});
			}
			if (scope.startsWith("target:")) {
				const [app, kind, ...rest] = after(scope, "target:").split(":");
				const id = rest.join(":");
				return kind === "event"
					? t("grants.event", "Run event {{id}} of app {{app}}", { id, app })
					: t("grants.page", "Use page {{id}} of app {{app}}", { id, app });
			}
			if (scope.startsWith("connection:")) {
				const [kind, host, , database] = after(scope, "connection:").split(":");
				return t(
					"grants.connection",
					"Attach {{kind}} database {{database}} on {{host}}",
					{
						kind,
						database,
						host,
					},
				);
			}
			return t("grants.other", "{{tool}}: {{scope}}", {
				tool: toolName(tool),
				scope,
			});
		},
		[t, people],
	);
}

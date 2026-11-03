"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback } from "react";
import {
	SCOPE_ALL,
	SCOPE_DOCUMENTS_EXPENSIVE,
} from "../../lib/tool-grant-scopes";

function toolName(tool: string): string {
	return tool.replace(/_/g, " ");
}

function after(scope: string, prefix: string): string {
	return scope.slice(prefix.length);
}

/** A grant or approval scope in plain words, e.g. `command:git` as "Run `git` on the command line". */
export function useGrantLabel() {
	const { t } = useTranslation("bots");
	return useCallback(
		(tool: string, scope: string): string => {
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
		[t],
	);
}

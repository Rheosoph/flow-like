"use client";

import { SOURCE_RESOURCES, useTranslation } from "@flow-like/locales";
import { useCallback } from "react";
import type { IAuditRecordView } from "./types";

export type AuditActorKind =
	| "user"
	| "technicalUser"
	| "apiKey"
	| "system"
	| "executor";

export interface IAuditActor {
	kind: AuditActorKind;
	/** How the actor authenticated: `openid`, `pat`, `api_key`, `executor`, … */
	method?: string;
	/** The account behind the actor, when the actor id names one. */
	userId?: string;
	/** Token, key or run id the actor used. */
	reference?: string;
	raw: string;
}

/** Methods whose middle segment is an account id rather than an app or system name. */
const ACCOUNT_METHODS = new Set([
	"openid",
	"pat",
	"api_key",
	"executor",
	"fork",
]);

/** Accepts both the database spelling (`TECHNICAL_USER`) and serde's (`TechnicalUser`). */
export function auditActorKind(actorType: string): AuditActorKind {
	switch (actorType.replace(/[^a-z]/gi, "").toLowerCase()) {
		case "user":
			return "user";
		case "technicaluser":
			return "technicalUser";
		case "apikey":
			return "apiKey";
		case "executor":
			return "executor";
		default:
			return "system";
	}
}

/** Actor ids have the shape `method:subject:reference`; the subject may itself contain `:`. */
export function parseAuditActor(
	actorId: string,
	actorType: string,
): IAuditActor {
	const kind = auditActorKind(actorType);
	const parts = actorId.split(":");
	if (parts.length < 3) return { kind, raw: actorId };
	const method = parts[0];
	const subject = parts.slice(1, -1).join(":");
	const reference = parts.at(-1) || undefined;
	const userId =
		ACCOUNT_METHODS.has(method) && subject && !subject.startsWith("app:")
			? subject
			: undefined;
	return { kind, method, userId, reference, raw: actorId };
}

export function auditActionKey(action: string): string {
	return action.replaceAll(".", "_");
}

export function humanizeAuditAction(action: string): string {
	return action.replace(/[._]+/g, " ").trim();
}

function formatDetailValue(value: unknown): string {
	if (typeof value === "string") return value;
	if (value === null || value === undefined) return "null";
	if (typeof value === "object") return JSON.stringify(value);
	return String(value);
}

/** Details hold ids, counts and short codes only, so a flat list reads fine. */
export function auditDetailEntries(details: unknown): Array<[string, string]> {
	if (details === null || details === undefined) return [];
	if (typeof details !== "object" || Array.isArray(details)) {
		return [["value", formatDetailValue(details)]];
	}
	return Object.entries(details as Record<string, unknown>).map(
		([key, value]) => [key, formatDetailValue(value)],
	);
}

const ACTION_SENTENCES: Readonly<Record<string, string>> =
	SOURCE_RESOURCES.audit.actions;

/**
 * The sentence that follows the actor's name. Interpolation values sit under
 * `details` so a detail named like an i18next option (`count`, `context`) can
 * never change how the key resolves.
 */
export function useAuditSentence() {
	const { t } = useTranslation("audit");
	return useCallback(
		(
			record: Pick<
				IAuditRecordView,
				"action" | "resource_type" | "resource_id" | "details"
			>,
		): string => {
			const key = auditActionKey(record.action);
			if (!Object.hasOwn(ACTION_SENTENCES, key)) {
				return humanizeAuditAction(record.action);
			}
			const sentenceKey: string = `actions.${key}`;
			return t(sentenceKey, {
				resource_type: record.resource_type,
				resource_id: record.resource_id,
				details: record.details ?? {},
			});
		},
		[t],
	);
}

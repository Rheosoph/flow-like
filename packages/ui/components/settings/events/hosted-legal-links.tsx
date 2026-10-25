"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { safeExternalHref } from "../../../lib/legal-links";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";

interface LegalDraft {
	notice: string;
	privacy: string;
}

const isInvalidLink = (value: string) =>
	value.trim() !== "" && !safeExternalHref(value.trim());

/**
 * App-level links every hosted route of the app shows. Saved on their own,
 * not with the event form, because they belong to the app, not this event.
 */
export function HostedLegalLinks({
	appId,
	canWrite,
}: Readonly<{ appId: string; canWrite: boolean }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const stored = useInvoke(
		backend.appState.getAppLegalLinks,
		backend.appState,
		[appId],
	);
	const [draft, setDraft] = useState<LegalDraft | null>(null);
	const [saving, setSaving] = useState(false);

	if (!stored.isSuccess) return null;
	const saved: LegalDraft = {
		notice: stored.data.legal_notice_url ?? "",
		privacy: stored.data.privacy_policy_url ?? "",
	};
	const current = draft ?? saved;
	const dirty =
		current.notice.trim() !== saved.notice ||
		current.privacy.trim() !== saved.privacy;
	const invalid =
		isInvalidLink(current.notice) || isInvalidLink(current.privacy);
	const invalidHint = t(
		"hostingLegalInvalidUrl",
		"Enter a full link starting with https://",
	);

	const save = async () => {
		setSaving(true);
		try {
			await backend.appState.setAppLegalLinks(appId, {
				legal_notice_url: current.notice.trim(),
				privacy_policy_url: current.privacy.trim(),
			});
			await stored.refetch();
			setDraft(null);
			toast.success(t("hostingLegalSaved", "Legal links saved"));
		} catch (error) {
			toast.error(
				t("hostingLegalSaveFailed", "Could not save the legal links"),
				{ description: error instanceof Error ? error.message : undefined },
			);
		} finally {
			setSaving(false);
		}
	};

	const field = (
		key: keyof LegalDraft,
		id: string,
		label: string,
		placeholder: string,
	) => (
		<div className="space-y-1.5">
			<Label htmlFor={id}>{label}</Label>
			<Input
				id={id}
				type="url"
				inputMode="url"
				value={current[key]}
				placeholder={placeholder}
				disabled={!canWrite || saving}
				aria-invalid={isInvalidLink(current[key])}
				onChange={(event) =>
					setDraft({ ...current, [key]: event.target.value })
				}
			/>
			{isInvalidLink(current[key]) && (
				<p className="text-xs text-destructive">{invalidHint}</p>
			)}
		</div>
	);

	return (
		<div className="space-y-4 border-t pt-5">
			<div className="space-y-1">
				<p className="text-sm font-medium">
					{t("hostingLegalTitle", "Legal links")}
				</p>
				<p className="text-sm text-muted-foreground">
					{t(
						"hostingLegalDescription",
						"Shown behind a small info button on every hosted route of this app. When you publish to the public, you are the provider of these pages and may need your own legal notice and privacy policy.",
					)}
				</p>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				{field(
					"notice",
					"hosted-legal-notice-url",
					t("hostingLegalNoticeUrl", "Legal notice URL"),
					"https://example.com/imprint",
				)}
				{field(
					"privacy",
					"hosted-privacy-policy-url",
					t("hostingPrivacyPolicyUrl", "Privacy policy URL"),
					"https://example.com/privacy",
				)}
			</div>
			{canWrite && (
				<div className="flex justify-end gap-2">
					{dirty && (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							disabled={saving}
							onClick={() => setDraft(null)}
						>
							{t("cancel", "Cancel")}
						</Button>
					)}
					<Button
						type="button"
						size="sm"
						disabled={!dirty || invalid || saving}
						onClick={() => void save()}
					>
						{t("hostingLegalSave", "Save links")}
					</Button>
				</div>
			)}
		</div>
	);
}

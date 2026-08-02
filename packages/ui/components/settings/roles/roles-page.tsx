"use client";

import { createId } from "@paralleldrive/cuid2";
import { Plus, SearchIcon, Shield } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { useInfiniteInvoke, useInvoke } from "../../../hooks/use-invoke";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { useBackend } from "../../../state/backend-state";
import type { IBackendRole } from "../../../state/backend-state/types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { type RoleTemplate, permissionsFromTemplate } from "./access-ladders";
import { RoleEditor } from "./role-editor";
import { LadderKey, RoleRow } from "./role-row";
import { TemplatePicker } from "./role-templates";

function emptyRole(appId: string, template: RoleTemplate): IBackendRole {
	const now = new Date().toISOString().replace("Z", "");
	return {
		id: createId(),
		app_id: appId,
		name: template.name,
		description: template.description,
		permissions: permissionsFromTemplate(template).toBigInt(),
		attributes: [],
		created_at: now,
		updated_at: now,
	};
}

export function RolesPage() {
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const backend = useBackend();
	const enabled = appId.length > 0;

	const roles = useInvoke(
		backend.roleState.getRoles,
		backend.roleState,
		[appId],
		enabled,
	);
	const team = useInfiniteInvoke(
		backend.teamState.getTeam,
		backend.teamState,
		[appId],
		50,
		enabled,
	);

	const [openRoleId, setOpenRoleId] = useState<string | undefined>();
	const [draft, setDraft] = useState<IBackendRole | undefined>();
	const [isNewRole, setIsNewRole] = useState(false);
	const [showTemplates, setShowTemplates] = useState(false);
	const [searchTerm, setSearchTerm] = useState("");

	/**
	 * Only reported when the team query actually succeeded — a viewer without
	 * ReadTeam would otherwise see every role claiming zero members.
	 */
	const memberCounts = useMemo(() => {
		if (team.isError || !team.data) return undefined;
		const counts = new Map<string, number>();
		for (const member of team.data.pages.flat()) {
			counts.set(member.role_id, (counts.get(member.role_id) ?? 0) + 1);
		}
		return counts;
	}, [team.data, team.isError]);

	const { visibleRoles, defaultRoleId, knownAttributes } = useMemo(() => {
		const persisted = roles.data?.[1] ?? [];
		const all = isNewRole && draft ? [...persisted, draft] : persisted;
		const term = searchTerm.toLowerCase();
		const filtered = all.filter(
			(role) =>
				role.name.toLowerCase().includes(term) ||
				role.description.toLowerCase().includes(term),
		);
		const sorted = filtered.toSorted((a, b) => {
			const permA = new RolePermissions(BigInt(a.permissions));
			const permB = new RolePermissions(BigInt(b.permissions));
			if (permA.contains(RolePermissions.Owner)) return -1;
			if (permB.contains(RolePermissions.Owner)) return 1;
			if (permA.contains(RolePermissions.Admin)) return -1;
			if (permB.contains(RolePermissions.Admin)) return 1;
			return a.name.localeCompare(b.name);
		});
		return {
			visibleRoles: sorted,
			defaultRoleId: roles.data?.[0],
			knownAttributes: [
				...new Set(all.flatMap((role) => role.attributes ?? [])),
			].toSorted(),
		};
	}, [roles.data, searchTerm, draft, isNewRole]);

	const persistedRole = useMemo(
		() => roles.data?.[1]?.find((role) => role.id === openRoleId),
		[roles.data, openRoleId],
	);
	const isDirty = useMemo(() => {
		if (!draft) return false;
		if (isNewRole) return true;
		if (!persistedRole) return false;
		return (
			draft.name !== persistedRole.name ||
			draft.description !== persistedRole.description ||
			BigInt(draft.permissions) !== BigInt(persistedRole.permissions) ||
			JSON.stringify(draft.attributes ?? []) !==
				JSON.stringify(persistedRole.attributes ?? [])
		);
	}, [draft, persistedRole, isNewRole]);

	const closeDraft = useCallback(() => {
		setOpenRoleId(undefined);
		setDraft(undefined);
		setIsNewRole(false);
	}, []);

	const toggleRole = useCallback(
		(role: IBackendRole) => {
			if (openRoleId === role.id) {
				if (isDirty && !confirm("Discard unsaved changes to this role?"))
					return;
				closeDraft();
				return;
			}
			if (isDirty && !confirm("Discard unsaved changes to this role?")) return;
			setIsNewRole(false);
			setOpenRoleId(role.id);
			setDraft({ ...role, attributes: [...(role.attributes ?? [])] });
		},
		[openRoleId, isDirty, closeDraft],
	);

	const createFromTemplate = useCallback(
		(template: RoleTemplate) => {
			if (!appId) return;
			const role = emptyRole(appId, template);
			setShowTemplates(false);
			setIsNewRole(true);
			setOpenRoleId(role.id);
			setDraft(role);
		},
		[appId],
	);

	const handleSave = useCallback(async () => {
		if (!appId || !draft) return;
		await backend.roleState.upsertRole(appId, { ...draft, app_id: appId });
		await roles.refetch();
		setIsNewRole(false);
	}, [appId, backend, draft, roles]);

	const handleDuplicate = useCallback(
		async (role: IBackendRole) => {
			if (!appId) return;
			const cleaned = new RolePermissions(BigInt(role.permissions))
				.remove(RolePermissions.Owner)
				.remove(RolePermissions.Admin);
			await backend.roleState.upsertRole(appId, {
				...role,
				id: createId(),
				name: `${role.name} (Copy)`,
				permissions: cleaned.toBigInt(),
			});
			closeDraft();
			await roles.refetch();
		},
		[appId, backend, roles, closeDraft],
	);

	const handleDelete = useCallback(
		async (roleId: string) => {
			if (!appId) return;
			if (isNewRole) {
				closeDraft();
				return;
			}
			await backend.roleState.deleteRole(appId, roleId);
			closeDraft();
			await roles.refetch();
		},
		[appId, backend, roles, isNewRole, closeDraft],
	);

	const handleSetDefault = useCallback(
		async (roleId: string) => {
			if (!appId) return;
			await backend.roleState.makeRoleDefault(appId, roleId);
			await roles.refetch();
		},
		[appId, backend, roles],
	);

	const affected = openRoleId ? (memberCounts?.get(openRoleId) ?? 0) : 0;

	return (
		<div className="flex flex-col h-full max-h-full overflow-hidden">
			<div className="flex-1 overflow-auto">
				<div className="max-w-5xl mx-auto flex flex-col gap-5 p-4 pb-8">
					<div className="flex items-start justify-between gap-6 flex-wrap">
						<div>
							<h1 className="text-2xl font-bold tracking-tight">Roles</h1>
							<p className="text-sm text-muted-foreground max-w-prose">
								Set how far each role reaches into a part of the app. Every
								level maps to real permissions — open Advanced to see them.
							</p>
						</div>
						<div className="flex items-center gap-2">
							<div className="relative">
								<SearchIcon className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
								<Input
									placeholder="Search roles"
									value={searchTerm}
									onChange={(event) => setSearchTerm(event.target.value)}
									className="pl-8 w-44"
								/>
							</div>
							<Button onClick={() => setShowTemplates(true)}>
								<Plus className="h-4 w-4 mr-2" />
								New role
							</Button>
						</div>
					</div>

					{showTemplates && (
						<TemplatePicker
							onPick={createFromTemplate}
							onCancel={() => setShowTemplates(false)}
						/>
					)}

					{visibleRoles.length > 0 && <LadderKey />}

					<div className="flex flex-col gap-2">
						{visibleRoles.map((role) => {
							const isOpen = role.id === openRoleId;
							const shown = isOpen && draft ? draft : role;
							return (
								<RoleRow
									key={role.id}
									role={shown}
									isDefault={role.id === defaultRoleId}
									isOpen={isOpen}
									memberCount={
										isNewRole && isOpen ? 0 : memberCounts?.get(role.id)
									}
									onToggle={() => toggleRole(role)}
								>
									{isOpen && draft && (
										<RoleEditor
											role={draft}
											memberCount={isNewRole ? 0 : memberCounts?.get(role.id)}
											isDefault={role.id === defaultRoleId}
											knownAttributes={knownAttributes}
											onChange={setDraft}
											onDuplicate={() => handleDuplicate(role)}
											onDelete={() => handleDelete(role.id)}
											onSetDefault={() => handleSetDefault(role.id)}
										/>
									)}
								</RoleRow>
							);
						})}
					</div>

					{visibleRoles.length === 0 && (
						<div className="text-center py-12">
							<Shield className="h-8 w-8 mx-auto text-muted-foreground mb-3" />
							<h3 className="text-base font-semibold mb-1">No roles found</h3>
							<p className="text-sm text-muted-foreground mb-4">
								{searchTerm
									? "Try a different search term."
									: "Create your first role to get started."}
							</p>
							{!searchTerm && (
								<Button
									variant="outline"
									size="sm"
									onClick={() => setShowTemplates(true)}
								>
									<Plus className="h-4 w-4 mr-2" />
									New role
								</Button>
							)}
						</div>
					)}
				</div>
			</div>

			{isDirty && draft && (
				<div className="flex items-center gap-3 px-4 py-2.5 border-t bg-card">
					<p className="flex-1 text-sm text-muted-foreground">
						<span className="inline-block w-1.5 h-1.5 rounded-full bg-primary mr-2 align-middle" />
						{isNewRole ? "New role " : "Unsaved changes to "}
						<strong className="text-foreground">
							{draft.name.trim() || "Untitled role"}
						</strong>
						{!isNewRole && memberCounts !== undefined && (
							<>
								{" — affects "}
								<strong className="text-foreground">{affected}</strong>
								{affected === 1 ? " member" : " members"}
							</>
						)}
						.
					</p>
					<Button variant="ghost" size="sm" onClick={closeDraft}>
						Discard
					</Button>
					<Button size="sm" onClick={handleSave} disabled={!draft.name.trim()}>
						{isNewRole ? "Create role" : "Save role"}
					</Button>
				</div>
			)}
		</div>
	);
}

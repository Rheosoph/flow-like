/** Not under `/store/packages/`: Cloudflare Pages' `_redirects` sends `/store/packages/:id` to the store detail. */
export const PACKAGE_WORKSPACE_PATH = "/store/package-workspace";

export const WORKSPACE_TABS = [
	"overview",
	"nodes",
	"test",
	"manifest",
	"listing",
	"access",
	"releases",
] as const;

export type WorkspaceTab = (typeof WORKSPACE_TABS)[number];

const DEFAULT_TAB: WorkspaceTab = "overview";

export interface PackageWorkspaceHrefInput {
	id?: string | null;
	project?: string | null;
	tab?: WorkspaceTab | null;
	published?: boolean;
}

export function packageWorkspaceHref({
	id,
	project,
	tab,
	published,
}: PackageWorkspaceHrefInput = {}): string {
	const params = new URLSearchParams();
	if (id) params.set("id", id);
	if (project) params.set("project", project);
	if (tab && tab !== DEFAULT_TAB) params.set("tab", tab);
	if (published) params.set("published", "1");
	const query = params.toString();
	return query ? `${PACKAGE_WORKSPACE_PATH}?${query}` : PACKAGE_WORKSPACE_PATH;
}

export function isWorkspaceTab(value: unknown): value is WorkspaceTab {
	return (
		typeof value === "string" &&
		(WORKSPACE_TABS as readonly string[]).includes(value)
	);
}

export function workspaceTabFromParam(
	value: string | null | undefined,
): WorkspaceTab {
	return isWorkspaceTab(value) ? value : DEFAULT_TAB;
}

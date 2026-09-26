import {
	type WorkspaceTab,
	packageWorkspaceHref,
} from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-href";
import type {
	DeveloperProject,
	PackageInspection,
} from "@flow-like/flow-like-ui/lib/schema/developer";
import type { PackageManifest } from "@flow-like/flow-like-ui/lib/schema/wasm";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { countBySeverity, lintNodes } from "../../lib/validate-nodes";
import type { MineLintCounts } from "./mine-model";

export interface StalePackageInfo {
	package_id: string;
	project_path?: string | null;
}

export interface WorkspaceRouteInput {
	id?: string | null;
	project?: string | null;
	tab?: WorkspaceTab;
}

/** Fallback when the backend cannot tell the language from the folder. */
const LANGUAGE_MARKERS: readonly [string, string][] = [
	["Cargo.toml", "rust"],
	["package.json", "typescript"],
	["tsconfig.json", "typescript"],
	["go.mod", "go"],
	["build.zig", "zig"],
	["moon.mod.json", "moonbit"],
	["nimble.nimble", "nim"],
	[".csproj", "csharp"],
	["build.gradle.kts", "kotlin"],
	["CMakeLists.txt", "cpp"],
	["requirements.txt", "python"],
	["pyproject.toml", "python"],
];

async function detectLanguage(path: string): Promise<string> {
	const { exists } = await import("@tauri-apps/plugin-fs");
	for (const [file, language] of LANGUAGE_MARKERS) {
		try {
			if (await exists(`${path}/${file}`)) return language;
		} catch {}
	}
	return "rust";
}

async function manifestPackageName(path: string): Promise<string | null> {
	try {
		const manifest = await invoke<Record<string, unknown>>(
			"developer_get_manifest",
			{ projectPath: path },
		);
		const pkg = manifest.package as Record<string, unknown> | undefined;
		return typeof pkg?.name === "string" ? pkg.name : null;
	} catch {
		return null;
	}
}

function folderName(path: string): string {
	return path.split(/[\\/]/).filter(Boolean).pop() ?? "Untitled";
}

export async function pickProjectFolder(): Promise<string | null> {
	const selected = await open({ directory: true, multiple: false });
	return typeof selected === "string" ? selected : null;
}

export async function addProjectFolder(
	path: string,
): Promise<DeveloperProject> {
	const [name, language] = await Promise.all([
		manifestPackageName(path),
		detectLanguage(path),
	]);
	return invoke<DeveloperProject>("developer_add_project", {
		input: { path, language, name: name ?? folderName(path) },
	});
}

export function listProjects(): Promise<DeveloperProject[]> {
	return invoke<DeveloperProject[]>("developer_list_projects");
}

export async function readManifest(
	path: string,
): Promise<PackageManifest | null> {
	try {
		return await invoke<PackageManifest>("developer_read_manifest", {
			projectPath: path,
		});
	} catch {
		return null;
	}
}

export function inspectPackage(path: string): Promise<PackageInspection> {
	return invoke<PackageInspection>("developer_inspect_package", {
		projectPath: path,
	});
}

export function lintCounts(inspection: PackageInspection): MineLintCounts {
	return countBySeverity(lintNodes(inspection.nodes ?? []));
}

export async function inspectLint(path: string): Promise<MineLintCounts> {
	return lintCounts(await inspectPackage(path));
}

export interface BuildArtifactInfo {
	sizeBytes: number;
	builtAt?: number;
}

export async function buildArtifactInfo(
	path: string,
): Promise<BuildArtifactInfo> {
	const { stat } = await import("@tauri-apps/plugin-fs");
	const info = await stat(path);
	return { sizeBytes: info.size, builtAt: info.mtime?.getTime() };
}

export function checkStaleness(): Promise<StalePackageInfo[]> {
	return invoke<StalePackageInfo[]>("developer_check_staleness");
}

export function openInEditor(path: string): Promise<void> {
	return invoke("developer_open_in_editor", { projectPath: path });
}

export function loadIntoCatalog(path: string): Promise<number> {
	return invoke<number>("developer_load_into_catalog", { projectPath: path });
}

export function removeProject(projectId: string): Promise<void> {
	return invoke("developer_remove_project", { projectId });
}

export async function revealProject(path: string): Promise<void> {
	const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
	await revealItemInDir(path);
}

export const projectRoutes = {
	publish: (path: string) =>
		`/developer/publish?project=${encodeURIComponent(path)}`,
	workspace: ({ id, project, tab }: WorkspaceRouteInput = {}) =>
		packageWorkspaceHref({ id, project, tab }),
	newPackage: (language?: string) =>
		language
			? `/developer/new?language=${encodeURIComponent(language)}`
			: "/developer/new",
};

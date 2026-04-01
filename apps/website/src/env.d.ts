/// <reference path="../.astro/types.d.ts" />

interface ImportMetaEnv {
	readonly REGISTRY_PAT: string;
	readonly REGISTRY_API_URL: string;
	readonly ENABLE_STORE_SEO: string;
}

interface ImportMeta {
	readonly env: ImportMetaEnv;
}

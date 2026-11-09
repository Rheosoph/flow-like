"use client";

import { createContext } from "react";

/** A host can resolve relative media and file links in its own storage. */
export interface EditorAssetResolver {
	accepts: (url: string) => boolean;
	resolve: (url: string) => Promise<string>;
}

export const EditorAssetResolverContext = createContext<
	EditorAssetResolver | undefined
>(undefined);

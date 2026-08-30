import { defineCollection } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";
import { z } from "astro/zod";

export const collections = {
	docs: defineCollection({
		loader: docsLoader(),
		schema: docsSchema({
			extend: z.object({
				seo: z
					.object({
						title: z.string().min(1).max(70).optional(),
						topics: z.array(z.string().min(1)).max(12).default([]),
						imageAlt: z.string().min(1).max(180).optional(),
					})
					.optional(),
			}),
		}),
	}),
};

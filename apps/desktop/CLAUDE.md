# Desktop App (Tauri + Next.js)

## Dev Server

```bash
bun run dev         # Next.js only
bun run dev:all     # Tauri + Next.js
bunx tsc --noEmit   # Type-check
bunx biome check .  # Lint
```

## TypeScript

- Use interfaces for data structures and type definitions.
- Prefer immutable data: `const`, `readonly`.
- Use optional chaining (`?.`) and nullish coalescing (`??`).
- Follow functional programming principles where possible.

## React

- Functional components with hooks only.
- `React.FC` for components with children.
- Follow hooks rules: never call hooks conditionally.
- Keep components small and focused — split into subcomponents.
- Colocate small subcomponents in the same file.

## UI Framework

- **shadcn** components are pre-installed — import them, never recreate.
- **Tailwind CSS** for all styling — use design system tokens, not raw hex values.
- **Lucide** for icons — `import { IconName } from "lucide-react"`.

## Performance

- `useMemo` / `useCallback` for expensive computations and stable references.
- Proper `useEffect` dependency arrays — no missing or extraneous deps.
- `useState` for local state, avoid unnecessary re-renders.

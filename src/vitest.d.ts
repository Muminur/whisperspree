// Makes the jest-dom matcher augmentation (`toBeInTheDocument`, …) visible to
// TypeScript when it typechecks `src/**/*.test.tsx` under `tsconfig.app.json`.
// `vitest.setup.ts` performs the same import for the runtime; this ambient
// declaration mirrors it for the type layer so `pnpm typecheck` (tsc -b) sees
// the extended `vitest` `Assertion` interface.
import "@testing-library/jest-dom/vitest";

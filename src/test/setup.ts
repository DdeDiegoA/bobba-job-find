import { configure } from "@testing-library/react";

/**
 * Testing Library waits 1s for `findBy*` and `waitFor` by default. This repo
 * runs its jsdom suite in parallel on machines that also build the Rust
 * worktree, so a correct query can miss that window (FX19: six files flaked
 * on `findByRole` under load and passed alone). Five seconds is generous
 * enough to absorb the load and still short enough that a real failure fails
 * the run; it sits under the 15s `testTimeout` in `vite.config.ts`.
 */
configure({ asyncUtilTimeout: 5000 });

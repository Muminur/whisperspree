/// <reference types="vite/client" />

import { describe, expect, it } from "vitest";

import workflow from "../.github/workflows/ci.yml?raw";
import viteConfig from "../vite.config.ts?raw";

interface WorkflowStep {
  name: string;
  body: string;
}

function parseNamedWorkflowSteps(workflow: string): WorkflowStep[] {
  return Array.from(
    workflow.matchAll(/^ {6}- name: (.+?)\n([\s\S]*?)(?=^ {6}- name:|(?![\s\S]))/gm),
    ([, name, body]) => ({ name, body }),
  );
}

function extractObjectBody(source: string, property: string): string {
  const propertyMatch = new RegExp(String.raw`\b${property}\s*:\s*\{`).exec(source);
  expect(propertyMatch, `${property} object is configured`).not.toBeNull();

  const openingBrace = source.indexOf("{", propertyMatch!.index);
  let depth = 0;
  for (let index = openingBrace; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    if (source[index] === "}") depth -= 1;
    if (depth === 0) return source.slice(openingBrace + 1, index);
  }

  throw new Error(`Unclosed ${property} object in vite.config.ts`);
}

describe("T0.5 frontend coverage contract", () => {
  it("fr_0_5_frontend_coverage_gate_uses_effective_command_and_85_thresholds", () => {
    const steps = parseNamedWorkflowSteps(workflow);
    const pnpmTestStepIndex = steps.findIndex((step) => step.name === "pnpm test");
    const coverageSteps = steps.filter((step) => step.name.startsWith("Frontend coverage"));

    expect(pnpmTestStepIndex).toBeGreaterThanOrEqual(0);
    expect(coverageSteps).toHaveLength(1);
    expect(coverageSteps[0]!.name).toBe("Frontend coverage (enforced ≥85%)");
    expect(steps.indexOf(coverageSteps[0]!)).toBeGreaterThan(pnpmTestStepIndex);
    expect(coverageSteps[0]!.body.trim()).toBe("run: pnpm exec vitest run --coverage");
    expect(workflow).not.toContain("pnpm test -- --coverage");
    expect(workflow).not.toContain("--coverage.thresholds.");

    const coverage = extractObjectBody(viteConfig, "coverage");
    const thresholds = extractObjectBody(coverage, "thresholds");

    expect(coverage).toMatch(/\bprovider\s*:\s*["']v8["']/);
    expect(coverage).toMatch(/\binclude\s*:\s*\[\s*["']src\/\*\*\/\*\.\{ts,tsx\}["']\s*,?\s*\]/);

    for (const metric of ["lines", "branches", "functions", "statements"]) {
      const threshold = new RegExp(String.raw`\b${metric}\s*:\s*(\d+(?:\.\d+)?)`).exec(thresholds);
      expect(threshold, `${metric} coverage threshold is configured`).not.toBeNull();
      expect(Number(threshold![1])).toBeGreaterThanOrEqual(85);
    }
  });

  it("r0_coverage_ignore_matches_only_intended_os_glue_paths", () => {
    const command = workflow.match(/--ignore-filename-regex '([^']+)'/)?.[1];
    expect(command).toBeDefined();
    const ignore = new RegExp(command!);
    expect(ignore.test("src-tauri/src/hotkey/mod.rs")).toBe(true);
    expect(ignore.test("src-tauri/src/inject/macos.rs")).toBe(true);
    expect(ignore.test("src-tauri/src/context/macos.rs")).toBe(true);
    expect(ignore.test("src-tauri/src/audio/capture.rs")).toBe(true);
    expect(ignore.test("src-tauri/src/lib.rs")).toBe(true);
    expect(ignore.test("src-tauri/src/main.rs")).toBe(true);
    expect(ignore.test("src-tauri/src/domain.rs")).toBe(false);
    expect(ignore.test("src-tauri/src/zlib.rs")).toBe(false);
  });
});

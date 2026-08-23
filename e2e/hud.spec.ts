import { expect, test } from "@playwright/test";

// FR-5.2 HUD state rendering driven by simulated §9.2 events (CLAUDE.md §5).
// Emission must run inside the page context: Playwright serializes return
// values across the bridge and would strip the mock's functions.
const emit = (page: import("@playwright/test").Page, payload: unknown) =>
  page.evaluate(
    (p) => (window as unknown as { __TAURI_MOCK__: { emit: (e: string, v: unknown) => void } }).__TAURI_MOCK__.emit("session:state", p),
    payload,
  );

test("fr_5_2_hud_renders_listening_from_session_state_event_and_auto_hides_when_idle", async ({ page }) => {
  await page.goto("/?e2e=1&window=hud");
  await expect(page.getByTestId("hud-window")).toBeVisible();

  await emit(page, { sessionId: "session-1", state: "listening", partial: "" });
  await expect(page.getByRole("status", { name: "Listening" })).toBeVisible();

  await emit(page, { sessionId: "session-1", state: "idle" });
  await expect(page.getByTestId("hud-window")).not.toBeVisible({ timeout: 3_000 });
});

test("fr_5_2_hud_notice_state_shows_the_notice_message", async ({ page }) => {
  await page.goto("/?e2e=1&window=hud");
  await expect(page.getByTestId("hud-window")).toBeVisible();

  await emit(page, { sessionId: "session-2", state: "listening", notice: "Didn't catch anything" });
  await expect(page.getByTestId("hud-window")).toContainText("Didn't catch anything");
});

import { expect, test } from "@playwright/test";

// FR-5.4 onboarding flow over the mock IPC adapter.
test("fr_5_4_onboarding_lists_permissions_and_recheck_round_trips", async ({ page }) => {
  await page.goto("/?e2e=1&window=onboarding");

  for (const key of ["microphone", "inputMonitoring", "accessibility"]) {
    await expect(page.getByTestId(`permission-${key}`)).toBeVisible();
    await expect(page.getByTestId(`state-${key}`)).toHaveText("undetermined");
  }

  await page.getByRole("button", { name: "Recheck permissions" }).click();
  // The mock keeps the conservative snapshot; the recheck round-trip must
  // complete without surfacing an error banner.
  await expect(page.getByTestId("onboarding-content")).not.toContainText("could not be checked");
});

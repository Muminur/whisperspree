import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OnboardingWindow } from "./onboarding";

describe("OnboardingWindow", () => {
  afterEach(() => cleanup());
  it("fr_5_4_lists_permission states and opens the matching pane", async () => {
    const open = vi.fn().mockResolvedValue(undefined);
    render(<OnboardingWindow check={async () => ({ microphone: "granted", inputMonitoring: "denied", accessibility: "undetermined" })} open={open} />);
    await waitFor(() => expect(screen.getByTestId("state-microphone")).toHaveTextContent("granted"));
    fireEvent.click(screen.getByTestId("permission-inputMonitoring").querySelector("button")!);
    expect(open).toHaveBeenCalledWith("inputMonitoring");
  });

  it("fr_5_4_rechecks after a permission-check failure", async () => {
    const check = vi.fn().mockRejectedValue(new Error("denied"));
    render(<OnboardingWindow check={check} open={vi.fn()} />);
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("could not be checked"));
    fireEvent.click(screen.getByRole("button", { name: "Recheck permissions" }));
    expect(check).toHaveBeenCalledTimes(2);
  });

  it("fr_5_4_surfaces a failed permission deep link", async () => {
    render(<OnboardingWindow check={async () => ({ microphone: "denied", inputMonitoring: "granted", accessibility: "granted" })} open={async () => { throw new Error("pane failed"); }} />);
    await waitFor(() => expect(screen.getByTestId("state-microphone")).toHaveTextContent("denied"));
    fireEvent.click(screen.getByTestId("permission-microphone").querySelector("button")!);
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Could not open Microphone settings"));
  });
});

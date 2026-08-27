import { describe, expect, it } from "vitest";

import { serialiser } from "../src/serialise.js";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe("serialiser", () => {
  it("runs a slow task and a fast task issued concurrently one at a time, in call order", async () => {
    const exclusive = serialiser();
    const events: string[] = [];

    const slow = exclusive(async () => {
      events.push("slow start");
      await sleep(50);
      events.push("slow end");
      return "slow";
    });
    const fast = exclusive(async () => {
      events.push("fast start");
      events.push("fast end");
      return "fast";
    });

    expect(await Promise.all([slow, fast])).toEqual(["slow", "fast"]);
    // Not interleaved: "fast start" never appears between "slow start" and
    // "slow end", which is exactly what two statements sharing one
    // connection must never do.
    expect(events).toEqual(["slow start", "slow end", "fast start", "fast end"]);
  });

  it("lets the next task run even after a task rejects", async () => {
    const exclusive = serialiser();
    await expect(
      exclusive(async () => {
        throw new Error("boom");
      }),
    ).rejects.toThrow("boom");

    await expect(exclusive(async () => "still fine")).resolves.toBe("still fine");
  });
});

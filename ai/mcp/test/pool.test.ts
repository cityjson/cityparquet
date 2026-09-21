import { describe, expect, it } from "vitest";

import type { Engine } from "../src/duckdb.js";
import { createEnginePool, PoolBusyError } from "../src/pool.js";

interface FakeEngine extends Engine {
  readonly id: number;
  closed: boolean;
}

function factory(delayMs = 5) {
  let next = 0;
  const made: FakeEngine[] = [];
  return {
    made,
    create: async (): Promise<Engine> => {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
      const engine = {
        id: next++,
        closed: false,
        sandbox: true,
        extensions: [],
        connection: {},
        exclusive: <T>(task: () => Promise<T>) => task(),
        async close() {
          engine.closed = true;
        },
      } as unknown as FakeEngine;
      made.push(engine);
      return engine;
    },
  };
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe("createEnginePool", () => {
  it("never hands the same engine out twice, and closes each one after use", async () => {
    const { made, create } = factory();
    const pool = createEnginePool({ size: 2, maxWaiting: 10, maxWaitMs: 5000, create });
    const seen: number[] = [];
    for (let i = 0; i < 5; i++) {
      await pool.use(async (engine) => { seen.push((engine as FakeEngine).id); });
    }
    expect(new Set(seen).size).toBe(5);
    for (const id of seen) expect(made[id]!.closed).toBe(true);
    await pool.close();
  });

  it("runs at most `size` tasks at once, queueing the rest", async () => {
    const { create } = factory();
    const pool = createEnginePool({ size: 2, maxWaiting: 10, maxWaitMs: 5000, create });
    let running = 0;
    let peak = 0;
    await Promise.all(
      Array.from({ length: 6 }, () =>
        pool.use(async () => {
          running += 1;
          peak = Math.max(peak, running);
          await sleep(20);
          running -= 1;
        }),
      ),
    );
    expect(peak).toBe(2);
    await pool.close();
  });

  it("refuses a request once the wait queue is full", async () => {
    const { create } = factory();
    const pool = createEnginePool({ size: 1, maxWaiting: 1, maxWaitMs: 5000, create });
    const hold = pool.use(() => sleep(50));
    await sleep(10); // let the first request take the only engine
    const queued = pool.use(async () => "queued");
    await expect(pool.use(async () => "refused")).rejects.toBeInstanceOf(PoolBusyError);
    await hold;
    await expect(queued).resolves.toBe("queued");
    await pool.close();
  });

  it("gives up on a waiter after maxWaitMs", async () => {
    const { create } = factory();
    const pool = createEnginePool({ size: 1, maxWaiting: 5, maxWaitMs: 30, create });
    const hold = pool.use(() => sleep(120));
    await sleep(10);
    await expect(pool.use(async () => "late")).rejects.toThrow(/within 30 ms/);
    await hold;
    await pool.close();
  });

  it("recovers when building an engine fails", async () => {
    const { create } = factory();
    let failures = 1;
    const errors: unknown[] = [];
    const pool = createEnginePool({
      size: 1,
      maxWaiting: 5,
      maxWaitMs: 5000,
      create: async () => {
        if (failures-- > 0) throw new Error("extension load failed");
        return create();
      },
      onError: (error) => errors.push(error),
    });
    await expect(pool.use(async () => "ok")).resolves.toBe("ok");
    expect(errors).toHaveLength(1);
    await pool.close();
  });

  it("releases the engine when the task throws", async () => {
    const { made, create } = factory();
    const pool = createEnginePool({ size: 1, maxWaiting: 5, maxWaitMs: 5000, create });
    await expect(pool.use(async () => { throw new Error("boom"); })).rejects.toThrow("boom");
    expect(made[0]!.closed).toBe(true);
    await expect(pool.use(async () => "next")).resolves.toBe("next");
    await pool.close();
  });

  it("keeps a closing engine counted against the size until it has closed", async () => {
    let live = 0;
    let peak = 0;
    const pool = createEnginePool({
      size: 1,
      maxWaiting: 10,
      maxWaitMs: 5000,
      create: async () => {
        live += 1;
        peak = Math.max(peak, live);
        return {
          sandbox: true,
          extensions: [],
          connection: {},
          exclusive: <T>(task: () => Promise<T>) => task(),
          async close() {
            await sleep(40); // a slow close: DuckDB freeing a large instance
            live -= 1;
          },
        } as unknown as Engine;
      },
    });
    // The second request arrives while the first engine is still closing:
    // its wait must not start a second build until the close is done.
    const first = pool.use(async () => {});
    await sleep(20);
    await Promise.all([first, pool.use(async () => {}), pool.use(async () => {})]);
    expect(peak).toBe(1);
    await pool.close();
  });

  it("closes an engine that finishes building after the pool closed, and waits for it", async () => {
    const { made, create } = factory(40);
    const pool = createEnginePool({ size: 1, maxWaiting: 1, maxWaitMs: 5000, create });
    await pool.close(); // the first build is still in flight
    expect(made).toHaveLength(1);
    expect(made[0]!.closed).toBe(true);
    await expect(pool.close()).resolves.toBeUndefined(); // idempotent
  });
});

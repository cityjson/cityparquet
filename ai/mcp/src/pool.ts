// Single-use engines for the hosted server, kept warm ahead of demand.

import type { Engine } from "./duckdb.js";

/**
 * Thrown by `acquire` when every engine is leased and the wait queue is full,
 * or when a waiter has waited longer than `maxWaitMs`. The HTTP entry point
 * answers it with 503 rather than an error in the tool result.
 */
export class PoolBusyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PoolBusyError";
  }
}

export interface PoolOptions {
  /** How many engines exist at once — the hosted server's concurrency. */
  readonly size: number;
  /** Requests allowed to wait for an engine before new ones are refused. */
  readonly maxWaiting: number;
  readonly maxWaitMs: number;
  readonly create: () => Promise<Engine>;
  /** Out-of-band failures: an engine that would not build, or would not close. */
  readonly onError?: (error: unknown) => void;
}

export interface EnginePool {
  /**
   * An engine no other request has touched, for the duration of `task`. It is
   * closed afterwards and never handed out again.
   */
  use<T>(task: (engine: Engine) => Promise<T>, options?: { readonly signal?: AbortSignal }): Promise<T>;
  close(): Promise<void>;
}

/**
 * Why single-use engines rather than one shared instance: DuckDB's attached
 * catalogs are instance-wide. A caller who creates a table — or attaches an
 * in-memory catalog to hold one — leaves it readable by every other
 * connection on the instance, which can list it with `duckdb_databases()`.
 * An engine per request is the only isolation that holds. It costs a few
 * hundred milliseconds to build one with the extensions already on disk, so
 * each slot builds its next engine as soon as the last one is released, and
 * a request normally finds one ready.
 */
export function createEnginePool(options: PoolOptions): EnginePool {
  const ready: Engine[] = [];
  const waiters: { resolve: (engine: Engine) => void; reject: (error: Error) => void; timer: NodeJS.Timeout }[] = [];
  // Builds and disposals still running, so `close` can wait for all of them.
  const inflight = new Set<Promise<void>>();
  let closing: Promise<void> | null = null;
  // Every engine that exists or is being made counts against `size` — one
  // being built, ready, leased, or still closing — so memory stays bounded
  // even while DuckDB frees a large instance.
  let building = 0;
  let leased = 0;

  function track(work: Promise<void>): void {
    inflight.add(work);
    void work.finally(() => inflight.delete(work));
  }

  async function dispose(engine: Engine): Promise<void> {
    try {
      await engine.close();
    } catch (error) {
      options.onError?.(error);
    }
  }

  function refill(): void {
    while (closing === null && ready.length + building + leased < options.size) {
      building += 1;
      track(
        options.create().then(
          async (engine) => {
            building -= 1;
            if (closing !== null) {
              await dispose(engine);
              return;
            }
            const waiter = waiters.shift();
            if (waiter) {
              clearTimeout(waiter.timer);
              leased += 1;
              waiter.resolve(engine);
            } else {
              ready.push(engine);
            }
          },
          (error: unknown) => {
            building -= 1;
            options.onError?.(error);
            // Retry after a pause rather than spinning on a broken build.
            if (closing === null) setTimeout(refill, 1000).unref();
          },
        ),
      );
    }
  }

  function acquire(signal?: AbortSignal): Promise<Engine> {
    if (closing !== null) return Promise.reject(new Error("the engine pool is closed"));
    if (signal?.aborted) return Promise.reject(new Error("request aborted before an engine was free"));
    const engine = ready.shift();
    if (engine) {
      leased += 1;
      return Promise.resolve(engine);
    }
    if (waiters.length >= options.maxWaiting) {
      return Promise.reject(new PoolBusyError("the server is at capacity; retry shortly"));
    }
    return new Promise<Engine>((resolve, reject) => {
      const leave = () => {
        const index = waiters.indexOf(waiter);
        if (index !== -1) waiters.splice(index, 1);
        clearTimeout(waiter.timer);
        signal?.removeEventListener("abort", onAbort);
      };
      // A request whose client has gone — Cloud Run cuts one off at its
      // timeout — must not go on to take an engine and run for nobody.
      const onAbort = () => {
        leave();
        reject(new Error("request aborted before an engine was free"));
      };
      const waiter = {
        resolve: (engine: Engine) => {
          signal?.removeEventListener("abort", onAbort);
          resolve(engine);
        },
        reject,
        timer: setTimeout(() => {
          leave();
          reject(new PoolBusyError(`no engine became free within ${options.maxWaitMs} ms; retry shortly`));
        }, options.maxWaitMs),
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      waiters.push(waiter);
      refill();
    });
  }

  async function release(engine: Engine): Promise<void> {
    const disposal = dispose(engine);
    track(disposal);
    await disposal;
    // Only now is the slot free: a replacement built while this engine was
    // still closing would put one engine more than `size` in memory.
    leased -= 1;
    refill();
  }

  refill();

  return {
    async use<T>(task: (engine: Engine) => Promise<T>, useOptions?: { readonly signal?: AbortSignal }): Promise<T> {
      const engine = await acquire(useOptions?.signal);
      try {
        return await task(engine);
      } finally {
        await release(engine);
      }
    },
    close() {
      closing ??= (async () => {
        for (const waiter of waiters.splice(0)) {
          clearTimeout(waiter.timer);
          waiter.reject(new Error("the engine pool is closed"));
        }
        await Promise.all(ready.splice(0).map(dispose));
        // Builds finishing now dispose of their own engine; leased engines
        // are disposed by their `use` when the task ends.
        while (inflight.size > 0) await Promise.allSettled([...inflight]);
      })();
      return closing;
    },
  };
}

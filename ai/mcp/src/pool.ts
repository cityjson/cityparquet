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
  use<T>(task: (engine: Engine) => Promise<T>): Promise<T>;
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
  let closed = false;
  let building = 0;
  let leased = 0;

  function refill(): void {
    while (!closed && ready.length + building + leased < options.size) {
      building += 1;
      options
        .create()
        .then((engine) => {
          building -= 1;
          if (closed) {
            void engine.close();
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
        })
        .catch((error: unknown) => {
          building -= 1;
          options.onError?.(error);
          // Retry after a pause rather than spinning on a broken build.
          if (!closed) setTimeout(refill, 1000).unref();
        });
    }
  }

  function acquire(): Promise<Engine> {
    if (closed) return Promise.reject(new Error("the engine pool is closed"));
    const engine = ready.shift();
    if (engine) {
      leased += 1;
      return Promise.resolve(engine);
    }
    if (waiters.length >= options.maxWaiting) {
      return Promise.reject(new PoolBusyError("the server is at capacity; retry shortly"));
    }
    return new Promise<Engine>((resolve, reject) => {
      const waiter = {
        resolve,
        reject,
        timer: setTimeout(() => {
          const index = waiters.indexOf(waiter);
          if (index !== -1) waiters.splice(index, 1);
          reject(new PoolBusyError(`no engine became free within ${options.maxWaitMs} ms; retry shortly`));
        }, options.maxWaitMs),
      };
      waiters.push(waiter);
      refill();
    });
  }

  async function release(engine: Engine): Promise<void> {
    leased -= 1;
    try {
      await engine.close();
    } catch (error) {
      options.onError?.(error);
    }
    refill();
  }

  refill();

  return {
    async use<T>(task: (engine: Engine) => Promise<T>): Promise<T> {
      const engine = await acquire();
      try {
        return await task(engine);
      } finally {
        await release(engine);
      }
    },
    async close() {
      closed = true;
      for (const waiter of waiters.splice(0)) {
        clearTimeout(waiter.timer);
        waiter.reject(new Error("the engine pool is closed"));
      }
      await Promise.all(ready.splice(0).map((engine) => engine.close()));
    },
  };
}

// Running one thing at a time.

/**
 * A queue of one: each task starts only once the previous has settled.
 *
 * This engine's five tools share a single `DuckDBConnection`. An MCP client
 * pipelines tool calls — nothing stops `cityparquet_query` and
 * `cityparquet_describe` from being in flight at once — and two statements in
 * flight on one connection interleave inside the engine rather than queueing.
 * A `query` timeout is the case that made this visible: it calls
 * `connection.interrupt()`, which is connection-global, so without
 * serialisation it can cancel a concurrently running `describe` (or another
 * `query`) and mislabel that unrelated result as its own timeout.
 *
 * A failed task rejects for its own caller and nothing else: the chain is
 * continued from a caught copy, so the next task still runs.
 */
export function serialiser(): <T>(task: () => Promise<T>) => Promise<T> {
  let tail: Promise<unknown> = Promise.resolve();
  return <T>(task: () => Promise<T>): Promise<T> => {
    // `then(task, task)` rather than `then(task)`: a rejected predecessor must
    // still let its successor start.
    const next = tail.then(task, task);
    tail = next.catch(() => {});
    return next;
  };
}

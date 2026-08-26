// Running one thing at a time.

/**
 * A queue of one: each task starts only once the previous has settled.
 *
 * DuckDB-Wasm is the reason this exists. There is a single connection into a
 * single WebAssembly instance, and two statements in flight on it do not queue
 * — they interleave and corrupt the engine's heap, surfacing as
 * `RuntimeError: memory access out of bounds` or `null function` from *both*
 * statements. That reads like a broken query rather than a broken instance,
 * which is what makes it worth a queue rather than a convention.
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

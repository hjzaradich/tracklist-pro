import { ERROR_KEYS, type ErrorKind, type IpcError } from "../bindings";
import i18n from "../i18n";

// Commands that can fail return an IpcError: a kind and the parameters its
// message needs, never error text (src-tauri/src/ipc/error.rs). The message
// for each kind is in src/locales/en/errors.json, found through the generated
// ERROR_KEYS table, so TypeScript checks every key exists.

/** What a command that can fail resolves to (tauri-specta's result shape). */
export type CommandResult<T> = { status: "ok"; data: T } | { status: "error"; error: IpcError };

const KINDS: readonly string[] = Object.keys(ERROR_KEYS);

/** Shown for anything that isn't an IpcError: a bug, not the user's doing. */
const INTERNAL: IpcError = { kind: "internal", params: {} };

/**
 * `value` if it's an IpcError, otherwise an internal error. The raw value
 * goes to the developer console in dev builds only: it's never shown.
 */
function asIpcError(value: unknown): IpcError {
  if (isIpcError(value)) return value;
  if (import.meta.env.DEV) console.error("A command failed with an unexpected error:", value);
  return INTERNAL;
}

/** True if `value` has an IpcError's shape and a kind this build knows. */
export function isIpcError(value: unknown): value is IpcError {
  if (typeof value !== "object" || value === null) return false;
  const { kind, params } = value as { kind?: unknown; params?: unknown };
  return (
    typeof kind === "string" &&
    KINDS.includes(kind) &&
    typeof params === "object" &&
    params !== null &&
    Object.values(params).every((p) => typeof p === "string" || typeof p === "number")
  );
}

/** A failed command, thrown by {@link unwrap}. Its `error` is always an IpcError. */
export class CommandError extends Error {
  readonly error: IpcError;

  constructor(error: IpcError) {
    // For the developer console only; the UI shows errorMessage(error).
    super(`command failed: ${error.kind}`);
    this.name = "CommandError";
    this.error = error;
  }

  get kind(): ErrorKind {
    return this.error.kind;
  }
}

/**
 * The data of a command that succeeded; throws a {@link CommandError} if it
 * failed. Anything else that goes wrong (Tauri refusing the arguments, or
 * throwing an Error, which the generated bindings pass straight through) is
 * thrown as an `internal` CommandError, so no raw text gets through.
 */
export async function unwrap<T>(call: Promise<CommandResult<T>>): Promise<T> {
  let result: CommandResult<T>;
  try {
    result = await call;
  } catch (thrown) {
    throw new CommandError(asIpcError(thrown));
  }
  if (result.status === "ok") return result.data;
  throw new CommandError(asIpcError(result.error));
}

/**
 * The message to show for a failed command, in the user's language. Takes
 * whatever was thrown or returned: an IpcError, a CommandError, or anything
 * else, which is shown as an internal error and never as its own text.
 */
export function errorMessage(error: unknown): string {
  const ipc = error instanceof CommandError ? error.error : asIpcError(error);
  return i18n.t(ERROR_KEYS[ipc.kind], ipc.params);
}

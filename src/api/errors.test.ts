import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import { commands, ERROR_KEYS, type ErrorKind, type IpcError } from "../bindings";
import i18n, { englishResources } from "../i18n";
import { CommandError, errorMessage, isIpcError, unwrap } from "./errors";

const kinds = Object.keys(ERROR_KEYS) as ErrorKind[];
const internalMessage = i18n.t("errors:internal");

describe("command errors", () => {
  afterEach(() => {
    clearMocks();
    vi.restoreAllMocks();
  });

  /** Silences, and records, what goes to the developer console. */
  function consoleErrors() {
    return vi.spyOn(console, "error").mockImplementation(() => {});
  }

  it("every error kind has an English message in the errors namespace", () => {
    expect(kinds.length).toBeGreaterThan(0);
    for (const kind of kinds) {
      const key = ERROR_KEYS[kind];
      expect(i18n.exists(key), key).toBe(true);
      expect(errorMessage({ kind, params: {} })).not.toBe(key);
    }
  });

  it("the errors namespace has no message without an error kind", () => {
    const used = new Set<string>(Object.values(ERROR_KEYS));
    for (const name of Object.keys(englishResources.errors)) {
      expect(used.has(`errors:${name}`), name).toBe(true);
    }
  });

  it("shows an error's message from its kind", () => {
    expect(errorMessage({ kind: "diskFull", params: {} })).toBe(
      "Disk full. Free up some space and try again.",
    );
    expect(errorMessage(new CommandError({ kind: "busy", params: {} }))).toBe(
      "The Library is busy. Try again in a moment.",
    );
  });

  it("never shows raw error text: anything that isn't an IpcError reads as internal", () => {
    consoleErrors();
    for (const raw of [
      "database error: no such table: setting",
      new Error("no such table: setting"),
      { kind: "no such table: setting", params: {} },
      { kind: "busy", params: { detail: { nested: "no such table" } } },
      { kind: "busy" },
      null,
      undefined,
      42,
    ]) {
      expect(isIpcError(raw)).toBe(false);
      expect(errorMessage(raw)).toBe(internalMessage);
    }
  });

  it("unwrap returns a command's data, or throws a CommandError with its kind", async () => {
    mockIPC((cmd) => {
      if (cmd === "cancel_job") return "stopping";
      if (cmd === "activity") throw { kind: "stopped", params: {} } satisfies IpcError;
      throw new Error(`unexpected command ${cmd}`);
    });
    await expect(unwrap(commands.cancelJob(1))).resolves.toBe("stopping");
    const failed = await unwrap(commands.activity()).catch((e: unknown) => e);
    expect(failed).toBeInstanceOf(CommandError);
    expect((failed as CommandError).kind).toBe("stopped");
    expect(errorMessage(failed)).toBe("The Library has stopped. Restart the app.");
  });

  it("unwrap turns a raw error string from Tauri into an internal error", async () => {
    consoleErrors();
    mockIPC(() => {
      throw "invalid args `notation` for command `set_key_notation`";
    });
    const failed = await unwrap(commands.setKeyNotation("camelot")).catch((e: unknown) => e);
    expect(failed).toBeInstanceOf(CommandError);
    expect((failed as CommandError).kind).toBe("internal");
    expect((failed as Error).message).not.toContain("notation");
    expect(errorMessage(failed)).toBe(internalMessage);
  });

  it("unwrap turns an Error thrown on the way into an internal error", async () => {
    consoleErrors();
    mockIPC(() => {
      throw new Error("x: no such table: setting");
    });
    const failed = await unwrap(commands.keyNotation()).catch((e: unknown) => e);
    expect(failed).toBeInstanceOf(CommandError);
    expect((failed as CommandError).kind).toBe("internal");
    expect((failed as Error).message).not.toContain("setting");
    expect(errorMessage(failed)).toBe(internalMessage);
  });

  it("logs an unexpected error's raw value to the developer console in dev builds", async () => {
    const logged = consoleErrors();
    const raw = new Error("no such table: setting");
    mockIPC(() => {
      throw raw;
    });
    await unwrap(commands.keyNotation()).catch(() => {});
    expect(import.meta.env.DEV).toBe(true);
    expect(logged).toHaveBeenCalledWith(expect.any(String), raw);
  });

  it("an IpcError is not logged: it's expected, and shown to the user", async () => {
    const logged = consoleErrors();
    mockIPC(() => {
      throw { kind: "busy", params: {} } satisfies IpcError;
    });
    await unwrap(commands.keyNotation()).catch(() => {});
    expect(logged).not.toHaveBeenCalled();
  });
});

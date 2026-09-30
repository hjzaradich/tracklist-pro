import { commands } from "../bindings";

/** Asks the Rust side which version of the app is running. */
export async function fetchAppVersion(): Promise<string> {
  const info = await commands.appInfo();
  return info.version;
}

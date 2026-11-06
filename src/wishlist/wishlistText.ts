import type { TFunction } from "i18next";
import type { Reason, WishlistRefusal } from "../bindings";
import { reasonOptions } from "../suggest/reasons";

/** A reason's text. Its key comes from Rust (`wishlist:reason.*`), so it can't be typed as a literal key. */
export function reasonText(t: TFunction<"wishlist">, reason: Reason): string {
  const translate = t as unknown as (key: string, options: object) => string;
  return translate(reason.key, reasonOptions(reason));
}

/** Why nothing was written, in words. */
export function refusalText(t: TFunction<"wishlist">, refusal: WishlistRefusal): string {
  return t(`refusal.${refusal}`);
}

/** A file's name without its folders and its extension (a name for a list made from it). */
export function stemOf(path: string): string {
  const name = path.split(/[/\\]/).pop() ?? path;
  const dot = name.lastIndexOf(".");
  return (dot > 0 ? name.slice(0, dot) : name).trim();
}

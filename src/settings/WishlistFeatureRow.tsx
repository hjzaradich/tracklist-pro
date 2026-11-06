import { useTranslation } from "react-i18next";
import { useSetWishlistFeature, useWishlistFeature } from "../wishlist/useWishlist";
import { SwitchRow } from "./SwitchRow";

/**
 * "Wishlists": the one switch that turns the wishlist feature on (ROADMAP
 * 2.3). Off until the user turns it on; while off, the sidebar entry is
 * hidden. It is a setting, so there is nothing to Undo.
 */
export function WishlistFeatureRow() {
  const { t } = useTranslation("wishlist");
  const stored = useWishlistFeature();
  const save = useSetWishlistFeature();
  // Until the setting has been read, the box is disabled and shows neither state.
  const known = stored.data !== undefined;
  const on = stored.data === true;
  const checked = save.isPending ? save.variables === true : on;
  return (
    <SwitchRow
      label={t("feature.label")}
      help={known ? t(on ? "feature.helpOn" : "feature.helpOff") : undefined}
      checked={checked}
      pending={!known || save.isPending}
      error={save.isError ? t("feature.error") : null}
      onChange={(next) => save.mutate(next)}
    />
  );
}

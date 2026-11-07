import { getVersion } from "@tauri-apps/api/app";
import { useQuery } from "@tanstack/react-query";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import styles from "./settings.module.css";

/** One shipped package, as scripts/third-party-licenses.mjs writes it. `text` indexes `texts`. */
export interface LicencePackage {
  name: string;
  version: string;
  license: string;
  text: number | null;
  repository: string | null;
}
export interface LicenceList {
  texts: string[];
  packages: LicencePackage[];
}

/** The list the installer build writes next to the app; null when it isn't there (dev builds). */
async function loadLicences(): Promise<LicenceList | null> {
  try {
    const response = await fetch("/third-party-licenses.json");
    if (!response.ok) return null;
    const list: unknown = await response.json();
    const ok =
      typeof list === "object" &&
      list !== null &&
      Array.isArray((list as LicenceList).texts) &&
      Array.isArray((list as LicenceList).packages);
    return ok ? (list as LicenceList) : null;
  } catch {
    return null;
  }
}

/** "About" on the Settings screen: GPL notice, source address as copyable text (the app opens no links), credits and the shipped packages' licences. */
export function AboutPanel() {
  const { t } = useTranslation("about");
  const titleId = useId();
  const version = useQuery({ queryKey: ["app-version"], queryFn: getVersion, retry: false });
  const licences = useQuery({ queryKey: ["third-party-licences"], queryFn: loadLicences, retry: false });
  const list = licences.data;
  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.panelTitle}>
        {t("panel")}
      </h2>
      {version.data ? <p className={styles.help}>{t("version", { version: version.data })}</p> : null}
      <p>{t("gpl")}</p>
      <p>
        {t("sourceLabel")} <code>{t("sourceUrl")}</code>
      </p>
      <h3>{t("credits.title")}</h3>
      <p>{t("credits.keyfinder")}</p>
      <h3>{t("licences.title")}</h3>
      {licences.isPending ? <p>{t("licences.loading")}</p> : null}
      {licences.isSuccess && !list ? <p>{t("licences.missing")}</p> : null}
      {list ? <LicenceGroups list={list} /> : null}
    </section>
  );
}

function LicenceGroups({ list }: { list: LicenceList }) {
  const { t } = useTranslation("about");
  const without = list.packages.filter((p) => p.text === null || list.texts[p.text] === undefined);
  return (
    <>
      <p>{t("licences.count", { count: list.packages.length })}</p>
      {list.texts.map((text, index) => {
        const users = list.packages.filter((p) => p.text === index);
        if (users.length === 0) return null;
        return (
          <details key={index}>
            <summary>
              {[...new Set(users.map((p) => p.license))].join(", ")} ({t("licences.used", { count: users.length })})
            </summary>
            <p>{users.map((p) => `${p.name} ${p.version}`).join(", ")}</p>
            <pre className={styles.licenceText}>{text}</pre>
          </details>
        );
      })}
      {without.length > 0 ? (
        <ul>
          {without.map((p) => (
            <li key={`${p.name}@${p.version}`}>
              {p.name} {p.version} ({p.license}) {t("licences.noText", { url: p.repository ?? "" })}
            </li>
          ))}
        </ul>
      ) : null}
    </>
  );
}

import { useCallback } from "react";
import i18n from "i18next";
import { initReactI18next, useTranslation } from "react-i18next";
import ja from "./ja.json";
import en from "./en.json";
import zhCN from "./zh-CN.json";

export type MessageKey = keyof typeof ja;

const enMessages: Record<MessageKey, string> = en;
const zhMessages: Record<MessageKey, string> = zhCN;

void i18n.use(initReactI18next).init({
  resources: {
    ja: { translation: ja },
    en: { translation: enMessages },
    "zh-CN": { translation: zhMessages },
  },
  lng: localStorage.getItem("systivra.lang") ?? "ja",
  fallbackLng: "ja",
  interpolation: { escapeValue: false },
});

export const LANGUAGES = [
  { code: "ja", labelKey: "lang.ja" },
  { code: "en", labelKey: "lang.en" },
  { code: "zh-CN", labelKey: "lang.zh" },
] as const satisfies ReadonlyArray<{ code: string; labelKey: MessageKey }>;

export function useT() {
  const { i18n: instance } = useTranslation();
  const t = useCallback(
    (key: MessageKey, options?: Record<string, unknown>) =>
      instance.t(key, options) as string,
    [instance],
  );
  const setLang = useCallback(
    (lang: string) => {
      localStorage.setItem("systivra.lang", lang);
      void instance.changeLanguage(lang);
    },
    [instance],
  );
  return { t, lang: instance.language, setLang };
}

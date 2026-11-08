export function formatDate(value: string | null | undefined, lang: string): string {
  if (!value) return "—";
  return new Intl.DateTimeFormat(lang, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    timeZone: "Asia/Tokyo",
  }).format(new Date(value));
}

export function formatNumber(value: number, lang: string): string {
  return new Intl.NumberFormat(lang).format(value);
}

export function formatBytes(value: number, lang: string): string {
  if (value < 1024) return `${formatNumber(value, lang)} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

export function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

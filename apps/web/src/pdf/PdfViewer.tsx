import { useEffect, useRef, useState } from "react";
import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy, PDFPageProxy } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { documentContentUrl } from "../lib/api";
import { useT } from "../i18n";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

interface Highlight {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface Props {
  documentId: string;
  page: number;
  quote: string;
}

function normalize(value: string): string {
  return value.normalize("NFKC").replace(/[\u0000-\u001f\u007f\s]+/g, "");
}

export function PdfViewer({ documentId, page, quote }: Props) {
  const { t } = useT();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [highlights, setHighlights] = useState<Highlight[]>([]);
  const [canvasSize, setCanvasSize] = useState({ width: 0, height: 0 });
  const [missing, setMissing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setPdf(null);
    setHighlights([]);
    setError(null);
    const task = pdfjs.getDocument({
      url: documentContentUrl(documentId),
      withCredentials: true,
    });
    task.promise.then(
      (document) => {
        if (!cancelled) setPdf(document);
      },
      () => {
        if (!cancelled) setError(t("error.server"));
      },
    );
    return () => {
      cancelled = true;
      void task.destroy();
    };
  }, [documentId, t]);

  useEffect(() => {
    if (!pdf) return;
    let cancelled = false;
    let renderTask: pdfjs.RenderTask | null = null;

    async function render() {
      const pdfPage: PDFPageProxy = await pdf!.getPage(page);
      if (cancelled) return;
      const canvas = canvasRef.current;
      const container = containerRef.current;
      if (!canvas || !container) return;

      const base = pdfPage.getViewport({ scale: 1 });
      const available = Math.max(320, container.clientWidth - 32);
      const scale = Math.min(2, Math.max(0.6, available / base.width));
      const viewport = pdfPage.getViewport({ scale });
      const outputScale = window.devicePixelRatio || 1;

      canvas.width = Math.floor(viewport.width * outputScale);
      canvas.height = Math.floor(viewport.height * outputScale);
      canvas.style.width = `${Math.floor(viewport.width)}px`;
      canvas.style.height = `${Math.floor(viewport.height)}px`;
      setCanvasSize({ width: viewport.width, height: viewport.height });

      const context = canvas.getContext("2d");
      if (!context) return;
      renderTask = pdfPage.render({
        canvas,
        canvasContext: context,
        viewport,
        transform: outputScale === 1 ? undefined : [outputScale, 0, 0, outputScale, 0, 0],
      });
      await renderTask.promise;
      if (cancelled) return;

      const content = await pdfPage.getTextContent();
      if (cancelled) return;
      const normalizedQuote = normalize(quote);
      const items: Array<{
        start: number;
        end: number;
        str: string;
        transform: number[];
        width: number;
        height: number;
      }> = [];
      let cursor = 0;
      for (const raw of content.items) {
        if (!("str" in raw) || !("transform" in raw)) continue;
        const item = raw as {
          str: string;
          transform: number[];
          width: number;
          height: number;
        };
        const text = normalize(item.str);
        if (!text) continue;
        items.push({ ...item, str: text, start: cursor, end: cursor + text.length });
        cursor += text.length;
      }
      if (!normalizedQuote) {
        setHighlights([]);
        setMissing(false);
        return;
      }
      const joined = items.map((item) => item.str).join("");
      const candidates = [
        normalizedQuote,
        normalizedQuote.slice(0, 240),
        normalizedQuote.slice(0, 120),
        normalizedQuote.slice(0, 60),
        normalizedQuote.slice(0, 30),
      ].filter((candidate) => candidate.length >= 8);
      let matchStart = -1;
      let matchLength = 0;
      for (const candidate of candidates) {
        const index = joined.indexOf(candidate);
        if (index >= 0) {
          matchStart = index;
          matchLength = candidate.length;
          break;
        }
      }
      if (matchStart < 0) {
        setHighlights([]);
        setMissing(true);
        return;
      }
      const matchEnd = matchStart + matchLength;
      const rects: Highlight[] = [];
      for (const item of items) {
        if (item.end <= matchStart || item.start >= matchEnd) continue;
        const transform = pdfjs.Util.transform(viewport.transform, item.transform);
        const fontHeight = Math.hypot(transform[2], transform[3]);
        rects.push({
          left: transform[4],
          top: transform[5] - fontHeight,
          width: Math.max(2, item.width * scale),
          height: Math.max(4, fontHeight),
        });
      }
      setHighlights(rects);
      setMissing(rects.length === 0);
    }

    void render().catch(() => {
      if (!cancelled) setError(t("error.server"));
    });

    return () => {
      cancelled = true;
      renderTask?.cancel();
    };
  }, [pdf, page, quote, t]);

  if (error) {
    return (
      <div className="empty">
        <strong>{error}</strong>
      </div>
    );
  }

  return (
    <div className="viewer-shell">
      <div className="viewer-toolbar">
        <span className="panel-title">{t("document.viewer")}</span>
        <span className="chip">
          {t("document.page")} {page}
        </span>
        {missing ? <span className="badge badge-warn">{t("document.highlightMissing")}</span> : null}
      </div>
      <div className="viewer-canvas-wrap" ref={containerRef}>
        {!pdf ? (
          <div className="empty">{t("common.loading")}</div>
        ) : (
          <div className="pdf-page" style={{ width: canvasSize.width, height: canvasSize.height }}>
            <canvas ref={canvasRef} />
            {highlights.map((highlight, index) => (
              <span
                key={index}
                className="pdf-highlight"
                style={{
                  left: highlight.left,
                  top: highlight.top,
                  width: highlight.width,
                  height: highlight.height,
                }}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

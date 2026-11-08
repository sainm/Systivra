import { useCallback, useRef, useState } from "react";

export interface AsyncAction {
  loading: boolean;
  error: unknown;
  run: <T>(task: (isCurrent: () => boolean) => Promise<T>) => Promise<T | undefined>;
  reset: () => void;
  setError: (error: unknown) => void;
}

export function useAsyncAction(): AsyncAction {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<unknown>(null);
  const sequence = useRef(0);

  const run = useCallback(async <T,>(task: (isCurrent: () => boolean) => Promise<T>) => {
    const id = sequence.current + 1;
    sequence.current = id;
    setLoading(true);
    setError(null);
    try {
      return await task(() => sequence.current === id);
    } catch (cause) {
      if (sequence.current === id) setError(cause);
      return undefined;
    } finally {
      if (sequence.current === id) setLoading(false);
    }
  }, []);

  const reset = useCallback(() => {
    sequence.current += 1;
    setLoading(false);
    setError(null);
  }, []);

  const setExternalError = useCallback((value: unknown) => {
    setError(value);
  }, []);

  return { loading, error, run, reset, setError: setExternalError };
}

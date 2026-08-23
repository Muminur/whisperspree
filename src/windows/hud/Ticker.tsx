const TICKER_MAX_CHARS = 60;

/** Retain the beginning and end of ASR partials without making the pill grow. */
export function middleEllipsize(text: string, maxChars = TICKER_MAX_CHARS): string {
  if (text.length <= maxChars) return text;
  const retained = maxChars - 1;
  const head = Math.ceil(retained / 2);
  const tail = Math.floor(retained / 2);
  return `${text.slice(0, head)}…${text.slice(-tail)}`;
}

export function Ticker({ text }: { text: string }) {
  return (
    <span data-testid="hud-partial-ticker" className="hud-ticker">
      {middleEllipsize(text)}
    </span>
  );
}

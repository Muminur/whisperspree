const METER_BARS = 12;

export function LevelMeter({ level }: { level: number }) {
  return (
    <div className="hud-meter" aria-label="Live input level">
      {Array.from({ length: METER_BARS }, (_, index) => {
        const threshold = (index + 1) / METER_BARS;
        return (
          <span
            data-testid="hud-level-bar"
            className="hud-level-bar"
            data-active={level >= threshold}
            key={index}
          />
        );
      })}
    </div>
  );
}

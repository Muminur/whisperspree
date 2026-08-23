export function QuickMenu({ visible }: { visible: boolean }) {
  if (!visible) return null;
  return (
    <div className="hud-quick-menu" role="menu" aria-label="Post-injection actions">
      <button type="button" role="menuitem">Copy raw</button>
      <button type="button" role="menuitem" aria-label="Transform">Transform ▾</button>
      <button type="button" role="menuitem">Open in History</button>
    </div>
  );
}

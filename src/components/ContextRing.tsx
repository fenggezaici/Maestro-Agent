import { useRef } from 'react';
import * as Tooltip from '@radix-ui/react-tooltip';
import { formatContextPercent, getRenderableContextUsage } from '../lib/contextUsage';

const compact = (value: number) => value >= 1000000 ? `${(value / 1000000).toFixed(1)}M` : value >= 1000 ? `${(value / 1000).toFixed(0)}K` : String(value);

/// Devin-style context usage ring: percentage in the center, detailed usage in
/// a tooltip; yellow at 80%, red at 90%.
export default function ContextRing({ used, context, size = 30 }: { used: number | null; context: number | null; size?: number }) {
  const renderable = getRenderableContextUsage(used, context);
  // A streaming turn reports usage only at the end, so a single frame without a
  // value must not unmount the ring. Keep the last known pair and mark it stale.
  const lastKnown = useRef<{ used: number; context: number } | null>(null);
  if (renderable) lastKnown.current = renderable;
  const shown = renderable ?? lastKnown.current;
  if (!shown) return null;
  const stale = !renderable;
  const ratio = Math.min(1, shown.used / shown.context);
  const percent = formatContextPercent(ratio);
  const tone = ratio >= 0.9 ? 'danger' : ratio >= 0.8 ? 'warn' : 'ok';
  const radius = 10;
  const circumference = 2 * Math.PI * radius;
  return <Tooltip.Root>
    <Tooltip.Trigger asChild>
      <span className={`context-ring ${tone}${stale ? ' is-stale' : ''}`} style={{ width: size, height: size }} role="img"
        data-stale={stale ? 'true' : undefined} aria-label={`上下文已用 ${percent}`}>
        <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
          <circle className="context-ring-track" cx="12" cy="12" r={radius} />
          <circle className="context-ring-value" cx="12" cy="12" r={radius}
            strokeDasharray={`${ratio * circumference} ${circumference}`} />
        </svg>
        <em>{percent}</em>
      </span>
    </Tooltip.Trigger>
    <Tooltip.Portal><Tooltip.Content className="tooltip-content" sideOffset={8}>
      {`${compact(shown.used)} / ${compact(shown.context)} 上下文已用${stale ? "（更新中）" : ""}`}
    </Tooltip.Content></Tooltip.Portal>
  </Tooltip.Root>;
}

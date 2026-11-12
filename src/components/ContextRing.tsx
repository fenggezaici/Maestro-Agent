import * as Tooltip from '@radix-ui/react-tooltip';

const compact = (value: number) => value >= 1000000 ? `${(value / 1000000).toFixed(1)}M` : value >= 1000 ? `${(value / 1000).toFixed(0)}K` : String(value);

/// Devin-style context usage ring: percentage in the center, detailed usage in
/// a tooltip; yellow at 80%, red at 90%.
export default function ContextRing({ used, context, size = 26 }: { used: number; context: number | null; size?: number }) {
  const ratio = context && context > 0 ? Math.min(1, used / context) : null;
  const percent = ratio === null ? null : Math.round(ratio * 100);
  const tone = ratio === null ? 'unknown' : ratio >= 0.9 ? 'danger' : ratio >= 0.8 ? 'warn' : 'ok';
  const radius = 10;
  const circumference = 2 * Math.PI * radius;
  return <Tooltip.Root>
    <Tooltip.Trigger asChild>
      <span className={`context-ring ${tone}`} style={{ width: size, height: size }} role="img"
        aria-label={percent === null ? 'Context usage unavailable' : `Context ${percent}% used`}>
        <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
          <circle className="context-ring-track" cx="12" cy="12" r={radius} />
          <circle className="context-ring-value" cx="12" cy="12" r={radius}
            strokeDasharray={`${(ratio ?? 0) * circumference} ${circumference}`} />
        </svg>
        {percent !== null && <em>{percent}%</em>}
      </span>
    </Tooltip.Trigger>
    <Tooltip.Portal><Tooltip.Content className="tooltip-content" sideOffset={8}>
      {context ? `${compact(used)} / ${compact(context)} context used` : `${compact(used)} tokens used`}
    </Tooltip.Content></Tooltip.Portal>
  </Tooltip.Root>;
}

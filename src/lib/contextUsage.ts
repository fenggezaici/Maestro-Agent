/// ZCode `getRenderableTaskUsage`: only render context usage when both the
/// provider-reported prompt and the window are finite and positive. Anything
/// else is hidden rather than shown as 0% or “unavailable”.
export function getRenderableContextUsage(
  used: number | null | undefined,
  context: number | null | undefined,
): { used: number; context: number } | null {
  if (used === null || used === undefined || context === null || context === undefined) return null;
  if (!Number.isFinite(used) || !Number.isFinite(context) || used <= 0 || context <= 0) return null;
  return { used, context };
}

/// ZCode hides low cache-hit rates in production; the detail popover keeps the
/// exact value.
export const CACHE_HIT_RATE_DISPLAY_THRESHOLD = 78;

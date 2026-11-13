/// Shared Monaco loader: one dynamic import for every preview and a cheap
/// preload hook used on idle / panel hover.
let pending: Promise<typeof import('./monaco')> | null = null;

export function loadMonaco() {
  return pending ??= import('./monaco');
}

export function preloadMonaco() {
  void loadMonaco().catch(() => undefined);
}

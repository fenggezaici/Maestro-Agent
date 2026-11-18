/// Shared code-viewer loader: one dynamic import for every preview and a cheap
/// preload hook used on idle / panel hover.
let pending: Promise<typeof import('./viewer')> | null = null;

export function loadViewer() {
  return (pending ??= import('./viewer'));
}

export function preloadViewer() {
  void loadViewer().catch(() => undefined);
}

/// Final Maestro-Agent mark: a two-tone M on a rounded dark tile.
export default function MaestroLogo({ size = 24, className }: { size?: number; className?: string }) {
  return <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width={size} height={size}
    className={className} aria-hidden="true" focusable="false" data-maestro-logo="true">
    <rect width="24" height="24" rx="6" fill="#18181B" />
    <path d="M5 16V8L12 14" fill="none" stroke="#E4E4E7" strokeWidth="3" strokeLinecap="square" strokeLinejoin="miter" />
    <path d="M19 16V8L12 14" fill="none" stroke="#8B5CF6" strokeWidth="3" strokeLinecap="square" strokeLinejoin="miter" />
  </svg>;
}

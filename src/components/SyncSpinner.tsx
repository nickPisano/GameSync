interface Props {
  size?: number;
}

/** A circular two-arrow "sync" glyph that spins (matches the app icon motif). */
export function SyncSpinner({ size = 15 }: Props) {
  return (
    <svg
      className="sync-spinner"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12a9 9 0 0 1-9 9 9 9 0 0 1-6.7-3" />
      <path d="M3 12a9 9 0 0 1 9-9 9 9 0 0 1 6.7 3" />
      <path d="M21 4v5h-5" />
      <path d="M3 20v-5h5" />
    </svg>
  );
}

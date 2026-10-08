export function Ratio(props: { value: number; total: number; label: string; meter?: boolean }) {
  const value = () => Math.min(props.total, Math.max(0, props.value));
  const percentage = () => (props.total > 0 ? (value() / props.total) * 100 : 0);
  return (
    <span
      class="d-inline-flex flex-items-center text-mono"
      style={{
        gap: 'var(--base-size-8)',
        'white-space': 'nowrap',
        'font-variant-numeric': 'tabular-nums',
      }}
    >
      <span
        role={props.meter ? 'meter' : 'progressbar'}
        aria-label={props.label}
        aria-valuemin={0}
        aria-valuemax={props.total}
        aria-valuenow={value()}
        aria-valuetext={`${props.value} / ${props.total}`}
      >
        <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" class="d-block">
          <circle
            cx="8"
            cy="8"
            r="6"
            fill="none"
            stroke="var(--borderColor-default)"
            stroke-width="2"
          />
          <circle
            cx="8"
            cy="8"
            r="6"
            fill="none"
            stroke="var(--fgColor-accent)"
            stroke-width="2"
            pathLength="100"
            stroke-dasharray={`${percentage()} 100`}
            transform="rotate(-90 8 8)"
          />
        </svg>
      </span>
      <span>
        {props.value} / {props.total}
      </span>
    </span>
  );
}

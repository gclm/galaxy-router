export const C_BLUE = 'var(--color-chart-1)'
export const C_GREEN = 'var(--color-chart-2)'
export const C_AMBER = 'var(--color-chart-3)'
export const C_VIOLET = 'var(--color-chart-4)'
export const C_ROSE = 'var(--color-chart-5)'

export const PIE_COLORS = [C_BLUE, C_GREEN, C_AMBER, C_VIOLET, C_ROSE]

export const tooltipStyle: React.CSSProperties = {
  backgroundColor: 'var(--color-popover)',
  border: '1px solid var(--color-border)',
  borderRadius: '0.75rem',
  color: 'var(--color-popover-foreground)',
  fontSize: 12,
  boxShadow: '0 4px 16px rgba(0,0,0,0.12)',
  padding: '8px 12px',
}

export const tickStyle = { fill: 'var(--color-muted-foreground)', fontSize: 11 }
export const legendStyle = { fontSize: 11, color: 'var(--color-foreground)' }

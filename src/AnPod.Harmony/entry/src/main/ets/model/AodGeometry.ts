export class AodGap {
  private seen: boolean = false;
  private since: number = -1;

  expired(connected: boolean, now: number): boolean {
    if (connected) {
      this.seen = true;
      this.since = -1;
    } else if (this.seen && this.since < 0) {
      this.since = now;
    }
    return this.since >= 0 && now - this.since > 15000;
  }
}

export function curve(history: number[], amplitude: number, ms: number, right: boolean): number[] {
  const center = history.length - 1;
  const values: number[] = [];
  if (center < 1) return values;
  for (let i = -center; i <= center; i++) {
    const age = Math.abs(i);
    const level = Math.log1p(history[age] * 10000) / Math.log1p(10000);
    const taper = (1 + Math.cos(Math.PI * age / center)) / 2;
    const phase = (Math.sqrt(age * age + 16) - 4) * 0.34 - ms * 0.015;
    const value = level * taper * taper * Math.cos(phase) * amplitude;
    values.push(right ? -value : value);
  }
  return values;
}

export function wrapMetrics(items: string[], width: number, measure: (s: string) => number): string[] {
  const rows: string[] = [];
  let row = '';
  for (const item of items) {
    const joined = row.length ? row + ' | ' + item : item;
    if (row.length && measure(joined) > width) {
      rows.push(row);
      row = item;
    } else {
      row = joined;
    }
  }
  if (row.length) rows.push(row);
  return rows;
}

export type CodexRequestHeatLevel = 'cool' | 'active' | 'warm' | 'hot';

/** Completed request timestamps only. Recent and closely spaced requests glow brighter. */
export function computeCodexRequestHeat(
  events: ReadonlyArray<{ timestamp: number }>,
  now: number = Date.now(),
): { strength: number; level: CodexRequestHeatLevel } {
  const windowMs = 5 * 60 * 1000;
  const recent = events
    .filter((event) => Number.isFinite(event.timestamp) && now - event.timestamp < windowMs)
    .slice()
    .sort((a, b) => b.timestamp - a.timestamp);
  const score = recent.reduce((total, event, index) => {
    const age = Math.max(0, now - event.timestamp);
    const recency = Math.max(0, 1 - age / windowMs);
    const gapMinutes = index === 0 ? 10 : Math.max(0, (recent[index - 1].timestamp - event.timestamp) / 60000);
    const cadence = index === 0 ? .12 : Math.max(0, 1 - gapMinutes / 10) * .28;
    return total + recency * (1 + cadence);
  }, 0);
  const strength = Math.min(1, score / 2.4);
  const level: CodexRequestHeatLevel = strength >= .7 ? 'hot'
    : strength >= .35 ? 'warm'
      : strength >= .02 ? 'active' : 'cool';
  return { strength, level };
}

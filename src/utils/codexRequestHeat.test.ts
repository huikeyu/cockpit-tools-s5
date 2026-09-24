import assert from 'node:assert/strict';
import { test } from 'node:test';
import { computeCodexRequestHeat } from './codexRequestHeat';

test('a single completed request fades gradually to the original color', () => {
  const events = [{ timestamp: 1_000_000 }];
  const immediate = computeCodexRequestHeat(events, 1_000_000);
  const twoMinutes = computeCodexRequestHeat(events, 1_120_000);
  const fourMinutes = computeCodexRequestHeat(events, 1_240_000);
  const expired = computeCodexRequestHeat(events, 1_300_001);
  assert.ok(immediate.strength > twoMinutes.strength);
  assert.ok(twoMinutes.strength > fourMinutes.strength);
  assert.ok(fourMinutes.strength > expired.strength);
  assert.equal(expired.strength, 0);
  assert.equal(expired.level, 'cool');
});

test('frequent requests glow brighter than one request of the same age', () => {
  const now = 2_000_000;
  const one = computeCodexRequestHeat([{ timestamp: now - 10_000 }], now);
  const busy = computeCodexRequestHeat([
    { timestamp: now - 10_000 }, { timestamp: now - 20_000 },
    { timestamp: now - 30_000 }, { timestamp: now - 40_000 },
  ], now);
  assert.ok(busy.strength > one.strength);
  assert.equal(busy.level, 'hot');
});

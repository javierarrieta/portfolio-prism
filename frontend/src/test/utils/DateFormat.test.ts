import { readFileSync, readdirSync, statSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { describe, it, expect, afterEach } from 'vitest';
import { formatDate, formatDateTime, isoDate } from '../../utils/dates';

/**
 * Dates in this app are ISO 8601 (`YYYY-MM-DD`), built from UTC parts. Two things went wrong
 * before that was a rule, and only one of them was cosmetic.
 *
 * The cosmetic one: `toLocaleDateString()` follows the browser, so a printed tax statement
 * read `11/09/2026` in one country and `09/11/2026` in another. Hardcoding `'en-US'`, which is
 * what the statement did, does not fix that — it just picks the winner.
 *
 * The real one is below, and the test suite was structurally incapable of seeing it:
 * `setup.ts` pins `TZ=UTC`, so every test ever run has executed in the single timezone where a
 * local-zone date shift cannot happen.
 */
const SRC = resolve(__dirname, '../..');

function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return tsxFiles(p);
    return p.endsWith('.tsx') || p.endsWith('.ts') ? [p] : [];
  });
}

const HOST_TZ = process.env.TZ;
const WEST_OF_UTC = 'America/Los_Angeles';

describe('the date helpers', () => {
  afterEach(() => {
    process.env.TZ = HOST_TZ ?? 'UTC';
  });

  it('formats a date-only value as ISO regardless of where the browser is', () => {
    process.env.TZ = WEST_OF_UTC;

    // The bug this exists to kill. The backend sends a `DATE` column as
    // `2026-10-01T00:00:00Z`; formatting it in the browser's zone puts it on the 30th, so a
    // user in Los Angeles sees their own transaction on the day before it happened. Asserted
    // through `getDate()` rather than a formatted string, because the string's shape depends
    // on the runtime's locale and a test that only passes on en-US is its own bug.
    const shifted = new Date('2026-10-01T00:00:00Z');
    expect(shifted.getDate()).toBe(30);
    expect(shifted.getUTCDate()).toBe(1);
    expect(formatDate('2026-10-01T00:00:00Z')).toBe('2026-10-01');
    expect(formatDate('2026-10-01')).toBe('2026-10-01');
    expect(formatDate(new Date('2026-10-01T23:30:00Z'))).toBe('2026-10-01');
  });

  it('agrees with the server about what day it is', () => {
    process.env.TZ = WEST_OF_UTC;
    // The report window's `to` is `Utc::now().date_naive()` server-side; the screen must not
    // print a date the server would answer 422 for.
    const now = new Date('2026-11-09T02:00:00Z');
    expect(formatDate(now)).toBe(isoDate(now));
    expect(isoDate(now)).toBe('2026-11-09');
  });

  it('prints a timestamp without the wire noise', () => {
    expect(formatDateTime('2026-11-09T10:30:00Z')).toBe('2026-11-09 10:30');
    expect(formatDateTime('2026-11-09T10:30:00Z')).not.toContain('T');
  });

  it('says so when there is no date, rather than printing NaN', () => {
    expect(formatDate(null)).toBe('—');
    expect(formatDate(undefined)).toBe('—');
    expect(formatDate('not a date')).toBe('—');
    expect(formatDateTime('')).toBe('—');
  });
});

describe('nothing formats a date through the browser locale', () => {
  /**
   * A scan, not a review note about one screen — the same shape as `FormControlStyles.test.ts`.
   * Three patterns, because the defect hides in three ways: the obvious call, the `new Date()`
   * one-liner, and a variable whose name ends in `Date`/`At` (which is how `fetchedAt` escaped
   * the first two).
   */
  const BANNED: { name: string; re: RegExp }[] = [
    { name: 'toLocaleDateString / toDateString', re: /\.to(Locale)?DateString\(/ },
    { name: 'new Date(...).toLocaleString', re: /new Date\([^)]*\)\.toLocaleString\(/ },
    { name: '<somethingDate|At>.toLocaleString', re: /[\w\]](?:[dD]ate|[aA]t)\.toLocaleString\(/ },
  ];

  const hits = tsxFiles(SRC)
    .filter((f) => !f.includes('/test/') && !f.endsWith('dates.ts'))
    .flatMap((f) => {
      const src = readFileSync(f, 'utf8');
      return src
        .split('\n')
        .map((line, i) => ({ line: i + 1, text: line }))
        .filter((l) => !l.text.trimStart().startsWith('*') && !l.text.trimStart().startsWith('//'))
        .flatMap((l) =>
          BANNED.filter((b) => b.re.test(l.text)).map((b) => `${b.name}\n  ${f.replace(SRC + '/', '')}:${l.line}  ${l.text.trim()}`),
        );
    });

  it('finds the calls it claims to find', () => {
    // The scan is not vacuous: it catches the exact call the statement used until this existed.
    expect(BANNED[0].re.test("x.toLocaleDateString('en-US', DATE_OPTS)")).toBe(true);
    expect(BANNED[1].re.test('new Date(tx.date).toLocaleString()')).toBe(true);
    expect(BANNED[2].re.test('{fetchedAt.toLocaleString(\'en-US\', DATETIME_OPTS)}')).toBe(true);
  });

  it('leaves no locale-formatted date in the app', () => {
    expect(
      hits,
      `These format a date through the browser's locale, so the output depends on where the\n` +
        `reader is — and a date-only value shifts a day for anyone west of UTC. Use\n` +
        `formatDate / formatDateTime from src/utils/dates.ts instead.\n\n${hits.join('\n')}`,
    ).toEqual([]);
  });
});

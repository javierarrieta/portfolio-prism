import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, it, expect } from 'vitest';

const css = readFileSync(resolve(__dirname, '../../print.css'), 'utf8');

describe('print.css', () => {
  it('scopes both hide rules with :has() so the other tabs keep their layout (not their palette or paper size)', () => {
    expect(css).toMatch(/\.app-container:has\(\[data-print="statement"\]\)\s*>\s*\*:not\(\.main-content\)/);
    expect(css).toMatch(/\.main-content:has\(\[data-print="statement"\]\)\s*>\s*\*:not\(\[data-print="statement"\]\)/);
  });

  it('sets the palette the smoke compares against, literally', () => {
    for (const decl of [
      '--text-primary: #111111', '--text-secondary: #333333', '--text-muted: #555555',
      '--bg-dark: #ffffff', '--bg-card: #ffffff', '--border-color: #333333',
    ]) {
      expect(css).toContain(decl);
    }
  });

  it('overrides the table body border, which is a literal in index.css and not a variable', () => {
    expect(css).toMatch(/\.statement-table\s+td\s*\{[^}]*border-bottom:\s*1px solid #333333/);
  });

  it('forces the sidebar to a light background, which the :root overrides cannot reach', () => {
    // .sidebar's own background is a literal rgba(10, 15, 28, 0.95) in index.css, so on a
    // tab where the hide rules do not match it would print dark-on-dark.
    expect(css).toMatch(/\.sidebar\s*\{[^}]*background:\s*#ffffff/);
  });

  it('shrinks the nine-column table to fit the printed page box', () => {
    // Without this the three rightmost columns are clipped silently by body's
    // overflow-x:hidden. The rule must cover th as well as td: th is the wider of the two
    // for min-content, and both tie .data-table's selector on specificity.
    expect(css).toMatch(/\.statement-table\s+th\s*,\s*\.statement-table\s+td\s*\{[^}]*padding:\s*6px 8px/);
    expect(css).toMatch(/\.statement-table\s+th\s*,\s*\.statement-table\s+td\s*\{[^}]*font-size:\s*0\.75rem/);
  });

  it('collapses the two-column grid and keeps the headline off a page break', () => {
    expect(css).toMatch(/grid-template-columns:\s*1fr/);
    expect(css).toMatch(/\.statement-headline\s*\{[^}]*break-inside:\s*avoid/);
    expect(css).toMatch(/tr\s*\{[^}]*break-inside:\s*avoid/);
  });

  it('declares A4 once, unscoped, because @page takes no selector', () => {
    const atPage = css.match(/@page\s*\{[^}]*\}/g) ?? [];
    expect(atPage).toHaveLength(1);
    expect(atPage[0]).toContain('size: A4');
    expect(atPage[0]).toContain('margin: 14mm');
  });
});

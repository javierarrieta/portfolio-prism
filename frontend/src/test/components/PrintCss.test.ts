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
    // Without this the three rightmost columns are clipped silently, at .main-content and
    // at the page box. The rule must cover th as well as td: th is the wider of the two for
    // min-content, and both tie .data-table's selector on specificity.
    expect(css).toMatch(/\.statement-table\s+th\s*,\s*\.statement-table\s+td\s*\{[^}]*padding:\s*4px 6px/);
    expect(css).toMatch(/\.statement-table\s+th\s*,\s*\.statement-table\s+td\s*\{[^}]*font-size:\s*0\.75rem/);
  });

  it('has no token-break rule anywhere, so an over-wide cell fails a check instead of shattering', () => {
    // print.css used to carry `overflow-wrap: anywhere` on td:nth-child(2) (Type) so the raw
    // MUTUAL_FUND enum could survive a narrow column. That rule is what let the table hold a box
    // it did not fit: the token broke in half and every check stayed green. The column prints a
    // display label now (PortfolioStatement.tsx), so nothing needs to break, and an overflow-wrap
    // anywhere on a numeric column would let a figure break mid-number -- worse than the overflow
    // it prevents. Pinned as an absence over the DECLARATIONS only -- the stylesheet's prose
    // names the retired rule on purpose, and a comment is not a declaration.
    const declarations = css.replace(/\/\*[\s\S]*?\*\//g, '');
    expect(declarations).not.toMatch(/overflow-wrap|word-break|hyphens/);
  });

  it('collapses .main-content print padding for the statement only, and keeps 24px at the bottom', () => {
    // The 80px this buys back is what the nine-column fit is measured against: the printed box
    // is the full 688px @page content box, not 608px. The :has() scope is load-bearing —
    // unscoped, every other tab loses its padding too — and specificity (0,2,0) beats
    // .main-content's (0,1,0) in index.css without !important.
    // This text assertion is the ONLY thing anywhere that sees the rule being deleted. That was
    // measured, not assumed: with the rule removed (box back to 608px) the smoke stayed green on
    // all nine assertions, so the behavioural cost at the shipped fixture is zero and the rule is
    // margin, not necessity. It used to look measurable as a Type column squeezed to 37px
    // with MUTUAL_FUND broken across lines, but that measurement belonged to the raw enum and to
    // the Type-column break rule this stylesheet no longer has.
    expect(css).toMatch(/\.main-content:has\(\[data-print="statement"\]\)\s*\{\s*padding:\s*0 0 24px/);
  });

  it('collapses the two-column grid and keeps the headline off a page break', () => {
    expect(css).toMatch(/grid-template-columns:\s*1fr/);
    expect(css).toMatch(/\.statement-headline\s*\{[^}]*break-inside:\s*avoid/);
    expect(css).toMatch(/tr\s*\{[^}]*break-inside:\s*avoid/);
  });

  it('declares exactly one @page block, carrying A4 and 14mm', () => {
    // Counts, so a second @page cannot be added unnoticed. It cannot see whether the block
    // is nested inside @media print — nothing in this file can; the smoke's on-screen checks
    // are what pin @media membership for the rules that would show on screen.
    const atPage = css.match(/@page\s*\{[^}]*\}/g) ?? [];
    expect(atPage).toHaveLength(1);
    expect(atPage[0]).toContain('size: A4');
    expect(atPage[0]).toContain('margin: 14mm');
  });
});

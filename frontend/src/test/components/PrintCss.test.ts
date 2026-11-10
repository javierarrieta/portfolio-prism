import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, it, expect } from 'vitest';

const css = readFileSync(resolve(__dirname, '../../print.css'), 'utf8');

describe('print.css', () => {
  it('scopes both hide rules with :has() so the other tabs print unchanged', () => {
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

  it('collapses the two-column grid and keeps the headline off a page break', () => {
    expect(css).toMatch(/grid-template-columns:\s*1fr/);
    expect(css).toMatch(/\.statement-headline\s*\{[^}]*break-inside:\s*avoid/);
    expect(css).toMatch(/tr\s*\{[^}]*break-inside:\s*avoid/);
  });

  it('declares A4 once, unscoped, because @page takes no selector', () => {
    expect(css).toMatch(/@page\s*\{\s*size:\s*A4;\s*margin:\s*14mm/);
  });
});

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, it, expect } from 'vitest';

/**
 * The stylesheet half of the collapsible rail.
 *
 * vitest does not load index.css into jsdom, so no component test can see any of this: the
 * DOM assertions in AppNavRail.test.tsx prove the accessible names and the tab order, and
 * these prove the five things only the CSS decides -- that the labels are clipped rather
 * than removed, that the collapsed track is 64px, that the content cap actually moves, that
 * nothing needed `!important` to get there, and that print.css still outranks the rail.
 *
 * Same idiom as PrintCss.test.ts: read the file, strip the comments, match the declarations.
 */

const indexCss = readFileSync(resolve(__dirname, '../../index.css'), 'utf8');
const printCss = readFileSync(resolve(__dirname, '../../print.css'), 'utf8');
const declarations = indexCss.replace(/\/\*[\s\S]*?\*\//g, '');

function ruleBody(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = declarations.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  if (!match) throw new Error(`no rule found for ${selector}`);
  return match[1];
}

/**
 * (a, b, c) packed into one number. `:has()` is counted as its argument rather than as a
 * pseudo-class of its own, which is what CSS really does -- and it is the whole reason the
 * print rule at (0,2,0) outranks the rail rule at (0,1,0).
 */
function specificity(selector: string): number {
  const flat = selector.replace(/:has\(([^)]*)\)/g, '$1');
  const ids = (flat.match(/#[\w-]+/g) ?? []).length;
  const classes = (flat.match(/\.[\w-]+/g) ?? []).length;
  const attrs = (flat.match(/\[[^\]]+\]/g) ?? []).length;
  const pseudoClasses = (flat.match(/:(?!:)[\w-]+/g) ?? []).length;
  const elements = (flat.match(/(?:^|[\s>+~])[a-z][\w-]*/gi) ?? []).length;
  return ids * 10000 + (classes + attrs + pseudoClasses) * 100 + elements;
}

const max_width_px = (body: string): number => {
  const m = /max-width:\s*(\d+)px/.exec(body);
  if (!m) throw new Error(`no px max-width in: ${body}`);
  return Number(m[1]);
};

describe('the collapsed rail in index.css', () => {
  it('takes 196px off the sidebar column, and leaves the expanded column alone', () => {
    expect(ruleBody('.app-container')).toMatch(/grid-template-columns:\s*260px 1fr/);
    expect(ruleBody('.app-container--nav-collapsed')).toMatch(/grid-template-columns:\s*64px 1fr/);
  });

  it('hides the collapsed labels by clipping them, never by taking them out of the tree', () => {
    // The `.bundle-file-input` idiom. `display: none` or `visibility: hidden` would drop
    // the words -- and with them the accessible name of every nav item -- out of the
    // accessibility tree, which is the one thing a collapsed rail must not do.
    const body = ruleBody('.app-container--nav-collapsed .nav-label');
    expect(body).toMatch(/position:\s*absolute/);
    expect(body).toMatch(/overflow:\s*hidden/);
    expect(body).toMatch(/clip-path:\s*inset\(50%\)/);
    expect(body).not.toMatch(/display:/);
    expect(body).not.toMatch(/visibility:/);
  });

  it('raises the content cap when the rail is collapsed, and keeps a cap', () => {
    // Without this the freed 196px only re-centres a pinned column. Without a cap at the
    // other end, the text-heavy tabs run a measure nobody can read down.
    const expanded = max_width_px(ruleBody('.main-content'));
    const collapsed = max_width_px(ruleBody('.app-container--nav-collapsed .main-content'));
    expect(expanded).toBe(1400);
    expect(collapsed).toBeGreaterThan(expanded);
    expect(collapsed).toBe(1600);
  });

  it('needs no !important anywhere in the rail rules', () => {
    const rail = declarations.slice(declarations.indexOf('.app-container--nav-collapsed {'));
    expect(rail).not.toMatch(/!important/);
    expect(rail).not.toContain('!important');
  });

  it('is outranked by print.css, so a collapsed rail cannot reach the printed statement', () => {
    // Two independent reasons the print output cannot change, both pinned:
    // 1. the grid. print.css collapses it with `.app-container:has([data-print=...])`,
    //    which outranks the single-class rail rule, so the page prints one column whether
    //    or not the rail was collapsed on screen.
    // 2. the sidebar itself. The hide rule takes every direct child of `.app-container`
    //    that is not `.main-content`, and the rail class sits on `.app-container` -- the
    //    element being selected, not a child of it -- so the rail is hidden whatever state
    //    it is in.
    const railGrid = specificity('.app-container--nav-collapsed');
    const printGrid = specificity('.app-container:has([data-print="statement"])');
    expect(railGrid).toBe(100);
    expect(printGrid).toBe(200);
    expect(printCss).toMatch(
      /\.app-container:has\(\[data-print="statement"\]\)\s*\{\s*grid-template-columns:\s*1fr/,
    );
    expect(printCss).toMatch(
      /\.app-container:has\(\[data-print="statement"\]\)\s*>\s*\*:not\(\.main-content\)\s*\{\s*display:\s*none/,
    );
  });

  it('states both split points for the Backup preview, expanded and collapsed', () => {
    // The 1198px figure is only true with a 260px sidebar. Once the rail can be 64px the
    // same arithmetic has two answers, and a comment carrying one of them is a comment
    // that is wrong half the time.
    expect(indexCss).toContain('856 + 342');
    expect(indexCss).toContain('856 + 146');
    expect(indexCss).toMatch(/1198px viewport with the nav rail expanded/);
    expect(indexCss).toMatch(/1002px viewport with it collapsed/);
  });
});

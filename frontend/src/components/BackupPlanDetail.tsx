/* Fast Refresh declines to hot-swap a module that exports non-components, so this file
 * would fall back to a full reload in dev. That is the whole cost, and the alternative
 * is worse: `countList`, `isCounts` and `isPlanSide` are this module's contract with
 * `BackupPanel` — the panel reads a plan body with them and spells the plan sentence with
 * the first — and splitting them into a second file would put one promise's two halves in
 * two places, which is how they start disagreeing. */
/* eslint-disable react-refresh/only-export-components */
import type { ReactNode } from 'react';

/** The three counts a plan side carries. Mirrors `backup::restore::Counts`. */
export type Counts = { portfolios: number; assets: number; transactions: number };

export type PlanAsset = {
  symbol: string;
  name: string;
  currency: string;
  transactions: number;
  first_date: string | null;
  last_date: string | null;
  zero_price: number;
  flags: string[];
};

export type PlanPortfolio = {
  name: string;
  currency: string;
  /** Exact, over every asset — not `shown.length`. */
  assets: number;
  transactions: number;
  shown: PlanAsset[];
  more_assets: number;
  /** Flag counts over every asset, so a collapsed header cannot hide one. */
  asset_flags: Record<string, number>;
  flags: string[];
};

export type PlanSide = {
  counts: Counts;
  shown: PlanPortfolio[];
  more_portfolios: number;
  transactions_in_shown: number;
};

/** Above this many assets a portfolio starts collapsed. A display choice, not a
 * contract: 25 portfolios x 50 assets is a wall, and a wall is not verification. */
const COLLAPSE_ABOVE = 5;

/** English for a flag code. An unknown code renders as itself, so a flag added
 * server-side degrades into something readable rather than a crash. */
const FLAG_LABELS: Record<string, string> = {
  no_assets: 'no assets',
  no_transactions: 'no transactions',
  zero_price: 'zero-price',
  future_dated: 'future-dated',
  currency_mismatch: 'currency mismatch',
  duplicate_symbol: 'duplicate symbol',
};

function flagLabel(code: string, count?: number): string {
  const label = FLAG_LABELS[code] ?? code;
  return count === undefined ? label : `${label} ×${count}`;
}

/** `{n} {noun}` with the plural spelled out. The counts are the server's; only the
 * English is here. Exported because `BackupPanel` spells the same sentence for the
 * restore outcome, and two spellings of one promise is how they drift. */
export function countList(counts: Counts): string {
  const named = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;
  return [
    named(counts.portfolios, 'portfolio'),
    named(counts.assets, 'asset'),
    named(counts.transactions, 'transaction'),
  ].join(', ');
}

export function isCounts(value: unknown): value is Counts {
  const c = value as Partial<Counts> | null;
  return !!c && typeof c === 'object'
    && Number.isFinite(c.portfolios) && Number.isFinite(c.assets) && Number.isFinite(c.transactions);
}

function isAsset(value: unknown): value is PlanAsset {
  const a = value as Partial<PlanAsset> | null;
  return !!a && typeof a === 'object' && typeof a.symbol === 'string'
    && typeof a.name === 'string' && typeof a.currency === 'string'
    && Number.isFinite(a.transactions) && Number.isFinite(a.zero_price)
    && (a.first_date === null || typeof a.first_date === 'string')
    && (a.last_date === null || typeof a.last_date === 'string')
    && Array.isArray(a.flags) && a.flags.every((f) => typeof f === 'string');
}

function isPortfolio(value: unknown): value is PlanPortfolio {
  const p = value as Partial<PlanPortfolio> | null;
  return !!p && typeof p === 'object' && typeof p.name === 'string' && typeof p.currency === 'string'
    && Number.isFinite(p.assets) && Number.isFinite(p.transactions) && Number.isFinite(p.more_assets)
    && Array.isArray(p.shown) && p.shown.every(isAsset)
    && Array.isArray(p.flags) && p.flags.every((f) => typeof f === 'string')
    && !!p.asset_flags && typeof p.asset_flags === 'object'
    && Object.values(p.asset_flags).every((n) => Number.isFinite(n));
}

/** A plan side off an untrusted body, or `null`. The reason it refuses rather than
 * casts is the reason `asPlan` exists in `BackupPanel`: a thrown render is a blank
 * page, and a blank page is the worst possible answer at the moment someone is
 * deciding whether to delete their data. */
export function isPlanSide(value: unknown): value is PlanSide {
  const s = value as Partial<PlanSide> | null;
  return !!s && typeof s === 'object' && isCounts(s.counts)
    && Array.isArray(s.shown) && s.shown.every(isPortfolio)
    && Number.isFinite(s.more_portfolios) && Number.isFinite(s.transactions_in_shown);
}

function FlagList({ codes, counts }: { codes: string[]; counts?: Record<string, number> }) {
  if (codes.length === 0) return null;
  return (
    <span data-testid="plan-flags" className="text-muted">
      {codes.map((c) => flagLabel(c, counts?.[c])).join(' · ')}
    </span>
  );
}

function AssetRow({ asset }: { asset: PlanAsset }) {
  const span = asset.first_date && asset.last_date
    ? `${asset.first_date} – ${asset.last_date}`
    : 'no transactions';
  return (
    <div data-testid="plan-asset" style={{ paddingLeft: '12px' }}>
      <strong>{asset.symbol}</strong>
      {` ${asset.name} · ${asset.currency} · ${asset.transactions} txn`}
      {` · ${span}`}
      {' '}
      <FlagList codes={asset.flags} counts={{ zero_price: asset.zero_price }} />
    </div>
  );
}

function PortfolioBlock({ portfolio }: { portfolio: PlanPortfolio }) {
  const one = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;
  // The header is built from `asset_flags`, which the server counts over *every*
  // asset, and never from `portfolio.shown`. A header aggregated over the listed
  // assets would under-report a flag sitting past `MAX_ASSETS_SHOWN`, and the
  // collapsed case is exactly where that under-report would be invisible.
  const header = (
    <span data-testid="plan-portfolio-header">
      <strong>{portfolio.name}</strong>
      {` · ${portfolio.currency} · ${one(portfolio.assets, 'asset')} · ${one(portfolio.transactions, 'transaction')}`}
      {' '}
      <FlagList codes={portfolio.flags} />
      <FlagList
        codes={Object.keys(portfolio.asset_flags)}
        counts={portfolio.asset_flags}
      />
    </span>
  );

  const rows: ReactNode[] = portfolio.shown.map((a, i) => <AssetRow key={`${a.symbol}-${i}`} asset={a} />);
  const more = portfolio.more_assets > 0
    ? <div className="text-muted">{`and ${portfolio.more_assets} more asset${portfolio.more_assets === 1 ? '' : 's'}`}</div>
    : null;

  if (portfolio.shown.length <= COLLAPSE_ABOVE) {
    return (
      <div data-testid="plan-portfolio" style={{ marginBottom: '8px' }}>
        <div>{header}</div>
        {rows}
        {more}
      </div>
    );
  }

  return (
    <details data-testid="plan-portfolio" style={{ marginBottom: '8px' }}>
      <summary>{header}</summary>
      {rows}
      {more}
    </details>
  );
}

/** One side of a plan — what a bundle holds, or what the database holds. Rendered
 * three times by `BackupPanel`, which is the whole reason it is a component. */
export function PlanSideView({ title, side }: { title: string; side: PlanSide }) {
  return (
    <div data-testid="plan-side">
      <p style={{ marginBottom: '4px' }}>{`${title} ${countList(side.counts)}.`}</p>
      {side.more_portfolios > 0 && (
        <p className="text-muted" data-testid="plan-cap">
          {`showing ${side.shown.length} of ${side.counts.portfolios} portfolios · `
            + `the portfolios shown hold ${side.transactions_in_shown} of ${side.counts.transactions} transactions`}
        </p>
      )}
      {side.shown.map((p, i) => <PortfolioBlock key={`${p.name}-${i}`} portfolio={p} />)}
    </div>
  );
}

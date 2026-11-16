/* `countList`, `isCounts` and `isPlanSide` are this module's contract with `BackupPanel`
 * — the panel reads a plan body with them and spells the plan sentence with the first —
 * and splitting them into a second file would put one promise's two halves in two places,
 * which is how they start disagreeing. The cost is that Fast Refresh declines to hot-swap
 * a module that exports non-components, so this file falls back to a full reload in dev.
 * `eslint.config.js` names those three exports to `react-refresh/only-export-components`
 * rather than this file switching the rule off, so the rule still bites on anything else
 * exported from here. */
import { useState } from 'react';

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

/** Above this many assets a portfolio starts collapsed, and the rows under it wait behind
 * a "show all N" control rather than being truncated away. A display choice, not a
 * contract: 25 portfolios x 50 assets is a wall, and a wall is not verification.
 *
 * This is *not* the server's cap. `more_assets` / `more_portfolios` are rows that never
 * reached this browser at all; this threshold only holds back rows already in hand, and
 * expanding shows exactly the ones the response carried. */
const COLLAPSE_ABOVE = 5;

/** Column counts, so a spanning note row cannot silently disagree with the header it
 * sits under. */
const PORTFOLIO_COLUMNS = 5;
const ASSET_COLUMNS = 7;

/** What a date cell says when there is no date to show — an asset with no transactions
 * has no first and no last. A dash rather than a word, because a word would read as a
 * value of a different kind. */
const NO_DATE = '—';

/** `{n} {noun}` with the English plural spelled out. `plural` covers the ones whose plural
 * is not `noun` + `s` — `txn`/`txns`. The counts are the server's; only the English is here. */
function one(n: number, noun: string, plural = `${noun}s`): string {
  return `${n} ${n === 1 ? noun : plural}`;
}

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

function flagLabel(code: string, count?: number, labels?: Record<string, string>): string {
  const label = labels?.[code] ?? FLAG_LABELS[code] ?? code;
  return count === undefined ? label : `${label} ×${count}`;
}

/** Copy that overrides `FLAG_LABELS` for one place only, because one code means two
 * different units depending on who counts it. `zero_price` on a `PlanAsset` is a count
 * of that asset's zero-price TRANSACTIONS; in a portfolio's `asset_flags` it is a count
 * of flagged ASSETS. The header disambiguates with its "across all assets" prefix, so
 * only the row needs its unit spelled out. */
const ASSET_ROW_LABELS: Record<string, string> = { zero_price: 'zero-price rows' };

/** The plan sentence. Exported because `BackupPanel` spells the same sentence for the
 * restore outcome, and two spellings of one promise is how they drift. */
export function countList(counts: Counts): string {
  return [
    one(counts.portfolios, 'portfolio'),
    one(counts.assets, 'asset'),
    one(counts.transactions, 'transaction'),
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

/** The flags of one row, each a badge. `prefix` labels the scope of the counts,
 * `labels` renames a code for this caller alone.
 *
 * The prefix is its own element rather than a run of prose so the flags can be badges and
 * still read as one sentence: the wrapper's `textContent` is `across all assets:
 * zero-price ×3`, which is the only thing keeping the two scopes distinguishable. */
function FlagList({ codes, counts, prefix, labels }: {
  codes: string[];
  counts?: Record<string, number>;
  prefix?: string;
  labels?: Record<string, string>;
}) {
  if (codes.length === 0) return null;
  return (
    <span data-testid="plan-flags" className="plan-flags">
      {prefix && <span className="plan-flag-scope text-muted">{prefix}</span>}
      {codes.map((c) => (
        <span key={c} className="badge plan-flag">{flagLabel(c, counts?.[c], labels)}</span>
      ))}
    </span>
  );
}

function AssetRow({ asset }: { asset: PlanAsset }) {
  return (
    <tr data-testid="plan-asset">
      <th scope="row">{asset.symbol}</th>
      <td>{asset.name}</td>
      <td>{asset.currency}</td>
      <td className="plan-cell-num">{one(asset.transactions, 'txn', 'txns')}</td>
      <td className="plan-cell-date">{asset.first_date ?? NO_DATE}</td>
      <td className="plan-cell-date">{asset.last_date ?? NO_DATE}</td>
      <td className="plan-cell-flags">
        <FlagList codes={asset.flags} counts={{ zero_price: asset.zero_price }} labels={ASSET_ROW_LABELS} />
      </td>
    </tr>
  );
}

/** The assets of one portfolio, as a table of their own inside the portfolio's row.
 *
 * They are a second table rather than more columns on the portfolio row because the two
 * count different things — a portfolio row is one portfolio, an asset row is one asset —
 * and forcing seven asset columns onto five portfolio columns would leave half the cells
 * in each row meaning nothing. */
function AssetTable({ title, portfolio }: { title: string; portfolio: PlanPortfolio }) {
  // `expanded` only ever reveals rows the response already delivered. It cannot invent
  // rows, which is the difference between this control and `more_assets` below.
  const [expanded, setExpanded] = useState(false);
  const collapsible = portfolio.shown.length > COLLAPSE_ABOVE;
  const collapsed = collapsible && !expanded;
  const visible = collapsed ? portfolio.shown.slice(0, COLLAPSE_ABOVE) : portfolio.shown;

  return (
    <table className="plan-table plan-table--assets" aria-label={`${title} — assets in ${portfolio.name}`}>
      <thead data-testid="plan-asset-head">
        <tr>
          <th scope="col">Symbol</th>
          <th scope="col">Name</th>
          <th scope="col">Currency</th>
          <th scope="col">Txns</th>
          <th scope="col">First</th>
          <th scope="col">Last</th>
          <th scope="col">Flags</th>
        </tr>
      </thead>
      <tbody>
        {/* A portfolio with no assets still gets a row, because an empty body reads as a
            * table that failed to fill in rather than as an answer. The wording is
            * deliberately not the `no_assets` badge's words, which are a warning on a
            * portfolio that is listed above this table, not a count of what is in it. */}
        {visible.length === 0 && (
          <tr className="plan-note-row">
            <td colSpan={ASSET_COLUMNS} className="plan-cell-muted" data-testid="plan-no-assets">
              No assets in this portfolio.
            </td>
          </tr>
        )}
        {visible.map((a, i) => <AssetRow key={`${a.symbol}-${i}`} asset={a} />)}
        {collapsible && (
          <tr className="plan-note-row">
            <td colSpan={ASSET_COLUMNS}>
              <button
                type="button"
                className="plan-toggle"
                data-testid="plan-show-all"
                aria-expanded={!collapsed}
                onClick={() => setExpanded((was) => !was)}
              >
                {collapsed ? `show all ${portfolio.shown.length} assets` : 'show fewer'}
              </button>
            </td>
          </tr>
        )}
        {/* The server's cap, and a different fact from the control above: these rows were
            * never sent, so nothing done on this page will produce them. */}
        {portfolio.more_assets > 0 && (
          <tr className="plan-note-row">
            <td colSpan={ASSET_COLUMNS} className="plan-cell-muted">
              {`and ${portfolio.more_assets} more asset${portfolio.more_assets === 1 ? '' : 's'}`}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

/** One portfolio: its own row, and the asset table under it, in one row group so the
 * two travel together and stay visually bound. */
function PortfolioGroup({ title, portfolio }: { title: string; portfolio: PlanPortfolio }) {
  return (
    <tbody data-testid="plan-portfolio">
      {/* The portfolio row is built from `asset_flags`, which the server counts over
        * *every* asset, and never from `portfolio.shown`. A row aggregated over the
        * listed assets would under-report a flag sitting past `MAX_ASSETS_SHOWN`, and
        * the collapsed case is exactly where that under-report would be invisible — so
        * this row is never the thing that collapses. */}
      <tr data-testid="plan-portfolio-header" className="plan-portfolio-row">
        <th scope="row">{portfolio.name}</th>
        <td>{portfolio.currency}</td>
        <td className="plan-cell-num">{one(portfolio.assets, 'asset')}</td>
        <td className="plan-cell-num">{one(portfolio.transactions, 'transaction')}</td>
        <td className="plan-cell-flags">
          <FlagList codes={portfolio.flags} />
          {/* `asset_flags` counts every asset the portfolio holds; the rows under this
            * one count a single asset each. Naming the scope is the only thing that stops
            * the two reading as the same number twice. */}
          <FlagList
            codes={Object.keys(portfolio.asset_flags)}
            counts={portfolio.asset_flags}
            prefix="across all assets: "
          />
        </td>
      </tr>
      <tr className="plan-assets-row">
        <td colSpan={PORTFOLIO_COLUMNS}>
          <AssetTable title={title} portfolio={portfolio} />
        </td>
      </tr>
    </tbody>
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
      {/* No table at all when there are no portfolios to list: a header over an empty body
          * is the shape this component refuses one row down, at the portfolio. */}
      {side.shown.length > 0 && (
        <div className="plan-table-scroll">
          <table className="plan-table" data-testid="plan-portfolio-table">
            <caption className="plan-sr-only">{`${title} — portfolios`}</caption>
            <thead data-testid="plan-portfolio-head">
              <tr>
                <th scope="col">Portfolio</th>
                <th scope="col">Currency</th>
                <th scope="col">Assets</th>
                <th scope="col">Transactions</th>
                <th scope="col">Flags</th>
              </tr>
            </thead>
            {side.shown.map((p, i) => <PortfolioGroup key={`${p.name}-${i}`} title={title} portfolio={p} />)}
          </table>
        </div>
      )}
    </div>
  );
}
